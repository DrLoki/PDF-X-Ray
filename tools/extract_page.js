import fs from 'fs';
import * as pdfjsLib from 'pdfjs-dist/legacy/build/pdf.mjs';

async function extract(pdfPath, pageNum = 1) {
  const data = fs.readFileSync(pdfPath);
  const loadingTask = pdfjsLib.getDocument({ data });
  const pdfDoc = await loadingTask.promise;
  const page = await pdfDoc.getPage(pageNum);
  const viewport = page.getViewport({ scale: 1.0 });

  const textContent = await page.getTextContent({ includeMarkedContent: true });
  const items = textContent.items
    .filter(item => item && item.transform && item.str !== undefined)
    .map(item => {
      const tx = item.transform[4];
      const ty = item.transform[5];
      const width = item.width;
      const height = item.height || item.transform[3];
      const rect = viewport.convertToViewportRectangle([tx, ty, tx + width, ty + height]);
      return {
        text: item.str,
        x: Math.min(rect[0], rect[2]),
        y: Math.min(rect[1], rect[3]),
        width: Math.abs(rect[2] - rect[0]),
        height: Math.abs(rect[3] - rect[1]),
        font_size: item.transform[3] || 0,
        font_name: item.fontName || null
      };
    }).filter(i => i.text && i.text.trim().length > 0);

  const pageBounds = { x: 0, y: 0, w: viewport.width, h: viewport.height };
  const borderedBoxes = [];

  return { items, pageBounds, borderedBoxes };
}

(async () => {
  const args = process.argv.slice(2);
  if (args.length < 2) {
    console.error('Usage: node tools/extract_page.js <pdfPath> <pageNum>');
    process.exit(1);
  }
  const pdfPath = args[0];
  const pageNum = parseInt(args[1], 10);
  try {
    const out = await extract(pdfPath, pageNum);
    console.log(JSON.stringify(out));
  } catch (e) {
    console.error('Extraction error:', e);
    process.exit(2);
  }
})();
