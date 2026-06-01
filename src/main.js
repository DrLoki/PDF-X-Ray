import * as pdfjsLib from './assets/pdfjs/pdf.min.mjs';
import { performXYCut, serializeLayoutTree } from './xycut.js';

// Setup PDFjs Worker
pdfjsLib.GlobalWorkerOptions.workerSrc = './assets/pdfjs/pdf.worker.min.mjs';

// Application State
let state = {
  pdfDoc: null,
  pageNum: 1,
  totalPageCount: 1,
  zoomScale: 1.0,
  pageRendering: false,
  pageNumPending: null,
  textItems: [],      // Screen-space text elements with bounds
  currentPageRectangles: [], // Extracted vector rectangles for bordered boxes
  xycutResult: null,   // Current XY-cut output
  selectedBlockId: null,
  hoveredBlockId: null,
  // Sliders config
  thresholdY: 10,
  thresholdX: 15,
  minWidth: 20,
  minHeight: 10,
  priority: 'Y',
  // Navigation mapping for outline
  pageRefMap: new Map(), // maps PDF object IDs to page numbers
  // Flag to prevent listener from reacting to programmatic checkbox changes
  programmaticCheckboxChange: false
};

// UI Elements Cache
let el = {};

window.addEventListener('DOMContentLoaded', () => {
  cacheElements();
  registerEvents();
  setupDragAndDrop();
  setupResizableSidebars();
  
  if (el.chkAutoGutter && el.chkAutoGutter.checked) {
    const disableSliders = true;
    el.rangeY.disabled = disableSliders;
    el.rangeX.disabled = disableSliders;
    el.rangeMinW.disabled = disableSliders;
    el.rangeMinH.disabled = disableSliders;
    el.selectPriority.disabled = disableSliders;
    if (el.selectAutoStrategy) el.selectAutoStrategy.disabled = false;
  } else {
    if (el.selectAutoStrategy) el.selectAutoStrategy.disabled = true;
  }
});

function cacheElements() {
  el.btnOpen = document.getElementById('btn-open-file');
  el.fileSelector = document.getElementById('file-selector');
  el.btnPrev = document.getElementById('btn-prev-page');
  el.btnNext = document.getElementById('btn-next-page');
  el.inputPageNum = document.getElementById('input-page-num');
  el.txtPageTotal = document.getElementById('txt-page-total');
  el.btnZoomIn = document.getElementById('btn-zoom-in');
  el.btnZoomOut = document.getElementById('btn-zoom-out');
  el.txtZoomPercent = document.getElementById('txt-zoom-percent');
  el.chkShowChars = document.getElementById('chk-show-chars');
  el.chkShowBlocks = document.getElementById('chk-show-blocks');
  el.chkShowProjections = document.getElementById('chk-show-projections');
  el.chkAutoGutter = document.getElementById('chk-auto-gutter');
  el.selectAutoStrategy = document.getElementById('select-auto-strategy');
  el.pdfCanvas = document.getElementById('pdf-canvas');
  el.overlayCanvas = document.getElementById('overlay-canvas');
  el.canvasContainer = document.getElementById('canvas-container');
  el.viewport = document.getElementById('canvas-viewport');
  el.spinner = document.getElementById('loading-spinner');
  
  // Tabs
  el.tabBookmarks = document.getElementById('tab-bookmarks');
  el.tabTagged = document.getElementById('tab-tagged');
  el.panelBookmarks = document.getElementById('panel-bookmarks');
  el.panelTagged = document.getElementById('panel-tagged');
  
  // Left Sidebar Trees
  el.outlineTree = document.getElementById('outline-tree');
  el.taggedTree = document.getElementById('tagged-tree');
  el.taggedBadge = document.getElementById('tagged-status-badge');
  
  // Right Sidebar Controls
  el.rangeY = document.getElementById('range-threshold-y');
  el.valY = document.getElementById('val-threshold-y');
  el.rangeX = document.getElementById('range-threshold-x');
  el.valX = document.getElementById('val-threshold-x');
  el.rangeMinW = document.getElementById('range-min-w');
  el.valMinW = document.getElementById('val-min-w');
  el.rangeMinH = document.getElementById('range-min-h');
  el.valMinH = document.getElementById('val-min-h');
  el.selectPriority = document.getElementById('select-priority');
  el.btnReanalyze = document.getElementById('btn-reanalyze');
  
  // Right Sidebar DOM & Inspector
  el.domTreeContainer = document.getElementById('dom-tree-container');
  el.inspectorPanel = document.getElementById('inspector-panel');
  el.btnExport = document.getElementById('btn-export');
  el.exportMenu = document.getElementById('export-menu');
}

function registerEvents() {
  // File Loading
  el.btnOpen.addEventListener('click', () => el.fileSelector.click());
  el.fileSelector.addEventListener('change', handleFileSelect);
  
  // Navigation
  el.btnPrev.addEventListener('click', onPrevPage);
  el.btnNext.addEventListener('click', onNextPage);
  el.inputPageNum.addEventListener('change', onPageNumInput);
  
  // Zoom
  el.btnZoomIn.addEventListener('click', onZoomIn);
  el.btnZoomOut.addEventListener('click', onZoomOut);
  
  // Visualization toggles
  el.chkShowChars.addEventListener('change', drawOverlay);
  el.chkShowBlocks.addEventListener('change', drawOverlay);
  el.chkShowProjections.addEventListener('change', drawOverlay);
  
  // Sidebar Tabs
  const tabButtons = [el.tabBookmarks, el.tabTagged];
  tabButtons.forEach(btn => {
    btn.addEventListener('click', () => {
      tabButtons.forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      
      const targetPanel = btn.dataset.target;
      [el.panelBookmarks, el.panelTagged].forEach(panel => {
        if (panel.id === targetPanel) {
          panel.classList.add('active');
        } else {
          panel.classList.remove('active');
        }
      });
    });
  });
  
  // Sliders interaction
  setupSlider(el.rangeY, el.valY, 'thresholdY');
  setupSlider(el.rangeX, el.valX, 'thresholdX');
  setupSlider(el.rangeMinW, el.valMinW, 'minWidth');
  setupSlider(el.rangeMinH, el.valMinH, 'minHeight');
  
  el.chkAutoGutter.addEventListener('change', () => {
    // Skip if this change was triggered programmatically
    if (state.programmaticCheckboxChange) {
      state.programmaticCheckboxChange = false;
      return;
    }
    
    const disableSliders = el.chkAutoGutter.checked;
    el.rangeY.disabled = disableSliders;
    el.rangeX.disabled = disableSliders;
    el.rangeMinW.disabled = disableSliders;
    el.rangeMinH.disabled = disableSliders;
    el.selectPriority.disabled = disableSliders;
    el.selectAutoStrategy.disabled = !disableSliders;
    runXYCutAnalysis();
  });
  
  el.selectAutoStrategy.addEventListener('change', () => {
    runXYCutAnalysis();
  });
  
  el.selectPriority.addEventListener('change', (e) => {
    state.priority = e.target.value;
  });
  
  el.btnReanalyze.addEventListener('click', runXYCutAnalysis);
  
  // Canvas Mouse interaction (Bi-directional Hover and Click)
  el.overlayCanvas.addEventListener('mousemove', handleCanvasMouseMove);
  el.overlayCanvas.addEventListener('mouseout', handleCanvasMouseOut);
  el.overlayCanvas.addEventListener('click', handleCanvasMouseClick);
  
  // Export Menu Dropdown
  el.btnExport.addEventListener('click', (e) => {
    e.stopPropagation();
    el.exportMenu.classList.toggle('show');
  });
  document.addEventListener('click', () => el.exportMenu.classList.remove('show'));
  
  el.exportMenu.querySelectorAll('button').forEach(btn => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const format = btn.dataset.format;
      el.exportMenu.classList.remove('show');
      exportLayoutDOM(format);
    });
  });
}

// Slider link helpers
function setupSlider(rangeInput, valDisplay, stateKey) {
  rangeInput.addEventListener('input', (e) => {
    const val = parseInt(e.target.value, 10);
    valDisplay.textContent = val + ' px';
    state[stateKey] = val;
  });
}

// --------------------------------------------------------------------------
// File Handling
// --------------------------------------------------------------------------

function setupDragAndDrop() {
  const ws = document.getElementById('document-workspace');
  
  ws.addEventListener('dragover', (e) => {
    e.preventDefault();
    ws.style.borderColor = 'var(--accent)';
    ws.style.backgroundColor = 'rgba(56, 189, 248, 0.03)';
  });
  
  const resetDragStyles = () => {
    ws.style.borderColor = '';
    ws.style.backgroundColor = '';
  };
  
  ws.addEventListener('dragleave', resetDragStyles);
  ws.addEventListener('drop', async (e) => {
    e.preventDefault();
    resetDragStyles();
    
    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
      const file = e.dataTransfer.files[0];
      if (file.type === 'application/pdf' || file.name.endsWith('.pdf')) {
        loadPDFFile(file);
      }
    }
  });
}

async function handleFileSelect(e) {
  if (e.target.files && e.target.files.length > 0) {
    loadPDFFile(e.target.files[0]);
  }
}

async function loadPDFFile(file) {
  showLoading(true);
  try {
    const arrayBuffer = await file.arrayBuffer();
    const loadingTask = pdfjsLib.getDocument({ data: arrayBuffer });
    
    state.pdfDoc = await loadingTask.promise;
    state.totalPageCount = state.pdfDoc.numPages;
    state.pageNum = 1;
    
    el.txtPageTotal.textContent = state.totalPageCount;
    el.inputPageNum.value = 1;
    el.inputPageNum.max = state.totalPageCount;
    
    // Resolve Page Refs mapping for bookmarks
    await buildPageRefMap();
    
    // Extract metadata
    await loadBookmarks();
    await loadTaggedPDFStructure();
    
    // Render first page
    await renderPage(state.pageNum);
  } catch (err) {
    console.error('Error loading PDF file:', err);
    alert('Errore nel caricamento del file PDF: ' + err.message);
  } finally {
    showLoading(false);
  }
}

// Generates a map of PDF Object references to 1-based page numbers
async function buildPageRefMap() {
  state.pageRefMap.clear();
  for (let i = 1; i <= state.totalPageCount; i++) {
    try {
      const page = await state.pdfDoc.getPage(i);
      const pageRef = page.ref;
      if (pageRef) {
        state.pageRefMap.set(pageRef.num + '_' + pageRef.gen, i);
      }
    } catch (e) {
      console.warn('Could not retrieve ref for page ' + i, e);
    }
  }
}

function showLoading(show) {
  el.spinner.style.display = show ? 'flex' : 'none';
}

// --------------------------------------------------------------------------
// Navigation & Zoom
// --------------------------------------------------------------------------

function queueRenderPage(num) {
  if (state.pageRendering) {
    state.pageNumPending = num;
  } else {
    renderPage(num);
  }
}

async function renderPage(num) {
  state.pageRendering = true;
  showLoading(true);
  
  try {
    const page = await state.pdfDoc.getPage(num);
    
    // Calculate Viewport Dimensions based on zoom scale
    const viewport = page.getViewport({ scale: state.zoomScale });
    
    // Set Canvas Dimensions
    el.pdfCanvas.width = viewport.width;
    el.pdfCanvas.height = viewport.height;
    el.overlayCanvas.width = viewport.width;
    el.overlayCanvas.height = viewport.height;
    
    el.canvasContainer.style.width = viewport.width + 'px';
    el.canvasContainer.style.height = viewport.height + 'px';
    
    const ctx = el.pdfCanvas.getContext('2d');
    
    // Render PDF page into canvas
    const renderContext = {
      canvasContext: ctx,
      viewport: viewport
    };
    
    await page.render(renderContext).promise;
    
    // Extract Text metrics from the rendered page
    const textContent = await page.getTextContent({ includeMarkedContent: true });
    
    // Convert PDF point coordinates into screen/canvas pixel coordinates
    state.textItems = textContent.items
      .filter(item => item && item.transform && item.str !== undefined)
      .map(item => {
        // item.transform matrix = [scaleX, skewY, skewX, scaleY, translateX, translateY]
        const tx = item.transform[4];
        const ty = item.transform[5];
        const width = item.width;
        const height = item.height || item.transform[3]; // Fallback to font size if height is missing
        
        // Convert to viewport coordinates: [left, top, right, bottom]
        const rect = viewport.convertToViewportRectangle([tx, ty, tx + width, ty + height]);
        
        // Rect returned is [x0, y0, x1, y1] where y1 > y0
        return {
          str: item.str,
          x0: Math.min(rect[0], rect[2]),
          y0: Math.min(rect[1], rect[3]),
          x1: Math.max(rect[0], rect[2]),
          y1: Math.max(rect[1], rect[3]),
          fontSize: item.transform[3] * state.zoomScale,
          fontName: item.fontName,
          direction: item.dir
        };
      }).filter(item => item.str.trim().length > 0); // Ignore pure whitespace items
    
    // Extract vector rectangles for bordered boxes first!
    state.currentPageRectangles = await extractRectangles(page, viewport);

    // Run Recursive XY-Cut Algorithm on this page!
    await runXYCutAnalysis();
    
    // Load structure tags for the current page
    loadTaggedPDFStructure();
    
  } catch (err) {
    console.error('Error rendering page:', err);
  } finally {
    state.pageRendering = false;
    showLoading(false);
    
    if (state.pageNumPending !== null) {
      const nextNum = state.pageNumPending;
      state.pageNumPending = null;
      renderPage(nextNum);
    }
  }
}

function onPrevPage() {
  if (state.pageNum <= 1) return;
  state.pageNum--;
  el.inputPageNum.value = state.pageNum;
  queueRenderPage(state.pageNum);
}

function onNextPage() {
  if (state.pageNum >= state.totalPageCount) return;
  state.pageNum++;
  el.inputPageNum.value = state.pageNum;
  queueRenderPage(state.pageNum);
}

function onPageNumInput() {
  let val = parseInt(el.inputPageNum.value, 10);
  if (isNaN(val) || val < 1) val = 1;
  if (val > state.totalPageCount) val = state.totalPageCount;
  
  state.pageNum = val;
  el.inputPageNum.value = val;
  queueRenderPage(state.pageNum);
}

function onZoomIn() {
  if (state.zoomScale >= 3.0) return;
  state.zoomScale += 0.2;
  el.txtZoomPercent.textContent = Math.round(state.zoomScale * 100) + '%';
  if (state.pdfDoc) queueRenderPage(state.pageNum);
}

function onZoomOut() {
  if (state.zoomScale <= 0.4) return;
  state.zoomScale -= 0.2;
  el.txtZoomPercent.textContent = Math.round(state.zoomScale * 100) + '%';
  if (state.pdfDoc) queueRenderPage(state.pageNum);
}

// --------------------------------------------------------------------------
// Bookmarks/Outline Extraction
// --------------------------------------------------------------------------

async function loadBookmarks() {
  el.outlineTree.innerHTML = '';
  try {
    const outline = await state.pdfDoc.getOutline();
    if (!outline || outline.length === 0) {
      el.outlineTree.innerHTML = '<div class="empty-state">No outline bookmarks in this PDF.</div>';
      return;
    }
    
    const rootUl = document.createElement('div');
    rootUl.className = 'tree-root';
    
    for (const item of outline) {
      rootUl.appendChild(await createOutlineNode(item));
    }
    
    el.outlineTree.appendChild(rootUl);
  } catch (err) {
    console.error('Error loading outline bookmarks:', err);
    el.outlineTree.innerHTML = '<div class="empty-state font-red">Errore nel caricamento dei segnalibri.</div>';
  }
}

async function createOutlineNode(item) {
  const node = document.createElement('div');
  node.className = 'tree-node';
  if (item.items && item.items.length > 0) {
    node.classList.add('has-children');
  }
  
  const header = document.createElement('div');
  header.className = 'tree-node-header';
  
  const toggle = document.createElement('span');
  toggle.className = 'tree-toggle';
  toggle.textContent = item.items && item.items.length > 0 ? '▶' : '';
  header.appendChild(toggle);
  
  const title = document.createElement('span');
  title.className = 'tree-node-title';
  title.textContent = item.title;
  header.appendChild(title);
  
  // Resolve Destination Page
  let pageNum = null;
  if (item.dest) {
    pageNum = await resolveDestPage(item.dest);
    if (pageNum) {
      const pageBadge = document.createElement('span');
      pageBadge.className = 'tree-node-badge';
      pageBadge.textContent = 'p. ' + pageNum;
      header.appendChild(pageBadge);
      
      header.addEventListener('click', (e) => {
        e.stopPropagation();
        state.pageNum = pageNum;
        el.inputPageNum.value = pageNum;
        queueRenderPage(pageNum);
        
        // Highlight outline node
        document.querySelectorAll('.tree-node-header').forEach(h => h.classList.remove('active'));
        header.classList.add('active');
      });
    }
  }
  
  node.appendChild(header);
  
  // Expand/Collapse sub-bookmarks
  if (item.items && item.items.length > 0) {
    const childrenContainer = document.createElement('div');
    childrenContainer.className = 'tree-children';
    
    for (const subItem of item.items) {
      childrenContainer.appendChild(await createOutlineNode(subItem));
    }
    node.appendChild(childrenContainer);
    
    toggle.addEventListener('click', (e) => {
      e.stopPropagation();
      node.classList.toggle('expanded');
    });
  }
  
  return node;
}

// Translates a PDF Destination Reference or string identifier into a page number
async function resolveDestPage(dest) {
  if (typeof dest === 'number') {
    return dest + 1; // PDF.js outline numbers are sometimes index-based
  }
  
  let destRef = null;
  if (typeof dest === 'string') {
    // Look up named destination
    try {
      const resolved = await state.pdfDoc.getDestination(dest);
      if (resolved && resolved.length > 0) {
        destRef = resolved[0];
      }
    } catch (e) {
      console.warn('Could not resolve named destination: ' + dest, e);
    }
  } else if (Array.isArray(dest)) {
    destRef = dest[0];
  }
  
  if (destRef && typeof destRef === 'object') {
    const key = destRef.num + '_' + destRef.gen;
    if (state.pageRefMap.has(key)) {
      return state.pageRefMap.get(key);
    }
  }
  return null;
}

// --------------------------------------------------------------------------
// Tagged PDF (StructTree) Extraction
// --------------------------------------------------------------------------

async function loadTaggedPDFStructure() {
  el.taggedTree.innerHTML = '';
  el.taggedBadge.className = 'status-badge checking';
  el.taggedBadge.textContent = 'Analyzing structure...';
  
  try {
    if (!state.pdfDoc) return;
    const page = await state.pdfDoc.getPage(state.pageNum);
    const structTree = await page.getStructTree();
    if (!structTree || !structTree.children || structTree.children.length === 0) {
      el.taggedBadge.className = 'status-badge untagged';
      el.taggedBadge.textContent = 'Untagged PDF';
      el.taggedTree.innerHTML = '<div class="empty-state">No logical structure tree (StructTree) found in this document. This is not a Tagged PDF.</div>';
      return;
    }
    
    el.taggedBadge.className = 'status-badge tagged';
    el.taggedBadge.textContent = 'Tagged PDF';
    
    const rootUl = document.createElement('div');
    rootUl.className = 'tree-root';
    
    for (const child of structTree.children) {
      rootUl.appendChild(renderStructNode(child));
    }
    
    el.taggedTree.appendChild(rootUl);
  } catch (err) {
    console.error('Error loading struct tree:', err);
    el.taggedBadge.className = 'status-badge untagged';
    el.taggedBadge.textContent = 'StructTree Error';
    el.taggedTree.innerHTML = '<div class="empty-state font-red">Errore nel caricamento della struttura dei Tag.</div>';
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
  
  // Tag Badge (e.g. H1, P, Table, Part)
  const tagBadge = document.createElement('span');
  tagBadge.className = 'tag-badge';
  const role = node.role || 'Element';
  tagBadge.textContent = role;
  
  // Apply specific styles for semantic roles
  if (role.startsWith('H') && role.length <= 3) {
    tagBadge.classList.add('header-tag');
  } else if (['P', 'Span', 'BlockQuote', 'Table', 'TR', 'TD'].includes(role)) {
    tagBadge.classList.add('block-tag');
  }
  header.appendChild(tagBadge);
  
  const title = document.createElement('span');
  title.className = 'tree-node-title';
  title.textContent = node.title || (isLeaf ? 'Content Span' : 'Section Block');
  header.appendChild(title);
  
  li.appendChild(header);
  
  if (!isLeaf) {
    li.classList.add('has-children');
    const childrenContainer = document.createElement('div');
    childrenContainer.className = 'tree-children';
    
    for (const child of node.children) {
      childrenContainer.appendChild(renderStructNode(child));
    }
    
    li.appendChild(childrenContainer);
    
    toggle.addEventListener('click', (e) => {
      e.stopPropagation();
      li.classList.toggle('expanded');
    });
  }
  
  return li;
}

// --------------------------------------------------------------------------
// Recursive XY-Cut execution
// --------------------------------------------------------------------------

async function runXYCutAnalysis() {
  if (state.textItems.length === 0) {
    el.domTreeContainer.innerHTML = '<div class="empty-state">No readable text contents found on this page.</div>';
    clearOverlay();
    return;
  }
  
  const pageBounds = {
    x: 0,
    y: 0,
    w: el.pdfCanvas.width,
    h: el.pdfCanvas.height
  };
  
  if (el.chkAutoGutter && el.chkAutoGutter.checked && window.__TAURI__) {
    const rustItems = state.textItems.map(item => ({
      text: item.str,
      x: item.x0,
      y: item.y0,
      width: item.x1 - item.x0,
      height: item.y1 - item.y0,
      font_size: item.fontSize,
      font_name: item.fontName || null
    }));

    const rustPageBounds = {
      x: pageBounds.x,
      y: pageBounds.y,
      w: pageBounds.w,
      h: pageBounds.h
    };

    const rustBorderedBoxes = state.currentPageRectangles.map(box => ({
      x: box.x,
      y: box.y,
      w: box.w,
      h: box.h
    }));

    showLoading(true);
    try {
      const jsonString = await window.__TAURI__.core.invoke('analyze_layout_auto_gutter', {
        items: rustItems,
        pageBounds: rustPageBounds,
        borderedBoxes: rustBorderedBoxes,
        strategy: el.selectAutoStrategy.value
      });
      state.xycutResult = JSON.parse(jsonString);

      // If Rust returned projection gaps, use them to set the Row/Column Gap UI
      try {
        if (state.xycutResult && state.xycutResult.projections) {
          const { xGaps = [], yGaps = [] } = state.xycutResult.projections;

          // Filter out spurious tiny gaps (<5 px) and prefer the smallest meaningful gap
          const significantThreshold = 5; // px
          const significantXGaps = xGaps.filter(g => g.size >= significantThreshold);
          const significantYGaps = yGaps.filter(g => g.size >= significantThreshold);

          const colGap = significantXGaps.length ? Math.min(...significantXGaps.map(g => g.size)) : (xGaps.length ? Math.min(...xGaps.map(g => g.size)) : null);
          const rowGap = significantYGaps.length ? Math.min(...significantYGaps.map(g => g.size)) : (yGaps.length ? Math.min(...yGaps.map(g => g.size)) : null);

          if (colGap !== null) {
            // state.thresholdX is stored unscaled; JS alg multiplies by zoomScale when used
            state.thresholdX = Math.max(1, Math.round(colGap / state.zoomScale));
            if (el.rangeX) el.rangeX.value = state.thresholdX;
            if (el.valX) el.valX.textContent = state.thresholdX + ' px';
          }

          if (rowGap !== null) {
            state.thresholdY = Math.max(1, Math.round(rowGap / state.zoomScale));
            if (el.rangeY) el.rangeY.value = state.thresholdY;
            if (el.valY) el.valY.textContent = state.thresholdY + ' px';
          }

          // Auto-detect works best with Vertical First (columns before rows)
          state.priority = 'X';
          if (el.selectPriority) {
            el.selectPriority.value = 'X';
          }

          // Disable auto-detect checkbox and re-analyze with the auto-detected parameters
          if (el.chkAutoGutter) {
            state.programmaticCheckboxChange = true;
            el.chkAutoGutter.checked = false;
          }
          // Re-run analysis with the new parameters to apply them properly
          const reanalysisOptions = {
            thresholdX: state.thresholdX * state.zoomScale,
            thresholdY: state.thresholdY * state.zoomScale,
            minWidth: state.minWidth * state.zoomScale,
            minHeight: state.minHeight * state.zoomScale,
            priority: state.priority,
            borderedBoxes: state.currentPageRectangles
          };
          state.xycutResult = performXYCut(state.textItems, pageBounds, reanalysisOptions);
        }
      } catch (e) {
        console.warn('Error applying auto-detected gaps to UI:', e);
      }
    } catch (err) {
      console.error('Failed to run Rust XY-Cut auto gutter:', err);
      // Fallback to JS XY-Cut
      const options = {
        thresholdX: state.thresholdX * state.zoomScale,
        thresholdY: state.thresholdY * state.zoomScale,
        minWidth: state.minWidth * state.zoomScale,
        minHeight: state.minHeight * state.zoomScale,
        priority: state.priority,
        borderedBoxes: state.currentPageRectangles
      };
      state.xycutResult = performXYCut(state.textItems, pageBounds, options);
    } finally {
      showLoading(false);
    }
  } else {
    const options = {
      thresholdX: state.thresholdX * state.zoomScale,
      thresholdY: state.thresholdY * state.zoomScale,
      minWidth: state.minWidth * state.zoomScale,
      minHeight: state.minHeight * state.zoomScale,
      priority: state.priority,
      borderedBoxes: state.currentPageRectangles
    };
    
    // Trigger RXY-Cut algorithm
    state.xycutResult = performXYCut(state.textItems, pageBounds, options);
  }
  
  state.selectedBlockId = null;
  state.hoveredBlockId = null;
  
  // Build visual DOM Tree inspector in right panel
  renderLayoutDOMTree();
  
  // Refresh overlay graphics
  drawOverlay();
  
  // Clear inspector details
  el.inspectorPanel.innerHTML = '<div class="empty-state">Seleziona un blocco XY-Cut per ispezionarlo.</div>';
}

// --------------------------------------------------------------------------
// Rendering Overlays (Cuts, Projection profiles & Bounding Boxes)
// --------------------------------------------------------------------------

function clearOverlay() {
  const ctx = el.overlayCanvas.getContext('2d');
  ctx.clearRect(0, 0, el.overlayCanvas.width, el.overlayCanvas.height);
}

function drawOverlay() {
  if (!state.xycutResult) return;
  
  const ctx = el.overlayCanvas.getContext('2d');
  ctx.clearRect(0, 0, el.overlayCanvas.width, el.overlayCanvas.height);
  
  // 1. Draw Raw Character/Word Boxes
  if (el.chkShowChars.checked) {
    ctx.strokeStyle = 'rgba(56, 189, 248, 0.2)';
    ctx.fillStyle = 'rgba(56, 189, 248, 0.03)';
    ctx.lineWidth = 1;
    for (const item of state.textItems) {
      ctx.fillRect(item.x0, item.y0, item.x1 - item.x0, item.y1 - item.y0);
      ctx.strokeRect(item.x0, item.y0, item.x1 - item.x0, item.y1 - item.y0);
    }
  }
  
  // 2. Draw Recursive Cut Blocks (color-coded by depth)
  if (el.chkShowBlocks.checked) {
    drawBlockRecursive(ctx, state.xycutResult.root);
  }
  
  // 3. Draw Hovered & Selected Highlighting overlays
  if (state.hoveredBlockId) {
    const hoveredNode = findBlockById(state.xycutResult.root, state.hoveredBlockId);
    if (hoveredNode) {
      ctx.strokeStyle = 'rgba(244, 63, 94, 0.85)'; // Crimson rose accent
      ctx.fillStyle = 'rgba(244, 63, 94, 0.08)';
      ctx.lineWidth = 2.5;
      const b = hoveredNode.bounds;
      ctx.fillRect(b.x, b.y, b.w, b.h);
      ctx.strokeRect(b.x, b.y, b.w, b.h);
    }
  }
  
  if (state.selectedBlockId) {
    const selectedNode = findBlockById(state.xycutResult.root, state.selectedBlockId);
    if (selectedNode) {
      ctx.strokeStyle = 'var(--accent)'; // Cyan selected focus
      ctx.fillStyle = 'rgba(56, 189, 248, 0.12)';
      ctx.lineWidth = 3;
      // Draw corner brackets to look highly premium!
      const b = selectedNode.bounds;
      ctx.fillRect(b.x, b.y, b.w, b.h);
      ctx.strokeRect(b.x, b.y, b.w, b.h);
    }
  }
  
  // 4. Draw Projection Profile density curves on bottom & right margins!
  if (el.chkShowProjections.checked) {
    drawProjectionProfiles(ctx);
  }
}

// Recursively draws the block boundaries color coded by depth
function drawBlockRecursive(ctx, block) {
  if (block.isBorderedBox) {
    ctx.strokeStyle = 'var(--accent-purple)';
    ctx.fillStyle = 'rgba(192, 132, 252, 0.04)';
    ctx.lineWidth = 2.5;
    ctx.fillRect(block.bounds.x, block.bounds.y, block.bounds.w, block.bounds.h);
    ctx.strokeRect(block.bounds.x, block.bounds.y, block.bounds.w, block.bounds.h);
    
    if (block.children) {
      for (const child of block.children) {
        drawBlockRecursive(ctx, child);
      }
    }
    return;
  }

  if (block.type === 'leaf') {
    // Leaf block
    ctx.strokeStyle = 'rgba(52, 211, 153, 0.65)'; // Mint Green for leaves
    ctx.fillStyle = 'rgba(52, 211, 153, 0.02)';
    ctx.lineWidth = 1.5;
    ctx.fillRect(block.bounds.x, block.bounds.y, block.bounds.w, block.bounds.h);
    ctx.strokeRect(block.bounds.x, block.bounds.y, block.bounds.w, block.bounds.h);
    return;
  }
  
  // Draw container bounding box with subtle dashed line
  ctx.strokeStyle = `hsla(${(block.depth * 55) % 360}, 75%, 65%, 0.4)`;
  ctx.lineWidth = Math.max(1, 3 - block.depth * 0.5);
  ctx.strokeRect(block.bounds.x, block.bounds.y, block.bounds.w, block.bounds.h);
  
  if (block.children) {
    for (const child of block.children) {
      drawBlockRecursive(ctx, child);
    }
  }
}

// Renders the math projection gaps overlay
function drawProjectionProfiles(ctx) {
  const w = el.overlayCanvas.width;
  const h = el.overlayCanvas.height;
  
  const marginSize = 25; // Margin overlay size
  
  ctx.fillStyle = 'rgba(0, 0, 0, 0.55)';
  
  // Bottom horizontal band for X Projection (Vertical Gaps)
  ctx.fillRect(0, h - marginSize, w, marginSize);
  // Right vertical band for Y Projection (Horizontal Gaps)
  ctx.fillRect(w - marginSize, 0, marginSize, h);
  
  ctx.strokeStyle = 'rgba(255,255,255,0.15)';
  ctx.lineWidth = 1;
  ctx.strokeRect(0, h - marginSize, w, marginSize);
  ctx.strokeRect(w - marginSize, 0, marginSize, h);
  
  // Calculate vertical projection densities (overlapping blocks)
  const xProfile = new Array(Math.ceil(w)).fill(0);
  const yProfile = new Array(Math.ceil(h)).fill(0);
  
  for (const item of state.textItems) {
    const x0 = Math.max(0, Math.floor(item.x0));
    const x1 = Math.min(w - 1, Math.ceil(item.x1));
    for (let x = x0; x <= x1; x++) {
      xProfile[x] = (xProfile[x] || 0) + 1;
    }
    
    const y0 = Math.max(0, Math.floor(item.y0));
    const y1 = Math.min(h - 1, Math.ceil(item.y1));
    for (let y = y0; y <= y1; y++) {
      yProfile[y] = (yProfile[y] || 0) + 1;
    }
  }
  
  // Find max values for scale
  const maxXVal = Math.max(...xProfile, 1);
  const maxYVal = Math.max(...yProfile, 1);
  
  // Draw X Projection Profile (Horizontal bottom band)
  ctx.fillStyle = 'rgba(56, 189, 248, 0.45)'; // Accent Cyan
  for (let x = 0; x < w; x++) {
    const val = xProfile[x] || 0;
    if (val > 0) {
      const barH = (val / maxXVal) * (marginSize - 4);
      ctx.fillRect(x, h - barH - 2, 1, barH);
    }
  }
  
  // Draw Y Projection Profile (Vertical right band)
  ctx.fillStyle = 'rgba(192, 132, 252, 0.45)'; // Accent Purple
  for (let y = 0; y < h; y++) {
    const val = yProfile[y] || 0;
    if (val > 0) {
      const barW = (val / maxYVal) * (marginSize - 4);
      ctx.fillRect(w - barW - 2, y, barW, 1);
    }
  }
  
  // Draw Gap Lines
  ctx.fillStyle = 'rgba(251, 191, 36, 0.25)'; // Amber
  if (state.xycutResult.projections) {
    const { xGaps, yGaps } = state.xycutResult.projections;
    
    // Draw columns vertical gap intervals
    for (const gap of xGaps) {
      ctx.fillRect(gap.start, 0, gap.end - gap.start, h - marginSize);
      ctx.strokeStyle = 'rgba(251, 191, 36, 0.4)';
      ctx.beginPath();
      ctx.moveTo(gap.start, 0); ctx.lineTo(gap.start, h - marginSize);
      ctx.moveTo(gap.end, 0); ctx.lineTo(gap.end, h - marginSize);
      ctx.stroke();
    }
    
    // Draw rows horizontal gap intervals
    for (const gap of yGaps) {
      ctx.fillRect(0, gap.start, w - marginSize, gap.end - gap.start);
      ctx.strokeStyle = 'rgba(251, 191, 36, 0.4)';
      ctx.beginPath();
      ctx.moveTo(0, gap.start); ctx.lineTo(w - marginSize, gap.start);
      ctx.moveTo(0, gap.end); ctx.lineTo(w - marginSize, gap.end);
      ctx.stroke();
    }
  }
}

// --------------------------------------------------------------------------
// Generated Layout DOM Hierarchy rendering
// --------------------------------------------------------------------------

function renderLayoutDOMTree() {
  el.domTreeContainer.innerHTML = '';
  if (!state.xycutResult) return;
  
  const rootNode = state.xycutResult.root;
  const treeContainer = document.createElement('div');
  treeContainer.className = 'dom-tree-root';
  
  treeContainer.appendChild(buildDOMNodeHTML(rootNode));
  el.domTreeContainer.appendChild(treeContainer);
}

function buildDOMNodeHTML(node) {
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
  
  // Format labels nicely
  if (node.type === 'root') {
    label.textContent = 'DocumentRoot';
    div.classList.add('expanded'); // Auto expand root
  } else if (node.isBorderedBox) {
    label.textContent = 'BorderedBox';
    div.classList.add('bordered-box-node');
  } else if (node.type === 'leaf') {
    label.textContent = 'TextLeaf';
  } else {
    label.textContent = `Block_${node.cutDirection || 'Cut'}`;
  }
  header.appendChild(label);
  
  // Add a small snippet of text if it's a leaf
  if (node.type === 'leaf') {
    const snippet = document.createElement('span');
    snippet.className = 'dom-node-text-clip';
    snippet.textContent = `"${node.text.substring(0, 30)}${node.text.length > 30 ? '...' : ''}"`;
    header.appendChild(snippet);
  }
  
  div.appendChild(header);
  
  // Mouse events inside list items
  header.addEventListener('mouseenter', (e) => {
    e.stopPropagation();
    state.hoveredBlockId = node.id;
    drawOverlay();
    
    // Highlight list node visually
    header.classList.add('hovered');
  });
  
  header.addEventListener('mouseleave', (e) => {
    e.stopPropagation();
    state.hoveredBlockId = null;
    drawOverlay();
    header.classList.remove('hovered');
  });
  
  header.addEventListener('click', (e) => {
    e.stopPropagation();
    selectDOMNode(node.id);
  });
  
  if (node.children && node.children.length > 0) {
    const childrenContainer = document.createElement('div');
    childrenContainer.className = 'dom-node-children';
    
    for (const child of node.children) {
      childrenContainer.appendChild(buildDOMNodeHTML(child));
    }
    
    div.appendChild(childrenContainer);
    
    toggle.addEventListener('click', (e) => {
      e.stopPropagation();
      div.classList.toggle('expanded');
    });
  }
  
  return div;
}

// Synchronized Selection
function selectDOMNode(nodeId) {
  state.selectedBlockId = nodeId;
  drawOverlay();
  
  // Highlight list element
  document.querySelectorAll('.dom-node-header').forEach(h => h.classList.remove('active'));
  const targetHeader = document.querySelector(`.dom-node[data-id="${nodeId}"] > .dom-node-header`);
  if (targetHeader) {
    targetHeader.classList.add('active');
    
    // Ensure parent nodes are expanded
    let parent = targetHeader.parentElement.parentElement;
    while (parent && parent.classList.contains('dom-node-children')) {
      const parentBlock = parent.parentElement;
      if (parentBlock) {
        parentBlock.classList.add('expanded');
      }
      parent = parentBlock.parentElement;
    }
    
    targetHeader.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }
  
  // Update properties inspector
  const block = findBlockById(state.xycutResult.root, nodeId);
  if (block) {
    populateInspector(block);
  }
}

// --------------------------------------------------------------------------
// Properties Formatting Inspector
// --------------------------------------------------------------------------

function populateInspector(block) {
  el.inspectorPanel.innerHTML = '';
  
  const container = document.createElement('div');
  container.className = 'inspector-layout';
  
  const grid = document.createElement('div');
  grid.className = 'inspector-grid';
  
  // Bounds
  const xTL = Math.round(block.bounds.x);
  const yTL = Math.round(block.bounds.y);
  const xBR = Math.round(block.bounds.x + block.bounds.w);
  const yBR = Math.round(block.bounds.y + block.bounds.h);

  grid.appendChild(createInspectorCard('ID & Type', block.id + ` (${block.type})`));
  grid.appendChild(createInspectorCard('Dimensions', `${Math.round(block.bounds.w)}x${Math.round(block.bounds.h)} px`));
  grid.appendChild(createInspectorCard('Coordinates', `TL: (${xTL}px, ${yTL}px) | BR: (${xBR}px, ${yBR}px)`));
  grid.appendChild(createInspectorCard('Hierarchy Depth', `Depth Level ${block.depth}`));
  
  if (block.type === 'leaf') {
    // Dominant Formatting parameters
    const f = block.formatting;
    grid.appendChild(createInspectorCard('Dominant Font', f.fontFamily, 'font-desc'));
    grid.appendChild(createInspectorCard('Average Size', `${f.avgFontSize} px (DOM Size: ${f.fontSize}px)`));
    grid.appendChild(createInspectorCard('Style / Weight', `${f.fontWeight} / ${f.fontStyle}`));
    grid.appendChild(createInspectorCard('Text Align', f.alignment.toUpperCase()));
    
    container.appendChild(grid);
    
    // Contained Text Box
    const textTitle = document.createElement('div');
    textTitle.className = 'inspector-card-label';
    textTitle.style.marginTop = '8px';
    textTitle.textContent = 'Contained Text Content:';
    container.appendChild(textTitle);
    
    const textBox = document.createElement('textarea');
    textBox.className = 'inspector-text-box';
    textBox.readOnly = true;
    textBox.value = block.text;
    textBox.title = 'Clicca per copiare';
    
    textBox.addEventListener('click', () => {
      textBox.select();
      navigator.clipboard.writeText(block.text);
      // Small visual confirm toast
      const oldLabel = textTitle.textContent;
      textTitle.textContent = '✓ Copiato negli appunti!';
      textTitle.style.color = 'var(--accent-green)';
      setTimeout(() => {
        textTitle.textContent = oldLabel;
        textTitle.style.color = '';
      }, 1500);
    });
    
    container.appendChild(textBox);
  } else {
    // Container block specifics
    grid.appendChild(createInspectorCard('Partition Split', `Along ${block.cutDirection}-Axis`, 'span-2'));
    grid.appendChild(createInspectorCard('Child Branches', `${block.children ? block.children.length : 0} blocks`, 'span-2'));
    container.appendChild(grid);
  }
  
  el.inspectorPanel.appendChild(container);
}

function createInspectorCard(label, val, extraClass = '') {
  const card = document.createElement('div');
  card.className = 'inspector-card';
  if (extraClass) card.classList.add(extraClass);
  
  const lblDiv = document.createElement('div');
  lblDiv.className = 'inspector-card-label';
  lblDiv.textContent = label;
  
  const valDiv = document.createElement('div');
  valDiv.className = 'inspector-card-val';
  valDiv.textContent = val;
  
  card.appendChild(lblDiv);
  card.appendChild(valDiv);
  return card;
}

// --------------------------------------------------------------------------
// Bi-directional interaction logic on Canvas
// --------------------------------------------------------------------------

function handleCanvasMouseMove(e) {
  if (!state.xycutResult) return;
  
  const rect = el.overlayCanvas.getBoundingClientRect();
  const scaleX = el.overlayCanvas.width / rect.width;
  const scaleY = el.overlayCanvas.height / rect.height;
  
  // Calculate relative pixel coordinates inside canvas
  const canvasX = (e.clientX - rect.left) * scaleX;
  const canvasY = (e.clientY - rect.top) * scaleY;
  
  // Find deepest leaf/container node enclosing this point
  const matchedBlock = findDeepestBlockAt(state.xycutResult.root, canvasX, canvasY);
  
  if (matchedBlock) {
    if (state.hoveredBlockId !== matchedBlock.id) {
      state.hoveredBlockId = matchedBlock.id;
      drawOverlay();
      
      // Update DOM list highlights
      document.querySelectorAll('.dom-node-header').forEach(h => h.classList.remove('hovered'));
      const header = document.querySelector(`.dom-node[data-id="${matchedBlock.id}"] > .dom-node-header`);
      if (header) {
        header.classList.add('hovered');
      }
    }
  } else {
    if (state.hoveredBlockId !== null) {
      state.hoveredBlockId = null;
      drawOverlay();
      document.querySelectorAll('.dom-node-header').forEach(h => h.classList.remove('hovered'));
    }
  }
}

function handleCanvasMouseOut() {
  if (state.hoveredBlockId !== null) {
    state.hoveredBlockId = null;
    drawOverlay();
    document.querySelectorAll('.dom-node-header').forEach(h => h.classList.remove('hovered'));
  }
}

function handleCanvasMouseClick(e) {
  if (state.hoveredBlockId) {
    selectDOMNode(state.hoveredBlockId);
  }
}

// --------------------------------------------------------------------------
// Block Search Utilities
// --------------------------------------------------------------------------

function findBlockById(root, id) {
  if (root.id === id) return root;
  if (root.children) {
    for (const child of root.children) {
      const found = findBlockById(child, id);
      if (found) return found;
    }
  }
  return null;
}

function findDeepestBlockAt(root, x, y) {
  const b = root.bounds;
  // If point is outside block, ignore branch
  if (x < b.x || x > b.x + b.w || y < b.y || y > b.y + b.h) {
    return null;
  }
  
  if (root.children && root.children.length > 0) {
    for (const child of root.children) {
      const matched = findDeepestBlockAt(child, x, y);
      if (matched) return matched;
    }
  }
  
  // Return the leaf (or deepest container) that encompasses the coordinate
  return root;
}

// --------------------------------------------------------------------------
// Exports
// --------------------------------------------------------------------------

function exportLayoutDOM(format) {
  if (!state.xycutResult) {
    alert('Nessun layout analizzato. Carica prima un PDF!');
    return;
  }
  
  try {
    const content = serializeLayoutTree(state.xycutResult.root, format);
    
    // Create a virtual download link
    const mimeMap = {
      'json': 'application/json',
      'xml': 'application/xml',
      'html': 'text/html'
    };
    
    const blob = new Blob([content], { type: mimeMap[format] || 'text/plain' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = `pdf_layout_page_${state.pageNum}.${format}`;
    document.body.appendChild(link);
    link.click();
    
    document.body.removeChild(link);
    URL.revokeObjectURL(url);
  } catch (err) {
    console.error('Error exporting layout:', err);
    alert('Errore durante l\'esportazione del file: ' + err.message);
  }
}

// --------------------------------------------------------------------------
// Custom Resizable Sidebars Drag Logic
// --------------------------------------------------------------------------

function setupResizableSidebars() {
  const container = document.querySelector('.app-container');
  const leftResizer = document.getElementById('left-resizer');
  const rightResizer = document.getElementById('right-resizer');
  const inspectorSection = document.getElementById('inspector-section');
  const inspectorResizer = document.getElementById('inspector-resizer');
  
  const MIN_LEFT_WIDTH = 320;
  const MIN_RIGHT_WIDTH = 340;
  const MIN_INSPECTOR_HEIGHT = 120;
  const MAX_INSPECTOR_HEIGHT = 450;
  
  leftResizer.addEventListener('mousedown', (e) => {
    e.preventDefault();
    leftResizer.classList.add('dragging');
    document.body.style.cursor = 'col-resize';
    
    function onMouseMove(moveEvent) {
      let newWidth = moveEvent.clientX - 16;
      if (newWidth < MIN_LEFT_WIDTH) newWidth = MIN_LEFT_WIDTH;
      const maxLeft = window.innerWidth * 0.45;
      if (newWidth > maxLeft) newWidth = maxLeft;
      
      container.style.setProperty('--left-width', newWidth + 'px');
    }
    
    function onMouseUp() {
      leftResizer.classList.remove('dragging');
      document.body.style.cursor = '';
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
    }
    
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  });
  
  rightResizer.addEventListener('mousedown', (e) => {
    e.preventDefault();
    rightResizer.classList.add('dragging');
    document.body.style.cursor = 'col-resize';
    
    function onMouseMove(moveEvent) {
      let newWidth = window.innerWidth - moveEvent.clientX - 16;
      if (newWidth < MIN_RIGHT_WIDTH) newWidth = MIN_RIGHT_WIDTH;
      const maxRight = window.innerWidth * 0.45;
      if (newWidth > maxRight) newWidth = maxRight;
      
      container.style.setProperty('--right-width', newWidth + 'px');
    }
    
    function onMouseUp() {
      rightResizer.classList.remove('dragging');
      document.body.style.cursor = '';
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
    }
    
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  });

  inspectorResizer.addEventListener('mousedown', (e) => {
    e.preventDefault();
    inspectorResizer.classList.add('dragging');
    document.body.style.cursor = 'row-resize';
    
    const startY = e.clientY;
    const startHeight = inspectorSection.getBoundingClientRect().height;
    
    function onMouseMove(moveEvent) {
      let newHeight = startHeight - (moveEvent.clientY - startY);
      if (newHeight < MIN_INSPECTOR_HEIGHT) newHeight = MIN_INSPECTOR_HEIGHT;
      if (newHeight > MAX_INSPECTOR_HEIGHT) newHeight = MAX_INSPECTOR_HEIGHT;
      
      container.style.setProperty('--inspector-height', newHeight + 'px');
    }
    
    function onMouseUp() {
      inspectorResizer.classList.remove('dragging');
      document.body.style.cursor = '';
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
    }
    
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  });
}

// --------------------------------------------------------------------------
// Rectangle Extraction from Page Vector Operations
// --------------------------------------------------------------------------

async function extractRectangles(page, viewport) {
  const rectangles = [];
  try {
    const opList = await page.getOperatorList();
    let currentPath = [];
    
    for (let i = 0; i < opList.fnArray.length; i++) {
      const fn = opList.fnArray[i];
      const args = opList.argsArray[i];
      
      if (fn === pdfjsLib.OPS.constructPath) {
        const pathOps = args[0];
        const pathArgs = args[1];
        
        if (pathOps && typeof pathOps.length === 'number') {
          let argIdx = 0;
          for (let k = 0; k < pathOps.length; k++) {
            const op = pathOps[k];
            if (op === 13) { // OPS.rectangle (re)
              const x = pathArgs[argIdx++];
              const y = pathArgs[argIdx++];
              const w = pathArgs[argIdx++];
              const h = pathArgs[argIdx++];
              
              const screenRect = viewport.convertToViewportRectangle([x, y, x + w, y + h]);
              rectangles.push({
                x: Math.min(screenRect[0], screenRect[2]),
                y: Math.min(screenRect[1], screenRect[3]),
                w: Math.abs(screenRect[2] - screenRect[0]),
                h: Math.abs(screenRect[3] - screenRect[1])
              });
            } else if (op === 1) { // OPS.moveTo (m)
              const x = pathArgs[argIdx++];
              const y = pathArgs[argIdx++];
              currentPath = [{ x, y }];
            } else if (op === 2) { // OPS.lineTo (l)
              const x = pathArgs[argIdx++];
              const y = pathArgs[argIdx++];
              currentPath.push({ x, y });
            } else if (op === 5) { // OPS.closePath (h)
              if (currentPath.length === 4 || currentPath.length === 5) {
                const xs = currentPath.map(p => p.x);
                const ys = currentPath.map(p => p.y);
                const minX = Math.min(...xs);
                const maxX = Math.max(...xs);
                const minY = Math.min(...ys);
                const maxY = Math.max(...ys);
                
                const w = maxX - minX;
                const h = maxY - minY;
                if (w > 10 && h > 10) {
                  const screenRect = viewport.convertToViewportRectangle([minX, minY, maxX, maxY]);
                  rectangles.push({
                    x: Math.min(screenRect[0], screenRect[2]),
                    y: Math.min(screenRect[1], screenRect[3]),
                    w: Math.abs(screenRect[2] - screenRect[0]),
                    h: Math.abs(screenRect[3] - screenRect[1])
                  });
                }
              }
              currentPath = [];
            }
          }
        }
      }
    }
  } catch (err) {
    console.warn('Error extracting rectangles from vector paths:', err);
  }
  
  const pageWidth = viewport.width;
  const pageHeight = viewport.height;
  
  return rectangles.filter(r => {
    return r.w > 20 && r.h > 10 && r.w < pageWidth * 0.98 && r.h < pageHeight * 0.98;
  });
}
