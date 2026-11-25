/**
 * OCR support in the frontend: rasterise a page with pdf.js and send the
 * pixels to the Rust OCR engine (`ocr_page`, raw IPC body), plus the model
 * download (`ocr_models_status`, `ocr_download_models`).
 *
 * The page is rendered on a white background at `OCR_DPI`; the returned text
 * runs are already in PDF points (viewport at scale 1, like `extract.js`).
 */

export const OCR_DPI = 300;
/** Longest side of the raster, in pixels (very large pages are rendered at a lower DPI). */
export const OCR_MAX_SIDE = 5000;

/** Render a page for OCR. Returns { canvas, width, height, scale (px per pt), dpi }. */
export async function rasterizePage(pdfDoc, pageNum, dpi = OCR_DPI) {
  const page = await pdfDoc.getPage(pageNum);
  const base = page.getViewport({ scale: 1 });
  let scale = dpi / 72;
  const longest = Math.max(base.width, base.height) * scale;
  if (longest > OCR_MAX_SIDE) scale *= OCR_MAX_SIDE / longest;
  const viewport = page.getViewport({ scale });
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(viewport.width));
  canvas.height = Math.max(1, Math.round(viewport.height));
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  ctx.fillStyle = '#ffffff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  await page.render({ canvasContext: ctx, viewport, background: 'rgb(255,255,255)' }).promise;
  return { canvas, width: canvas.width, height: canvas.height, scale: canvas.width / base.width, dpi: scale * 72 };
}

/** Run the OCR on a rendered page. Returns { page, items, scale } (see `ocr_page`). */
export async function recognizeCanvas(invoke, raster) {
  const ctx = raster.canvas.getContext('2d', { willReadFrequently: true });
  const img = ctx.getImageData(0, 0, raster.width, raster.height);
  const bytes = new Uint8Array(img.data.buffer, img.data.byteOffset, img.data.byteLength);
  return invoke('ocr_page', bytes, {
    headers: {
      'x-width': String(raster.width),
      'x-height': String(raster.height),
      'x-scale': String(raster.scale),
    },
  });
}

export function formatBytes(n) {
  if (n >= 1024 * 1024) return (n / 1024 / 1024).toFixed(1).replace('.', ',') + ' MB';
  if (n >= 1024) return Math.round(n / 1024) + ' kB';
  return n + ' B';
}

/** Summary of an OCR result for the UI. */
export function ocrSummary(res) {
  const lines = res.page.lines;
  const conf = lines.length ? lines.reduce((a, l) => a + l.conf, 0) / lines.length : 0;
  const low = lines.filter(l => l.conf < LOW_CONF).length;
  return { lines: lines.length, conf, low, ms: res.page.detectMs + res.page.recognizeMs };
}

/** Runs below this confidence are highlighted in the overlay. */
export const LOW_CONF = 0.8;
