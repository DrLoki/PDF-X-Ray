# 🩻 PDF X-Ray - Structure & Layout Analyzer

![Tauri](https://img.shields.io/badge/Tauri-2.0-FFC131?logo=tauri&logoColor=white)
![Vite](https://img.shields.io/badge/Vite-6.0-646CFF?logo=vite&logoColor=white)
![JavaScript](https://img.shields.io/badge/JavaScript-Vanilla-F7DF1E?logo=javascript&logoColor=black)

An advanced desktop application built on **Tauri v2** and **Vanilla Web Technologies** designed to parse PDF documents, extract hierarchical bookmarks, inspect logical structure tags (Tagged PDF), and perform real-time **Recursive XY-CUT** layout decomposition.

---

## 🚀 Key Features

1. **Document Navigation & Bookmarks (Outline)**:
   - Extracts document bookmarks hierarchically.
   - Provides an interactive, collapsible sidebar.
   - Clicking an item jumps instantly to the respective page.

2. **Tagged PDF logical Structure Viewer**:
   - Queries the PDF's `StructTree` logical node tree.
   - Tells you immediately if the PDF is tagged or untagged via a status badge.
   - Visualizes document tags (like `<Document>`, `<Section>`, `<H1>`, `<P>`, `<Table>`) with role-based semantic colors.

3. **Layout Analysis (Rust engine)**:
   - Text runs are rebuilt into lines (gap-aware word joining, drop caps), then segmented with a **recursive XY-Cut** that never loses text (conservation invariant, centred titles across gutters, figures and boxes as obstacles).
   - Parameters: automatic thresholds (4 strategies) or manual **Row Gap ($T_y$)**, **Column Gap ($T_x$)**, **Min Block Width/Height** in points, plus **Priority** (rows first, columns first, largest gap).
   - Bordered boxes and ruled tables are detected from the vector graphics (CTM-aware, pdf.js 6 path format).
   - Live overlays: text runs, blocks coloured by role, projection histograms and gaps.

4. **Roles & Reading Flow**:
   - Every block gets a role: `body`, `heading`, `header`, `footer`, `page-number`, `caption`, `footnote`, `sidebar`, `table`, `figure-text`. Running heads and folios are confirmed by their repetition across pages.
   - Reading order is a topological sort (Breuel) that is robust to figures and full-width elements; Tagged PDFs use their structure tree and `/Artifact` marks.
   - Blocks are split into paragraphs (leading, indents, hanging indents, short last lines, bullets, size/weight changes) and paragraphs interrupted by **column, page, box or figure breaks are stitched** back together, with de-hyphenation.
   - The **Paragrafi** tab lists the logical paragraphs; the **Flow** overlay shows reading-order badges and continuation arrows (`↪ p.N` / `↩ p.N`).

5. **Translation Overlay**:
   - Each logical paragraph is a translation unit (export: *Unità di traduzione · JSON*; import the same file with the `translation` fields filled, or `{ "par12": "…" }`).
   - The translated text is re-distributed over the original pieces of the paragraph (proportional to their length, cut at sentence/word boundaries) and fitted into each box. **Pseudo** generates a +30 % pseudo-translation to test the layout without a translation engine.

6. **Bi-directional Synced Interlock & Inspector**:
   - Hovering/clicking a block on the canvas highlights it in the layout DOM tree and vice versa; the inspector shows role, reading order, font, alignment, the block text and the logical paragraphs it belongs to (full stitched text and translation).

7. **Snapshots for heuristic tuning**:
   - **📸 Snapshot** saves the page data, the settings, the results with the current and with the automatic thresholds (including per-node diagnostics: thresholds, largest gaps, decisions), your verdict, a note and a PNG of the view to `Documents/PDF-X-Ray/snapshots`.
   - `cli_analyze --eval <folder>` scores the automatic settings against the validated segmentations, so heuristic changes can be measured across all annotated pages.

8. **Export**:
   - Page layout tree as **JSON**, **XML** or **HTML**; whole document result as JSON; logical paragraphs as text; translation units as JSON.

---

## 📐 How the Pipeline Works

```
runs ─► lines ─► XY-Cut blocks ─► roles ─► reading order ─► paragraphs ─► stitching ─► translation units
```

1. **Extraction** (`src/extract.js`, pdf.js): run boxes in PDF points (rotation-aware), real font names, Tagged-PDF marked content and structure roles, vector rectangles and images.
2. **Lines** (`lines.rs`): runs swept left→right into lines; wide gaps split columns; drop caps merged.
3. **XY-Cut** (`xycut.rs`): recursive cuts over lines, every line assigned to exactly one leaf.
4. **Roles** (`roles.rs`): tags/artifacts, margins, page-number and caption patterns, image adjacency, footnote markers, repetition across pages.
5. **Reading order** (`order.rs`): Breuel topological order, XY-Cut order as tie-break.
6. **Paragraphs & stitching** (`flow.rs`): paragraph split inside blocks, continuation across blocks/pages, de-hyphenation.
7. **Re-flow** (`reflow.rs`): translated text distributed over the paragraph's segments.

Full details: [`docs/layout-pipeline.md`](docs/layout-pipeline.md).

---

## 🛠️ Installation & Getting Started

### Prerequisites
- [Node.js](https://nodejs.org/) (v22.13.0 or higher recommended)
- [Rust & Cargo](https://www.rust-lang.org/) (for compiling the Tauri desktop wrapper)

### Setup Instructions
1. Clone or open the repository folder.
2. Install Node dependencies:
   ```powershell
   npm.cmd install
   ```

---

## 💻 Development & Build Workflows

### Run in Desktop Development Mode
Start the application in interactive development mode. This compiles the Rust backend, opens the desktop window, and hooks live reloads:
```powershell
npm.cmd run tauri dev
```

### Production Build
Compile the application into a optimized, self-contained standalone desktop executable (`.exe` for Windows):
```powershell
npm.cmd run tauri build
```
The compiled binaries will be outputted under:
`src-tauri/target/release/`

---

## 🔧 Developer Tools

The `tools/` directory and the Rust `src-tauri/src/bin/` directory contain helper utilities for offline development and debugging of the layout pipeline without launching the full desktop app.

### `tools/extract_page.js` — PDF Extractor (Node.js)

Uses the **same extractor as the desktop app** (`src/extract.js`, pdf.js 6), so the output is identical to what the app analyses, Tagged-PDF information and graphics included.

```powershell
node tools/extract_page.js "C:/path/to/document.pdf" 6      > page6.json   # one page
node tools/extract_page.js "C:/path/to/document.pdf" all    > doc.json     # whole document
node tools/extract_page.js "C:/path/to/document.pdf" 10-14  > range.json   # page range
```

### `tools/extract_page_py.py` — Legacy Page Extractor (Python)

PyMuPDF-based extractor kept for the existing fixtures (no tags, no graphics). `pip install pymupdf`.

```powershell
python tools/extract_page_py.py "C:/path/to/document.pdf" <pageNum> > page.json
```

---

### `src-tauri/src/bin/cli_analyze.rs` — Headless Analyzer (Rust)

Runs the same pipeline as the app on a page or a whole document.

```powershell
cd src-tauri
cargo build --features cli --bin cli_analyze

# logical paragraphs in reading order, with roles and joins
target/debug/cli_analyze --text ../doc.json

# full JSON result, columns first
target/debug/cli_analyze --priority X ../doc.json > result.json
```

See [`docs/cli-analyze.md`](docs/cli-analyze.md) for flags and the input/output schema.

---

### Tests

```powershell
npm test        # = cargo test --manifest-path src-tauri/Cargo.toml
```

Regression fixtures live in `src-tauri/tests/fixtures/`; `src-tauri/tests/pipeline.rs` checks text conservation for every strategy/priority, drop caps, run-in heads, hanging indents, hyphenation, stitching across columns and pages, running heads and the Tagged-PDF path.

---

### `tools/generate_xray_icons.py` — App Icon Generator (Python)

Generates the full set of application icons (`32x32.png`, `128x128.png`, `256x256.png`, `512x512.png`, `icon.png`, `icon.ico`, `icon.icns`) from scratch using procedural drawing. Run this to regenerate all icons after a visual style change.

**Requirements:** `pip install pillow`

```powershell
# Run from the repository root
python tools/generate_xray_icons.py
```

Output files are written directly to `src-tauri/icons/`.

---

### `tools/regenerate_xray_icons.py` — Icon Resizer (Python)

Lightweight utility that derives the `128x128@2x.png` and `32x32.png` variants by downscaling the existing `256x256.png` with Lanczos resampling. Use this instead of `generate_xray_icons.py` when you only want to refresh the derivative sizes without redrawing the base artwork.

**Requirements:** `pip install pillow`

```powershell
# Run from the repository root
python tools/regenerate_xray_icons.py
```

---

## ⚖️ License
This project is licensed under a custom license. See the [LICENSE](LICENSE) file for details.
