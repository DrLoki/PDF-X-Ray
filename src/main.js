import * as pdfjsLib from './assets/pdfjs/pdf.min.mjs';
import { extractPage } from './extract.js';
import { serializeLayoutTree, serializeUnits, serializeParagraphsText, parseTranslations } from './export.js';
import { pseudoTranslate, renderTranslationLayer } from './translate.js';
import { ImageDocument, zipSource, filesSource, looksLikeZip, isImageName } from './imagedoc.js';
import { rasterizePage, recognizeCanvas, formatBytes, ocrSummary, LOW_CONF, OCR_DPI } from './ocr.js';

pdfjsLib.GlobalWorkerOptions.workerSrc = './assets/pdfjs/pdf.worker.min.mjs';

// Resources pdf.js 6 loads on demand (copied by tools/vendor_pdfjs.mjs).
// Without `wasmUrl` CCITT fax / JBIG2 (black-and-white scans) and JPEG 2000
// images are not decoded and the page renders blank; `iccUrl` serves the
// CMYK profile. Absolute URLs: the worker resolves them on its own base.
const pdfjsAsset = (dir) => new URL(`./assets/pdfjs/${dir}/`, location.href).href;
const PDFJS_RESOURCES = {
  wasmUrl: pdfjsAsset('wasm'),
  iccUrl: pdfjsAsset('iccs'),
};

// Colours per block role (overlay, DOM tree, paragraph list).
const ROLE_COLORS = {
  body: '#34d399',
  heading: '#38bdf8',
  header: '#94a3b8',
  footer: '#94a3b8',
  'page-number': '#64748b',
  caption: '#fbbf24',
  footnote: '#c084fc',
  sidebar: '#f472b6',
  table: '#facc15',
  'figure-text': '#fb923c',
  decoration: '#78716c',
  // comic pages
  balloon: '#22d3ee',
  narration: '#f59e0b',
  'comic-text': '#a3e635',
  sfx: '#e879f9',
};
const roleColor = (r) => ROLE_COLORS[r] || '#34d399';

const state = {
  pdfDoc: null,
  loadingTask: null,         // pdf.js loading task: the handle used to release the document
  fileName: null,
  pageNum: 1,
  totalPageCount: 1,
  zoomScale: 1.0,
  pageRendering: false,
  pageNumPending: null,
  loadToken: 0,

  pageData: new Map(),       // pageNum → PageInput sent to Rust (PDF points)
  textLayer: new Map(),      // pageNum → PageInput as extracted from the PDF (text layer)
  pageKinds: new Map(),      // pageNum → { kind: 'text'|'scan', chars, imageCover, reason }
  ocrResults: new Map(),     // pageNum → { page, items, scale, dpi } from `ocr_page`
  pageModes: new Map(),      // pageNum → 'text'|'ocr' (overrides the document mode)
  ocrDeclined: false,        // the user refused the model download in this session
  ocrFailed: new Set(),      // pages whose OCR failed (not retried by the background queue)
  ocrWorker: null,           // load token of the running background OCR queue, or null
  ocrStats: { done: 0, t0: 0, lastAnalysis: 0 },
  analysisSeq: 0,            // only the latest analysis result is applied
  extractionDone: false,
  doc: null,                 // DocumentResult
  pageResults: new Map(),    // pageNum → PageResult
  paraById: new Map(),

  selectedBlockId: null,
  selectedParaId: null,
  hoveredBlockId: null,

  translations: new Map(),   // paragraph id → translated text
  translationSource: null,   // 'pseudo' | 'import' | 'google'
  mtCache: new Map(),        // `${lang}\u0000${paragraph text}` → machine translation
  autoTranslate: false,      // machine-translate every page shown (after "Traduci")
  translating: false,
  pieces: new Map(),         // paragraph id → [text per segment]

  thresholdY: 10,
  thresholdX: 15,
  minWidth: 20,
  minHeight: 0,
  priority: 'Y',
  pageRefMap: new Map(),
};

const el = {};
const isTauri = () => !!(window.__TAURI__ && window.__TAURI__.core);

window.addEventListener('DOMContentLoaded', () => {
  cacheElements();
  registerEvents();
  setupDragAndDrop();
  setupResizableSidebars();
  applyAutoGutterUIState();
  if (!isTauri()) {
    setStatus('Motore di analisi disponibile solo nell\'app desktop (npm run tauri dev).', 'warn');
  }
});

function applyAutoGutterUIState() {
  const autoOn = el.chkAutoGutter.checked;
  el.rangeY.disabled = autoOn;
  el.rangeX.disabled = autoOn;
  el.rangeMinW.disabled = autoOn;
  el.rangeMinH.disabled = autoOn;
  el.selectAutoStrategy.disabled = !autoOn;
  el.autoHint.hidden = !autoOn;
  syncAutoSliders();
}

const SLIDERS = () => [
  { key: 'thresholdY', range: el.rangeY, label: el.valY, diag: 'thresholdY' },
  { key: 'thresholdX', range: el.rangeX, label: el.valX, diag: 'thresholdX' },
  { key: 'minWidth', range: el.rangeMinW, label: el.valMinW, diag: 'minWidth' },
  { key: 'minHeight', range: el.rangeMinH, label: el.valMinH, diag: 'minHeight' },
];

const fmtPt = (v) => (Number.isInteger(v) ? v : v.toFixed(1)) + ' pt';
const round1 = (v) => Math.round(v * 10) / 10;

/** Diagnostics of the first automatic cut decision of the page (root level). */
function pageAutoDiag(pr) {
  if (!pr) return null;
  const queue = [pr.root];
  while (queue.length) {
    const n = queue.shift();
    if (n.diag && n.diag.source && n.diag.source.startsWith('auto')) return n.diag;
    queue.push(...(n.children || []));
  }
  return null;
}

/**
 * In automatic mode the (disabled) sliders show the thresholds the engine
 * really used on the current page. They also become the starting point when
 * switching back to manual.
 */
function syncAutoSliders() {
  if (!el.chkAutoGutter.checked) return;
  const autoResult = state.docOptions && !state.docOptions.manual;
  const d = autoResult ? pageAutoDiag(currentPageResult()) : null;
  if (!d) {
    el.autoHint.textContent = state.doc ? 'Soglie automatiche: in attesa della nuova analisi…' : 'Le soglie automatiche compariranno dopo l\'analisi.';
    return;
  }
  for (const s of SLIDERS()) {
    const v = round1(d[s.diag]);
    state[s.key] = v;
    s.range.value = Math.round(Math.min(Number(s.range.max), Math.max(Number(s.range.min), v)));
    s.label.textContent = fmtPt(v);
  }
  el.autoHint.textContent = `Soglie calcolate dal motore (${d.source.replace('auto:', '')}) per la pagina ${state.pageNum}, ` +
    `primo livello: corpo dominante ${round1(d.dominantFont)} pt, gap massimi X ${round1(d.largestXGap)} / Y ${round1(d.largestYGap)} pt. ` +
    `I blocchi interni ricalcolano le soglie sul proprio corpo: seleziona un blocco per vederle.`;
}

function cacheElements() {
  const ids = {
    btnOpen: 'btn-open-file', fileSelector: 'file-selector', btnOpenFolder: 'btn-open-folder', folderSelector: 'folder-selector', btnPrev: 'btn-prev-page', btnNext: 'btn-next-page',
    inputPageNum: 'input-page-num', txtPageTotal: 'txt-page-total', btnZoomIn: 'btn-zoom-in', btnZoomOut: 'btn-zoom-out',
    txtZoomPercent: 'txt-zoom-percent', chkShowChars: 'chk-show-chars', chkShowBlocks: 'chk-show-blocks',
    chkShowProjections: 'chk-show-projections', chkShowFlow: 'chk-show-flow', chkShowTranslation: 'chk-show-translation',
    btnPseudo: 'btn-pseudo', btnImportTr: 'btn-import-tr', btnTranslate: 'btn-translate', selectTrLang: 'select-tr-lang', chkXray: 'chk-xray', trFileSelector: 'tr-file-selector', analysisStatus: 'analysis-status',
    chkAutoGutter: 'chk-auto-gutter', selectAutoStrategy: 'select-auto-strategy', chkUseTags: 'chk-use-tags', chkStitch: 'chk-stitch',
    pdfCanvas: 'pdf-canvas', overlayCanvas: 'overlay-canvas', translationLayer: 'translation-layer',
    canvasContainer: 'canvas-container', spinner: 'loading-spinner',
    tabBookmarks: 'tab-bookmarks', tabTagged: 'tab-tagged', tabParagraphs: 'tab-paragraphs',
    panelBookmarks: 'panel-bookmarks', panelTagged: 'panel-tagged', panelParagraphs: 'panel-paragraphs',
    outlineTree: 'outline-tree', taggedTree: 'tagged-tree', taggedBadge: 'tagged-status-badge',
    paragraphList: 'paragraph-list', analysisStats: 'analysis-stats', chkAllParagraphs: 'chk-all-paragraphs',
    rangeY: 'range-threshold-y', valY: 'val-threshold-y', rangeX: 'range-threshold-x', valX: 'val-threshold-x',
    rangeMinW: 'range-min-w', valMinW: 'val-min-w', rangeMinH: 'range-min-h', valMinH: 'val-min-h',
    selectPriority: 'select-priority', btnReanalyze: 'btn-reanalyze',
    domTreeContainer: 'dom-tree-container', inspectorPanel: 'inspector-panel', btnExport: 'btn-export', exportMenu: 'export-menu',
    progressPanel: 'progress-panel', progressPhase: 'progress-phase', progressDetail: 'progress-detail',
    progressEta: 'progress-eta', progressBar: 'progress-bar',
    btnSnapshot: 'btn-snapshot', snapshotDialog: 'snapshot-dialog', snapPage: 'snap-page', snapSettings: 'snap-settings',
    snapNote: 'snap-note', snapPng: 'snap-png', autoHint: 'auto-hint',
    selectOcrDoc: 'select-ocr-doc', selectOcrPage: 'select-ocr-page', ocrPageKind: 'ocr-page-kind', ocrHint: 'ocr-hint', selectComicDir: 'select-comic-dir',
    ocrModelsDialog: 'ocr-models-dialog', ocrModelsWhy: 'ocr-models-why', ocrModelsFiles: 'ocr-models-files',
    ocrModelsWhere: 'ocr-models-where', snapOcr: 'snap-ocr', snapOcrText: 'snap-ocr-text',
    snapOcrTextLayerLabel: 'snap-ocr-textlayer-label',
  };
  for (const [k, id] of Object.entries(ids)) el[k] = document.getElementById(id);
}


function registerEvents() {
  el.btnOpen.addEventListener('click', () => el.fileSelector.click());
  el.fileSelector.addEventListener('change', (e) => { const f = [...(e.target.files || [])]; e.target.value = ''; if (f.length) openFiles(f); });
  el.btnOpenFolder.addEventListener('click', () => el.folderSelector.click());
  el.folderSelector.addEventListener('change', (e) => { const f = [...(e.target.files || [])]; e.target.value = ''; if (f.length) openFiles(f, true); });

  el.btnPrev.addEventListener('click', () => gotoPage(state.pageNum - 1));
  el.btnNext.addEventListener('click', () => gotoPage(state.pageNum + 1));
  el.inputPageNum.addEventListener('change', () => gotoPage(parseInt(el.inputPageNum.value, 10) || 1));

  el.btnZoomIn.addEventListener('click', () => setZoom(state.zoomScale + 0.2));
  el.btnZoomOut.addEventListener('click', () => setZoom(state.zoomScale - 0.2));

  [el.chkShowChars, el.chkShowBlocks, el.chkShowProjections, el.chkShowFlow].forEach(c => c.addEventListener('change', drawOverlay));
  el.chkShowTranslation.addEventListener('change', refreshTranslationLayer);
  el.chkAllParagraphs.addEventListener('change', renderParagraphList);

  const tabs = [el.tabBookmarks, el.tabTagged, el.tabParagraphs];
  const panels = [el.panelBookmarks, el.panelTagged, el.panelParagraphs];
  tabs.forEach(btn => btn.addEventListener('click', () => {
    tabs.forEach(b => b.classList.toggle('active', b === btn));
    panels.forEach(p => p.classList.toggle('active', p.id === btn.dataset.target));
  }));

  setupSlider(el.rangeY, el.valY, 'thresholdY');
  setupSlider(el.rangeX, el.valX, 'thresholdX');
  setupSlider(el.rangeMinW, el.valMinW, 'minWidth');
  setupSlider(el.rangeMinH, el.valMinH, 'minHeight');

  el.chkAutoGutter.addEventListener('change', () => { applyAutoGutterUIState(); runAnalysis(); });
  el.selectAutoStrategy.addEventListener('change', runAnalysis);
  el.selectPriority.addEventListener('change', (e) => { state.priority = e.target.value; runAnalysis(); });
  el.chkUseTags.addEventListener('change', runAnalysis);
  el.chkStitch.addEventListener('change', runAnalysis);
  el.btnReanalyze.addEventListener('click', runAnalysis);
  el.selectOcrDoc.addEventListener('change', () => {
    if (el.selectOcrDoc.value === 'ocr' || el.selectOcrDoc.value === 'comic') state.ocrDeclined = false; // an explicit choice asks again
    applyTextSources();
  });
  el.selectOcrPage.addEventListener('change', () => {
    const v = el.selectOcrPage.value;
    if (v) state.pageModes.set(state.pageNum, v);
    else state.pageModes.delete(state.pageNum);
    if (v === 'ocr' || v === 'comic') state.ocrDeclined = false;
    applyTextSources();
  });
  el.selectComicDir.addEventListener('change', runAnalysis);
  el.snapOcrText.addEventListener('input', () => {
    el.snapshotDialog.querySelector('input[name="ocrref"][value="user"]').checked = true;
  });
  el.btnSnapshot.addEventListener('click', openSnapshotDialog);
  el.snapshotDialog.addEventListener('close', () => {
    if (el.snapshotDialog.returnValue === 'save') saveSnapshot();
  });

  el.btnPseudo.addEventListener('click', applyPseudoTranslation);
  el.btnTranslate.addEventListener('click', () => { state.autoTranslate = true; machineTranslatePage(state.pageNum, true); });
  el.selectTrLang.addEventListener('change', () => {
    try { localStorage.setItem('trLang', el.selectTrLang.value); } catch {}
    if (state.translationSource === 'google') {
      applyMachineTranslations();
      recomputePieces().then(() => { renderParagraphList(); refreshTranslationLayer(); });
      if (state.autoTranslate) machineTranslatePage(state.pageNum);
    }
  });
  try { const l = localStorage.getItem('trLang'); if (l && [...el.selectTrLang.options].some(o => o.value === l)) el.selectTrLang.value = l; } catch {}
  const applyXray = () => {
    document.body.classList.toggle('no-xray', !el.chkXray.checked);
    try { localStorage.setItem('xray', el.chkXray.checked ? '1' : '0'); } catch {}
  };
  try { if (localStorage.getItem('xray') === '0') el.chkXray.checked = false; } catch {}
  el.chkXray.addEventListener('change', applyXray);
  applyXray();
  el.btnImportTr.addEventListener('click', () => el.trFileSelector.click());
  el.trFileSelector.addEventListener('change', importTranslations);

  el.overlayCanvas.addEventListener('mousemove', handleCanvasMouseMove);
  el.overlayCanvas.addEventListener('mouseout', handleCanvasMouseOut);
  el.overlayCanvas.addEventListener('click', () => state.hoveredBlockId && selectBlock(state.hoveredBlockId));

  el.btnExport.addEventListener('click', (e) => { e.stopPropagation(); el.exportMenu.classList.toggle('show'); });
  document.addEventListener('click', () => el.exportMenu.classList.remove('show'));
  el.exportMenu.querySelectorAll('button').forEach(btn => btn.addEventListener('click', (e) => {
    e.stopPropagation();
    el.exportMenu.classList.remove('show');
    exportData(btn.dataset.format);
  }));
}

function setupSlider(rangeInput, valDisplay, key) {
  const sync = () => { valDisplay.textContent = fmtPt(state[key]); };
  rangeInput.value = state[key];
  sync();
  rangeInput.addEventListener('input', (e) => { state[key] = parseInt(e.target.value, 10); sync(); });
  rangeInput.addEventListener('change', runAnalysis);
}

function setStatus(text, kind = '') {
  el.analysisStatus.textContent = text;
  el.analysisStatus.className = 'analysis-status' + (kind ? ' ' + kind : '');
}

// --------------------------------------------------------------------------
// Progress card: preparation -> page extraction -> layout analysis
// --------------------------------------------------------------------------

// Share of the bar given to each phase of a full document load.
const PHASE_WEIGHT = { prepare: 0.04, extract: 0.86, analyze: 0.10 };

const progress = {
  start: 0,            // start of the whole run (ms)
  phaseStart: 0,
  extractRate: null,   // smoothed ms per extracted page
  analyzeMsPerPage: 6, // learnt from previous analyses in this session
  analyzeOverhead: 250,
  timer: null,
  hideTimer: null,
};

function formatDuration(ms) {
  if (!isFinite(ms) || ms < 0) return '';
  const s = Math.round(ms / 1000);
  if (s < 1) return 'meno di 1 s';
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  return `${m} min ${String(s % 60).padStart(2, '0')} s`;
}

function estimateAnalysisMs(pages) {
  return progress.analyzeOverhead + pages * progress.analyzeMsPerPage;
}

function paintProgress(phase, detail, fraction, eta, busy = false) {
  clearTimeout(progress.hideTimer);
  const pct = Math.max(0, Math.min(100, fraction * 100));
  el.progressPanel.hidden = false;
  el.progressPanel.classList.remove('done');
  el.progressPanel.classList.toggle('busy', busy);
  el.progressPanel.setAttribute('aria-valuenow', String(Math.round(pct)));
  el.progressPhase.textContent = phase;
  el.progressDetail.textContent = detail;
  el.progressEta.textContent = eta;
  el.progressBar.style.width = pct.toFixed(1) + '%';
  el.progressBar.style.background = '';
}

function progressBegin() {
  stopAnalysisTicker();
  progress.start = progress.phaseStart = performance.now();
  progress.extractRate = null;
  paintProgress('Preparazione', 'apertura documento…', 0, '', true);
}

function progressPrepare(done, total) {
  const f = total ? done / total : 1;
  paintProgress('Preparazione', `indice pagine ${done}/${total}`, PHASE_WEIGHT.prepare * f, '', true);
}

function progressExtract(done, total, pageNum) {
  const now = performance.now();
  if (done === 0) {
    progress.phaseStart = now;
    paintProgress('Estrazione', `pagina ${pageNum} · 0/${total}`, PHASE_WEIGHT.prepare, 'stima in corso…', true);
    return;
  }
  // per-page time: overall average, smoothed so that one slow page does not make the ETA jump
  const avg = (now - progress.phaseStart) / done;
  progress.extractRate = progress.extractRate === null ? avg : progress.extractRate * 0.7 + avg * 0.3;
  const remaining = (total - done) * progress.extractRate + estimateAnalysisMs(total);
  const f = PHASE_WEIGHT.prepare + PHASE_WEIGHT.extract * (done / total);
  const eta = done < Math.min(3, total) ? 'stima in corso…' : `~${formatDuration(remaining)} rimanenti`;
  paintProgress('Estrazione', `pagina ${pageNum} · ${done}/${total}`, f, eta);
}

/** Layout analysis is a single Rust call: show an estimate and the elapsed time. */
function progressAnalyze(pages, fullRun) {
  stopAnalysisTicker();
  if (!fullRun) progress.start = performance.now();
  const t0 = performance.now();
  const estimate = estimateAnalysisMs(pages);
  const base = fullRun ? PHASE_WEIGHT.prepare + PHASE_WEIGHT.extract : 0;
  const span = fullRun ? PHASE_WEIGHT.analyze : 1;
  const tick = () => {
    const elapsed = performance.now() - t0;
    // approach the end of the phase asymptotically (63% at the estimate, 86% at twice it):
    // the bar keeps moving if the estimate is optimistic and never claims 100% before the result
    const f = base + span * 0.97 * (1 - Math.exp(-elapsed / estimate));
    const left = estimate - elapsed;
    const eta = left > 500 ? `~${formatDuration(left)} rimanenti` : 'quasi finito…';
    paintProgress('Analisi del layout', `${pages} pagine · ${formatDuration(elapsed) || '0 s'}`, f, eta, true);
  };
  tick();
  progress.timer = setInterval(tick, 250);
  return t0;
}

function stopAnalysisTicker() {
  if (progress.timer) clearInterval(progress.timer);
  progress.timer = null;
}

function progressDone(pages, analysisStart, summary) {
  stopAnalysisTicker();
  // learn the analysis speed for the next estimates
  const ms = performance.now() - analysisStart;
  if (pages > 0) progress.analyzeMsPerPage = Math.max(0.5, (ms - progress.analyzeOverhead) / pages);
  const total = performance.now() - progress.start;
  paintProgress('Completato', summary, 1, `in ${formatDuration(total)}`);
  progress.hideTimer = setTimeout(() => {
    el.progressPanel.classList.add('done');
    progress.hideTimer = setTimeout(() => { el.progressPanel.hidden = true; }, 450);
  }, 2500);
}

function progressFail(message) {
  stopAnalysisTicker();
  paintProgress('Errore', message, 1, '');
  el.progressBar.style.background = 'var(--accent-red)';
  progress.hideTimer = setTimeout(() => { el.progressPanel.hidden = true; }, 6000);
}

// --------------------------------------------------------------------------
// File loading & document extraction
// --------------------------------------------------------------------------

function setupDragAndDrop() {
  const ws = document.getElementById('document-workspace');
  ws.addEventListener('dragover', (e) => { e.preventDefault(); ws.classList.add('drag-over'); });
  ws.addEventListener('dragleave', () => ws.classList.remove('drag-over'));
  ws.addEventListener('drop', async (e) => {
    e.preventDefault();
    ws.classList.remove('drag-over');
    // a dropped folder: its files, read through the entries API
    const entries = [...(e.dataTransfer.items || [])].map(i => i.webkitGetAsEntry?.()).filter(Boolean);
    if (entries.length === 1 && entries[0].isDirectory) {
      const files = await readDroppedFolder(entries[0]);
      if (files.length) openFiles(files, true);
      return;
    }
    const files = [...(e.dataTransfer.files || [])];
    if (files.length) openFiles(files);
  });
}

/** Files of a dropped folder (recursive), with `webkitRelativePath`-like paths. */
async function readDroppedFolder(dir) {
  const out = [];
  const walk = async (entry, path) => {
    if (entry.isFile) {
      const file = await new Promise((res, rej) => entry.file(res, rej));
      // File.webkitRelativePath is read-only: keep the path alongside
      out.push(Object.defineProperty(file, 'webkitRelativePath', { value: path + file.name }));
    } else if (entry.isDirectory) {
      const reader = entry.createReader();
      for (;;) {
        const batch = await new Promise((res, rej) => reader.readEntries(res, rej));
        if (!batch.length) break;
        for (const child of batch) await walk(child, path + entry.name + '/');
      }
    }
  };
  const reader = dir.createReader();
  for (;;) {
    const batch = await new Promise((res, rej) => reader.readEntries(res, rej));
    if (!batch.length) break;
    for (const child of batch) await walk(child, dir.name + '/');
  }
  return out;
}

/**
 * Open what the user picked or dropped: a PDF (pdf.js), a CBZ / zip, a
 * folder or one or more image files (page images, `imagedoc.js`).
 */
async function openFiles(files, isFolder = false) {
  if (!isFolder && files.length === 1) {
    const f = files[0];
    if (f.type === 'application/pdf' || /\.pdf$/i.test(f.name)) return loadDocument(f.name, () => openPdf(f));
    if (/\.(cbz|zip)$/i.test(f.name) || await looksLikeZip(f)) return loadDocument(f.name, async (progress) => openImages(await zipSource(f), progress));
  }
  const images = files.filter(f => isImageName(f.webkitRelativePath || f.name));
  if (!images.length) {
    alert('Formato non supportato: apri un PDF, un CBZ, una cartella di immagini o delle immagini di pagina.');
    return;
  }
  const src = await filesSource(files);
  return loadDocument(src.name, (progress) => openImages(src, progress));
}

/** pdf.js document; `release` frees it when another document is opened. */
async function openPdf(file) {
  const data = await file.arrayBuffer();
  // pdf.js 6: only the loading task can be destroyed (PDFDocumentProxy.destroy was removed)
  const task = pdfjsLib.getDocument({ data, ...PDFJS_RESOURCES });
  const doc = await task.promise;
  return { doc, release: () => task.destroy() };
}

/** Document of page images (no conversion: pages are decoded when used). */
async function openImages(source, progress) {
  const doc = await ImageDocument.open(source, progress);
  return { doc, release: () => doc.destroy() };
}

/**
 * Open a document and analyse it. `open(progress)` returns { doc, release }:
 * the new document is opened first, then the old one released, so a failed
 * open keeps the current file.
 */
async function loadDocument(name, open) {
  const token = ++state.loadToken;
  showLoading(true);
  progressBegin();
  try {
    const opened = await open((done, total) => {
      paintProgress('Apertura', `pagina ${done}/${total}`, PHASE_WEIGHT.prepare * (done / total), '', false);
    });
    const doc = opened.doc;
    if (token !== state.loadToken) { opened.release(); return; }
    const old = state.loadingTask;
    state.loadingTask = { destroy: opened.release };
    state.pdfDoc = doc;
    if (old) Promise.resolve(old.destroy()).catch(err => console.warn('Error releasing previous document:', err));
    state.fileName = name;
    state.totalPageCount = state.pdfDoc.numPages;
    state.pageNum = 1;
    state.pageData.clear();
    state.textLayer.clear();
    state.pageKinds.clear();
    state.ocrResults.clear();
    state.pageModes.clear();
    state.ocrFailed.clear();
    state.ocrWorker = null;
    state.pageResults.clear();
    state.paraById.clear();
    state.doc = null;
    state.extractionDone = false;
    state.translations.clear();
    state.translationSource = null;
    state.autoTranslate = false;
    state.pieces.clear();
    state.selectedBlockId = state.selectedParaId = state.hoveredBlockId = null;

    if (doc.isImageDocument) {
      // manga declared right-to-left in ComicInfo.xml
      if (doc.info?.rtl) el.selectComicDir.value = 'rtl';
      if (doc.failed.size) setStatus(`${doc.failed.size} immagini non leggibili (${[...doc.failed.keys()].slice(0, 3).map(i => doc.source.pages[i].name).join(', ')}${doc.failed.size > 3 ? '…' : ''})`, 'warn');
    }

    el.txtPageTotal.textContent = state.totalPageCount;
    el.inputPageNum.value = 1;
    el.inputPageNum.max = state.totalPageCount;

    await buildPageRefMap(token);
    if (token !== state.loadToken) return;
    await loadBookmarks();
    await renderPage(1);
    showLoading(false);
    await extractDocument(token);
  } catch (err) {
    console.error('Error loading PDF file:', err);
    if (token === state.loadToken) progressFail(err.message);
    alert('Errore nel caricamento del file PDF: ' + err.message);
  } finally {
    showLoading(false);
  }
}

/** Extract every page (text runs, tags, graphics), then analyse the whole document. */
async function extractDocument(token) {
  const n = state.totalPageCount;
  // current page first so that its overlay can be drawn immediately
  const order = [state.pageNum, ...Array.from({ length: n }, (_, i) => i + 1).filter(p => p !== state.pageNum)];
  let done = 0;
  progressExtract(0, n, order[0]);
  for (const p of order) {
    if (token !== state.loadToken) return;
    if (!state.pageData.has(p)) {
      try {
        const r = await extractPage(pdfjsLib, state.pdfDoc, p);
        state.pageData.set(p, r.data);
      } catch (err) {
        console.warn('Extraction failed on page ' + p, err);
        state.pageData.set(p, { pageNum: p, pageBounds: { x: 0, y: 0, w: 1, h: 1 }, items: [], borderedBoxes: [], images: [] });
      }
      state.textLayer.set(p, state.pageData.get(p));
    }
    done++;
    if (done === 1) drawOverlay();
    progressExtract(done, n, p);
    if (done % 5 === 0 || done === n) setStatus(`Estrazione ${done}/${n}…`);
  }
  if (token !== state.loadToken) return;
  state.extractionDone = true;
  await classifyPages();
  const ocrRan = await prepareTextSources(token);
  if (token !== state.loadToken) return;
  await runAnalysis(!ocrRan);
}

// --------------------------------------------------------------------------
// Text source per page: PDF text layer or OCR
// --------------------------------------------------------------------------

/**
 * 'text' | 'ocr' | 'comic' for a page: page override, else document mode,
 * else the classification (scans are OCR'd; the OCR result then tells
 * whether the page is a comic page).
 */
function effectiveMode(p) {
  const mode = state.pageModes.get(p) || el.selectOcrDoc.value;
  if (mode !== 'auto') return mode;
  if (state.pageKinds.get(p)?.kind !== 'scan') return 'text';
  return state.ocrResults.get(p)?.comic?.isComic ? 'comic' : 'ocr';
}

const usesOcr = (mode) => mode === 'ocr' || mode === 'comic';

/** Page input built from the OCR of page `p` (comic layout included on comic pages). */
function ocrPageInput(p) {
  const r = state.ocrResults.get(p);
  const input = { ...state.textLayer.get(p), items: r.items };
  if (effectiveMode(p) === 'comic') input.comic = r.comic;
  else delete input.comic;
  return input;
}

async function classifyPages() {
  if (!isTauri()) return;
  const pages = [...state.textLayer.values()];
  try {
    const kinds = await window.__TAURI__.core.invoke('classify_pages', { pages });
    state.pageKinds = new Map(kinds.map(k => [k.pageNum, k]));
  } catch (err) {
    console.warn('Page classification failed:', err);
  }
}

/**
 * Make `pageData` follow the text source of every page: restore the text
 * layer where it is used, OCR the current page right away (asking for the
 * models first) and leave the other pages to the background queue
 * (`ocrBackground`), so the document can be read while the OCR goes on.
 * Returns true when the OCR ran on the current page.
 */
async function prepareTextSources(token) {
  if (!isTauri()) return false;
  const n = state.totalPageCount;
  for (let p = 1; p <= n; p++) {
    if (effectiveMode(p) === 'text' && state.textLayer.has(p)) state.pageData.set(p, state.textLayer.get(p));
    else if (state.ocrResults.has(p)) state.pageData.set(p, ocrPageInput(p));
  }
  const missing = ocrQueue();
  if (missing.length && !(await ensureOcrModels(missing.length))) {
    // without models the text layer stays in use
    for (let p = 1; p <= n; p++) if (usesOcr(effectiveMode(p)) && state.textLayer.has(p)) state.pageData.set(p, state.textLayer.get(p));
    refreshOcrControls();
    return false;
  }
  let ran = false;
  if (missing[0] === state.pageNum) {
    if (state.ocrWorker !== token) state.ocrStats = { done: 0, t0: performance.now(), lastAnalysis: 0 };
    ran = await ocrOnePage(state.pageNum, token);
  }
  if (token !== state.loadToken) return ran;
  if (ocrQueue().length) ocrBackground(token);
  refreshOcrControls();
  return ran;
}

/**
 * Pages still waiting for the OCR, in the order the background queue takes
 * them: the current page, the following pages, then the preceding ones.
 */
function ocrQueue() {
  const n = state.totalPageCount;
  const out = [];
  for (let k = 0; k < n; k++) {
    const p = ((state.pageNum - 1 + k) % n) + 1;
    if (usesOcr(effectiveMode(p)) && !state.ocrResults.has(p) && !state.ocrFailed.has(p)) out.push(p);
  }
  return out;
}

/** Rasterise and recognise one page; its input switches to the OCR text. */
async function ocrOnePage(p, token) {
  const total = state.ocrStats.done + ocrQueue().length;
  progressOcr(state.ocrStats.done, total, p, state.ocrStats.t0);
  try {
    const raster = await rasterizePage(state.pdfDoc, p, OCR_DPI);
    const res = await recognizeCanvas(window.__TAURI__.core.invoke, raster);
    if (token !== state.loadToken) return false;
    state.ocrResults.set(p, { ...res, dpi: raster.dpi });
  } catch (err) {
    console.error('OCR failed on page ' + p, err);
    state.ocrFailed.add(p);
    setStatus(`OCR fallito a pagina ${p}: ${err}`, 'warn');
    return false;
  }
  state.ocrStats.done++;
  state.pageData.set(p, ocrPageInput(p));
  if (p === state.pageNum) { drawOverlay(); refreshOcrControls(); }
  return true;
}

/**
 * Background OCR of the remaining pages. The queue is re-read after every
 * page, so moving to a page not recognised yet makes it the next one, and a
 * change of mode adds or removes pages. The layout is re-analysed when a
 * page near the one being read is done (and at least every 20 s), then once
 * more at the end.
 */
async function ocrBackground(token) {
  if (state.ocrWorker === token) return; // already running: it reads the queue again after each page
  state.ocrWorker = token;
  if (!state.ocrStats.t0) state.ocrStats = { done: 0, t0: performance.now(), lastAnalysis: 0 };
  try {
    for (;;) {
      if (token !== state.loadToken) return;
      const p = ocrQueue()[0];
      if (!p) break;
      const ok = await ocrOnePage(p, token);
      if (token !== state.loadToken) return;
      const near = p >= state.pageNum - 1 && p <= state.pageNum + 2;
      if (ok && (near || performance.now() - state.ocrStats.lastAnalysis > 20000)) {
        state.ocrStats.lastAnalysis = performance.now();
        await runAnalysis(false);
      }
    }
  } finally {
    if (state.ocrWorker === token) state.ocrWorker = null;
  }
  if (token !== state.loadToken) return;
  const { done, t0 } = state.ocrStats;
  state.ocrStats = { done: 0, t0: 0, lastAnalysis: 0 };
  if (done) {
    progressOcr(done, done, state.pageNum, t0);
    await runAnalysis(false);
  }
  refreshOcrControls();
}

/** Re-apply the text sources after a change of mode, then re-analyse. */
async function applyTextSources() {
  refreshOcrControls();
  if (!state.pdfDoc || !state.extractionDone) return;
  const token = state.loadToken;
  await prepareTextSources(token);
  if (token !== state.loadToken) return;
  await runAnalysis(false);
}

function progressOcr(done, total, pageNum, t0) {
  const elapsed = performance.now() - t0;
  const per = done ? elapsed / done : null;
  const eta = per === null ? 'stima in corso…' : done >= total ? '' : `~${formatDuration(per * (total - done))} rimanenti`;
  const background = done > 0 && done < total ? ' · le pagine pronte sono già consultabili' : '';
  paintProgress('OCR', `pagina ${pageNum} · ${done}/${total}${background}`, total ? done / total : 1, eta, done === 0);
  if (done >= total) setStatus(`OCR completato: ${total} pagine`);
}

/** " · OCR k/N in corso" while the background queue runs. */
function ocrStatusSuffix() {
  if (state.ocrWorker !== state.loadToken) return '';
  const left = ocrQueue().length;
  return left ? ` · OCR in corso: ${left} pagine in coda` : '';
}

/** Ask for the model download when the models are missing. Resolves to true when they are installed. */
async function ensureOcrModels(pages) {
  const invoke = window.__TAURI__.core.invoke;
  let status;
  try {
    status = await invoke('ocr_models_status');
  } catch (err) {
    setStatus('OCR non disponibile: ' + err, 'warn');
    return false;
  }
  if (status.installed) return true;
  if (state.ocrDeclined) return false;

  el.ocrModelsWhy.textContent = `${pages} ${pages === 1 ? 'pagina non ha' : 'pagine non hanno'} testo selezionabile e ${pages === 1 ? 'richiede' : 'richiedono'} l'OCR.`;
  el.ocrModelsFiles.innerHTML = '';
  for (const f of status.files.filter(f => !f.present)) {
    const li = document.createElement('li');
    li.textContent = `${f.name} · ${f.role} · ${formatBytes(f.size)}`;
    el.ocrModelsFiles.appendChild(li);
  }
  el.ocrModelsWhere.textContent = `Totale ${formatBytes(status.missingBytes)} da ${status.sources.join(' (o ')}${status.sources.length > 1 ? ')' : ''}, salvati in ${status.dir}.`;
  const answer = await new Promise((resolve) => {
    el.ocrModelsDialog.addEventListener('close', () => resolve(el.ocrModelsDialog.returnValue), { once: true });
    el.ocrModelsDialog.returnValue = '';
    el.ocrModelsDialog.showModal();
  });
  if (answer !== 'download') {
    state.ocrDeclined = true;
    setStatus('OCR non attivo: modelli non scaricati. Le pagine senza testo restano vuote.', 'warn');
    return false;
  }

  let unlisten = null;
  try {
    paintProgress('Download modelli OCR', 'connessione…', 0, '', true);
    unlisten = await window.__TAURI__.event.listen('ocr-download-progress', (e) => {
      const { file, total, done } = e.payload;
      paintProgress('Download modelli OCR', `${file} · ${formatBytes(done)} / ${formatBytes(total)}`, total ? done / total : 0, '');
    });
    await invoke('ocr_download_models');
    setStatus('Modelli OCR installati.', 'ok');
    return true;
  } catch (err) {
    console.error('Model download failed:', err);
    progressFail('download dei modelli OCR fallito');
    setStatus('Download dei modelli OCR fallito: ' + err, 'warn');
    alert('Download dei modelli OCR fallito:\n' + err);
    return false;
  } finally {
    if (unlisten) unlisten();
  }
}

/** Page selector, page kind badge and OCR summary for the current page. */
function refreshOcrControls() {
  if (!el.selectOcrPage) return;
  const p = state.pageNum;
  el.selectOcrPage.value = state.pageModes.get(p) || '';
  const kind = state.pageKinds.get(p);
  const mode = state.pdfDoc ? effectiveMode(p) : null;
  const res = state.ocrResults.get(p);
  const usingOcr = usesOcr(mode) && res && state.pageData.get(p)?.items === res.items;
  const comic = usingOcr && mode === 'comic';
  el.ocrPageKind.textContent = !kind ? '—' : (kind.kind === 'scan' ? 'scansione' : 'testo') + (comic ? ' · fumetto' : usingOcr ? ' · OCR' : '');
  if (!kind) { el.ocrHint.hidden = true; return; }
  let hint = `Classificazione: ${kind.reason}.`;
  if (usingOcr) {
    const s = ocrSummary(res);
    hint += ` OCR (${res.page.engine}, ${Math.round(res.dpi)} DPI): ${s.lines} righe, confidenza media ${(s.conf * 100).toFixed(1)}%` +
      (s.low ? `, ${s.low} righe incerte (in arancione con "Words")` : '') + ` · ${formatDuration(s.ms) || s.ms + ' ms'}.`;
    if (res.comic) {
      const c = res.comic;
      const count = (k) => c.balloons.filter(b => b.kind === k).length;
      const found = `${c.panels.length} vignette, ${count('balloon')} balloon, ${count('narration')} didascalie, ${count('comic-text')} testi liberi, ${count('sfx')} onomatopee`;
      hint += comic ? ` Fumetto: ${found}.` : c.isComic ? ` Sembra un fumetto (${found}).` : '';
    }
  } else if (usesOcr(mode)) {
    hint += state.ocrDeclined ? ' OCR non attivo: modelli non scaricati.'
      : state.ocrFailed.has(p) ? ' OCR fallito su questa pagina.'
      : state.ocrWorker === state.loadToken ? ' OCR in corso in background: questa pagina è la prossima.' : ' OCR in attesa.';
  }
  el.ocrHint.textContent = hint;
  el.ocrHint.hidden = false;
}

async function buildPageRefMap(token) {
  state.pageRefMap.clear();
  const n = state.totalPageCount;
  for (let i = 1; i <= n; i++) {
    if (token !== state.loadToken) return;
    if (i === 1 || i % 10 === 0 || i === n) progressPrepare(i, n);
    try {
      const page = await state.pdfDoc.getPage(i);
      if (page.ref) state.pageRefMap.set(page.ref.num + '_' + page.ref.gen, i);
    } catch (e) {
      console.warn('Could not retrieve ref for page ' + i, e);
    }
  }
}

function showLoading(show) {
  el.spinner.style.display = show ? 'flex' : 'none';
}

// --------------------------------------------------------------------------
// Analysis (Rust engine)
// --------------------------------------------------------------------------

function analysisOptions() {
  const opts = {
    strategy: el.selectAutoStrategy.value,
    priority: state.priority,
    useTags: el.chkUseTags.checked,
    stitch: el.chkStitch.checked,
    rtl: el.selectComicDir.value === 'rtl',
  };
  if (!el.chkAutoGutter.checked) {
    opts.manual = {
      thresholdX: state.thresholdX,
      thresholdY: state.thresholdY,
      minWidth: state.minWidth,
      minHeight: state.minHeight,
    };
  }
  return opts;
}

/** `fullRun`: first analysis after loading, the bar continues from the extraction phase. */
async function runAnalysis(fullRun = false) {
  if (!state.pdfDoc) return;
  if (!isTauri()) {
    setStatus('Motore di analisi disponibile solo nell\'app desktop.', 'warn');
    if (fullRun === true) progressFail('motore di analisi disponibile solo nell\'app desktop');
    return;
  }
  if (!state.extractionDone) return; // analysis starts automatically when extraction ends
  const token = state.loadToken;
  const seq = ++state.analysisSeq;
  const pages = Array.from({ length: state.totalPageCount }, (_, i) => state.pageData.get(i + 1)).filter(Boolean);
  // during the background OCR the progress panel belongs to the OCR
  const quiet = state.ocrWorker === token;
  if (!quiet) setStatus('Analisi in corso…');
  const t0 = quiet ? performance.now() : progressAnalyze(pages.length, fullRun === true);
  try {
    const options = analysisOptions();
    const doc = await window.__TAURI__.core.invoke('analyze_document', { pages, options });
    if (token !== state.loadToken || seq !== state.analysisSeq) return;
    state.doc = doc;
    state.docOptions = options;
    state.pageResults = new Map(doc.pages.map(p => [p.pageNum, p]));
    state.paraById = new Map(doc.paragraphs.map(p => [p.id, p]));
    if (!state.paraById.has(state.selectedParaId)) state.selectedParaId = null;
    state.selectedBlockId = null;
    const s = doc.stats;
    const lost = s.inputChars - s.outputChars;
    setStatus(`${s.paragraphCount} paragrafi · ${s.stitchedJoins} cuciture · ${Math.round(performance.now() - t0)} ms` + (lost ? ` · ⚠ ${lost} caratteri persi` : '') + ocrStatusSuffix(), lost ? 'warn' : 'ok');
    if (!quiet) progressDone(pages.length, t0, `${s.pageCount} pagine · ${s.paragraphCount} paragrafi · ${s.stitchedJoins} cuciture`);
    // machine translations follow the paragraph text: the ids change when the
    // background OCR re-analyses the document
    if (state.translationSource === 'google') applyMachineTranslations();
    await recomputePieces();
    refreshPageUI();
    if (state.autoTranslate) machineTranslatePage(state.pageNum);
  } catch (err) {
    console.error('Analysis failed:', err);
    setStatus('Analisi fallita: ' + err, 'warn');
    if (token === state.loadToken && !quiet) progressFail('analisi fallita: ' + err);
  }
}

function currentPageResult() {
  return state.pageResults.get(state.pageNum) || null;
}

// --------------------------------------------------------------------------
// Navigation & rendering
// --------------------------------------------------------------------------

function gotoPage(num) {
  if (!state.pdfDoc) return;
  num = Math.max(1, Math.min(state.totalPageCount, num));
  state.pageNum = num;
  el.inputPageNum.value = num;
  state.selectedBlockId = null;
  queueRenderPage(num);
}

function setZoom(z) {
  z = Math.round(Math.max(0.4, Math.min(3.0, z)) * 10) / 10;
  state.zoomScale = z;
  el.txtZoomPercent.textContent = Math.round(z * 100) + '%';
  if (state.pdfDoc) queueRenderPage(state.pageNum);
}

function queueRenderPage(num) {
  if (state.pageRendering) state.pageNumPending = num;
  else renderPage(num);
}

async function renderPage(num) {
  state.pageRendering = true;
  try {
    const page = await state.pdfDoc.getPage(num);
    const viewport = page.getViewport({ scale: state.zoomScale });
    const ratio = window.devicePixelRatio || 1;
    for (const c of [el.pdfCanvas, el.overlayCanvas]) {
      c.width = Math.floor(viewport.width * ratio);
      c.height = Math.floor(viewport.height * ratio);
      c.style.width = viewport.width + 'px';
      c.style.height = viewport.height + 'px';
    }
    el.canvasContainer.style.width = viewport.width + 'px';
    el.canvasContainer.style.height = viewport.height + 'px';
    el.translationLayer.style.width = viewport.width + 'px';
    el.translationLayer.style.height = viewport.height + 'px';

    const ctx = el.pdfCanvas.getContext('2d');
    await page.render({
      canvasContext: ctx,
      viewport,
      transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : undefined,
    }).promise;

    refreshPageUI();
    loadTaggedPDFStructure();
    // after "Traduci", every page shown is translated too
    if (state.autoTranslate) machineTranslatePage(num);
  } catch (err) {
    console.error('Error rendering page:', err);
  } finally {
    state.pageRendering = false;
    if (state.pageNumPending !== null) {
      const next = state.pageNumPending;
      state.pageNumPending = null;
      renderPage(next);
    }
  }
}

function refreshPageUI() {
  refreshOcrControls();
  syncAutoSliders();
  renderLayoutDOMTree();
  renderParagraphList();
  drawOverlay();
  refreshTranslationLayer();
  if (state.selectedBlockId) selectBlock(state.selectedBlockId, false);
  else if (state.selectedParaId) populateParagraphInspector(state.paraById.get(state.selectedParaId));
  else el.inspectorPanel.innerHTML = '<div class="empty-state">Seleziona un blocco o un paragrafo per ispezionarlo.</div>';
}

// --------------------------------------------------------------------------
// Bookmarks & Tagged structure (navigation panels)
// --------------------------------------------------------------------------

async function loadBookmarks() {
  el.outlineTree.innerHTML = '';
  try {
    const outline = await state.pdfDoc.getOutline();
    if (!outline || outline.length === 0) {
      el.outlineTree.innerHTML = '<div class="empty-state">No outline bookmarks in this PDF.</div>';
      return;
    }
    const root = document.createElement('div');
    root.className = 'tree-root';
    for (const item of outline) root.appendChild(await createOutlineNode(item));
    el.outlineTree.appendChild(root);
  } catch (err) {
    console.error('Error loading outline bookmarks:', err);
    el.outlineTree.innerHTML = '<div class="empty-state font-red">Errore nel caricamento dei segnalibri.</div>';
  }
}

async function createOutlineNode(item) {
  const node = document.createElement('div');
  node.className = 'tree-node';
  const hasKids = item.items && item.items.length > 0;
  if (hasKids) node.classList.add('has-children');

  const header = document.createElement('div');
  header.className = 'tree-node-header';
  const toggle = document.createElement('span');
  toggle.className = 'tree-toggle';
  toggle.textContent = hasKids ? '▶' : '';
  header.appendChild(toggle);
  const title = document.createElement('span');
  title.className = 'tree-node-title';
  title.textContent = item.title;
  header.appendChild(title);

  if (item.dest) {
    const pageNum = await resolveDestPage(item.dest);
    if (pageNum) {
      const badge = document.createElement('span');
      badge.className = 'tree-node-badge';
      badge.textContent = 'p. ' + pageNum;
      header.appendChild(badge);
      header.addEventListener('click', (e) => {
        e.stopPropagation();
        gotoPage(pageNum);
        document.querySelectorAll('.tree-node-header').forEach(h => h.classList.remove('active'));
        header.classList.add('active');
      });
    }
  }
  node.appendChild(header);

  if (hasKids) {
    const kids = document.createElement('div');
    kids.className = 'tree-children';
    for (const sub of item.items) kids.appendChild(await createOutlineNode(sub));
    node.appendChild(kids);
    toggle.addEventListener('click', (e) => { e.stopPropagation(); node.classList.toggle('expanded'); });
  }
  return node;
}

async function resolveDestPage(dest) {
  if (typeof dest === 'number') return dest + 1;
  let ref = null;
  if (typeof dest === 'string') {
    try {
      const resolved = await state.pdfDoc.getDestination(dest);
      if (resolved && resolved.length > 0) ref = resolved[0];
    } catch (e) {
      console.warn('Could not resolve named destination: ' + dest, e);
    }
  } else if (Array.isArray(dest)) {
    ref = dest[0];
  }
  if (ref && typeof ref === 'object') return state.pageRefMap.get(ref.num + '_' + ref.gen) || null;
  if (typeof ref === 'number') return ref + 1;
  return null;
}

async function loadTaggedPDFStructure() {
  el.taggedTree.innerHTML = '';
  el.taggedBadge.className = 'status-badge checking';
  el.taggedBadge.textContent = 'Analyzing structure...';
  try {
    if (!state.pdfDoc) return;
    const page = await state.pdfDoc.getPage(state.pageNum);
    const tree = await page.getStructTree();
    if (!tree || !tree.children || tree.children.length === 0) {
      el.taggedBadge.className = 'status-badge untagged';
      el.taggedBadge.textContent = 'Untagged PDF';
      el.taggedTree.innerHTML = '<div class="empty-state">Nessuna struttura logica (StructTree): l\'ordine di lettura è ricostruito dalla geometria.</div>';
      return;
    }
    const used = currentPageResult()?.taggedOrder;
    el.taggedBadge.className = 'status-badge tagged';
    el.taggedBadge.textContent = used ? 'Tagged PDF · ordine dai tag' : 'Tagged PDF';
    const root = document.createElement('div');
    root.className = 'tree-root';
    for (const child of tree.children) root.appendChild(renderStructNode(child));
    el.taggedTree.appendChild(root);
  } catch (err) {
    console.error('Error loading struct tree:', err);
    el.taggedBadge.className = 'status-badge untagged';
    el.taggedBadge.textContent = 'StructTree Error';
  }
}

function renderStructNode(node) {
  const li = document.createElement('div');
  li.className = 'tree-node';
  const header = document.createElement('div');
  header.className = 'tree-node-header';
  const isLeaf = !node.children || node.children.length === 0;
  const toggle = document.createElement('span');
  toggle.className = 'tree-toggle';
  toggle.textContent = isLeaf ? '' : '▶';
  header.appendChild(toggle);

  const tagBadge = document.createElement('span');
  tagBadge.className = 'tag-badge';
  const role = node.role || (node.type === 'content' ? 'MC' : 'Element');
  tagBadge.textContent = role;
  if (/^H\d?$/.test(role)) tagBadge.classList.add('header-tag');
  else if (['P', 'Span', 'BlockQuote', 'Table', 'TR', 'TD'].includes(role)) tagBadge.classList.add('block-tag');
  header.appendChild(tagBadge);

  const title = document.createElement('span');
  title.className = 'tree-node-title';
  title.textContent = node.alt || node.title || (node.type === 'content' ? node.id : isLeaf ? 'Content' : 'Section');
  header.appendChild(title);
  li.appendChild(header);

  if (!isLeaf) {
    li.classList.add('has-children');
    const kids = document.createElement('div');
    kids.className = 'tree-children';
    for (const c of node.children) kids.appendChild(renderStructNode(c));
    li.appendChild(kids);
    toggle.addEventListener('click', (e) => { e.stopPropagation(); li.classList.toggle('expanded'); });
  }
  return li;
}

// --------------------------------------------------------------------------
// Overlay
// --------------------------------------------------------------------------

function overlayContext() {
  const ctx = el.overlayCanvas.getContext('2d');
  const ratio = window.devicePixelRatio || 1;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, el.overlayCanvas.width, el.overlayCanvas.height);
  // draw in PDF points
  const k = ratio * state.zoomScale;
  ctx.setTransform(k, 0, 0, k, 0, 0);
  return ctx;
}

function drawOverlay() {
  if (!state.pdfDoc) return;
  const ctx = overlayContext();
  const z = state.zoomScale;
  const px = (n) => n / z; // screen pixels → points (line widths)
  const data = state.pageData.get(state.pageNum);
  const pr = currentPageResult();

  if (el.chkShowChars.checked && data) {
    ctx.lineWidth = px(1);
    for (const it of data.items) {
      // OCR runs: violet, orange when the recognition is uncertain
      const ocr = it.source === 'ocr';
      const low = ocr && it.conf < LOW_CONF;
      ctx.strokeStyle = low ? 'rgba(251, 146, 60, 0.9)' : ocr ? 'rgba(167, 139, 250, 0.5)' : 'rgba(56, 189, 248, 0.25)';
      ctx.fillStyle = low ? 'rgba(251, 146, 60, 0.12)' : ocr ? 'rgba(167, 139, 250, 0.06)' : 'rgba(56, 189, 248, 0.04)';
      ctx.fillRect(it.x, it.y, it.width, it.height);
      ctx.strokeRect(it.x, it.y, it.width, it.height);
    }
  }
  if (!pr) return;

  if (el.chkShowBlocks.checked) drawBlockRecursive(ctx, pr.root, px);

  if (el.chkShowFlow.checked) drawFlow(ctx, pr, px);

  if (state.selectedParaId) {
    const p = state.paraById.get(state.selectedParaId);
    if (p) {
      ctx.fillStyle = 'rgba(244, 114, 182, 0.16)';
      ctx.strokeStyle = 'rgba(244, 114, 182, 0.9)';
      ctx.lineWidth = px(2);
      for (const s of p.segments) {
        if (s.pageNum !== state.pageNum) continue;
        for (const l of s.lines) ctx.fillRect(l.x, l.y, l.w, l.h);
        ctx.strokeRect(s.bounds.x - px(2), s.bounds.y - px(2), s.bounds.w + px(4), s.bounds.h + px(4));
      }
    }
  }

  const hl = (id, stroke, fill, w) => {
    const n = id && findBlockById(pr.root, id);
    if (!n) return;
    ctx.strokeStyle = stroke;
    ctx.fillStyle = fill;
    ctx.lineWidth = px(w);
    ctx.fillRect(n.bounds.x, n.bounds.y, n.bounds.w, n.bounds.h);
    ctx.strokeRect(n.bounds.x, n.bounds.y, n.bounds.w, n.bounds.h);
  };
  hl(state.hoveredBlockId, 'rgba(244, 63, 94, 0.85)', 'rgba(244, 63, 94, 0.08)', 2.5);
  hl(state.selectedBlockId, '#00f0ff', 'rgba(0, 240, 255, 0.10)', 3);

  if (el.chkShowProjections.checked) drawProjectionProfiles(ctx, pr, px);
}

function drawBlockRecursive(ctx, block, px) {
  const b = block.bounds;
  const kind = block.containerKind;
  if (kind === 'panel' || kind === 'balloon' || kind === 'narration') {
    // comic pages: panels in blue, containers in the colour of their role
    ctx.setLineDash([]);
    ctx.strokeStyle = kind === 'panel' ? 'rgba(59, 130, 246, 0.85)' : roleColor(kind);
    ctx.lineWidth = px(kind === 'panel' ? 2.5 : 2);
    ctx.strokeRect(b.x, b.y, b.w, b.h);
  } else if (block.isBorderedBox) {
    ctx.setLineDash([]);
    ctx.strokeStyle = block.isTable ? 'rgba(250, 204, 21, 0.8)' : 'rgba(216, 180, 254, 0.85)';
    ctx.lineWidth = px(2);
    ctx.strokeRect(b.x, b.y, b.w, b.h);
  } else if (block.type === 'leaf') {
    const c = roleColor(block.role);
    ctx.setLineDash([]);
    ctx.strokeStyle = c;
    ctx.globalAlpha = 0.75;
    ctx.lineWidth = px(1.5);
    ctx.strokeRect(b.x, b.y, b.w, b.h);
    ctx.globalAlpha = 0.06;
    ctx.fillStyle = c;
    ctx.fillRect(b.x, b.y, b.w, b.h);
    ctx.globalAlpha = 1;
    return;
  } else if (block.type !== 'root') {
    ctx.setLineDash([px(4), px(3)]);
    ctx.strokeStyle = `hsla(${(block.depth * 55) % 360}, 75%, 65%, 0.35)`;
    ctx.lineWidth = px(1);
    ctx.strokeRect(b.x, b.y, b.w, b.h);
    ctx.setLineDash([]);
  }
  for (const c of block.children || []) drawBlockRecursive(ctx, c, px);
}

/** Reading-order badges + continuation arrows between the pieces of a paragraph. */
function drawFlow(ctx, pr, px) {
  const leaves = [];
  (function walk(n) { if (n.type === 'leaf') leaves.push(n); (n.children || []).forEach(walk); })(pr.root);

  ctx.font = `600 ${px(11)}px Inter, sans-serif`;
  ctx.textBaseline = 'middle';
  for (const l of leaves) {
    if (l.readingOrder === undefined || l.readingOrder === null) continue;
    const label = String(l.readingOrder + 1);
    const w = ctx.measureText(label).width + px(8);
    const h = px(14);
    const x = l.bounds.x - w - px(2);
    const y = l.bounds.y;
    ctx.fillStyle = roleColor(l.role);
    roundRect(ctx, Math.max(0, x), y, w, h, px(3));
    ctx.fill();
    ctx.fillStyle = '#04111d';
    ctx.fillText(label, Math.max(0, x) + px(4), y + h / 2);
  }

  if (!state.doc) return;
  const pageNum = state.pageNum;
  for (const p of state.doc.paragraphs) {
    if (p.segments.length < 2 || !p.segments.some(s => s.pageNum === pageNum)) continue;
    const selected = p.id === state.selectedParaId;
    ctx.strokeStyle = selected ? 'rgba(244, 114, 182, 1)' : 'rgba(244, 114, 182, 0.75)';
    ctx.fillStyle = ctx.strokeStyle;
    ctx.lineWidth = px(selected ? 2.5 : 1.5);
    for (let k = 0; k + 1 < p.segments.length; k++) {
      const a = p.segments[k], b = p.segments[k + 1];
      const aLast = a.lines[a.lines.length - 1] || a.bounds;
      const bFirst = b.lines[0] || b.bounds;
      const from = [aLast.x + aLast.w, aLast.y + aLast.h / 2];
      const to = [bFirst.x, bFirst.y + bFirst.h / 2];
      const join = p.joins.find(j => j.atSegment === k + 1);
      if (a.pageNum === pageNum && b.pageNum === pageNum) {
        drawArrow(ctx, from, to, px);
      } else if (a.pageNum === pageNum) {
        tag(ctx, `↪ p.${b.pageNum}${join?.dehyphenated ? ' ‐' : ''}`, from[0] + px(4), from[1], px);
      } else if (b.pageNum === pageNum) {
        // just above the first line: the reading-order badge sits to its left
        tag(ctx, `↩ p.${a.pageNum}`, bFirst.x, bFirst.y - px(9), px);
      }
    }
  }
}

function drawArrow(ctx, [x1, y1], [x2, y2], px) {
  ctx.setLineDash([px(5), px(3)]);
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  const mx = (x1 + x2) / 2;
  ctx.bezierCurveTo(mx, y1, mx, y2, x2, y2);
  ctx.stroke();
  ctx.setLineDash([]);
  const ang = Math.atan2(y2 - (y1 + y2) / 2, x2 - mx) || 0;
  const s = px(7);
  ctx.beginPath();
  ctx.moveTo(x2, y2);
  ctx.lineTo(x2 - s * Math.cos(ang - 0.4), y2 - s * Math.sin(ang - 0.4));
  ctx.lineTo(x2 - s * Math.cos(ang + 0.4), y2 - s * Math.sin(ang + 0.4));
  ctx.closePath();
  ctx.fill();
}

function tag(ctx, text, x, y, px, alignRight = false) {
  ctx.font = `600 ${px(10)}px Inter, sans-serif`;
  const w = ctx.measureText(text).width + px(8);
  const h = px(14);
  const x0 = alignRight ? x - w : x;
  ctx.save();
  ctx.fillStyle = 'rgba(244, 114, 182, 0.95)';
  roundRect(ctx, x0, y - h / 2, w, h, px(3));
  ctx.fill();
  ctx.fillStyle = '#1b0612';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, x0 + px(4), y);
  ctx.restore();
}

function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function drawProjectionProfiles(ctx, pr, px) {
  const { w, h } = pr.pageBounds;
  const band = px(25);
  ctx.fillStyle = 'rgba(0, 0, 0, 0.55)';
  ctx.fillRect(0, h - band, w, band);
  ctx.fillRect(w - band, 0, band, h);

  const xs = new Array(Math.ceil(w)).fill(0);
  const ys = new Array(Math.ceil(h)).fill(0);
  for (const l of pr.lines) {
    for (let x = Math.max(0, Math.floor(l.bounds.x)); x <= Math.min(w - 1, Math.ceil(l.bounds.x + l.bounds.w)); x++) xs[x]++;
    for (let y = Math.max(0, Math.floor(l.bounds.y)); y <= Math.min(h - 1, Math.ceil(l.bounds.y + l.bounds.h)); y++) ys[y]++;
  }
  const mx = Math.max(...xs, 1), my = Math.max(...ys, 1);
  ctx.fillStyle = 'rgba(56, 189, 248, 0.5)';
  xs.forEach((v, x) => { if (v) { const bh = (v / mx) * (band - px(4)); ctx.fillRect(x, h - bh - px(2), 1, bh); } });
  ctx.fillStyle = 'rgba(192, 132, 252, 0.5)';
  ys.forEach((v, y) => { if (v) { const bw = (v / my) * (band - px(4)); ctx.fillRect(w - bw - px(2), y, bw, 1); } });

  ctx.fillStyle = 'rgba(251, 191, 36, 0.15)';
  for (const g of pr.projections.xGaps) ctx.fillRect(g.start, 0, g.end - g.start, h - band);
  for (const g of pr.projections.yGaps) ctx.fillRect(0, g.start, w - band, g.end - g.start);
}

// --------------------------------------------------------------------------
// Layout DOM tree
// --------------------------------------------------------------------------

function renderLayoutDOMTree() {
  el.domTreeContainer.innerHTML = '';
  const pr = currentPageResult();
  if (!pr) {
    el.domTreeContainer.innerHTML = `<div class="empty-state">${state.pdfDoc ? 'Analisi in corso…' : 'No layout analyzed yet.'}</div>`;
    return;
  }
  const wrap = document.createElement('div');
  wrap.className = 'dom-tree-root';
  wrap.appendChild(buildDOMNode(pr.root));
  el.domTreeContainer.appendChild(wrap);
}

function buildDOMNode(node) {
  const div = document.createElement('div');
  div.className = 'dom-node';
  div.dataset.id = node.id;

  const header = document.createElement('div');
  header.className = 'dom-node-header';
  const toggle = document.createElement('span');
  toggle.className = 'dom-node-toggle';
  toggle.textContent = node.type === 'leaf' ? '' : '▶';
  header.appendChild(toggle);

  const label = document.createElement('span');
  label.className = 'dom-node-label';
  if (node.type === 'root') {
    label.textContent = `Page ${state.pageNum}`;
    div.classList.add('expanded');
  } else if (node.isBorderedBox) {
    label.textContent = node.isTable ? 'Table' : 'Box';
    div.classList.add('bordered-box-node');
  } else if (node.type === 'leaf') {
    label.textContent = `#${(node.readingOrder ?? 0) + 1}`;
  } else {
    label.textContent = `Cut_${node.cutDirection || '?'}`;
  }
  header.appendChild(label);

  if (node.type === 'leaf') {
    const chip = document.createElement('span');
    chip.className = 'role-chip';
    chip.textContent = node.role || 'body';
    chip.style.setProperty('--role-color', roleColor(node.role));
    header.appendChild(chip);
    const snippet = document.createElement('span');
    snippet.className = 'dom-node-text-clip';
    const t = (node.text || '').replace(/\n/g, ' ');
    snippet.textContent = `"${t.substring(0, 40)}${t.length > 40 ? '…' : ''}"`;
    header.appendChild(snippet);
  }
  div.appendChild(header);

  header.addEventListener('mouseenter', () => { state.hoveredBlockId = node.id; header.classList.add('hovered'); drawOverlay(); });
  header.addEventListener('mouseleave', () => { state.hoveredBlockId = null; header.classList.remove('hovered'); drawOverlay(); });
  header.addEventListener('click', (e) => { e.stopPropagation(); selectBlock(node.id); });

  if (node.children && node.children.length > 0) {
    const kids = document.createElement('div');
    kids.className = 'dom-node-children';
    for (const c of node.children) kids.appendChild(buildDOMNode(c));
    div.appendChild(kids);
    toggle.addEventListener('click', (e) => { e.stopPropagation(); div.classList.toggle('expanded'); });
  }
  return div;
}

function selectBlock(nodeId, scroll = true) {
  const pr = currentPageResult();
  if (!pr) return;
  const block = findBlockById(pr.root, nodeId);
  if (!block) return;
  state.selectedBlockId = nodeId;
  if (block.paragraphIds?.length && !block.paragraphIds.includes(state.selectedParaId)) {
    state.selectedParaId = block.paragraphIds[0];
  }
  drawOverlay();
  refreshTranslationLayer();
  highlightParagraphItem();

  document.querySelectorAll('.dom-node-header').forEach(h => h.classList.remove('active'));
  const target = document.querySelector(`.dom-node[data-id="${CSS.escape(nodeId)}"] > .dom-node-header`);
  if (target) {
    target.classList.add('active');
    let parent = target.parentElement.parentElement;
    while (parent && parent.classList.contains('dom-node-children')) {
      parent.parentElement?.classList.add('expanded');
      parent = parent.parentElement?.parentElement;
    }
    if (scroll) target.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }
  populateBlockInspector(block);
}

// --------------------------------------------------------------------------
// Logical paragraphs panel
// --------------------------------------------------------------------------

function renderParagraphList() {
  const list = el.paragraphList;
  list.innerHTML = '';
  const doc = state.doc;
  if (!doc) {
    el.analysisStats.textContent = state.pdfDoc ? 'Analisi non ancora disponibile.' : 'Nessuna analisi.';
    return;
  }
  const s = doc.stats;
  const pr = currentPageResult();
  el.analysisStats.innerHTML = `<b>${s.paragraphCount}</b> paragrafi logici · <b>${s.stitchedJoins}</b> cuciture · corpo ${s.bodyFontSize} pt` +
    (pr?.taggedOrder ? ' · <span class="ok">ordine dai tag</span>' : '');

  const all = el.chkAllParagraphs.checked;
  const paras = doc.paragraphs.filter(p => all || p.segments.some(seg => seg.pageNum === state.pageNum));
  if (paras.length === 0) {
    list.innerHTML = '<div class="empty-state">Nessun testo su questa pagina.</div>';
    return;
  }
  const frag = document.createDocumentFragment();
  for (const p of paras.slice(0, 2000)) {
    const item = document.createElement('div');
    item.className = 'para-item';
    item.dataset.id = p.id;
    if (p.id === state.selectedParaId) item.classList.add('active');
    const pages = [...new Set(p.segments.map(seg => seg.pageNum))];
    const head = document.createElement('div');
    head.className = 'para-head';
    head.innerHTML = `<span class="role-chip" style="--role-color:${roleColor(p.role)}">${p.role}</span>` +
      `<span class="para-id">${p.id}</span>` +
      (pages.length > 1 ? `<span class="para-span">p.${pages[0]}→${pages[pages.length - 1]}</span>` : '') +
      (p.joins.length ? `<span class="para-joins" title="${p.joins.map(j => j.kind + (j.interrupted ? ' (interrotto)' : '') + (j.dehyphenated ? ' (sillabato)' : '')).join(', ')}">⟲ ${p.joins.length}</span>` : '') +
      (state.translations.has(p.id) ? '<span class="para-tr" title="Tradotto">✓</span>' : '');
    const body = document.createElement('div');
    body.className = 'para-text';
    body.textContent = p.text.length > 180 ? p.text.slice(0, 180) + '…' : p.text;
    item.append(head, body);
    item.addEventListener('click', () => selectParagraph(p.id));
    frag.appendChild(item);
  }
  list.appendChild(frag);
}

function highlightParagraphItem() {
  el.paragraphList.querySelectorAll('.para-item').forEach(i => i.classList.toggle('active', i.dataset.id === state.selectedParaId));
}

function selectParagraph(id) {
  const p = state.paraById.get(id);
  if (!p) return;
  state.selectedParaId = id;
  state.selectedBlockId = null;
  const onPage = p.segments.find(s => s.pageNum === state.pageNum);
  if (!onPage) {
    gotoPage(p.segments[0].pageNum);
  } else {
    drawOverlay();
    refreshTranslationLayer();
  }
  highlightParagraphItem();
  populateParagraphInspector(p);
}

// --------------------------------------------------------------------------
// Inspector
// --------------------------------------------------------------------------

function card(label, val, extra = '') {
  const c = document.createElement('div');
  c.className = 'inspector-card' + (extra ? ' ' + extra : '');
  const l = document.createElement('div');
  l.className = 'inspector-card-label';
  l.textContent = label;
  const v = document.createElement('div');
  v.className = 'inspector-card-val';
  v.textContent = val;
  c.append(l, v);
  return c;
}

function textBox(title, text) {
  const wrap = document.createDocumentFragment();
  const t = document.createElement('div');
  t.className = 'inspector-card-label';
  t.style.marginTop = '8px';
  t.textContent = title;
  const box = document.createElement('textarea');
  box.className = 'inspector-text-box';
  box.readOnly = true;
  box.value = text;
  box.title = 'Clicca per copiare';
  box.addEventListener('click', () => {
    box.select();
    navigator.clipboard?.writeText(text);
    const old = t.textContent;
    t.textContent = '✓ Copiato negli appunti!';
    setTimeout(() => { t.textContent = old; }, 1200);
  });
  wrap.append(t, box);
  return wrap;
}

function populateBlockInspector(block) {
  el.inspectorPanel.innerHTML = '';
  const c = document.createElement('div');
  c.className = 'inspector-layout';
  const grid = document.createElement('div');
  grid.className = 'inspector-grid';
  const b = block.bounds;
  grid.appendChild(card('ID / tipo', `${block.id} (${block.type})`));
  grid.appendChild(card('Ruolo', block.role || (block.isBorderedBox ? (block.isTable ? 'table' : 'box') : '—')));
  grid.appendChild(card('Dimensioni', `${Math.round(b.w)}×${Math.round(b.h)} pt`));
  grid.appendChild(card('Ordine di lettura', block.readingOrder !== undefined && block.readingOrder !== null ? `#${block.readingOrder + 1}` : '—'));
  if (block.type === 'leaf') {
    const f = block.formatting;
    grid.appendChild(card('Font', f.fontFamily, 'font-desc'));
    grid.appendChild(card('Corpo', `${f.fontSize} pt (media ${f.avgFontSize})`));
    grid.appendChild(card('Peso / stile', `${f.fontWeight} / ${f.fontStyle}`));
    grid.appendChild(card('Allineamento', f.alignment.toUpperCase()));
    c.appendChild(grid);
    c.appendChild(textBox('Testo del blocco:', block.text || ''));
    for (const pid of block.paragraphIds || []) {
      const p = state.paraById.get(pid);
      if (!p) continue;
      const btn = document.createElement('button');
      btn.className = 'btn secondary-btn w-full para-link';
      btn.textContent = `${pid} · ${p.role}${p.segments.length > 1 ? ` · ${p.segments.length} parti` : ''} →`;
      btn.addEventListener('click', () => selectParagraph(pid));
      c.appendChild(btn);
    }
  } else {
    grid.appendChild(card('Taglio', block.cutDirection ? `asse ${block.cutDirection}` : '—', 'span-2'));
    grid.appendChild(card('Figli', `${block.children ? block.children.length : 0}`, 'span-2'));
    c.appendChild(grid);
  }
  const d = block.diag;
  if (d && d.source) {
    const dg = document.createElement('div');
    dg.className = 'inspector-grid';
    dg.style.marginTop = '8px';
    dg.appendChild(card(`Soglie usate (${d.source})`, `Ty ${fmtPt(round1(d.thresholdY))} · Tx ${fmtPt(round1(d.thresholdX))} · min W ${fmtPt(round1(d.minWidth))} · min H ${fmtPt(round1(d.minHeight))}`, 'span-2'));
    dg.appendChild(card('Gap massimi', `X ${fmtPt(round1(d.largestXGap))} · Y ${fmtPt(round1(d.largestYGap))} · corpo ${fmtPt(round1(d.dominantFont))}`, 'span-2'));
    dg.appendChild(card('Decisione', d.decision + (d.tried.length ? ` — ${d.tried.join('; ')}` : ''), 'span-2'));
    c.appendChild(dg);
  } else if (d) {
    const dg = document.createElement('div');
    dg.className = 'inspector-grid';
    dg.style.marginTop = '8px';
    dg.appendChild(card('Decisione', d.decision, 'span-2'));
    c.appendChild(dg);
  }
  el.inspectorPanel.appendChild(c);
}

function populateParagraphInspector(p) {
  if (!p) return;
  el.inspectorPanel.innerHTML = '';
  const c = document.createElement('div');
  c.className = 'inspector-layout';
  const grid = document.createElement('div');
  grid.className = 'inspector-grid';
  const pages = [...new Set(p.segments.map(s => s.pageNum))];
  grid.appendChild(card('Paragrafo', `${p.id} · ${p.role}`));
  grid.appendChild(card('Pagine', pages.join(', ')));
  grid.appendChild(card('Parti', `${p.segments.length} (${p.segments.map(s => s.blockId).join(', ')})`, 'span-2'));
  if (p.joins.length) {
    const desc = p.joins.map(j => `${j.kind}${j.interrupted ? ', interrotto' : ''}${j.dehyphenated ? ', sillabazione ricomposta' : ''}`).join(' · ');
    grid.appendChild(card('Cuciture', desc, 'span-2'));
  }
  c.appendChild(grid);
  c.appendChild(textBox('Testo ricostruito (unità di traduzione):', p.text));
  if (state.translations.has(p.id)) c.appendChild(textBox('Traduzione:', state.translations.get(p.id)));
  el.inspectorPanel.appendChild(c);
}

// --------------------------------------------------------------------------
// Canvas interaction
// --------------------------------------------------------------------------

function canvasPoint(e) {
  const rect = el.overlayCanvas.getBoundingClientRect();
  return [(e.clientX - rect.left) / state.zoomScale, (e.clientY - rect.top) / state.zoomScale];
}

function handleCanvasMouseMove(e) {
  const pr = currentPageResult();
  if (!pr) return;
  const [x, y] = canvasPoint(e);
  const m = findDeepestBlockAt(pr.root, x, y);
  const id = m && m.type !== 'root' ? m.id : null;
  if (id !== state.hoveredBlockId) {
    state.hoveredBlockId = id;
    drawOverlay();
    document.querySelectorAll('.dom-node-header.hovered').forEach(h => h.classList.remove('hovered'));
    if (id) document.querySelector(`.dom-node[data-id="${CSS.escape(id)}"] > .dom-node-header`)?.classList.add('hovered');
  }
}

function handleCanvasMouseOut() {
  if (state.hoveredBlockId !== null) {
    state.hoveredBlockId = null;
    drawOverlay();
    document.querySelectorAll('.dom-node-header.hovered').forEach(h => h.classList.remove('hovered'));
  }
}

function findBlockById(root, id) {
  if (root.id === id) return root;
  for (const c of root.children || []) {
    const f = findBlockById(c, id);
    if (f) return f;
  }
  return null;
}

function findDeepestBlockAt(root, x, y) {
  const b = root.bounds;
  if (x < b.x || x > b.x + b.w || y < b.y || y > b.y + b.h) return null;
  for (const c of root.children || []) {
    const m = findDeepestBlockAt(c, x, y);
    if (m) return m;
  }
  return root;
}

// --------------------------------------------------------------------------
// Translation
// --------------------------------------------------------------------------

async function recomputePieces() {
  state.pieces.clear();
  if (!state.doc || state.translations.size === 0 || !isTauri()) return;
  const paras = state.doc.paragraphs.filter(p => state.translations.has(p.id));
  const units = paras.map(p => ({ text: state.translations.get(p.id), weights: p.segments.map(s => s.charCount) }));
  try {
    const parts = await window.__TAURI__.core.invoke('distribute_translations', { units });
    paras.forEach((p, i) => state.pieces.set(p.id, parts[i]));
  } catch (err) {
    console.error('distribute_translations failed', err);
  }
}

function refreshTranslationLayer() {
  const show = el.chkShowTranslation.checked;
  el.translationLayer.style.display = show ? 'block' : 'none';
  if (!show) return;
  const r = renderTranslationLayer(el.translationLayer, state.doc, state.pieces, state.pageNum, state.zoomScale, state.selectedParaId);
  if (state.translations.size === 0) {
    setStatus('Nessuna traduzione: usa "Pseudo" o "Importa".', 'warn');
  } else if (r.rendered) {
    setStatus(`Traduzione: ${r.rendered} blocchi · ${r.shrunk} ridotti` + (r.overflow ? ` · ⚠ ${r.overflow} non entrano` : ''), r.overflow ? 'warn' : 'ok');
  }
}

const mtKey = (lang, text) => `${lang}\u0000${text}`;

/** Paragraph id → machine translation, from the cache, for the current analysis. */
function applyMachineTranslations() {
  const lang = el.selectTrLang.value;
  state.translations = new Map();
  for (const p of state.doc?.paragraphs || []) {
    const t = state.mtCache.get(mtKey(lang, p.text));
    if (t !== undefined) state.translations.set(p.id, t);
  }
}

/**
 * Machine-translate the paragraphs of a page that are not translated yet
 * (Google Translate free endpoint, as GianoReader's free mode; the request
 * runs in Rust). One request batch at a time: a page asked for while
 * another is being translated is picked up afterwards.
 */
async function machineTranslatePage(pageNum, explicit = false) {
  if (!state.doc || !isTauri()) return;
  if (state.translationSource !== 'google') {
    state.translationSource = 'google';
    applyMachineTranslations();
  }
  if (!el.chkShowTranslation.checked) el.chkShowTranslation.checked = true;
  if (state.translating) { state.pendingTranslatePage = pageNum; return; }
  const lang = el.selectTrLang.value;
  const paras = state.doc.paragraphs.filter(p => p.translatable && p.segments.some(s => s.pageNum === pageNum) && !state.mtCache.has(mtKey(lang, p.text)));
  if (!paras.length) {
    await recomputePieces();
    renderParagraphList();
    refreshTranslationLayer();
    return;
  }
  state.translating = true;
  el.btnTranslate.disabled = true;
  setStatus(`Traduzione di ${paras.length} paragrafi…`);
  let unlisten = null;
  try {
    unlisten = await window.__TAURI__.event.listen('translate-progress', (e) => {
      setStatus(`Traduzione ${e.payload.done}/${e.payload.total} paragrafi…`);
    });
    const out = await window.__TAURI__.core.invoke('translate_free', { texts: paras.map(p => p.text), source: 'auto', target: lang });
    paras.forEach((p, i) => state.mtCache.set(mtKey(lang, p.text), out[i] || ''));
    applyMachineTranslations();
    await recomputePieces();
    renderParagraphList();
    refreshTranslationLayer();
  } catch (err) {
    console.error('Translation failed:', err);
    setStatus('Traduzione fallita: ' + err, 'warn');
    if (explicit) alert('Traduzione fallita:\n' + err);
    state.autoTranslate = false;
  } finally {
    if (unlisten) unlisten();
    state.translating = false;
    el.btnTranslate.disabled = false;
  }
  const next = state.pendingTranslatePage;
  state.pendingTranslatePage = null;
  if (next && state.autoTranslate) machineTranslatePage(next);
}

async function applyPseudoTranslation() {
  if (!state.doc) return;
  state.translations = new Map(state.doc.paragraphs.filter(p => p.translatable).map(p => [p.id, pseudoTranslate(p.text)]));
  state.translationSource = 'pseudo';
  state.autoTranslate = false;
  await recomputePieces();
  el.chkShowTranslation.checked = true;
  renderParagraphList();
  refreshTranslationLayer();
}

async function importTranslations(e) {
  const file = e.target.files?.[0];
  e.target.value = '';
  if (!file) return;
  try {
    const map = parseTranslations(await file.text());
    if (map.size === 0) throw new Error('nessuna traduzione trovata nel file');
    const known = state.doc ? [...map.keys()].filter(k => state.paraById.has(k)).length : 0;
    state.translations = map;
    state.translationSource = 'import';
    state.autoTranslate = false;
    await recomputePieces();
    el.chkShowTranslation.checked = true;
    renderParagraphList();
    refreshTranslationLayer();
    if (state.doc && known < map.size) {
      setStatus(`Importate ${map.size} traduzioni, ${map.size - known} con id sconosciuto (analisi cambiata?)`, 'warn');
    }
  } catch (err) {
    alert('Importazione fallita: ' + err.message);
  }
}

// --------------------------------------------------------------------------
// Export
// --------------------------------------------------------------------------

function download(content, name, mime) {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function exportData(format) {
  if (!state.doc) {
    alert('Nessun layout analizzato. Carica prima un PDF!');
    return;
  }
  const base = (state.fileName || 'document').replace(/\.pdf$/i, '');
  const pr = currentPageResult();
  switch (format) {
    case 'json':
    case 'xml':
    case 'html':
      if (!pr) return;
      download(serializeLayoutTree(pr.root, format), `${base}_p${state.pageNum}.${format}`,
        { json: 'application/json', xml: 'application/xml', html: 'text/html' }[format]);
      break;
    case 'doc-json':
      download(JSON.stringify(state.doc, null, 2), `${base}_layout.json`, 'application/json');
      break;
    case 'txt':
      download(serializeParagraphsText(state.doc), `${base}_paragraphs.txt`, 'text/plain');
      break;
    case 'units':
      download(serializeUnits(state.doc, state.fileName), `${base}_units.json`, 'application/json');
      break;
  }
}

// --------------------------------------------------------------------------
// Resizable sidebars
// --------------------------------------------------------------------------

function setupResizableSidebars() {
  const container = document.querySelector('.app-container');
  const drag = (handle, cursor, onMove) => {
    handle.addEventListener('mousedown', (e) => {
      e.preventDefault();
      handle.classList.add('dragging');
      document.body.style.cursor = cursor;
      const start = { x: e.clientX, y: e.clientY };
      const move = (ev) => onMove(ev, start);
      const up = () => {
        handle.classList.remove('dragging');
        document.body.style.cursor = '';
        window.removeEventListener('mousemove', move);
        window.removeEventListener('mouseup', up);
      };
      window.addEventListener('mousemove', move);
      window.addEventListener('mouseup', up);
    });
  };
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  const px = (name) => parseFloat(getComputedStyle(container).getPropertyValue(name)) || 0;
  drag(document.getElementById('left-resizer'), 'col-resize', (ev) => {
    container.style.setProperty('--left-width', clamp(ev.clientX - 16, 300, window.innerWidth * 0.4) + 'px');
  });
  // the Layout Engine column: its left edge follows the mouse, the DOM column stays
  drag(document.getElementById('engine-resizer'), 'col-resize', (ev) => {
    const w = window.innerWidth - ev.clientX - px('--right-width') - 32;
    container.style.setProperty('--engine-width', clamp(w, 260, window.innerWidth * 0.35) + 'px');
  });
  drag(document.getElementById('right-resizer'), 'col-resize', (ev) => {
    container.style.setProperty('--right-width', clamp(window.innerWidth - ev.clientX - 16, 300, window.innerWidth * 0.35) + 'px');
  });
  const inspector = document.getElementById('inspector-section');
  let startH = 0;
  const ir = document.getElementById('inspector-resizer');
  ir.addEventListener('mousedown', () => { startH = inspector.getBoundingClientRect().height; });
  drag(ir, 'row-resize', (ev, start) => {
    container.style.setProperty('--inspector-height', clamp(startH - (ev.clientY - start.y), 120, 450) + 'px');
  });
}

// --------------------------------------------------------------------------
// Snapshots: settings + page data + manual vs automatic results, for tuning
// --------------------------------------------------------------------------

const SNAPSHOT_VERSION = 1;

function describeOptions(o) {
  if (!o) return '—';
  const base = `priorità ${o.priority} · tag ${o.useTags ? 'sì' : 'no'} · cucitura ${o.stitch ? 'sì' : 'no'}`;
  if (o.manual) {
    const m = o.manual;
    return `MANUALI: Ty ${m.thresholdY} pt · Tx ${m.thresholdX} pt · min W ${m.minWidth} pt · min H ${m.minHeight} pt · ${base}`;
  }
  return `AUTOMATICHE (${o.strategy}) · ${base}`;
}

function openSnapshotDialog() {
  if (!state.doc || !currentPageResult()) {
    alert("Nessuna analisi disponibile: carica un PDF e attendi la fine dell'analisi.");
    return;
  }
  el.snapPage.textContent = state.pageNum;
  el.snapSettings.textContent = describeOptions(state.docOptions);
  // default verdict follows the mode in use
  const manual = !!state.docOptions?.manual;
  el.snapshotDialog.querySelector(`input[name="verdict"][value="${manual ? 'manual-better' : 'reference'}"]`).checked = true;
  el.snapNote.value = '';
  // OCR pages: reference transcription for the OCR evaluation
  const ocr = currentOcr();
  el.snapOcr.hidden = !ocr;
  if (ocr) {
    el.snapOcrText.value = ocr.page.lines.map(l => l.text).join('\n');
    const hasLayer = (state.pageKinds.get(state.pageNum)?.chars || 0) >= 40;
    el.snapOcrTextLayerLabel.hidden = !hasLayer;
    el.snapshotDialog.querySelector(`input[name="ocrref"][value="${hasLayer ? 'text-layer' : 'user'}"]`).checked = true;
  }
  el.snapshotDialog.showModal();
}

/** OCR result in use on the current page, or null. */
function currentOcr() {
  const p = state.pageNum;
  const res = state.ocrResults.get(p);
  return res && state.pageData.get(p)?.items === res.items ? res : null;
}

/** OCR section of a snapshot: engine, image, lines and the reference chosen in the dialog. */
async function ocrSnapshot(pageNum, res, stem) {
  const choice = el.snapshotDialog.querySelector('input[name="ocrref"]:checked')?.value || 'none';
  let reference = null;
  if (choice === 'user') {
    reference = { source: 'user', lines: el.snapOcrText.value.split('\n').map(s => s.trim()).filter(Boolean) };
  } else if (choice === 'ocr') {
    reference = { source: 'ocr', lines: res.page.lines.map(l => l.text) };
  } else if (choice === 'text-layer') {
    // the lines the layout engine builds from the PDF text layer of the same page
    const layer = state.textLayer.get(pageNum);
    const doc = await window.__TAURI__.core.invoke('analyze_document', { pages: [layer], options: state.docOptions });
    const pr = doc.pages.find(p => p.pageNum === pageNum);
    reference = { source: 'text-layer', lines: (pr?.lines || []).map(l => l.text).filter(t => t.trim()) };
  }
  // the exact image given to the OCR (same DPI, same renderer), for --eval-ocr
  const raster = await rasterizePage(state.pdfDoc, pageNum, res.dpi);
  return {
    section: {
      engine: res.page.engine,
      dpi: res.dpi,
      image: `${stem}_ocr.png`,
      width: res.page.width,
      height: res.page.height,
      detectMs: res.page.detectMs,
      recognizeMs: res.page.recognizeMs,
      lines: res.page.lines,
      reference,
    },
    png: raster.canvas.toDataURL('image/png'),
  };
}

function leafSummaries(root) {
  const out = [];
  (function walk(n) {
    if (n.type === 'leaf') {
      out.push({ id: n.id, bounds: n.bounds, role: n.role || null, readingOrder: n.readingOrder ?? null, text: (n.text || '').slice(0, 160) });
    }
    (n.children || []).forEach(walk);
  })(root);
  return out;
}

function pageSummary(doc, pageNum, options) {
  const pr = doc.pages.find(p => p.pageNum === pageNum);
  if (!pr) return null;
  return {
    options,
    root: pr.root, // full tree; nodes carry `diag` (thresholds, gaps, decision)
    leaves: leafSummaries(pr.root),
    taggedOrder: pr.taggedOrder,
    paragraphs: doc.paragraphs
      .filter(p => p.segments.some(s => s.pageNum === pageNum))
      .map(p => ({
        id: p.id,
        role: p.role,
        text: p.text.slice(0, 300),
        segments: p.segments.map(s => ({ pageNum: s.pageNum, blockId: s.blockId, bounds: s.bounds })),
        joins: p.joins,
      })),
  };
}

function viewPng() {
  try {
    const c = document.createElement('canvas');
    c.width = el.pdfCanvas.width;
    c.height = el.pdfCanvas.height;
    const ctx = c.getContext('2d');
    ctx.drawImage(el.pdfCanvas, 0, 0);
    ctx.drawImage(el.overlayCanvas, 0, 0);
    return c.toDataURL('image/png');
  } catch (err) {
    console.warn('Snapshot image unavailable:', err);
    return null;
  }
}

async function saveSnapshot() {
  const pageNum = state.pageNum;
  const verdict = el.snapshotDialog.querySelector('input[name="verdict"]:checked')?.value || 'reference';
  const note = el.snapNote.value.trim();
  const current = state.docOptions;
  const pages = [pageNum - 1, pageNum, pageNum + 1].map(p => state.pageData.get(p)).filter(Boolean);
  setStatus('Snapshot: calcolo dei risultati automatici…');
  try {
    const invoke = window.__TAURI__.core.invoke;
    // the same pages analysed in isolation, so the CLI can reproduce every result exactly
    const autoSameOpts = { ...current };
    delete autoSameOpts.manual;
    const autoDefaultOpts = { strategy: 'combined', priority: 'Y', useTags: current.useTags, stitch: current.stitch };
    const [subCurrent, subAutoSame, subAutoDefault] = await Promise.all([
      invoke('analyze_document', { pages, options: current }),
      invoke('analyze_document', { pages, options: autoSameOpts }),
      invoke('analyze_document', { pages, options: autoDefaultOpts }),
    ]);

    const curSum = pageSummary(subCurrent, pageNum, current);
    const autoSum = pageSummary(subAutoSame, pageNum, autoSameOpts);
    let reference = null;
    if (verdict === 'manual-better' || verdict === 'reference') reference = { source: 'snapshot-settings', leaves: curSum.leaves };
    else if (verdict === 'auto-better') reference = { source: 'auto-same-priority', leaves: autoSum.leaves };

    const snapshot = {
      kind: 'pdf-xray-snapshot',
      version: SNAPSHOT_VERSION,
      createdAt: new Date().toISOString(),
      file: state.fileName,
      pageNum,
      pageCount: state.totalPageCount,
      verdict,
      note,
      settings: {
        options: current,
        ui: {
          autoThresholds: el.chkAutoGutter.checked,
          strategy: el.selectAutoStrategy.value,
          priority: state.priority,
          thresholdX: state.thresholdX,
          thresholdY: state.thresholdY,
          minWidth: state.minWidth,
          minHeight: state.minHeight,
          zoom: state.zoomScale,
        },
      },
      document: { bodyFontSize: state.doc.stats.bodyFontSize, stats: state.doc.stats },
      // the full-document view the user was looking at (may differ slightly from the isolated pages)
      displayed: pageSummary(state.doc, pageNum, current),
      results: {
        snapshotSettings: curSum,
        autoSamePriority: autoSum,
        autoDefault: pageSummary(subAutoDefault, pageNum, autoDefaultOpts),
      },
      reference,
      pages,
    };

    const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
    const base = (state.fileName || 'document').replace(/\.pdf$/i, '').slice(0, 40);
    const stem = `${base}_p${pageNum}_${verdict}_${stamp}`;
    const png = el.snapPng.checked ? viewPng() : null;
    let ocrPng = null;
    const ocrRes = currentOcr();
    if (ocrRes && isTauri()) {
      setStatus('Snapshot: immagine OCR…');
      const o = await ocrSnapshot(pageNum, ocrRes, stem.replace(/[^\p{L}\p{N}._-]/gu, '_'));
      snapshot.ocr = o.section;
      ocrPng = o.png;
    }
    const json = JSON.stringify(snapshot, null, 1);

    if (isTauri()) {
      const path = await invoke('save_snapshot', { fileStem: stem, json, pngBase64: png, ocrPngBase64: ocrPng });
      setStatus(`Snapshot salvato: ${path}`, 'ok');
      try { await window.__TAURI__.opener?.revealItemInDir(path); } catch (_) { /* best effort */ }
    } else {
      download(json, `${stem}.json`, 'application/json');
      setStatus('Snapshot scaricato.', 'ok');
    }
  } catch (err) {
    console.error('Snapshot failed:', err);
    setStatus('Snapshot fallito: ' + err, 'warn');
  }
}
