# Gutter Detection — Technical Reference

## Overview

When **Auto-Detect Gutters (Rust)** is enabled, the layout analysis is delegated to a native Rust backend (via Tauri IPC) instead of the JavaScript XY-Cut engine. The Rust module (`gutter_detection.rs`) analyses the geometric distribution of text elements on the page and automatically determines the four XY-Cut threshold parameters:

| UI Parameter | Internal name | Role |
|---|---|---|
| Row Gap (`$T_y$`) | `dynamic_threshold_y` | Minimum vertical white space to trigger a horizontal cut (Y-Cut) |
| Column Gap (`$T_x$`) | `dynamic_threshold_x` | Minimum horizontal white space to trigger a vertical cut (X-Cut) |
| Min Block Width | `min_width` | Minimum block width in pixels below which recursion stops |
| Min Block Height | `min_height` | Minimum block height in pixels below which recursion stops |

Once the Rust analysis completes, the detected gap values are written back to the four sliders and the checkbox is automatically unchecked, so the user can see the inferred values and fine-tune them manually if needed.

---

## The Four Auto-Gutter Strategies

The strategy is selected via the **Auto Gutter Strategy** dropdown and passed as a string to the Rust `perform_auto_xycut` entrypoint. It controls how `dynamic_threshold_x` and `dynamic_threshold_y` are computed inside `subdivide_node` at each level of the recursion tree.

### Strategy 1 — Delta-X Statistics (`delta-x`)

**Thresholds produced:**
- `T_x` = valley point between word-spacing peak and gutter peak (histogram analysis)
- `T_y` = `dominant_font × 1.0`, floored at `4 px`

**How it works:**

This strategy analyses the *horizontal distance between adjacent text elements* on the same visual line.

1. **Line grouping.** Text elements are clustered into visual rows by grouping elements whose `y` coordinates differ by less than 4 px (baseline tolerance).
2. **Delta computation.** For each row, elements are sorted left-to-right and the gap between the right edge of element *i* (`x + width`) and the left edge of element *i+1* is recorded. Only positive deltas below 150 pt are kept.
3. **Histogram construction.** All collected deltas are binned into a 1 pt-resolution histogram (100 bins, range 0–100 pt).
4. **Peak B — Word Spacing.** The bin with the highest frequency in the range 2–10 pt is identified as typical word spacing.
5. **Peak C — Column Gutter.** The bin with the highest frequency in the range `(Peak_B + 4)` to `80 pt` is identified as the candidate column gutter.
6. **Valley detection.** If Peak C exists and has a frequency ≥ 2, the algorithm scans the histogram between Peak B and Peak C and finds the *minimum frequency bin* (the valley). This valley coordinate becomes `T_x`.
7. **Fallback.** If no Peak C is found (single-column or uniform spacing), the threshold falls back to `dominant_font × 1.5`.

**Best suited for:** multi-column documents with clearly distinct word-spacing and column-gutter distributions (e.g. technical manuals, newspapers).

---

### Strategy 2 — Zero-Run Projection (`zero-run`)

**Thresholds produced:**
- `T_x` = `10 px` (fixed)
- `T_y` = `5 px` (fixed)

**How it works:**

This strategy does not compute adaptive thresholds. Instead it relies on a minimal fixed threshold and lets the projection gap finder (`find_projection_gaps`) do all the work.

`find_projection_gaps` projects all text bounding boxes onto a single axis (X or Y) by merging their overlapping intervals, then enumerates the *empty spans* (zero-run lengths) between merged occupied zones. Any gap larger than the fixed threshold becomes a valid cut point.

Because the thresholds are very low (10 px / 5 px), this strategy will detect even narrow white-space corridors. This makes it sensitive but also prone to over-segmentation on dense layouts.

**Best suited for:** sparse layouts, wide-margin documents, or pages where the gutter is narrow and hard to pick up statistically.

---

### Strategy 3 — Dominant Font (`dominant-font`)

**Thresholds produced:**
- `T_x` = `dominant_font × 1.5`
- `T_y` = `dominant_font × 1.0`

**How it works:**

This strategy derives thresholds directly from the *typographic scale* of the current block.

1. **Dominant font size.** The statistical **mode** of all `font_size` values in the block is computed. Because font sizes are rounded to 1 decimal place before binning, the mode reliably identifies the body-text size (e.g. 10 pt) even when headings or captions are present.
2. **Column gap formula.** `T_x = dominant_font × 1.5`. Editorial convention dictates that a gutter between columns is at least 1.5× the body text size, so any horizontal gap smaller than this is treated as a tab stop or paragraph indent, not a column boundary.
3. **Row gap formula.** `T_y = dominant_font × 1.0`. A vertical gap equal to one full line height is considered the minimum inter-paragraph spacing.

Because thresholds scale automatically with the document's font size, this strategy is portable across different page sizes and point sizes without manual calibration.

**Best suited for:** standard editorial layouts (books, reports) where body text size is consistent and column gutters follow typographic conventions.

---

### Strategy 4 — Combined (`combined`, default)

**Thresholds produced:**
- `T_x` = `max(delta_x_histogram_result, dominant_font × 1.5)`
- `T_y` = `dominant_font × 1.0`, floored at `4 px`

**How it works:**

This is the most conservative strategy. It runs the full **Delta-X histogram analysis** (Strategy 1) to compute a data-driven column gap estimate, then takes the **maximum** of that estimate and the **Dominant Font heuristic** (Strategy 3):

```
T_x = max(detect_dynamic_column_gap(elements, dominant_font),
          dominant_font × 1.5)
```

The max ensures that even if the histogram valley falls in a very low position (common on pages with many small word gaps), the threshold will never drop below the typographically safe minimum of `dominant_font × 1.5`.

The row gap threshold follows Strategy 3 exactly: `dominant_font × 1.0`, with a floor of 4 px to avoid cuts smaller than a single pixel row.

**Best suited for:** unknown or mixed documents. It is the recommended default because it combines the precision of statistical analysis with the safety net of the typographic heuristic.

---

## Shared Processing Pipeline

Regardless of the selected strategy, `perform_auto_xycut` always executes the following preprocessing steps before the recursive subdivision:

### 1. Footnote Isolation

Elements whose `y ≥ page_height × 0.85` **and** `font_size < doc_mode_font` are extracted from the main element pool and wrapped in a dedicated footnote block. This prevents footnote text from polluting the column-gap statistics of the main body.

### 2. Bordered Box Isolation

Vector rectangles detected on the page (callout boxes, tables with borders) are processed before the main body. Text elements whose centre point falls inside a bordered rectangle are removed from the main pool and subdivided independently within that rectangle. Boxes are processed largest-first to handle nesting correctly.

### 3. Recursive XY-Cut

The remaining elements are passed to `subdivide_node`, which at each level:

1. Computes the dominant font size for the current sub-block.
2. Derives `T_x` and `T_y` according to the selected strategy.
3. Calls `find_projection_gaps` on both axes to enumerate white-space spans.
4. Filters gaps smaller than the respective threshold.
5. **Prefers Y-Cut** (horizontal split) over X-Cut when both are available — this isolates spanning titles and section breaks before attempting column detection.
6. Validates X-Cut positions: gaps whose midpoint falls within the outer 5% of the block width are discarded to avoid spurious margin splits.
7. Recurses on each child block, up to a hard limit of **24 levels**.

### 4. Leaf Node Finalisation

When no further cuts are possible the node is finalised as a leaf: its text fragments are sorted into reading order (top-to-bottom, left-to-right), dominant font family and size are extracted, and text alignment (`left`, `center`, `right`, `justify`) is inferred from line-edge deviations relative to the block boundary.

---

## Slider Update Flow

After the Rust analysis returns, `main.js` reads the `projections` object from the result:

```json
{
  "xGaps": [{ "start": 320, "end": 345, "size": 25 }],
  "yGaps": [{ "start": 85,  "end": 92,  "size": 7  }]
}
```

The smallest *significant gap* (≥ 5 px) on each axis is selected and converted back to unscaled pixel units (`gap / zoomScale`). The four slider values and their labels are updated programmatically, the Auto-Detect checkbox is unchecked, and the JS XY-Cut engine is re-run once with the new values so the overlay reflects the detected layout immediately.
