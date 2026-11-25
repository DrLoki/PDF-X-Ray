# `cli_analyze` — Headless Analyzer

`src-tauri/examples/cli_analyze.rs` runs the same Rust pipeline as the desktop
app (`pipeline::analyze_document`) from the command line: batch analysis,
regression checks and parameter comparisons without the GUI.

## Build

```powershell
cd src-tauri
cargo build --example cli_analyze            # target/debug/examples/cli_analyze.exe
cargo build --release --example cli_analyze
```

## Usage

```
cli_analyze [--text] [--priority X|Y|max-gap] [--strategy NAME] [--no-tags] [--no-stitch] [FILE]
cli_analyze --distribute [FILE]
cli_analyze --eval [--json] SNAPSHOT_OR_DIR...
cli_analyze --ocr IMAGE [--models DIR] [--det-max-side N] [--ref FILE] [--dpi N [--text]] [--json]
cli_analyze --eval-ocr [--models DIR] [--det-max-side N] [--json] SNAPSHOT_OR_DIR...
cli_analyze --ocr IMAGE --comic [--rtl] [--debug-png OUT] [--dpi N] [--text]
cli_analyze --eval-comic [--json] SNAPSHOT_OR_DIR...
```

`FILE` (or stdin when omitted) contains either

* a **document**: `{ "pages": [PageInput, …], "options": AnalysisOptions? }`, or
* a **single page** (legacy fixtures): `PageInput` plus optional top-level
  `"strategy"` and `"priority"`.

| Flag | Effect |
|---|---|
| `--text` | print the logical paragraphs (`[id p.pages role ⟲joins] text`) and statistics on stderr instead of JSON |
| `--priority` | `Y` (rows first, default), `X` (columns first), `max-gap` |
| `--strategy` | `combined` (default), `delta-x`, `zero-run`, `dominant-font` |
| `--no-tags` | ignore Tagged-PDF information |
| `--no-stitch` | do not stitch paragraphs across blocks / pages |
| `--distribute` | input `{ "units": [ { "text", "weights": [..] } ] }`, output the text split per segment (same as the `distribute_translations` command) |
| `--eval` | score snapshots (files or folders) against their reference segmentation, see below; `--json` for machine-readable output |
| `--ocr IMAGE` | run the OCR engine on an image (PNG) and print the recognised lines, see [OCR](#ocr---ocr---eval-ocr) |
| `--eval-ocr` | score the OCR snapshots against their reference transcription |
| `--models DIR` | OCR model folder (default `$PDFXRAY_MODEL_DIR`, else the app's folder) |
| `--det-max-side N` | longer side of the detection input in pixels (default 1600) |
| `--ref FILE` | with `--ocr`: reference transcription, one line per line; prints CER / WER |
| `--dpi N` | with `--ocr`: convert the lines to text runs (pixels → points at N DPI) and run the layout pipeline |
| `--comic` | with `--ocr`: detect panels and containers and lay the page out as a comic (default 72 DPI: points = pixels) |
| `--rtl` | with `--comic`: read panels and balloons right to left |
| `--debug-png OUT` | with `--comic`: draw panels (blue), balloons (green), narration (orange), sfx (magenta), free text (red) and OCR lines (grey) on the image |
| `--eval-comic` | score the comic snapshots: stored vs re-detected panels and containers |

## Typical pipeline

```powershell
# whole document (same extractor as the app, Node 18+)
node tools/extract_page.js "C:/path/doc.pdf" all > doc.json
src-tauri/target/debug/examples/cli_analyze --text doc.json

# a page range, columns first
node tools/extract_page.js "C:/path/doc.pdf" 10-14 | src-tauri/target/debug/examples/cli_analyze --text --priority X

# full JSON result
src-tauri/target/debug/examples/cli_analyze doc.json > result.json
```

Example `--text` output:

```
[par3 p.1 body ⟲column] This document checks that paragraphs survive column and page breaks. …
[par4 p.1 sidebar] Sidebar: this boxed note must not break the main text …
[par5 p.1-2 body ⟲page] A second paragraph starts … and then goes on to the next page, where …
[par8 p.2 caption] Figure 1: A sample picture.
pages=2 paragraphs=11 stitched=2 body_fs=10 chars in/out=725/725
```

`chars in/out` is the conservation check: every non-blank input character
must appear in exactly one paragraph segment.

## Evaluating snapshots (`--eval`)

Snapshots are saved from the app with **📸 Snapshot** (default folder
`Documents/PDF-X-Ray/snapshots`, or `$PDFXRAY_SNAPSHOT_DIR`). Each one holds the
extracted input of the page and its neighbours, the settings in use, the
user's verdict and — unless the verdict is "both wrong" — a **reference
segmentation** (the validated leaf blocks).

```powershell
src-tauri/target/debug/examples/cli_analyze --eval "$env:USERPROFILE/Documents/PDF-X-Ray/snapshots"
```

```
snapshot                                      page  ref | snapshot sett. | auto (same pr.) |   auto default
test2col_p1_manual-better_20261116-080548.js     1    6 | 1.000 (  6 bl) | 1.000 (  6 bl) | 1.000 (  6 bl)
MEAN F1                                                 |          1.000 |          1.000 |          1.000
```

Columns: the settings stored in the snapshot, automatic thresholds with the
snapshot's strategy and priority, default automatic settings (`combined`,
rows first). The score is the pairwise F1 over lines (two lines are
"together" when they fall in the same block): 1.0 means the same grouping as
the reference. A block count lower than the reference means blocks were
merged (columns not separated); higher means over-segmentation.

The `diag` field of every node in `results.*.root` explains each decision:
thresholds used, largest gap on each axis (even below threshold), candidate
gaps, plans tried and the final decision.

## OCR (`--ocr`, `--eval-ocr`)

The OCR engine (PaddleOCR PP-OCRv5, Latin script, on `rten`) needs its model
files: the app downloads them on first use into
`<app data>/com.drloki.pdfxray/models/paddleocr-v5-latin/` (Windows:
`%APPDATA%\com.drloki.pdfxray\...`). The CLI looks there, or in
`$PDFXRAY_MODEL_DIR`, or in `--models DIR`. Build it in release: debug builds
of the OCR are several times slower.

```powershell
# recognised lines: box in pixels, detection score / recognition confidence, direction, text
cli_analyze --ocr page.png --ref page.txt
#   [  614,  477  1206 x 239] 1.00/1.00 r0   INTRODUCTION
#   [ 2308, 1949    70 x 988] 0.98/0.98 r90  Opposite: Storming of the Bastille by Jean-Pierre Houel
#   65 lines · detect 525 ms · recognize 3244 ms · 2475x3225 px
#   CER 0.15% · WER 1.10% · lines ref 66 / ocr 65 · paired 65 · missed 1 · extra 0

# the same page through the layout pipeline (image rendered at 300 DPI)
cli_analyze --ocr page.png --dpi 300 --text
```

OCR snapshots (taken in the app on a page whose text comes from the OCR)
contain the image given to the OCR (`<snapshot>_ocr.png`, next to the JSON),
the lines recognised at the time and a reference transcription: corrected by
the user, the OCR output confirmed as correct, or — for a digital page forced
to OCR — the lines of the PDF text layer ("simulated scan").

```powershell
cli_analyze --eval-ocr "$env:USERPROFILE/Documents/PDF-X-Ray/snapshots"
```

```
snapshot                                      page reference  |    stored CER/WER |   current CER/WER |      ms
scan_test_p1_reference_20261125-083909.json      1 user       |    0.15% /  1.47% |    0.15% /  1.47% |    5002
MEAN CER                                                      |             0.15% |             0.15% |
```

*stored* scores the lines saved in the snapshot, *current* re-runs today's
engine on the saved image: the difference measures a change to the engine.
Lines are paired regardless of reading order, and quotes, dashes, ligatures
and whitespace are normalised (`ocr::metrics`). `"maxOcrCer": 0.01` in the
snapshot's `ocr` section makes `tests/snapshots.rs` fail when the current
engine does worse (the OCR check is skipped when the models are not installed).

## Comics (`--comic`, `--eval-comic`)

```powershell
cli_analyze --ocr page.jpg --comic --debug-png page_debug.png --text
#   comic: 4 panels · 9 containers · looks like comic: true · 34 ms
#     panel 0: [44,55 444x426]
#     balloon    [575,58 121x68] HEY-YO / LITTLE-
#   [par1 balloon] MEANWHILE, TROUBLE BREWS BY THE SIDE OF THE SLEEK BLACK BEAUTY.
#   [par2 balloon] SO YOU WORK ROPES LOOSE?
```

Comic snapshots (a comic page saved with 📸 with its `_ocr.png`) are scored
with the stored OCR lines, so only the comic detection is measured:

```
snapshot                                      page  ref |   stored F1/order/pan |  current F1/order/pan |    ms
comic_test_p5_reference_20261125-151118.json     5    9 |   1.000 / 1.000 /   4 |   1.000 / 1.000 /   4 |   439
```

*F1* is the pairwise grouping of the lines into containers, *order* the share
of reference container pairs read in the same order, *pan* the panels found.

## Input schema

### `PageInput`

| Field | Type | Notes |
|---|---|---|
| `pageNum` | number | 1-based (default 1) |
| `pageBounds` | `Bounds` | page box in points (`{x, y, w, h}`) |
| `items` | `TextElement[]` | text runs |
| `borderedBoxes` | `Bounds[]` | vector rectangles (optional) |
| `images` | `Bounds[]` | placed images (optional) |
| `comic` | `ComicLayout?` | comic page: `{ panels: Bounds[], balloons: [{ bounds, kind, lines? }], isComic }`, `kind` = `balloon` \| `narration` \| `comic-text` \| `sfx`; when present the page is laid out from it (no XY-Cut) |

`page_bounds` / `bordered_boxes` are accepted as aliases.

### `TextElement`

| Field | Type | Notes |
|---|---|---|
| `text` | string | |
| `x`, `y`, `width`, `height` | number | box in points, origin top-left |
| `font_size` | number | points |
| `font_name` | string? | real font name when available (bold/italic are inferred from it) |
| `mcid` | string? | marked-content id |
| `artifact` | bool | inside `/Artifact` |
| `struct_id` | number? | block-level structure element (tree order) |
| `struct_role` | string? | its role (`P`, `H1`, `Caption`, …) |
| `rot` | number? | baseline direction in degrees, screen coordinates, clockwise (0 horizontal, 90 top→bottom, 270 bottom→top); when absent, a tall run of ≥ 3 characters is taken as 90 |
| `source` | string? | `ocr` for runs recognised from the page image |
| `conf` | number? | OCR recognition confidence (0–1) |

Coordinates may be `null` (a non-finite value from the extractor): such runs,
boxes and images are dropped on input. A run painted twice at the same place
(faux bold) is read once.

### `AnalysisOptions`

```json
{
  "strategy": "combined",
  "priority": "Y",
  "manual": { "thresholdX": 15, "thresholdY": 10, "minWidth": 20, "minHeight": 0 },
  "useTags": true,
  "stitch": true,
  "rtl": false
}
```

`rtl` reads the panels and balloons of comic pages right to left.

`manual` is optional; when present it replaces the automatic thresholds.

## Output schema (`DocumentResult`)

```jsonc
{
  "pages": [{
    "pageNum": 1,
    "pageBounds": {…},
    "root": LayoutNode,          // XY-Cut tree; leaves carry role, readingOrder, paragraphIds, text, formatting
    "projections": { "xGaps": [], "yGaps": [], "pageBounds": {…} },
    "lines": [{ "bounds": {…}, "text": "…", "fontSize": 10 }],
    "dropCaps": [Bounds],
    "taggedOrder": false
  }],
  "paragraphs": [{
    "id": "par5",
    "role": "body",              // body, heading, header, footer, page-number, caption, footnote, sidebar, table, figure-text, decoration;
                                 // comic pages: balloon, narration, comic-text, sfx
    "text": "…",                 // stitched, de-hyphenated: the unit of translation
    "translatable": true,
    "segments": [{ "pageNum": 1, "blockId": "p1-b7", "bounds": {…}, "lines": [Bounds], "text": "…",
                   "charCount": 180, "fontSize": 10, "fontName": "…", "bold": false, "italic": false,
                   "alignment": "justify", "rot": 0,      // rot: reading direction, 0/90/180/270
                   "balloon": { "bounds": {…}, "kind": "balloon",            // comic pages only: the container
                                "shape": [[y, x0, x1], …], "fill": "#f4efe2" } }],  // inside of the shape (rows) and its colour
    "joins": [{ "atSegment": 1, "kind": "page", "interrupted": true, "dehyphenated": false }]
  }],
  "stats": { "pageCount": 2, "paragraphCount": 11, "stitchedJoins": 2, "bodyFontSize": 10,
             "inputChars": 725, "outputChars": 725 }
}
```

See [`layout-pipeline.md`](layout-pipeline.md) for how each field is computed.
