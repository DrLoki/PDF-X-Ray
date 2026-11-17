# `cli_analyze` — Headless Analyzer

`src-tauri/src/bin/cli_analyze.rs` runs the same Rust pipeline as the desktop
app (`pipeline::analyze_document`) from the command line: batch analysis,
regression checks and parameter comparisons without the GUI.

## Build

```powershell
cd src-tauri
cargo build --features cli --bin cli_analyze        # target/debug/cli_analyze.exe
cargo build --release --features cli --bin cli_analyze
```

## Usage

```
cli_analyze [--text] [--priority X|Y|max-gap] [--strategy NAME] [--no-tags] [--no-stitch] [FILE]
cli_analyze --distribute [FILE]
cli_analyze --eval [--json] SNAPSHOT_OR_DIR...
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

## Typical pipeline

```powershell
# whole document (same extractor as the app, Node 18+)
node tools/extract_page.js "C:/path/doc.pdf" all > doc.json
src-tauri/target/debug/cli_analyze --text doc.json

# a page range, columns first
node tools/extract_page.js "C:/path/doc.pdf" 10-14 | src-tauri/target/debug/cli_analyze --text --priority X

# full JSON result
src-tauri/target/debug/cli_analyze doc.json > result.json
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
src-tauri/target/debug/cli_analyze --eval "$env:USERPROFILE/Documents/PDF-X-Ray/snapshots"
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

## Input schema

### `PageInput`

| Field | Type | Notes |
|---|---|---|
| `pageNum` | number | 1-based (default 1) |
| `pageBounds` | `Bounds` | page box in points (`{x, y, w, h}`) |
| `items` | `TextElement[]` | text runs |
| `borderedBoxes` | `Bounds[]` | vector rectangles (optional) |
| `images` | `Bounds[]` | placed images (optional) |

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

### `AnalysisOptions`

```json
{
  "strategy": "combined",
  "priority": "Y",
  "manual": { "thresholdX": 15, "thresholdY": 10, "minWidth": 20, "minHeight": 0 },
  "useTags": true,
  "stitch": true
}
```

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
    "role": "body",              // body, heading, header, footer, page-number, caption, footnote, sidebar, table, figure-text
    "text": "…",                 // stitched, de-hyphenated: the unit of translation
    "translatable": true,
    "segments": [{ "pageNum": 1, "blockId": "p1-b7", "bounds": {…}, "lines": [Bounds], "text": "…",
                   "charCount": 180, "fontSize": 10, "fontName": "…", "bold": false, "italic": false,
                   "alignment": "justify" }],
    "joins": [{ "atSegment": 1, "kind": "page", "interrupted": true, "dehyphenated": false }]
  }],
  "stats": { "pageCount": 2, "paragraphCount": 11, "stitchedJoins": 2, "bodyFontSize": 10,
             "inputChars": 725, "outputChars": 725 }
}
```

See [`layout-pipeline.md`](layout-pipeline.md) for how each field is computed.
