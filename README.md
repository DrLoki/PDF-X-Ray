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

## ⚖️ License
This project is licensed under a custom license. See the [LICENSE](LICENSE) file for details.
