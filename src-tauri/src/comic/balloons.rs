//! Balloon detection: from the OCR lines to the containers they sit in.
//!
//! 1. Lines are grouped into *text blocks*: stacked lines with small
//!    leading, overlapping horizontally, of similar height. Blocks set the
//!    size limits of the fills and group the free text.
//! 2. For each line the background colour behind the letters is estimated
//!    (most frequent colour inside the line box: letters cover less than
//!    half of it) and that colour is flood-filled starting from the line.
//!    The line box itself (shrunk a little) is passable, so the fill crosses
//!    the letters. Lines are filled one by one: a block of stacked lines
//!    may span a narration box and the balloons right below it.
//! 3. A fill that stays enclosed is the inside of a container: a speech
//!    balloon, or a narration box when it is almost rectangular. A fill
//!    that leaks (the text sits on the artwork or in the gutter) means the
//!    block has no container: free text, or a sound effect when its
//!    letters are much bigger than the dialogue. A fill leaks when it
//!    reaches the page edge, when it covers most of its panel (it is the
//!    panel's background), or, outside panels, when it is much larger than
//!    the text. An enclosed area whose box is much bigger than the text's
//!    box (a spotlight, a wall) is artwork, not a container.
//! 4. Lines whose fills cover the same area are one container. A fill that only grazes
//!    another container (a line box reaching over into a neighbouring
//!    balloon) does not merge them.
//! 5. Text with fewer than two letters or digits is dropped: it is artwork
//!    read as text (hatching, a star, a stray mark).

use super::panels::dist;
use super::{BalloonDraft, KIND_BALLOON, KIND_NARRATION, KIND_SFX, KIND_TEXT};
use crate::model::Bounds;
use crate::ocr::OcrLine;
use image::RgbImage;
use std::collections::{HashMap, VecDeque};

/// Colour distance (max over channels) within which a pixel belongs to the
/// container's background.
const FILL_TOL: i32 = 32;
/// Outside panels, a fill larger than this many times the text area has leaked.
const MAX_FILL_PER_TEXT: f32 = 25.0;
/// A fill larger than this share of the page has leaked.
const MAX_FILL_PAGE: f32 = 0.15;
/// A fill whose box covers this share of its panel is the panel background.
const MAX_PANEL_COVER: f32 = 0.6;
/// A container's box is at most this many times the box of its text
/// (padded by one line height on each side): a bigger enclosed area (a
/// spotlight, a wall) is part of the artwork.
const MAX_BOX_PER_TEXT_BOX: f32 = 4.0;
/// A container this rectangular (fill area / bbox area) is a narration box.
const RECT_FILL: f32 = 0.88;
/// Pixels two fills of the same text block must share to be merged.
const MIN_TOUCH: usize = 16;
/// Free text with letters this many times taller than the dialogue is a
/// sound effect.
const SFX_HEIGHT: f32 = 1.9;

fn union_find_root(p: &mut [usize], mut i: usize) -> usize {
    while p[i] != i {
        p[i] = p[p[i]];
        i = p[i];
    }
    i
}

/// Group lines into text blocks (indices into `lines`, top to bottom).
pub(crate) fn text_blocks(lines: &[OcrLine]) -> Vec<Vec<usize>> {
    let n = lines.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in i + 1..n {
            // vertical columns (rot 90/270) are compared in their own frame:
            // swap the axes so that columns side by side look like stacked lines
            if (lines[i].rot == 0) != (lines[j].rot == 0) {
                continue;
            }
            let frame = |b: &Bounds| if lines[i].rot == 0 { *b } else { Bounds { x: b.y, y: b.x, w: b.h, h: b.w } };
            let (a, b) = (&frame(&lines[i].bounds), &frame(&lines[j].bounds));
            let hmin = a.h.min(b.h);
            let hmax = a.h.max(b.h);
            if hmax > hmin * 1.7 {
                continue;
            }
            let (top, bottom) = if a.y <= b.y { (a, b) } else { (b, a) };
            let gap = bottom.y - top.y1();
            // stacked with small leading (a little overlap is fine)
            if gap > hmin * 0.9 || gap < -hmin * 0.6 {
                continue;
            }
            // horizontally overlapping, or centred on each other
            let overlap = a.h_overlap(b);
            let centred = (a.cx() - b.cx()).abs() < a.w.max(b.w) * 0.5;
            if overlap <= 0.0 && !centred {
                continue;
            }
            let (ri, rj) = (union_find_root(&mut parent, i), union_find_root(&mut parent, j));
            if ri != rj {
                parent[ri] = rj;
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = union_find_root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    for g in &mut out {
        g.sort_by(|&a, &b| lines[a].bounds.y.total_cmp(&lines[b].bounds.y));
    }
    out.sort_by(|a, b| {
        let (p, q) = (&lines[a[0]].bounds, &lines[b[0]].bounds);
        p.y.total_cmp(&q.y).then(p.x.total_cmp(&q.x))
    });
    out
}

/// Line box shrunk so that it does not reach across a tight balloon outline
/// (by a share of its thickness: the height of a line, the width of a column).
fn inner_box(b: &Bounds, img_w: u32, img_h: u32) -> (u32, u32, u32, u32) {
    let m = (b.h.min(b.w) * 0.18).max(1.0);
    let x0 = (b.x + m).max(0.0) as u32;
    let y0 = (b.y + m).max(0.0) as u32;
    let x1 = ((b.x1() - m).max(b.x + m + 1.0) as u32).min(img_w);
    let y1 = ((b.y1() - m).max(b.y + m + 1.0) as u32).min(img_h);
    (x0.min(img_w.saturating_sub(1)), y0.min(img_h.saturating_sub(1)), x1.max(x0 + 1), y1.max(y0 + 1))
}

/// Background colour behind the letters: the most frequent colour (quantised,
/// then averaged) among the lighter half of the pixels of the boxes. Bold
/// lettering can cover most of a tight line box, so the darker half (ink)
/// is left out. Light text on a dark box is not handled.
fn background_colour(img: &RgbImage, boxes: &[(u32, u32, u32, u32)]) -> [u8; 3] {
    let lum = |p: [u8; 3]| (p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10;
    let mut pix: Vec<[u8; 3]> = Vec::new();
    for &(x0, y0, x1, y1) in boxes {
        for y in y0..y1.min(img.height()) {
            for x in x0..x1.min(img.width()) {
                pix.push(img.get_pixel(x, y).0);
            }
        }
    }
    if pix.is_empty() {
        return [255, 255, 255];
    }
    let mut ls: Vec<u32> = pix.iter().map(|&p| lum(p)).collect();
    ls.sort_unstable();
    let median = ls[ls.len() / 2];
    let mut hist: HashMap<u32, (u32, [u64; 3])> = HashMap::new();
    for p in pix.into_iter().filter(|&p| lum(p) >= median) {
        let key = ((p[0] as u32 >> 5) << 6) | ((p[1] as u32 >> 5) << 3) | (p[2] as u32 >> 5);
        let e = hist.entry(key).or_insert((0, [0; 3]));
        e.0 += 1;
        for c in 0..3 {
            e.1[c] += p[c] as u64;
        }
    }
    match hist.values().max_by_key(|e| e.0) {
        Some((n, s)) if *n > 0 => [(s[0] / *n as u64) as u8, (s[1] / *n as u64) as u8, (s[2] / *n as u64) as u8],
        _ => [255, 255, 255],
    }
}

struct Fill {
    /// Pixel indices of the region (empty when it leaked).
    pixels: Vec<u32>,
    bounds: Bounds,
    leaked: bool,
}

/// Flood fill from the first box, crossing all the `boxes`; `label` marks pixels already taken by
/// other containers (their id + 1); `touched` counts the pixels of each.
/// `stamp` / `gen`: reusable visited marks (a pixel is visited when its
/// stamp equals `gen`).
#[allow(clippy::too_many_arguments)]
fn fill(img: &RgbImage, boxes: &[(u32, u32, u32, u32)], bg: [u8; 3], limit: usize, label: &[u32], touched: &mut HashMap<u32, usize>, stamp: &mut [u32], gen: u32) -> Fill {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let inside_box = |x: usize, y: usize| boxes.iter().any(|&(x0, y0, x1, y1)| x >= x0 as usize && x < x1 as usize && y >= y0 as usize && y < y1 as usize);
    let passable = |x: usize, y: usize| inside_box(x, y) || dist(img.get_pixel(x as u32, y as u32).0, bg) <= FILL_TOL;
    let mut q: VecDeque<(usize, usize)> = VecDeque::new();
    let mut pixels: Vec<u32> = Vec::new();
    // seeds: the first box; the others are only passable
    for &(x0, y0, x1, y1) in &boxes[..1] {
        for y in y0 as usize..(y1 as usize).min(h) {
            for x in x0 as usize..(x1 as usize).min(w) {
                let i = (y * w + x) as u32;
                if stamp[i as usize] != gen {
                    stamp[i as usize] = gen;
                    q.push_back((x, y));
                }
            }
        }
    }
    let (mut bx0, mut by0, mut bx1, mut by1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    while let Some((x, y)) = q.pop_front() {
        let i = y * w + x;
        if label[i] != 0 {
            *touched.entry(label[i]).or_insert(0) += 1;
        }
        pixels.push(i as u32);
        bx0 = bx0.min(x);
        by0 = by0.min(y);
        bx1 = bx1.max(x + 1);
        by1 = by1.max(y + 1);
        if pixels.len() > limit || x == 0 || y == 0 || x + 1 == w || y + 1 == h {
            return Fill { pixels: Vec::new(), bounds: Bounds::default(), leaked: true };
        }
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            let ni = (ny * w + nx) as u32;
            if stamp[ni as usize] != gen && passable(nx, ny) {
                stamp[ni as usize] = gen;
                q.push_back((nx, ny));
            }
        }
    }
    let bounds = Bounds { x: bx0 as f32, y: by0 as f32, w: (bx1 - bx0) as f32, h: (by1 - by0) as f32 };
    Fill { pixels, bounds, leaked: false }
}

/// Box blur (separable, radius `r`): halftone dots and paper grain inside a
/// balloon must not stop the fill; outlines stay dark enough to stop it.
fn smooth(img: &RgbImage, r: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let pass = |src: &RgbImage, horizontal: bool| {
        let mut out = RgbImage::new(w, h);
        let (n, m) = if horizontal { (h, w) } else { (w, h) };
        for a in 0..n {
            let px = |b: u32| if horizontal { src.get_pixel(b, a).0 } else { src.get_pixel(a, b).0 };
            let mut sum = [0u32; 3];
            let mut cnt = 0u32;
            for b in 0..=r.min(m - 1) {
                let p = px(b);
                for c in 0..3 {
                    sum[c] += p[c] as u32;
                }
                cnt += 1;
            }
            for b in 0..m {
                let v = image::Rgb([(sum[0] / cnt) as u8, (sum[1] / cnt) as u8, (sum[2] / cnt) as u8]);
                if horizontal {
                    out.put_pixel(b, a, v);
                } else {
                    out.put_pixel(a, b, v);
                }
                if b + r + 1 < m {
                    let p = px(b + r + 1);
                    for c in 0..3 {
                        sum[c] += p[c] as u32;
                    }
                    cnt += 1;
                }
                if b >= r {
                    let p = px(b - r);
                    for c in 0..3 {
                        sum[c] -= p[c] as u32;
                    }
                    cnt -= 1;
                }
            }
        }
        out
    };
    let tmp = pass(img, true);
    pass(&tmp, false)
}

/// Containers of the text lines, in image pixels (unordered).
pub fn detect_balloons(img: &RgbImage, lines: &[OcrLine], panels: &[Bounds]) -> Vec<BalloonDraft> {
    let (w, h) = img.dimensions();
    // about 1 px at 150 dpi, 2 px at 300 dpi
    let img = &smooth(img, (w.max(h) as f32 / 1500.0).round().max(1.0) as u32);
    let page_area = (w as f32) * (h as f32);
    let blocks = text_blocks(lines);
    let mut label: Vec<u32> = vec![0; (w as usize) * (h as usize)];
    let mut stamp: Vec<u32> = vec![0; (w as usize) * (h as usize)];
    // container id → (lines, fill pixels count, bounds, text block, background)
    #[allow(clippy::type_complexity)]
    let mut containers: Vec<Option<(Vec<usize>, usize, Bounds, usize, [u8; 3])>> = Vec::new();
    let mut leaked: Vec<usize> = Vec::new();
    let mut gen = 0u32;
    for (bi, block) in blocks.iter().enumerate() {
        let text_area: f32 = block.iter().map(|&i| lines[i].bounds.area()).sum();
        let tb = Bounds::union_all(block.iter().map(|&i| &lines[i].bounds)).unwrap_or_default();
        let panel = super::order::assign_panels(panels, &[tb])[0].map(|k| panels[k]);
        let limit = match panel {
            Some(_) => page_area * MAX_FILL_PAGE,
            None => (text_area * MAX_FILL_PER_TEXT).min(page_area * MAX_FILL_PAGE),
        }
        .max(64.0) as usize;
        let pad = block.iter().map(|&i| lines[i].bounds.h.min(lines[i].bounds.w)).fold(0.0f32, f32::max);
        // one fill per line: a block of stacked lines may span a narration
        // box and the balloons right below it. The fill may cross the
        // boxes of the block's lines on the same background (tight leading
        // leaves no clear path between the lines).
        let inner: Vec<(u32, u32, u32, u32)> = block.iter().map(|&i| inner_box(&lines[i].bounds, w, h)).collect();
        let bgs: Vec<[u8; 3]> = inner.iter().map(|b| background_colour(img, std::slice::from_ref(b))).collect();
        for (k, &li) in block.iter().enumerate() {
            let bg = bgs[k];
            let mut boxes = vec![inner[k]];
            boxes.extend((0..block.len()).filter(|&j| j != k && dist(bgs[j], bg) <= FILL_TOL).map(|j| inner[j]));
            let mut touched: HashMap<u32, usize> = HashMap::new();
            gen += 1;
            let f = fill(img, &boxes, bg, limit, &label, &mut touched, &mut stamp, gen);
            let covers_panel = panel.is_some_and(|p| f.bounds.area() >= p.area() * MAX_PANEL_COVER);
            let loose = f.bounds.area() > (tb.w + 2.0 * pad) * (tb.h + 2.0 * pad) * MAX_BOX_PER_TEXT_BOX;
            if std::env::var_os("PDFXRAY_COMIC_DEBUG").is_some() {
                eprintln!(
                    "fill {:?} bg {:?} boxes {} leaked {} px {} box {:?} covers {} loose {} touched {:?}",
                    lines[li].text, bg, boxes.len(), f.leaked, f.pixels.len(), f.bounds, covers_panel, loose, touched
                );
            }
            if f.leaked || covers_panel || loose {
                leaked.push(li);
                continue;
            }
            // merge with the container already filled at the same place
            let mut ls = vec![li];
            let mut bounds = f.bounds;
            let id = containers.len() as u32 + 1;
            let mut area = 0usize;
            for (t, n) in touched {
                let Some((_, other, _, oblock, obg)) = containers[(t - 1) as usize].as_ref() else { continue };
                // the same area (most of the smaller region is shared), or
                // two lines of one block on the same background whose fills
                // meet (tiny lettering nearly closing the way between lines)
                let same_area = n * 2 >= (*other).min(f.pixels.len());
                let same_block = *oblock == bi && dist(*obg, bg) <= FILL_TOL && n >= MIN_TOUCH;
                // a small fill inside another container (a bold word whose
                // background was misread as its ink)
                let ob = containers[(t - 1) as usize].as_ref().unwrap().2;
                let inside = n >= MIN_TOUCH && f.bounds.h_overlap(&ob).max(0.0) * f.bounds.v_overlap(&ob).max(0.0) >= f.bounds.area() * 0.9;
                if !(same_area || same_block || inside) {
                    continue;
                }
                if let Some((ols, _, ob, _, _)) = containers[(t - 1) as usize].take() {
                    ls.extend(ols);
                    bounds = bounds.union(&ob);
                    for l in label.iter_mut() {
                        if *l == t {
                            *l = id;
                            area += 1;
                        }
                    }
                }
            }
            // pixels of a grazed neighbour stay with it
            for &p in &f.pixels {
                if label[p as usize] == 0 {
                    label[p as usize] = id;
                    area += 1;
                }
            }
            containers.push(Some((ls, area, bounds, bi, bg)));
        }
    }
    // free text: blocks of the lines without a container
    let sub: Vec<OcrLine> = leaked.iter().map(|&i| lines[i].clone()).collect();
    let free: Vec<Vec<usize>> = text_blocks(&sub).into_iter().map(|b| b.into_iter().map(|k| leaked[k]).collect()).collect();

    let mut out: Vec<BalloonDraft> = Vec::new();
    let alnum = |ls: &[usize]| -> usize { ls.iter().map(|&i| lines[i].text.chars().filter(|c| c.is_alphanumeric()).count()).sum() };
    for (k, c) in containers.into_iter().enumerate() {
        let Some((ls, area, bounds, _, bg)) = c else { continue };
        if alnum(&ls) < 2 {
            continue;
        }
        let rect = area as f32 / bounds.area().max(1.0);
        let kind = if rect >= RECT_FILL { KIND_NARRATION } else { KIND_BALLOON };
        let shape = shape_rows(&label, w as usize, k as u32 + 1, &bounds);
        out.push(BalloonDraft { bounds, kind, lines: sorted(lines, ls), shape, fill: Some(bg) });
    }
    // free text: sound effects are much bigger than the dialogue
    let mut dialogue_h: Vec<f32> = out.iter().flat_map(|b| b.lines.iter().map(|&i| lines[i].bounds.h)).collect();
    if dialogue_h.is_empty() {
        dialogue_h = lines.iter().map(|l| if l.rot == 0 { l.bounds.h } else { l.bounds.w }).collect();
    }
    dialogue_h.sort_by(|a, b| a.total_cmp(b));
    let ref_h = dialogue_h.get(dialogue_h.len() / 2).copied().unwrap_or(0.0);
    for ls in free {
        if alnum(&ls) < 2 {
            continue;
        }
        let bounds = Bounds::union_all(ls.iter().map(|&i| &lines[i].bounds)).unwrap_or_default();
        let mut hs: Vec<f32> = ls.iter().map(|&i| if lines[i].rot == 0 { lines[i].bounds.h } else { lines[i].bounds.w }).collect();
        hs.sort_by(|a, b| a.total_cmp(b));
        let bh = hs[hs.len() / 2];
        let kind = if ref_h > 0.0 && bh >= ref_h * SFX_HEIGHT { KIND_SFX } else { KIND_TEXT };
        out.push(BalloonDraft { bounds, kind, lines: sorted(lines, ls), shape: Vec::new(), fill: None });
    }
    out
}

/// Rows `[y, x0, x1]` of the region labelled `id` inside `bounds`: the
/// leftmost and rightmost pixel of the region on evenly spaced rows (about
/// one every 4 px, 8 to 40 rows), so the letters (holes in the region) are
/// inside the extent.
fn shape_rows(label: &[u32], w: usize, id: u32, bounds: &Bounds) -> Vec<[f32; 3]> {
    let (x0, x1) = (bounds.x.max(0.0) as usize, (bounds.x1() as usize).min(w));
    let (y0, y1) = (bounds.y.max(0.0) as usize, bounds.y1() as usize);
    if y1 <= y0 || x1 <= x0 {
        return Vec::new();
    }
    let n = ((y1 - y0) / 4).clamp(8, 40).min(y1 - y0);
    let mut rows = Vec::with_capacity(n);
    for k in 0..n {
        let y = if n == 1 { y0 } else { y0 + k * (y1 - 1 - y0) / (n - 1) };
        let row = &label[y * w..y * w + w];
        let first = (x0..x1).find(|&x| row[x] == id);
        let last = (x0..x1).rev().find(|&x| row[x] == id);
        if let (Some(a), Some(b)) = (first, last) {
            rows.push([y as f32 + 0.5, a as f32, b as f32 + 1.0]);
        }
    }
    rows
}

fn sorted(lines: &[OcrLine], mut ls: Vec<usize>) -> Vec<usize> {
    ls.sort_by(|&a, &b| lines[a].bounds.y.total_cmp(&lines[b].bounds.y).then(lines[a].bounds.x.total_cmp(&lines[b].bounds.x)));
    ls.dedup();
    ls
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    pub(crate) fn line(x: f32, y: f32, w: f32, h: f32, text: &str) -> OcrLine {
        OcrLine { bounds: Bounds { x, y, w, h }, text: text.into(), conf: 0.99, det_score: 0.9, rot: 0, words: Vec::new() }
    }

    /// Draw an ellipse outline (balloon) and fake letters (black dashes).
    pub(crate) fn draw_balloon(img: &mut RgbImage, cx: f32, cy: f32, rx: f32, ry: f32) {
        for y in 0..img.height() {
            for x in 0..img.width() {
                let d = ((x as f32 - cx) / rx).powi(2) + ((y as f32 - cy) / ry).powi(2);
                if (0.92..=1.08).contains(&d) {
                    img.put_pixel(x, y, Rgb([0, 0, 0]));
                } else if d < 0.92 {
                    img.put_pixel(x, y, Rgb([255, 255, 255]));
                }
            }
        }
    }

    pub(crate) fn letters(img: &mut RgbImage, l: &OcrLine) {
        let b = l.bounds;
        let mut x = b.x + b.h * 0.3;
        while x + 6.0 < b.x1() - b.h * 0.3 {
            for yy in (b.y + b.h * 0.25) as u32..(b.y1() - b.h * 0.25) as u32 {
                for xx in x as u32..x as u32 + 5 {
                    img.put_pixel(xx, yy, Rgb([0, 0, 0]));
                }
            }
            x += 9.0;
        }
    }

    #[test]
    fn blocks_group_stacked_lines() {
        let ls = vec![line(100.0, 100.0, 120.0, 20.0, "A"), line(95.0, 122.0, 130.0, 20.0, "B"), line(400.0, 100.0, 80.0, 20.0, "C"), line(100.0, 300.0, 100.0, 20.0, "D")];
        let b = text_blocks(&ls);
        assert_eq!(b, vec![vec![0, 1], vec![2], vec![3]]);
    }

    #[test]
    fn balloon_vs_free_text_vs_sfx() {
        // artwork: a mid-grey page with a white balloon and free text on it
        let mut img = RgbImage::from_pixel(800, 600, Rgb([110, 150, 190]));
        draw_balloon(&mut img, 200.0, 150.0, 150.0, 80.0);
        let ls = vec![
            line(130.0, 115.0, 140.0, 22.0, "HELLO"),
            line(120.0, 140.0, 160.0, 22.0, "THERE"),
            line(500.0, 400.0, 220.0, 70.0, "BOOM"),
            line(520.0, 100.0, 120.0, 22.0, "SIGN"),
        ];
        for l in &ls {
            letters(&mut img, l);
        }
        let mut out = detect_balloons(&img, &ls, &[]);
        out.sort_by(|a, b| a.lines[0].cmp(&b.lines[0]));
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(out[0].kind, KIND_BALLOON);
        assert_eq!(out[0].lines, vec![0, 1]);
        assert!(out[0].bounds.w > 250.0 && out[0].bounds.w < 310.0, "{:?}", out[0].bounds);
        // the shape follows the ellipse: wide in the middle, narrow at the ends
        let shape = &out[0].shape;
        assert!(shape.len() >= 8, "{shape:?}");
        let width = |r: &[f32; 3]| r[2] - r[1];
        let mid = shape.iter().min_by(|a, b| (a[0] - 150.0).abs().total_cmp(&(b[0] - 150.0).abs())).unwrap();
        assert!(width(mid) > 260.0, "{mid:?}");
        assert!(width(&shape[0]) < width(mid) * 0.6 && width(shape.last().unwrap()) < width(mid) * 0.6);
        assert!(out[0].fill.is_some_and(|c| c.iter().all(|&v| v > 240)));
        assert!(out[1].shape.is_empty() && out[1].fill.is_none());
        assert_eq!(out[1].kind, KIND_SFX);
        assert_eq!(out[2].kind, KIND_TEXT);
    }

    #[test]
    fn rectangular_box_is_narration_and_split_text_merges() {
        let mut img = RgbImage::from_pixel(800, 600, Rgb([60, 60, 60]));
        // yellow caption box with a black frame
        for y in 40..160 {
            for x in 40..440 {
                let edge = !(43..437).contains(&x) || !(43..157).contains(&y);
                img.put_pixel(x, y, Rgb(if edge { [0, 0, 0] } else { [250, 230, 90] }));
            }
        }
        // two text blocks far apart inside the same box
        let ls = vec![line(60.0, 60.0, 150.0, 22.0, "MEANWHILE"), line(260.0, 110.0, 150.0, 22.0, "LATER")];
        for l in &ls {
            letters(&mut img, l);
        }
        let out = detect_balloons(&img, &ls, &[]);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].kind, KIND_NARRATION);
        assert_eq!(out[0].lines, vec![0, 1]);
    }
}
