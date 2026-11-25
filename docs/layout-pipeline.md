# Layout & Reading-Flow Pipeline — Technical Reference

The goal of the engine is to turn a PDF into **logical paragraphs**: units of
text that can be translated as a whole even when the typesetter split them
across columns, pages, figures or boxes, and that know exactly where each of
their pieces sits on the page so the translation can be laid back on top.

All the analysis runs in Rust (`src-tauri/src/`), on the whole document at
once, in **PDF points** (pdf.js viewport at scale 1, origin top-left). Zoom
only affects drawing.

```
runs ─► lines ─► XY-Cut blocks ─► roles ─► reading order ─► units ─► stitching ─► logical paragraphs
(extract.js) (lines.rs) (xycut.rs)  (roles.rs)  (order.rs)     (flow.rs)  (flow.rs)    (pipeline.rs)
                                                                                          │
                                         translated text ─► reflow.rs ─► overlay (translate.js)
```

| Module | Responsibility |
|---|---|
| `src/extract.js` | pdf.js extraction: run boxes (rotation-aware), real font names, Tagged-PDF info (MCID, `/Artifact`, structure element + role), CTM-aware vector rectangles and images |
| `model.rs` | Input (`PageInput`, `TextElement`, `AnalysisOptions`) and output (`DocumentResult`, `PageResult`, `LayoutNode`, `LogicalParagraph`, `Segment`, `Join`) |
| `text.rs` | Normalisation (ligatures, soft hyphens), gap-based run joining, de-hyphenation, lexical heuristics (terminal punctuation, page numbers, caption prefixes, bullets, repetition keys) |
| `lines.rs` | Runs → lines, drop caps |
| `xycut.rs` | Recursive XY-Cut over lines with a conservation invariant; bordered boxes and tables |
| `roles.rs` | Block roles (page level + running heads across pages) |
| `order.rs` | Reading order (Breuel topological sort) |
| `flow.rs` | Paragraph split inside blocks, Tagged-PDF units, stitching |
| `reflow.rs` | Distribution of a translated paragraph over its segments |
| `pipeline.rs` | Orchestration, statistics |
| `ocr/` | Pages without a text layer: classification, PaddleOCR engine on `rten`, model download, OCR metrics (see [OCR](#ocr-ocr-srcocrjs)) |

---

## 1. Extraction (`src/extract.js`)

* **Boxes.** Each run's four corners are computed from its text matrix and the
  font ascent/descent, then mapped through the viewport: rotated and vertical
  text get a correct axis-aligned box. Font size is `|(c, d)|` of the text
  matrix (not `transform[3]`, which is wrong for rotated text).
* **Fonts.** The real PDF font name (`ABCDEF+Garamond-Bold`) is read from
  `page.commonObjs`; bold/italic flags are appended when the name hides them.
* **Tagged PDF.** `getTextContent({includeMarkedContent: true})` returns
  begin/end marked-content markers; a stack gives each run its innermost MCID
  and whether it is inside `/Artifact`. `page.getStructTree()` is walked to map
  every MCID to its *block-level* structure element (`P`, `H1…H6`, `Caption`,
  `Note`, `LI`, `TD`…). Inside list items, table cells, captions and notes the
  outermost element owns the content, so a list item is one unit.
* **Graphics.** The operator list is replayed with a CTM stack (`save`,
  `restore`, `transform`, form XObjects). pdf.js ≥ 5 encodes paths as
  `constructPath [paintOp, [Float32Array], minMax]` with `DrawOPS`; closed
  axis-aligned quadrilaterals that are stroked or filled become rectangles,
  and so do rounded rectangles (axis-aligned sides joined by corner curves,
  the sides covering ≥ 70 % of the perimeter, no oblique segment);
  `paintImageXObject` & co. become image boxes (the unit square under the CTM).

## 2. Lines (`lines.rs`)

Runs are swept **left → right**. A run joins the line whose right end is
horizontally adjacent (gap ≤ 1 em of the *smaller* of the two fonts, so a
display heading cannot stretch a body line across a gutter) and whose core overlaps it vertically by at
least half the smaller height; otherwise it starts a new line. Consequences:

* column gutters and table gaps always split lines;
* a tall run in one column can never bridge two lines of another column;
* superscripts stay on their line.

Text inside a line is joined by **geometric gap**: a space is inserted only
when the gap exceeds 0.15 em, so words split into several runs (kerning, font
changes) are rebuilt (`tra` + `duzione` → `traduzione`).

**Drop caps**: a 1–2 letter run ≥ 2.5× the body size with smaller text to its
right is merged into the first line it introduces (`T` + `he Cosmere` →
`The Cosmere`); its box is remembered so the indented lines beside it are not
taken for new paragraphs.

**Rotated text**: runs are grouped by reading direction (0°, 90°, 180°,
270°) and swept into lines in their own frame, where the text is horizontal:
text rotated 90° reads top→bottom with its lines from right to left, 270°
bottom→top with lines from left to right. Each line keeps its frame bounds
(`fb`); row grouping and indentation use them, so a rotated caption reads in
order and small-caps runs of a credit join up (`GORDON M` + `C` + `ALPIN`).

**Decorative glyphs**: a font used on the page only for stacked 1–2 character
runs of letters/digits at ≥ body size, at least four times (an ornament column
drawn with a symbol font), is a *glyph font*. Its runs are never drop caps,
never join lines of other fonts, and lines made only of them are flagged
`decor`: XY-Cut leaves them out of the cuts (one separate leaf) and they get
the `decoration` role (not translatable). Bullets (not alphanumeric),
superscripts (smaller than body text) and drop caps (isolated) don't qualify.

## 3. XY-Cut (`xycut.rs`)

Classic recursive XY-Cut, but over *lines*, with three changes:

1. **Conservation invariant.** A cut is a set of split coordinates (gap
   mid-points); every line is assigned by its centre, so nothing can fall
   into a gap. Groups thinner than `min_width` / `min_height` are merged with
   the nearest neighbour instead of being skipped. The old engine silently
   dropped text there (e.g. the centred title and the footer of `page5`).
2. **Per-row column fallback.** When a centred title spans the gutter the
   global X projection has no gap. Rows vote for a consistent split line; the
   lines crossing it are isolated with horizontal cuts first.
3. **Obstacles.** Images and boxes take part in the projections, so a cut
   cannot run "through" a figure.

Thresholds (`AnalysisOptions.strategy`, overridden by `manual`):

| Strategy | `T_x` | `T_y` |
|---|---|---|
| `combined` (default) | max(delta-x valley, 1 em) | leading-relative (below) |
| `delta-x` | valley between word-spacing and gutter peaks of the run-gap histogram | leading-relative (below) |
| `zero-run` | 10 pt | 5 pt |
| `dominant-font` | 1 em | 1 em |

**Leading-relative `T_y`**: 3.5 × the median gap between consecutive lines of
the block, clamped to [max(4 pt, ½ em), 1 em] (1 em when the block has fewer
than 5 line gaps). Section spacing and the space above headings is usually
2–4× the normal line gap but often below 1 em (e.g. 7–9 pt with 9.5 pt type).

`priority` (`Y` rows first, `X` columns first, `max-gap`) decides which
direction is tried first. When the global X gaps only isolate thin groups (a
vertical credit in the margin, a folio) and collapse on partition, the
per-row column vote is tried as well.

**Background images.** An image is decorative artwork, not an obstacle, when
at least 3 lines are printed over it covering ≥ 10 % of its area, when its
edge runs through ≥ 2 lines (text flows around figures, never across them),
or when it bleeds off the page with text on it (an ornamental page edge).
Inside a block, an obstacle with one of the block's own lines printed on it
is ignored as well. Background images are not reading-order separators and
don't make captions.

**Boxes.** Rectangles are cleaned (page frames, rules, duplicates removed);
groups of ≥ 3 touching rectangles become one **table**. A rectangle that
bleeds off the page or whose edge runs through ≥ 2 lines is decoration and is
dropped. Each line goes to the smallest box containing its centre; a box gets
its own sub-tree. A shading behind a single row (highlighted heading, shaded
table row) is not a box: its lines go to the next enclosing box, or to the
body. For the body cuts a box is an obstacle by its *text extent* (the frame's
padding and the ornaments inside it must not narrow a neighbouring gutter).

**Per-row column vote.** When the lines crossing the voted split cannot be
isolated with horizontal cuts and they outnumber the voting rows, the votes
are word spaces of loosely justified lines: no split. When they can be
isolated but that takes more than 2 × votes + 2 cuts, the "column" is a local
feature (OCR drop-cap debris, a short indented run) and isolating would
shatter running text into single lines: no split either.

**Rotated lines** are kept out of the body cuts and subdivided on their own,
per direction; their blocks get the `figure-text` role (also when a Tagged
PDF marks them as artifacts).

## 4. Roles (`roles.rs`)

Per block, in this order:

1. **Tagged PDF**: ≥ 80 % `/Artifact` → `header` / `footer` / `page-number`;
   structure roles map to `heading`, `caption`, `footnote`, `table`, `figure-text`.
2. **Decorative glyphs** (see Lines) → `decoration`.
3. **Boxes**: `table`, `sidebar` (or `caption` if it starts with "Figure…").
4. **Rotated or vertical text** (credits, margin labels) → `figure-text`.
5. **Margins**: page-number patterns (`12`, `xii`, `Page 3`, `3 / 120`,
   `Pag. 7 di 30`) in the outer 12 % → `page-number`; ≤ 2 rows in the outer
   7 % → `header` / `footer`.
6. **Captions**: caption prefix (`Fig.`, `Figure`, `Tabella`, `Abb.`, `Source:`…)
   and/or adjacency to an image, with smaller or italic type.
7. **Footnotes**: lower part of the page, type ≤ 90 % of the body size,
   starting with a marker (digit, `*`, `†`, superscript).
8. Otherwise `body` (later split into `heading` / `body` per paragraph).

**Running heads across pages**: blocks in the outer 12 % bands with ≤ 2 rows
whose *repetition key* (lower-case, digits → `#`, roman folios folded) recurs
at the same height within ±4 pages become `header` / `footer` / `page-number`.

## 5. Reading order (`order.rs`)

XY-Cut order depends on which cut is tried first: with rows-first a
horizontal gap between a figure and its caption can make the right column be
read before the left one. The order is therefore recomputed as a
**topological sort of Breuel's relations**:

1. `a` before `b` if they overlap horizontally and `a` is above `b`;
2. `a` before `b` if `a` is entirely left of `b` and no block or figure
   lying vertically between them overlaps both.

Ties follow the XY-Cut order. Headers come first; footnotes, figure text,
footers and folios last.

## 6. Units (`flow.rs`)

**Untagged pages.** Each block is split into paragraphs between rows `p`, `c` when:

| Signal | Rule |
|---|---|
| size change | font size differs by > 15 % |
| bullet | `c` starts with `•`, `–`, `1.`, `a)`… |
| extra leading | line step > 1.4 × median leading |
| short last line | justified: `p` ends ≥ 2 em before the right edge and ends a sentence or `c` starts upper-case; ragged: `p` ends ≥ 25 % short and ends a sentence |
| indent / hanging indent | `c` indented (or outdented w.r.t. `p`) and `p` ends a sentence |
| weight change | whole-row bold ↔ regular and `p` ends a sentence or is short |

A unit of ≤ 3 rows in a larger size (≥ 1.15× body), or entirely bold and not
ending with a full stop, is a `heading`. Bold *run-in* heads ("**Chapter 1.**
This walks you…") stay in their paragraph because a row is bold only when
≥ 95 % of its characters are.

**Tagged pages** (≥ 70 % of the non-artifact text has an MCID): one unit per
structure element in structure order; a `P` that continues in another column
is a single unit with two segments.

**De-hyphenation.** A line-final soft hyphen (U+00AD) is always removed. A
line-final `-` before a lower-case word is removed (`tradu-` + `zione`) —
unless the document uses soft hyphens for discretionary breaks, in which case
`-` is a real compound and is kept (`storm-` + `blasted` → `storm-blasted`).

## 7. Stitching (`flow.rs`)

Units are walked in document order. Body text forms the `main` thread, each
box its own thread; captions, footnotes, headers… never join a thread but do
not break it either (they are recorded as an *interruption*). A heading closes
the main thread. The last unit of a thread is continued by the next unit of
the same thread when:

* the first is the last unit of its block and the second the first of its block;
* the first does not end a sentence (or ends with a hyphen);
* font sizes match within 12 % and the second does not start with a bullet;
* and either the second starts in lower case, or the first ended with a hyphen,
  or (upper-case start) the first has ≥ 2 rows, a full last line, the second is
  not indented and the weights match;
* structure-tagged units are only stitched across pages (inside a page the
  structure tree already decided).

Each `Join` records its kind (`column`, `page`, `box`), whether something was
interrupted, and whether a hyphenated word was rebuilt.

## 8. Translation overlay (`reflow.rs`, `src/translate.js`)

A translated paragraph is split over its segments proportionally to the
characters each segment held in the original, cutting at word boundaries and
preferring sentence boundaries within ±12 %. Each piece is drawn in an HTML
box over its segment, with the original leading, weight, style and alignment,
shrinking the font (down to 45 %) until it fits; boxes that still overflow
are outlined in amber.

Each segment also carries its reading direction (`rot`: 0, 90, 180 or 270,
the prevailing direction of its lines). For rotated segments the alignment is
measured in the reading frame (`fb`), where the lines run horizontally.
GianoReader uses `rot` to lay out the translation box in the text direction
and rotate it like the original.

## OCR (`ocr/`, `src/ocr.js`)

Pages whose text is only in the pixels (scans, image-only pages) go through
OCR; the recognised lines become ordinary `TextElement`s, so everything from
§2 on is unchanged.

```
extract.js ─► classify_pages ─► (scan) rasterise 300 DPI ─► ocr_page ─► TextElements ─► pipeline
                               (text) text layer as is
```

**Classification** (`ocr/classify.rs`). A page is a `scan` when its text layer
has fewer than 40 visible characters and images cover at least half of it
(union of the image boxes, sampled on a 48 × 48 grid, so image strips add
up). Scans that already carry a text layer (an invisible OCR layer) keep it.
The UI can override per document (*Automatico* / *Strato di testo* / *OCR su
tutte le pagine*) and per page.

**Rasterisation** (`src/ocr.js`). pdf.js renders the page on white at 300 DPI
(at most 5000 px on the longer side); the RGBA pixels go to Rust as a raw IPC
body (`ocr_page`, headers `x-width`, `x-height`, `x-scale` = pixels per point).

**Engine** (`ocr/paddle.rs`, PaddleOCR PP-OCRv5 on `rten`, pure Rust, CPU):

1. *Detection* (`PP-OCRv5_mobile_det`, DBNet): the image is resized to a longer
   side of 1600 px (multiple of 32), normalised with the ImageNet mean/std in
   BGR. The probability map is thresholded at 0.3; each connected region with
   mean probability ≥ 0.6 becomes a box, grown by the DB unclip distance
   (area × 1.5 / perimeter), since the model is trained on shrunk regions.
2. *Recognition* (`latin_PP-OCRv5_mobile_rec`, CTC, 836 characters including
   accented Latin letters): each box is cropped from the full-resolution image,
   resized to height 48, normalised to [−1, 1], batched by aspect ratio and
   decoded greedily (blank 0, dictionary, space). Boxes 1.5 × taller than wide
   are also read rotated by ±90°; the most confident reading wins and sets
   `rot` (vertical credits). Lines below 0.5 confidence are dropped.
3. *Word boxes* come from the CTC time steps of each character.

**To text runs** (`ocr::to_text_elements`). Box heights depend on the letters
of a line (no ascenders → lower box), which the paragraph heuristics would
read as size and leading changes. Lines are grouped around the modes of their
thickness (±20 %, weighted by characters) and every line gets its group's
median thickness, centred on the line; the font size is thickness / 1.3
(calibrated on the text layer of digital pages). A line is split into several
runs where two words are farther apart than the line height, so a box that
crosses a gutter does not glue two columns. Runs carry `source: "ocr"` and
`conf`; the **Words** overlay shows them in violet, orange below 0.8.

**Models** (`ocr/models.rs`). Not in the installer: on first use the app
shows the files (12.3 MB), their source and the destination folder, and
downloads them only after confirmation (`ocr_download_models`, progress
events). Mirrors: the project's GitHub release `models-v1`, then the official
PaddlePaddle repositories on Hugging Face. Every file is checked against its
SHA-256 before it is renamed into place. Recognition never uses the network.
`$PDFXRAY_MODEL_DIR` overrides the folder (development, tests).

**Measuring the OCR**. OCR snapshots store the exact image given to the
engine and a reference transcription; `cli_analyze --eval-ocr` re-runs the
current engine and reports CER / WER (`ocr/metrics.rs`: lines paired by edit
distance regardless of order, typography normalised). See below and
[`cli-analyze.md`](cli-analyze.md#ocr---ocr---eval-ocr).

Measured on a 300 DPI rendering of a two-column RPG page (the text layer as
reference): CER 0.15 %, WER 1.1 %, 3.8 s per page in a release build
(detection 0.5 s, recognition 3.2 s); the layout from the OCR runs matches the
one from the text layer (same paragraphs, body size 9 pt).

## Tuning with snapshots

Automatic thresholds are heuristics; the way to improve them is to collect
pages where manual settings do better and measure.

1. In the app, find settings that segment a page correctly (manual sliders or
   another strategy/priority), then press **📸 Snapshot** and give a verdict
   and a note. The JSON (plus a PNG of the view) is written to
   `Documents/PDF-X-Ray/snapshots`.
2. The snapshot contains the page input (with ±1 neighbour pages), the
   options, the results of the stored settings, of automatic thresholds with
   the same strategy/priority and of the default automatic settings — each
   with the full tree and its **diagnostics** (`diag` on every node:
   thresholds, largest gaps, plans tried, decision) — and the reference
   segmentation.
3. `cli_analyze --eval <folder>` scores every snapshot (pairwise F1 over
   lines against the reference) so a heuristic change can be measured on all
   annotated pages at once.
4. Copy representative snapshots to `src-tauri/tests/snapshots/`; adding
   `"minAutoScore": 0.95` to one turns it into a regression test
   (`tests/snapshots.rs`).
5. On OCR pages the dialog also asks for the **reference text**: correct the
   recognised lines in the text box, confirm them as correct, or (for a
   digital page forced to OCR from *Questa pagina*) use the PDF text layer —
   a free reference that makes any digital PDF a test set for the OCR. The
   image given to the OCR is saved as `<snapshot>_ocr.png`;
   `cli_analyze --eval-ocr` scores it, and `"maxOcrCer"` in the `ocr` section
   locks a page against regressions (copy the PNG along with the JSON).

## Tests

```powershell
cargo test --manifest-path src-tauri/Cargo.toml   # or: npm test
```

* unit tests in each module (joining, de-hyphenation, page numbers, captions,
  drop caps, reading order, reflow);
* `src-tauri/tests/pipeline.rs`: conservation on the fixtures for every
  strategy × priority, drop cap / title / footer on `page5`, run-in heads,
  hanging indent and hyphen policy on `page6`, a synthetic two-page document
  (column + page stitching, caption interruption, running heads, folios) and a
  Tagged-PDF page (structure order, artifacts, captions).

## Known limits

* Lists of "Label: value" lines without punctuation (credits pages) are kept
  as large units.
* Hard-hyphen removal without soft hyphens in the document is heuristic
  (no dictionary): a real compound split at a line end loses its hyphen.
* Tables are detected only from ruled grids; borderless tables are segmented
  as text blocks.
* Text drawn as vector outlines (no text layer, no image) is not detected as
  needing OCR: force OCR on those pages from the interface.
* OCR: Latin script only; no reading of text on curved baselines; a large
  drop cap is often not detected (the first letter of the paragraph is lost);
  font weight and style are not recognised (OCR runs are never bold/italic).
* OCR runs on the whole document before the first analysis: a long scanned
  book takes a few seconds per page.
