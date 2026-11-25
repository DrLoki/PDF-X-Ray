// Copy the pdf.js files the app loads from `node_modules/pdfjs-dist` into
// `src/assets/pdfjs/`, so they always match the installed version.
//
//   node tools/vendor_pdfjs.mjs
//
// Runs on `npm install` (postinstall) and before `tauri dev` / `tauri build`
// (see src-tauri/tauri.conf.json). The copies are not versioned (.gitignore).
//
// - build: the library and its worker;
// - wasm: decoders for CCITT fax / JBIG2 (black-and-white scans) and
//   JPEG 2000 images, plus the ICC colour management module — without them
//   those pages render blank (pdf.js 6 loads them through `wasmUrl`);
// - iccs: the default CMYK profile.
// CMaps (CJK encodings) and standard font data are left out: the app reads
// Latin-script documents and the WebView provides system fonts.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const src = path.join(root, 'node_modules', 'pdfjs-dist');
const dst = path.join(root, 'src', 'assets', 'pdfjs');

const FILES = [
  ['build/pdf.min.mjs', 'pdf.min.mjs'],
  ['build/pdf.worker.min.mjs', 'pdf.worker.min.mjs'],
  ['LICENSE', 'LICENSE'],
  ['wasm/jbig2.wasm', 'wasm/jbig2.wasm'],
  ['wasm/jbig2_nowasm_fallback.js', 'wasm/jbig2_nowasm_fallback.js'],
  ['wasm/openjpeg.wasm', 'wasm/openjpeg.wasm'],
  ['wasm/openjpeg_nowasm_fallback.js', 'wasm/openjpeg_nowasm_fallback.js'],
  ['wasm/qcms_bg.wasm', 'wasm/qcms_bg.wasm'],
  ['wasm/LICENSE_JBIG2', 'wasm/LICENSE_JBIG2'],
  ['wasm/LICENSE_OPENJPEG', 'wasm/LICENSE_OPENJPEG'],
  ['wasm/LICENSE_QCMS', 'wasm/LICENSE_QCMS'],
  ['wasm/LICENSE_PDFJS_JBIG2', 'wasm/LICENSE_PDFJS_JBIG2'],
  ['wasm/LICENSE_PDFJS_OPENJPEG', 'wasm/LICENSE_PDFJS_OPENJPEG'],
  ['wasm/LICENSE_PDFJS_QCMS', 'wasm/LICENSE_PDFJS_QCMS'],
];
const DIRS = [['iccs', 'iccs']];

if (!fs.existsSync(path.join(src, 'package.json'))) {
  console.error('vendor_pdfjs: pdfjs-dist is not installed — run `npm install` first.');
  process.exit(1);
}
const version = JSON.parse(fs.readFileSync(path.join(src, 'package.json'), 'utf8')).version;

let copied = 0;
const copy = (from, to) => {
  const a = path.join(src, from);
  const b = path.join(dst, to);
  if (!fs.existsSync(a)) {
    console.error(`vendor_pdfjs: missing ${from} in pdfjs-dist ${version}`);
    process.exit(1);
  }
  const data = fs.readFileSync(a);
  if (fs.existsSync(b) && fs.readFileSync(b).equals(data)) return;
  fs.mkdirSync(path.dirname(b), { recursive: true });
  fs.writeFileSync(b, data);
  copied++;
};
for (const [from, to] of FILES) copy(from, to);
for (const [from, to] of DIRS) {
  for (const f of fs.readdirSync(path.join(src, from))) copy(path.join(from, f), path.join(to, f));
}
console.log(`vendor_pdfjs: pdf.js ${version} → src/assets/pdfjs (${copied} file${copied === 1 ? '' : 's'} updated)`);
