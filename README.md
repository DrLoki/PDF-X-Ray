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

3. **Recursive XY-CUT Layout Analysis**:
   - Analyzes raw character boxes and segments them into coherent visual blocks (columns, paragraphs, headings).
   - Dynamic parameter sliders: adjust **Row Gap ($T_y$)**, **Column Gap ($T_x$)**, **Minimum Block Width**, **Minimum Block Height**, and **Priority Direction** (Horizontal First, Vertical First, or Largest Gap First).
   - Live visual overlays:
     - **Word Bounding Boxes** (translucent cyan outline).
     - **Decomposed Blocks** (color-coded by recursion levels).
     - **Math Projection Histograms** (density graph curves rendered on the bottom and right margins).
     - **Valid Split Cuts** (dotted partition lines showing where cuts occurred).

4. **Bi-directional Synced Interlock**:
   - Hovering over a block on the PDF canvas highlights the corresponding node in the layout DOM tree.
   - Hovering over a list item in the DOM tree highlights its bounding box on the canvas.
   - Clicking a canvas element or DOM list item loads its properties into the Formatting Inspector.

5. **Formatting Inspector**:
   - Extracts font family, average size, bold/italic style flags, precise coordinate metrics, and margins.
   - Evaluates block alignment (Left, Right, Center, Justified) based on horizontal baseline offsets.
   - Copies block text to your clipboard with a single click.

6. **DOM Layout Export**:
   - Save the segmented page structure hierarchy as **JSON** structures, clean **XML** schemas, or self-contained styled **HTML** documents with absolute layouts.

---

## 📐 How the XY-Cut Algorithm Works

The app implements a custom, resolution-independent **Recursive XY-Cut** algorithm in `src/xycut.js`:

1. **Interval-Based Projection**:
   Instead of using pixel bins, the algorithm projects the horizontal $[x_0, x_1]$ or vertical $[y_0, y_1]$ span of each text fragment onto the coordinate axes.
2. **Interval Merge (Interval Union)**:
   All overlapping coordinate intervals are sorted and merged into disjoint occupied intervals.
3. **Gap Detection**:
   The unoccupied spaces between these merged intervals are the "valleys" or "white-space gaps". Gaps larger than the thresholds ($T_x$ or $T_y$) are candidates for cuts.
4. **Recursive Slicing**:
   The block is split along the valid gaps into multiple sub-blocks, which are recursively partitioned. If no gaps exceed the threshold or the block is smaller than the minimum dimensions, it is finalized as a **Leaf Node**.
5. **Text Reordering**:
   Fragments in leaf nodes are clustered into rows using a baseline tolerance threshold of `5px` to reassemble natural reading order from top-to-bottom and left-to-right.

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

The `tools/` directory and the Rust `src-tauri/src/bin/` directory contain helper utilities for offline development and debugging of the XY-Cut pipeline without launching the full desktop app.

### `tools/extract_page_py.py` — PDF Page Extractor (Python)

Extracts text elements from a single PDF page using **PyMuPDF** and outputs the JSON format expected by `cli_analyze`.

**Requirements:** `pip install pymupdf`

```powershell
python tools/extract_page_py.py "C:/path/to/document.pdf" <pageNum> > page.json
```

Output JSON contains `items`, `pageBounds`, and `borderedBoxes` fields ready for piping into `cli_analyze`.

> **Recommended extractor.** Works reliably with Node.js 22+ without any native canvas dependency.

---

### `tools/extract_page.js` — PDF Page Extractor (Node.js)

Alternative extractor that uses the same **pdf.js** engine as the desktop app, producing coordinate-identical output.

```powershell
node tools/extract_page.js "C:/path/to/document.pdf" <pageNum> > page.json
```

> **Note:** Requires a Node.js build with native `DOMMatrix` support (canvas package or Node ≤ 18). With Node.js 22+ it may fail with `DOMMatrix is not defined` — use the Python extractor instead.

---

### `src-tauri/src/bin/cli_analyze.rs` — XY-Cut CLI Analyzer (Rust)

A command-line interface to the same Rust XY-Cut engine used by the desktop app. Reads a JSON page description from a file or stdin and writes the full layout tree to stdout. Useful for batch analysis, regression testing, and strategy comparison without opening the GUI.

**Build:**
```powershell
cd src-tauri
cargo build --bin cli_analyze
# binary: src-tauri/target/debug/cli_analyze.exe
```

**Usage:**
```powershell
# From file
src-tauri/target/debug/cli_analyze page.json

# From stdin (pipe from extractor)
python tools/extract_page_py.py "doc.pdf" 6 | src-tauri/target/debug/cli_analyze

# Specify a strategy
$input = Get-Content page.json -Raw | ConvertFrom-Json
$input | Add-Member -NotePropertyName strategy -NotePropertyValue "zero-run" -Force
$input | ConvertTo-Json -Depth 20 -Compress | src-tauri/target/debug/cli_analyze
```

**Available strategies:** `combined` (default), `delta-x`, `zero-run`, `dominant-font`.

Full pipeline — extract page 6 and analyze with all strategies:
```powershell
python tools/extract_page_py.py "C:/path/to/document.pdf" 6 > page6.json

foreach ($s in @("combined", "delta-x", "zero-run", "dominant-font")) {
    $inp = Get-Content page6.json -Raw | ConvertFrom-Json
    $inp | Add-Member -NotePropertyName strategy -NotePropertyValue $s -Force
    $inp | ConvertTo-Json -Depth 20 -Compress |
        src-tauri/target/debug/cli_analyze > "result_$s.json"
    Write-Host "[$s] root children: $((Get-Content result_$s.json | ConvertFrom-Json).root.children.Count)"
}
```

See [`docs/cli-analyze.md`](docs/cli-analyze.md) for the full input/output schema reference.

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
