// Extract analysis input from a PDF with the same extractor used by the app.
//
//   node tools/extract_page.js <file.pdf> <pageNum>     -> single page JSON
//   node tools/extract_page.js <file.pdf> all           -> { "pages": [...] } document JSON
//   node tools/extract_page.js <file.pdf> 3-7           -> document JSON for a page range
//
// The output feeds `cli_analyze` (see docs/cli-analyze.md).
import fs from 'fs';
import * as pdfjsLib from 'pdfjs-dist/legacy/build/pdf.mjs';
import { extractPage } from '../src/extract.js';

async function main() {
  const [pdfPath, which = '1'] = process.argv.slice(2);
  if (!pdfPath) {
    console.error('Usage: node tools/extract_page.js <pdfPath> <pageNum|all|from-to>');
    process.exit(1);
  }
  const data = new Uint8Array(fs.readFileSync(pdfPath));
  const pdfDoc = await pdfjsLib.getDocument({ data, verbosity: 0 }).promise;

  let pages;
  if (which === 'all') {
    pages = Array.from({ length: pdfDoc.numPages }, (_, i) => i + 1);
  } else if (/^\d+-\d+$/.test(which)) {
    const [a, b] = which.split('-').map(Number);
    pages = Array.from({ length: Math.max(0, Math.min(b, pdfDoc.numPages) - a + 1) }, (_, i) => a + i);
  } else {
    const n = parseInt(which, 10);
    const r = await extractPage(pdfjsLib, pdfDoc, n);
    console.log(JSON.stringify(r.data));
    return;
  }
  const out = [];
  for (const p of pages) out.push((await extractPage(pdfjsLib, pdfDoc, p)).data);
  console.log(JSON.stringify({ pages: out }));
}

main().catch(e => {
  console.error('Extraction error:', e);
  process.exit(2);
});
