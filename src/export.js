/**
 * Serialisation of analysis results (layout trees, logical paragraphs).
 */

const esc = (s) => String(s)
  .replace(/&/g, '&amp;')
  .replace(/</g, '&lt;')
  .replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

/** Layout tree of one page as JSON, XML or a self-contained HTML page. */
export function serializeLayoutTree(node, format = 'json') {
  if (format === 'json') {
    return JSON.stringify(node, null, 2);
  }

  if (format === 'xml') {
    const toXML = (n, ind) => {
      const b = n.bounds;
      const boundsStr = `x="${b.x.toFixed(1)}" y="${b.y.toFixed(1)}" w="${b.w.toFixed(1)}" h="${b.h.toFixed(1)}"`;
      if (n.type === 'leaf') {
        const f = n.formatting || {};
        const attrs = [
          `id="${esc(n.id)}"`, boundsStr,
          n.role ? `role="${esc(n.role)}"` : '',
          n.readingOrder !== undefined ? `order="${n.readingOrder}"` : '',
          n.paragraphIds?.length ? `paragraphs="${esc(n.paragraphIds.join(' '))}"` : '',
          `font-family="${esc(f.fontFamily)}" font-size="${f.fontSize}" font-weight="${f.fontWeight}" font-style="${f.fontStyle}" align="${f.alignment}"`,
        ].filter(Boolean).join(' ');
        return `${ind}<Block ${attrs}>\n${ind}  <Text>${esc(n.text || '')}</Text>\n${ind}</Block>`;
      }
      const tag = n.type === 'root' ? 'PageLayout' : n.isBorderedBox ? (n.isTable ? 'Table' : 'Box') : 'Container';
      const dir = n.cutDirection ? ` cut="${n.cutDirection}"` : '';
      const kids = (n.children || []).map(c => toXML(c, ind + '  ')).join('\n');
      return `${ind}<${tag} id="${esc(n.id)}" ${boundsStr}${dir}>\n${kids}\n${ind}</${tag}>`;
    };
    return `<?xml version="1.0" encoding="UTF-8"?>\n${toXML(node, '')}`;
  }

  if (format === 'html') {
    const toHTML = (n) => {
      const b = n.bounds;
      const pos = `position:absolute;left:${b.x}px;top:${b.y}px;width:${b.w}px;height:${b.h}px;`;
      if (n.type === 'leaf') {
        const f = n.formatting || {};
        const style = `${pos}font-family:${esc(f.fontFamily)},serif;font-size:${f.fontSize}px;font-weight:${f.fontWeight};font-style:${f.fontStyle};text-align:${f.alignment};border:1px solid rgba(0,0,0,.15);box-sizing:border-box;overflow:hidden;line-height:1.15;`;
        return `<div id="${esc(n.id)}" class="leaf role-${esc(n.role || 'body')}" title="${esc(n.role || '')}" style="${style}">${esc(n.text || '').replace(/\n/g, '<br/>')}</div>`;
      }
      return (n.children || []).map(toHTML).join('\n');
    };
    const b = node.bounds;
    return `<!DOCTYPE html>\n<html>\n<head>\n<meta charset="utf-8">\n<title>PDF X-Ray layout</title>\n</head>\n<body style="margin:0;padding:20px;background:#eee;">\n<div style="position:relative;width:${b.w}px;height:${b.h}px;background:#fff;">\n${toHTML(node)}\n</div>\n</body>\n</html>`;
  }
  return '';
}

/** Translation units: the logical paragraphs that should be translated. */
export function serializeUnits(doc, sourceName) {
  return JSON.stringify({
    source: sourceName || null,
    generated: new Date().toISOString(),
    instructions: 'Fill "translation" for each unit and re-import this file. Keep the ids.',
    units: doc.paragraphs
      .filter(p => p.translatable)
      .map(p => ({
        id: p.id,
        role: p.role,
        pages: [...new Set(p.segments.map(s => s.pageNum))],
        text: p.text,
        translation: '',
      })),
  }, null, 2);
}

/** Plain-text dump of the reading flow, one logical paragraph per block. */
export function serializeParagraphsText(doc) {
  return doc.paragraphs
    .map(p => {
      const pages = [...new Set(p.segments.map(s => s.pageNum))].join('-');
      return `[${p.id} · p.${pages} · ${p.role}]\n${p.text}`;
    })
    .join('\n\n');
}

/**
 * Parse an imported translation file. Accepted shapes:
 *   { "par12": "testo", ... }
 *   { "units": [ { "id": "par12", "translation": "..." } ] }
 *   [ { "id": "par12", "translation" | "text": "..." } ]
 */
export function parseTranslations(json) {
  const data = JSON.parse(json);
  const out = new Map();
  const list = Array.isArray(data) ? data : Array.isArray(data.units) ? data.units : null;
  if (list) {
    for (const u of list) {
      const t = u.translation ?? u.translated ?? u.target ?? (u.text !== undefined && u.translation === undefined && !data.units ? u.text : undefined);
      if (u.id && typeof t === 'string' && t.trim()) out.set(String(u.id), t);
    }
  } else if (data && typeof data === 'object') {
    for (const [k, v] of Object.entries(data)) {
      if (typeof v === 'string' && v.trim()) out.set(k, v);
    }
  }
  return out;
}
