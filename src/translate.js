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
      const b = seg.bounds;
      const el = document.createElement('div');
      el.className = 'tr-seg';
      if (p.id === selectedParaId) el.classList.add('selected');
      el.dataset.paraId = p.id;
      el.title = `${p.id} · ${p.role}${p.segments.length > 1 ? ` · parte ${k + 1}/${p.segments.length}` : ''}`;
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
