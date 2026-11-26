/**
 * Documents made of page images (CBZ, a folder of images, a few image
 * files): a small stand-in for the part of pdf.js the app uses, so the
 * viewer, the OCR, the comic detection and the snapshots work on them
 * without a PDF.
 *
 * Nothing is converted. Opening reads only the list of pages (for a zip:
 * its central directory, read from the end of the file); a page is read and
 * decoded when it is shown or recognised, and the last few decoded pages
 * are kept.
 *
 * A *source* is `{ name, pages: [{ name, blob(): Promise<Blob> }], info }`:
 * - `zipSource(file)`: CBZ / zip — entries stored or deflated, read by
 *   ranges of the file (the archive is never loaded whole);
 * - `filesSource(files, name)`: a folder picked in the file dialog, or
 *   several image files.
 * Pages are the image entries in natural order of their paths ("page2"
 * before "page10"); `__MACOSX` and hidden files are skipped. Other formats
 * only need a new source.
 *
 * Page geometry: the longer side of every page is 11 in (792 pt), so the
 * 300 DPI raster of the OCR is close to the native resolution of good
 * scans and enlarges small ones like a scanned PDF would.
 */

const IMAGE_EXT = /\.(jpe?g|png|webp|gif|bmp|avif)$/i;
export const PAGE_LONG_SIDE_PT = 792;
const CACHE_PAGES = 4;

/** True for a file name the image sources accept as a page. */
export const isImageName = (name) => IMAGE_EXT.test(name);

/** Natural order: "page2" before "page10". */
export function naturalCompare(a, b) {
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
}

/** Image paths of an archive or folder, in reading order. */
export function pagePaths(paths) {
  return paths
    .filter(p => IMAGE_EXT.test(p) && !p.endsWith('/'))
    .filter(p => !p.split('/').some(part => part === '__MACOSX' || part.startsWith('.')))
    .sort(naturalCompare);
}

// ---------------------------------------------------------------- zip (by ranges)

const u16 = (b, o) => b[o] | (b[o + 1] << 8);
const u32 = (b, o) => (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0;
const u64 = (b, o) => Number(new DataView(b.buffer, b.byteOffset + o, 8).getBigUint64(0, true));

async function readRange(blob, start, end) {
  return new Uint8Array(await blob.slice(start, end).arrayBuffer());
}

/** Entries of a zip held in a Blob/File: { name, method, offset, compSize, size, encrypted }. */
export async function zipEntries(blob) {
  const tailLen = Math.min(blob.size, 65557 + 20);
  const tail = await readRange(blob, blob.size - tailLen, blob.size);
  let eocd = -1;
  for (let i = tail.length - 22; i >= 0; i--) {
    if (u32(tail, i) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error('archivio zip non valido (directory centrale non trovata)');
  let count = u16(tail, eocd + 10);
  let cdSize = u32(tail, eocd + 12);
  let cdOffset = u32(tail, eocd + 16);
  if (count === 0xffff || cdOffset === 0xffffffff || cdSize === 0xffffffff) {
    // zip64: the locator (20 bytes) precedes the end record
    const loc = eocd - 20;
    if (loc >= 0 && u32(tail, loc) === 0x07064b50) {
      const recAt = u64(tail, loc + 8);
      const rec = await readRange(blob, recAt, recAt + 56);
      count = u64(rec, 32);
      cdSize = u64(rec, 40);
      cdOffset = u64(rec, 48);
    }
  }
  const cd = await readRange(blob, cdOffset, cdOffset + cdSize);
  const dec = new TextDecoder();
  const out = [];
  let p = 0;
  for (let k = 0; k < count && p + 46 <= cd.length && u32(cd, p) === 0x02014b50; k++) {
    const flags = u16(cd, p + 8);
    const method = u16(cd, p + 10);
    let compSize = u32(cd, p + 20);
    let size = u32(cd, p + 24);
    const nameLen = u16(cd, p + 28), extraLen = u16(cd, p + 30), commentLen = u16(cd, p + 32);
    let offset = u32(cd, p + 42);
    const nameBytes = cd.subarray(p + 46, p + 46 + nameLen);
    // bit 11: UTF-8 names; otherwise CP437 (fine for ASCII names)
    const name = flags & 0x800 ? dec.decode(nameBytes) : String.fromCharCode(...nameBytes);
    let e = p + 46 + nameLen;
    const eEnd = e + extraLen;
    while (e + 4 <= eEnd) {
      const id = u16(cd, e), len = u16(cd, e + 2);
      if (id === 0x0001) { // zip64 sizes / offset, in this order when saturated
        let q = e + 4;
        if (size === 0xffffffff) { size = u64(cd, q); q += 8; }
        if (compSize === 0xffffffff) { compSize = u64(cd, q); q += 8; }
        if (offset === 0xffffffff) { offset = u64(cd, q); }
      }
      e += 4 + len;
    }
    out.push({ name, method, offset, compSize, size, encrypted: !!(flags & 1) });
    p = eEnd + commentLen;
  }
  return out;
}

/** Uncompressed content of a zip entry, as a Blob. */
export async function zipEntryBlob(blob, entry) {
  if (entry.encrypted) throw new Error(`${entry.name}: file cifrato`);
  const head = await readRange(blob, entry.offset, entry.offset + 30);
  if (u32(head, 0) !== 0x04034b50) throw new Error(`${entry.name}: intestazione locale non valida`);
  const start = entry.offset + 30 + u16(head, 26) + u16(head, 28);
  const data = blob.slice(start, start + entry.compSize);
  if (entry.method === 0) return data;
  if (entry.method === 8) return new Response(data.stream().pipeThrough(new DecompressionStream('deflate-raw'))).blob();
  throw new Error(`${entry.name}: compressione ${entry.method} non supportata`);
}

/** ComicInfo.xml: reading direction, language and title, when declared. */
export function parseComicInfo(xml) {
  const tag = (t) => (new RegExp(`<${t}>([^<]*)</${t}>`, 'i').exec(xml) || [])[1]?.trim() || null;
  return { rtl: /^YesAndRightToLeft$/i.test(tag('Manga') || ''), language: tag('LanguageISO'), title: tag('Title') };
}

/** True when a file starts like a zip archive. */
export async function looksLikeZip(file) {
  const b = await readRange(file, 0, 4);
  return b.length === 4 && b[0] === 0x50 && b[1] === 0x4b && (b[2] === 3 || b[2] === 5);
}

// ---------------------------------------------------------------- sources

/** A CBZ / zip archive, read by ranges. */
export async function zipSource(file) {
  const entries = await zipEntries(file);
  const byName = new Map(entries.map(e => [e.name, e]));
  const pages = pagePaths(entries.map(e => e.name)).map(name => ({ name, blob: () => zipEntryBlob(file, byName.get(name)) }));
  let info = { rtl: false, language: null, title: null };
  const ci = entries.find(e => /(^|\/)comicinfo\.xml$/i.test(e.name));
  if (ci) {
    try { info = parseComicInfo(await (await zipEntryBlob(file, ci)).text()); } catch { /* optional */ }
  }
  return { name: file.name, pages, info };
}

/** Image files: a folder from the file dialog (webkitRelativePath) or a selection. */
export async function filesSource(files, name) {
  const list = [...files];
  const path = (f) => f.webkitRelativePath || f.name;
  const byPath = new Map(list.map(f => [path(f), f]));
  const pages = pagePaths(list.map(path)).map(p => ({ name: p, blob: async () => byPath.get(p) }));
  let info = { rtl: false, language: null, title: null };
  const ci = list.find(f => /^comicinfo\.xml$/i.test(f.name));
  if (ci) {
    try { info = parseComicInfo(await ci.text()); } catch { /* optional */ }
  }
  const folder = list[0]?.webkitRelativePath?.split('/')[0];
  return { name: name || folder || list[0]?.name || 'immagini', pages, info };
}

// ---------------------------------------------------------------- pdf.js stand-in

/** Viewport with the fields of pdf.js' PageViewport the app reads. */
function makeViewport(wPt, hPt, scale) {
  return {
    width: wPt * scale,
    height: hPt * scale,
    scale,
    rotation: 0,
    transform: [scale, 0, 0, scale, 0, 0],
    convertToViewportPoint: (x, y) => [x * scale, y * scale],
  };
}

class ImagePage {
  constructor(doc, index, width, height) {
    this.doc = doc;
    this.pageNumber = index + 1;
    this.index = index;
    this.ref = null;
    this.setSize(width, height);
  }
  setSize(width, height) {
    this.pxWidth = width;
    this.pxHeight = height;
    const k = PAGE_LONG_SIDE_PT / Math.max(width, height, 1);
    this.wPt = width * k;
    this.hPt = height * k;
  }
  getViewport({ scale = 1 } = {}) {
    return makeViewport(this.wPt, this.hPt, scale);
  }
  /** Draw the image on the canvas of the viewport (white background). */
  render({ canvasContext: ctx, viewport, transform, background }) {
    const promise = (async () => {
      const bmp = await this.doc.bitmap(this.index);
      ctx.save();
      if (transform) ctx.setTransform(...transform);
      ctx.fillStyle = background || '#ffffff';
      ctx.fillRect(0, 0, viewport.width, viewport.height);
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = 'high';
      ctx.drawImage(bmp, 0, 0, viewport.width, viewport.height);
      ctx.restore();
    })();
    return { promise, cancel() {} };
  }
  async getStructTree() { return null; }
  async getTextContent() { return { items: [], styles: {} }; }
  cleanup() {}
}

/**
 * The page input of `extract.js` for an image page: no text, one image
 * covering the page (so the classification sees a scan).
 */
function imagePageData(page) {
  const b = { x: 0, y: 0, w: page.wPt, h: page.hPt };
  return { pageNum: page.pageNumber, pageBounds: b, items: [], borderedBoxes: [], images: [b] };
}

export class ImageDocument {
  /** Use `ImageDocument.open(source)`: page sizes are read before the first use. */
  constructor(source) {
    this.isImageDocument = true;
    this.source = source;
    this.info = source.info;
    this.name = source.name;
    this.numPages = source.pages.length;
    this.pages = [];
    this.cache = new Map(); // index → ImageBitmap (most recent last)
    this.failed = new Map(); // index → error message
  }

  /**
   * Open a source. The size of every page is needed for the page geometry;
   * it is read from the image headers for JPEG / PNG / WebP / GIF / BMP
   * (a few bytes each) and by decoding otherwise.
   */
  static async open(source, progress = () => {}) {
    if (!source.pages.length) throw new Error('nessuna immagine di pagina trovata');
    const doc = new ImageDocument(source);
    for (let i = 0; i < source.pages.length; i++) {
      let size = null;
      try {
        size = await imageSize(await source.pages[i].blob());
      } catch (err) {
        doc.failed.set(i, String(err?.message || err));
      }
      doc.pages.push(new ImagePage(doc, i, size?.width || 1000, size?.height || 1500));
      if (i % 8 === 0 || i + 1 === source.pages.length) progress(i + 1, source.pages.length);
    }
    return doc;
  }

  async getPage(n) {
    const p = this.pages[n - 1];
    if (!p) throw new Error(`pagina ${n} inesistente`);
    return p;
  }

  async bitmap(index) {
    if (this.cache.has(index)) {
      const b = this.cache.get(index);
      this.cache.delete(index);
      this.cache.set(index, b);
      return b;
    }
    const blob = await this.source.pages[index].blob();
    const bmp = await createImageBitmap(blob);
    const page = this.pages[index];
    if (bmp.width !== page.pxWidth || bmp.height !== page.pxHeight) page.setSize(bmp.width, bmp.height);
    this.cache.set(index, bmp);
    while (this.cache.size > CACHE_PAGES) {
      const [k, old] = this.cache.entries().next().value;
      this.cache.delete(k);
      old.close?.();
    }
    return bmp;
  }

  /** `extract.js` short-cut: the page input without going through pdf.js operators. */
  async extractPage(n) {
    const page = await this.getPage(n);
    return { page, viewport: page.getViewport({ scale: 1 }), tagged: false, data: imagePageData(page) };
  }

  async getOutline() { return null; }
  async getDestination() { return null; }
  async getPageIndex() { return 0; }
  async destroy() {
    for (const b of this.cache.values()) b.close?.();
    this.cache.clear();
  }
}

// ---------------------------------------------------------------- image sizes from headers

/** Width and height from the image header; decodes when the format is unknown. */
export async function imageSize(blob) {
  const b = new Uint8Array(await blob.slice(0, 65536).arrayBuffer());
  const be16 = (o) => (b[o] << 8) | b[o + 1];
  const be32 = (o) => ((b[o] << 24) | (b[o + 1] << 16) | (b[o + 2] << 8) | b[o + 3]) >>> 0;
  // PNG
  if (b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) return { width: be32(16), height: be32(20) };
  // GIF
  if (b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46) return { width: u16(b, 6), height: u16(b, 8) };
  // BMP
  if (b[0] === 0x42 && b[1] === 0x4d) return { width: u32(b, 18), height: Math.abs(u32(b, 22) | 0) };
  // WebP
  if (b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45) {
    const chunk = String.fromCharCode(b[12], b[13], b[14], b[15]);
    if (chunk === 'VP8X') return { width: 1 + (b[24] | (b[25] << 8) | (b[26] << 16)), height: 1 + (b[27] | (b[28] << 8) | (b[29] << 16)) };
    if (chunk === 'VP8 ') return { width: u16(b, 26) & 0x3fff, height: u16(b, 28) & 0x3fff };
    if (chunk === 'VP8L') {
      const v = u32(b, 21);
      return { width: (v & 0x3fff) + 1, height: ((v >> 14) & 0x3fff) + 1 };
    }
  }
  // JPEG: SOFn marker
  if (b[0] === 0xff && b[1] === 0xd8) {
    let p = 2;
    while (p + 9 < b.length) {
      if (b[p] !== 0xff) { p++; continue; }
      const m = b[p + 1];
      if (m === 0xd8 || m === 0x01 || (m >= 0xd0 && m <= 0xd7) || m === 0xff) { p += m === 0xff ? 1 : 2; continue; }
      if (m >= 0xc0 && m <= 0xcf && m !== 0xc4 && m !== 0xc8 && m !== 0xcc) return { width: be16(p + 7), height: be16(p + 5) };
      p += 2 + be16(p + 2);
    }
  }
  // anything else (AVIF, a JPEG with a huge header): decode
  const bmp = await createImageBitmap(blob);
  const size = { width: bmp.width, height: bmp.height };
  bmp.close?.();
  return size;
}
