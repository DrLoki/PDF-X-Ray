//! Panel detection on the page raster.
//!
//! 1. The page is reduced to a work grid (about 1000 px on the long side).
//! 2. The *background* colour is the median colour of the page border; the
//!    gutter is the background region connected to the border (flood fill),
//!    so white areas *inside* panels (sky, balloons) are not gutter.
//! 3. Recursive XY-cut: a row (column) is a gutter when at least
//!    [`GUTTER_SHARE`] of it is background colour (tolerating a balloon, a
//!    title or a foot crossing the gutter). A run of such rows is a cut when
//!    part of it is connected to the page border, or when it is thin
//!    ([`MAX_UNLINKED_GAP`]): pages framed as a whole have gutters that never
//!    reach the border, while a white sky inside a framed panel is thick. Rows are tried first (Western page
//!    layout: tiers of panels), then columns.
//! 4. A cell that cannot be cut (a round panel overlapping its neighbours,
//!    gutters that stop half way) is split into the connected regions of
//!    artwork separated by background, when there are several large ones.
//! 5. Cells too small to be a panel (text lines in a gutter, ornaments) are
//!    dropped.

use crate::model::Bounds;
use image::RgbImage;
use std::collections::VecDeque;

/// Long side of the work grid.
const WORK_SIDE: u32 = 1000;
/// Colour distance (max over channels) within which a pixel is background.
const BG_TOL: i32 = 40;
/// Share of a row/column that must be gutter to cut there.
const GUTTER_SHARE: f32 = 0.88;
/// Share of a gutter row/column that must be connected to the page border.
const MIN_LINKED: f32 = 0.05;
/// Longest gutter not connected to the border, as a share of the page side.
const MAX_UNLINKED_GAP: f32 = 0.035;
/// Minimum panel size, as a share of the page width / height.
const MIN_PANEL_W: f32 = 0.08;
const MIN_PANEL_H: f32 = 0.05;
/// A cell must be at least this much non-gutter to be a panel.
const MIN_INK_SHARE: f32 = 0.5;

pub(crate) struct Grid {
    pub w: usize,
    pub h: usize,
    /// Image pixels per grid cell.
    pub step: f32,
    pub px: Vec<[u8; 3]>,
}

impl Grid {
    pub fn new(img: &RgbImage) -> Grid {
        let (iw, ih) = img.dimensions();
        let step = ((iw.max(ih) as f32) / WORK_SIDE as f32).max(1.0);
        let w = ((iw as f32 / step).floor() as usize).max(1);
        let h = ((ih as f32 / step).floor() as usize).max(1);
        let mut px = vec![[0u8; 3]; w * h];
        for gy in 0..h {
            for gx in 0..w {
                // box average over the cell
                let x0 = (gx as f32 * step) as u32;
                let y0 = (gy as f32 * step) as u32;
                let x1 = (((gx + 1) as f32 * step) as u32).clamp(x0 + 1, iw);
                let y1 = (((gy + 1) as f32 * step) as u32).clamp(y0 + 1, ih);
                let mut s = [0u32; 3];
                let mut n = 0u32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = img.get_pixel(x, y);
                        for c in 0..3 {
                            s[c] += p[c] as u32;
                        }
                        n += 1;
                    }
                }
                px[gy * w + gx] = [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8];
            }
        }
        Grid { w, h, step, px }
    }
}

pub(crate) fn dist(a: [u8; 3], b: [u8; 3]) -> i32 {
    (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).max().unwrap_or(0)
}

/// Median colour of the outer border of the grid.
fn border_colour(g: &Grid) -> [u8; 3] {
    let mut chans: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let band = (g.w.min(g.h) / 100).max(1);
    for y in 0..g.h {
        for x in 0..g.w {
            if x < band || y < band || x >= g.w - band || y >= g.h - band {
                let p = g.px[y * g.w + x];
                for c in 0..3 {
                    chans[c].push(p[c]);
                }
            }
        }
    }
    let mut out = [255u8; 3];
    for c in 0..3 {
        chans[c].sort_unstable();
        if !chans[c].is_empty() {
            out[c] = chans[c][chans[c].len() / 2];
        }
    }
    out
}

/// Background pixels, and the background connected to the page border.
fn gutter_mask(g: &Grid) -> (Vec<bool>, Vec<bool>) {
    let bg = border_colour(g);
    let is_bg: Vec<bool> = g.px.iter().map(|&p| dist(p, bg) <= BG_TOL).collect();
    let mut mask = vec![false; g.w * g.h];
    let mut q = VecDeque::new();
    for y in 0..g.h {
        for x in 0..g.w {
            if (x == 0 || y == 0 || x == g.w - 1 || y == g.h - 1) && is_bg[y * g.w + x] && !mask[y * g.w + x] {
                mask[y * g.w + x] = true;
                q.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = q.pop_front() {
        let mut visit = |nx: usize, ny: usize| {
            let i = ny * g.w + nx;
            if is_bg[i] && !mask[i] {
                mask[i] = true;
                q.push_back((nx, ny));
            }
        };
        if x > 0 {
            visit(x - 1, y);
        }
        if y > 0 {
            visit(x, y - 1);
        }
        if x + 1 < g.w {
            visit(x + 1, y);
        }
        if y + 1 < g.h {
            visit(x, y + 1);
        }
    }
    (is_bg, mask)
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

struct Cutter<'a> {
    w: usize,
    bg: &'a [bool],
    mask: &'a [bool],
    min_gap: usize,
    max_unlinked_x: usize,
    max_unlinked_y: usize,
    out: Vec<Rect>,
}

impl Cutter<'_> {
    /// (mostly background, connected to the border)
    fn gutter_state(&self, idx: impl Iterator<Item = usize>, n: usize) -> (bool, bool) {
        let (mut bg, mut linked) = (0usize, 0usize);
        for i in idx {
            bg += self.bg[i] as usize;
            linked += self.mask[i] as usize;
        }
        (bg as f32 >= n as f32 * GUTTER_SHARE, linked as f32 >= (n as f32 * MIN_LINKED).max(1.0))
    }
    fn row_state(&self, y: usize, r: &Rect) -> (bool, bool) {
        self.gutter_state((r.x0..r.x1).map(|x| y * self.w + x), r.x1 - r.x0)
    }
    fn col_state(&self, x: usize, r: &Rect) -> (bool, bool) {
        self.gutter_state((r.y0..r.y1).map(|y| y * self.w + x), r.y1 - r.y0)
    }
    fn row_is_gutter(&self, y: usize, r: &Rect) -> bool {
        let (g, l) = self.row_state(y, r);
        g && l
    }
    fn col_is_gutter(&self, x: usize, r: &Rect) -> bool {
        let (g, l) = self.col_state(x, r);
        g && l
    }

    /// Shrink a rectangle to its non-gutter content.
    fn trim(&self, mut r: Rect) -> Option<Rect> {
        while r.y0 < r.y1 && self.row_is_gutter(r.y0, &r) {
            r.y0 += 1;
        }
        while r.y1 > r.y0 && self.row_is_gutter(r.y1 - 1, &r) {
            r.y1 -= 1;
        }
        while r.x0 < r.x1 && self.col_is_gutter(r.x0, &r) {
            r.x0 += 1;
        }
        while r.x1 > r.x0 && self.col_is_gutter(r.x1 - 1, &r) {
            r.x1 -= 1;
        }
        (r.x1 > r.x0 && r.y1 > r.y0).then_some(r)
    }

    /// Spans of content between gutter runs at least `min_gap` thick; a run
    /// not connected to the border must be thinner than `max_unlinked`.
    fn spans(&self, n0: usize, n1: usize, max_unlinked: usize, state: impl Fn(usize) -> (bool, bool)) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut start = n0;
        let mut i = n0;
        while i < n1 {
            if state(i).0 {
                let g0 = i;
                let mut linked = false;
                while i < n1 {
                    let (g, l) = state(i);
                    if !g {
                        break;
                    }
                    linked |= l;
                    i += 1;
                }
                if i - g0 >= self.min_gap && (linked || i - g0 <= max_unlinked) && g0 > start {
                    spans.push((start, g0));
                    start = i;
                }
            } else {
                i += 1;
            }
        }
        if start < n1 {
            spans.push((start, n1));
        }
        spans
    }

    fn cut(&mut self, r: Rect, depth: usize) {
        let Some(r) = self.trim(r) else { return };
        if depth > 12 {
            self.out.push(r);
            return;
        }
        let rows = self.spans(r.y0, r.y1, self.max_unlinked_y, |y| self.row_state(y, &r));
        if rows.len() > 1 {
            for (a, b) in rows {
                self.cut(Rect { y0: a, y1: b, ..r }, depth + 1);
            }
            return;
        }
        let cols = self.spans(r.x0, r.x1, self.max_unlinked_x, |x| self.col_state(x, &r));
        if cols.len() > 1 {
            for (a, b) in cols {
                self.cut(Rect { x0: a, x1: b, ..r }, depth + 1);
            }
            return;
        }
        self.out.push(r);
    }

    fn ink_share(&self, r: &Rect) -> f32 {
        let mut ink = 0usize;
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                if !self.mask[y * self.w + x] {
                    ink += 1;
                }
            }
        }
        ink as f32 / ((r.x1 - r.x0) * (r.y1 - r.y0)).max(1) as f32
    }
}

/// Boxes of the regions of artwork inside `r`, separated by background.
/// Background thinner than 3 cells (hatching, light areas of the drawing)
/// counts as artwork.
fn components(w: usize, bg: &[bool], r: &Rect) -> Vec<Rect> {
    let (rw, rh) = (r.x1 - r.x0, r.y1 - r.y0);
    let at = |x: usize, y: usize| bg[(r.y0 + y) * w + r.x0 + x];
    // artwork, with background eroded by one cell
    let mut solid = vec![false; rw * rh];
    for y in 0..rh {
        for x in 0..rw {
            let mut s = !at(x, y);
            if !s {
                'n: for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx >= 0 && ny >= 0 && (nx as usize) < rw && (ny as usize) < rh && !at(nx as usize, ny as usize) {
                            s = true;
                            break 'n;
                        }
                    }
                }
            }
            solid[y * rw + x] = s;
        }
    }
    let mut seen = vec![false; rw * rh];
    let mut out = Vec::new();
    let mut q = VecDeque::new();
    for start in 0..rw * rh {
        if !solid[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        q.push_back(start);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        while let Some(i) = q.pop_front() {
            let (x, y) = (i % rw, i / rw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            let mut push = |j: usize| {
                if solid[j] && !seen[j] {
                    seen[j] = true;
                    q.push_back(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < rw {
                push(i + 1);
            }
            if y > 0 {
                push(i - rw);
            }
            if y + 1 < rh {
                push(i + rw);
            }
        }
        out.push(Rect { x0: r.x0 + x0, y0: r.y0 + y0, x1: r.x0 + x1, y1: r.y0 + y1 });
    }
    out
}

/// Panels of a page, in image pixels (unordered).
pub fn detect_panels(img: &RgbImage) -> Vec<Bounds> {
    let g = Grid::new(img);
    detect_on_grid(&g)
}

pub(crate) fn detect_on_grid(g: &Grid) -> Vec<Bounds> {
    let (bg, mask) = gutter_mask(g);
    let mut c = Cutter { w: g.w, bg: &bg, mask: &mask, min_gap: (g.w.max(g.h) / 250).max(2),
        max_unlinked_x: (g.w as f32 * MAX_UNLINKED_GAP) as usize,
        max_unlinked_y: (g.h as f32 * MAX_UNLINKED_GAP) as usize,
        out: Vec::new(),
    };
    c.cut(Rect { x0: 0, y0: 0, x1: g.w, y1: g.h }, 0);
    let cells = std::mem::take(&mut c.out);
    let big = |r: &Rect| (r.x1 - r.x0) as f32 >= g.w as f32 * MIN_PANEL_W && (r.y1 - r.y0) as f32 >= g.h as f32 * MIN_PANEL_H;
    let mut panels: Vec<Rect> = Vec::new();
    for r in cells {
        let parts: Vec<Rect> = components(g.w, &bg, &r).into_iter().filter(|p| big(p)).collect();
        // a region inside another one's box is part of that panel (artwork
        // on a white panel background inside the frame)
        let inside = |a: &Rect, b: &Rect| {
            let ix = a.x1.min(b.x1).saturating_sub(a.x0.max(b.x0));
            let iy = a.y1.min(b.y1).saturating_sub(a.y0.max(b.y0));
            (ix * iy) as f32 >= ((a.x1 - a.x0) * (a.y1 - a.y0)) as f32 * 0.8
        };
        let parts: Vec<Rect> = parts
            .iter()
            .enumerate()
            .filter(|(i, p)| !parts.iter().enumerate().any(|(j, q)| j != *i && (q.x1 - q.x0) * (q.y1 - q.y0) > (p.x1 - p.x0) * (p.y1 - p.y0) && inside(p, q)))
            .map(|(_, p)| *p)
            .collect();
        if parts.len() >= 2 {
            panels.extend(parts);
        } else {
            panels.push(r);
        }
    }
    panels
        .into_iter()
        .filter(|r| big(r) && c.ink_share(r) >= MIN_INK_SHARE)
        .map(|r| {
            let s = g.step;
            Bounds { x: r.x0 as f32 * s, y: r.y0 as f32 * s, w: (r.x1 - r.x0) as f32 * s, h: (r.y1 - r.y0) as f32 * s }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// White page with filled rectangles.
    pub(crate) fn page(w: u32, h: u32, rects: &[(u32, u32, u32, u32, [u8; 3])]) -> RgbImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([255, 255, 255]));
        for &(x, y, rw, rh, c) in rects {
            for yy in y..y + rh {
                for xx in x..x + rw {
                    img.put_pixel(xx, yy, Rgb(c));
                }
            }
        }
        img
    }

    #[test]
    fn finds_a_grid_of_panels() {
        // two tiers: one wide panel, then two side by side; plus a caption
        // line in the bottom margin that is not a panel
        let grey = [120, 140, 160];
        let img = page(
            800,
            1000,
            &[(40, 40, 720, 400, grey), (40, 480, 340, 460, grey), (420, 480, 340, 460, grey), (300, 960, 200, 12, [0, 0, 0])],
        );
        let mut p = detect_panels(&img);
        p.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap());
        assert_eq!(p.len(), 3, "{p:?}");
        assert!((p[0].w - 720.0).abs() < 4.0 && (p[0].h - 400.0).abs() < 4.0);
        assert!((p[1].x - 40.0).abs() < 4.0 && (p[2].x - 420.0).abs() < 4.0);
    }

    #[test]
    fn panels_that_cannot_be_cut_are_split_by_regions() {
        // a tall panel on the left, a short one on the right and a third
        // one below them, overlapping the left panel's bottom: no straight
        // cut separates them all
        let grey = [120, 140, 160];
        let img = page(800, 1000, &[(40, 40, 340, 600, grey), (420, 40, 340, 300, grey), (420, 380, 340, 580, grey), (40, 680, 340, 280, grey)]);
        let p = detect_panels(&img);
        assert_eq!(p.len(), 4, "{p:?}");
    }

    #[test]
    fn white_areas_inside_a_panel_are_not_gutter() {
        // a framed panel with a white interior (sky) and a balloon touching
        // the frame is still one panel
        let k = [0, 0, 0];
        let img = page(600, 600, &[(50, 50, 500, 4, k), (50, 546, 500, 4, k), (50, 50, 4, 500, k), (546, 50, 4, 500, k), (100, 400, 300, 100, [200, 60, 60])]);
        let p = detect_panels(&img);
        assert_eq!(p.len(), 1, "{p:?}");
    }
}
