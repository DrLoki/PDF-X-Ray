/**
 * Translation layer: pseudo-translation for layout testing and rendering of
 * translated paragraph pieces over the original text blocks.
 */

const ACCENTS = {
  a: 'á', e: 'é', i: 'í', o: 'ö', u: 'ü', c: 'ç', n: 'ñ', y: 'ý', s: 'š', z: 'ž',
  A: 'Á', E: 'É', I: 'Í', O: 'Ö', U: 'Ü', C: 'Ç', N: 'Ñ', Y: 'Ý', S: 'Š', Z: 'Ž',
};

/**
 * Pseudo-localisation: accented letters plus ~30% expansion (typical of
 * EN→IT/DE/FR), so that re-flow and fit-to-box can be tested without a
 * translation engine. Paragraph boundaries are visible as ⟦ … ⟧.
 */
export function pseudoTranslate(text) {
  let vowelCount = 0;
  const body = text.replace(/[A-Za-z]/g, ch => {
    const acc = ACCENTS[ch] || ch;
    if ('aeiouAEIOU'.includes(ch) && (++vowelCount % 3 === 0)) return acc + acc.toLowerCase();
    return acc;
  });
  return `⟦${body}⟧`;
}

function genericFamily(fontName) {
  const n = (fontName || '').toLowerCase();
  if (/mono|courier|consol|code/.test(n)) return 'ui-monospace, "JetBrains Mono", monospace';
  if (/sans|arial|helvetica|inter|roboto|verdana|segoe|gothic|grotesk|futura|myriad|frutiger|laski/.test(n)) return 'Inter, Arial, sans-serif';
  return 'Georgia, "Times New Roman", serif';
}

/** Shrink the font until the text fits in its box (min 45% of the original). */
function fitText(el, startPx) {
  let fs = startPx;
  const min = startPx * 0.45;
  el.style.fontSize = fs + 'px';
  let guard = 0;
  while ((el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1) && fs > min && guard++ < 40) {
    fs *= 0.93;
    el.style.fontSize = fs + 'px';
  }
  el.dataset.scale = (fs / startPx).toFixed(2);
  if (el.scrollHeight > el.clientHeight + 1) el.classList.add('overflow');
  return fs;
}

// ---------------------------------------------------------------------------
// Comic balloons: the translation replaces the lettering inside the shape
// ---------------------------------------------------------------------------

const COMIC_FONT = '"Comic Sans MS", "Comic Neue", "Segoe UI", sans-serif';
const COMIC_WEIGHT = 600;
const BALLOON_LINE_HEIGHT = 1.12; // lettering is tightly leaded
const BALLOON_PAD = 0.12;         // inner margin, in ems
const MIN_SCALE = 0.45;           // smallest font, relative to the original lettering

let measureCtx = null;
function measure(text, fs, font) {
  if (!measureCtx) measureCtx = document.createElement('canvas').getContext('2d');
  measureCtx.font = `${COMIC_WEIGHT} ${fs}px ${font}`;
  return measureCtx.measureText(text).width;
}

/** Horizontal extent [x0, x1] of the shape at `y`, interpolated between rows; null outside. */
export function spanAt(shape, y) {
  if (!shape.length || y < shape[0][0] || y > shape[shape.length - 1][0]) return null;
  if (shape.length === 1) return [shape[0][1], shape[0][2]];
  for (let i = 0; i + 1 < shape.length; i++) {
    const a = shape[i], b = shape[i + 1];
    if (y >= a[0] && y <= b[0]) {
      const t = b[0] === a[0] ? 0 : (y - a[0]) / (b[0] - a[0]);
      return [a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    }
  }
  return null;
}

/** Usable extent of a line between `top` and `bottom`: the narrowest of its top, middle and bottom. */
function lineSpan(shape, top, bottom, pad) {
  const spans = [spanAt(shape, top), spanAt(shape, (top + bottom) / 2), spanAt(shape, bottom)];
  if (spans.some(s => !s)) return null;
  const x0 = Math.max(...spans.map(s => s[0])) + pad;
  const x1 = Math.min(...spans.map(s => s[1])) - pad;
  return x1 > x0 ? [x0, x1] : null;
}

/**
 * Wrap `words` inside `shape` at font size `fs` (points): n lines centred
 * on `anchorY`, each as wide as the shape allows at its height; the fewest
 * lines that hold every word. Returns the lines, or null when they do not fit.
 */
export function wrapInShape(words, shape, anchorY, fs, font, measureFn = measure) {
  const lh = fs * BALLOON_LINE_HEIGHT;
  const pad = fs * BALLOON_PAD;
  const top = shape[0][0] + pad * 0.5;
  const bottom = shape[shape.length - 1][0] - pad * 0.5;
  for (let n = 1; n * lh <= bottom - top; n++) {
    const y0 = Math.max(top, Math.min(anchorY - (n * lh) / 2, bottom - n * lh));
    const lines = [];
    let wi = 0;
    for (let i = 0; i < n && wi < words.length; i++) {
      const yt = y0 + i * lh;
      // the letters sit in the middle of the line box
      const span = lineSpan(shape, yt + lh * 0.12, yt + lh * 0.88, pad);
      if (!span) continue;
      const width = span[1] - span[0];
      const cur = [];
      while (wi < words.length && measureFn([...cur, words[wi]].join(' '), fs, font) <= width) cur.push(words[wi++]);
      if (cur.length) lines.push({ text: cur.join(' '), y: yt, x0: span[0], x1: span[1] });
    }
    if (wi >= words.length) return { lines, fs, lh };
  }
  return null;
}

/** Shrink the font until the words fit the shape (down to MIN_SCALE of the lettering). */
export function fitInShape(text, shape, anchorY, startFs, font, measureFn = measure) {
  const words = text.split(/\s+/).filter(Boolean);
  if (!words.length || shape.length < 2) return null;
  for (let fs = startFs; fs >= startFs * MIN_SCALE; fs *= 0.93) {
    const r = wrapInShape(words, shape, anchorY, fs, font, measureFn);
    if (r) return r;
  }
  return null;
}

function inkFor(fill) {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(fill || '');
  if (!m) return '#111';
  const [r, g, b] = m.slice(1).map(h => parseInt(h, 16));
  return 0.3 * r + 0.59 * g + 0.11 * b > 140 ? '#111' : '#fafafa';
}

/**
 * Draw a translated balloon: the inside of the shape is painted with the
 * balloon colour (hiding the original lettering) and the translation is
 * wrapped inside it. Returns the font size used, or null when the text does
 * not fit (the caller falls back to the rectangle of the segment).
 */
function renderBalloon(layer, seg, text, zoom, paraId, selected, title) {
  const bl = seg.balloon;
  const font = COMIC_FONT;
  const fit = fitInShape(text, bl.shape, seg.bounds.y + seg.bounds.h / 2, seg.fontSize, font);
  if (!fit) return null;
  const b = bl.bounds;
  const wrap = document.createElement('div');
  wrap.className = 'tr-balloon';
  if (selected) wrap.classList.add('selected');
  wrap.dataset.paraId = paraId;
  wrap.title = title;
  wrap.style.left = b.x * zoom + 'px';
  wrap.style.top = b.y * zoom + 'px';
  wrap.style.width = b.w * zoom + 'px';
  wrap.style.height = b.h * zoom + 'px';
  // cover: the shape as a polygon (left edges down, right edges up), a hair wider
  const cover = document.createElement('div');
  cover.className = 'tr-balloon-cover';
  const e = 0.6;
  const pts = [
    ...bl.shape.map(r => [r[1] - e, r[0]]),
    ...bl.shape.slice().reverse().map(r => [r[2] + e, r[0]]),
  ].map(([x, y]) => `${((x - b.x) * zoom).toFixed(1)}px ${((y - b.y) * zoom).toFixed(1)}px`);
  cover.style.clipPath = `polygon(${pts.join(', ')})`;
  cover.style.background = bl.fill || '#fff';
  wrap.appendChild(cover);
  const ink = inkFor(bl.fill);
  for (const l of fit.lines) {
    const d = document.createElement('div');
    d.className = 'tr-balloon-line';
    d.style.left = (l.x0 - b.x) * zoom + 'px';
    d.style.width = (l.x1 - l.x0) * zoom + 'px';
    d.style.top = (l.y - b.y) * zoom + 'px';
    d.style.height = fit.lh * zoom + 'px';
    d.style.lineHeight = fit.lh * zoom + 'px';
    d.style.fontSize = fit.fs * zoom + 'px';
    d.style.fontFamily = font;
    d.style.fontWeight = String(COMIC_WEIGHT);
    d.style.textAlign = bl.kind === 'narration' ? 'left' : 'center';
    d.style.color = ink;
    d.textContent = l.text;
    wrap.appendChild(d);
  }
  layer.appendChild(wrap);
  return fit.fs;
}

/**
 * Render the translated pieces that fall on `pageNum` into `layer`.
 * `pieces` maps paragraph id → array of strings (one per segment).
 */
export function renderTranslationLayer(layer, doc, pieces, pageNum, zoom, selectedParaId) {
  layer.innerHTML = '';
  if (!doc || !pieces || pieces.size === 0) return { rendered: 0, shrunk: 0, overflow: 0 };
  let rendered = 0, shrunk = 0, overflow = 0;
  for (const p of doc.paragraphs) {
    const parts = pieces.get(p.id);
    if (!parts) continue;
    p.segments.forEach((seg, k) => {
      if (seg.pageNum !== pageNum) return;
      const text = parts[k] || '';
      const title = `${p.id} · ${p.role}${p.segments.length > 1 ? ` · parte ${k + 1}/${p.segments.length}` : ''}`;
      // comic balloons and narration boxes: inside their shape
      const kind = seg.balloon?.kind;
      if ((kind === 'balloon' || kind === 'narration') && seg.balloon.shape?.length >= 2) {
        const fs = renderBalloon(layer, seg, text, zoom, p.id, p.id === selectedParaId, title);
        if (fs !== null) {
          rendered++;
          if (fs < seg.fontSize * 0.98) shrunk++;
          return;
        }
      }
      const b = seg.bounds;
      const el = document.createElement('div');
      el.className = 'tr-seg';
      if (p.id === selectedParaId) el.classList.add('selected');
      el.dataset.paraId = p.id;
      el.title = title;
      const pad = 1;
      el.style.left = (b.x - pad) * zoom + 'px';
      el.style.top = (b.y - pad) * zoom + 'px';
      el.style.width = (b.w + 2 * pad) * zoom + 'px';
      el.style.height = (b.h + 2 * pad) * zoom + 'px';
      el.style.fontFamily = genericFamily(seg.fontName);
      el.style.fontWeight = seg.bold ? '700' : '400';
      el.style.fontStyle = seg.italic ? 'italic' : 'normal';
      el.style.textAlign = seg.alignment === 'justify' ? 'justify' : seg.alignment;
      // keep the original leading when we can measure it
      let lh = 1.18;
      if (seg.lines.length >= 2) {
        const step = (seg.lines[seg.lines.length - 1].y - seg.lines[0].y) / (seg.lines.length - 1);
        if (step > 0) lh = Math.min(1.6, Math.max(1.0, step / seg.fontSize));
      }
      el.style.lineHeight = String(lh);
      el.textContent = text;
      layer.appendChild(el);
      const fs = fitText(el, seg.fontSize * zoom);
      rendered++;
      if (fs < seg.fontSize * zoom * 0.98) shrunk++;
      if (el.classList.contains('overflow')) overflow++;
    });
  }
  return { rendered, shrunk, overflow };
}
