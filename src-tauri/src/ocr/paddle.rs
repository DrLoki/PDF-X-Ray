//! PaddleOCR PP-OCRv5 on `rten`.
//!
//! - Detection (`PP-OCRv5_mobile_det`, DBNet): the image is resized so its
//!   longer side is at most `det_max_side` (multiple of 32), normalised with
//!   the ImageNet mean/std in BGR order; the output is a text-probability
//!   map. Pixels above `DB_THRESH` form connected regions; a region is kept
//!   when its mean probability reaches `DB_BOX_THRESH`, and its box is grown
//!   back by the DB "unclip" distance (area × ratio / perimeter), since the
//!   model is trained on shrunk text regions.
//! - Recognition (`latin_PP-OCRv5_mobile_rec`, CTC): each box is cropped from
//!   the full-resolution image, resized to height 48, normalised to [-1, 1]
//!   (BGR) and decoded greedily. Index 0 is the CTC blank, then the
//!   characters of the dictionary, then the space. Tall boxes are also tried
//!   rotated by ±90° and the most confident reading wins (vertical credits).
//!
//! The parameters follow the `inference.yml` shipped with the models.
//!
//! Japanese (the `paddleocr-v5-cjk` set): vertical columns are read with the
//! same rotation (PaddleOCR turns tall crops counter-clockwise); the
//! furigana (ruby: small kana beside a column, above a horizontal line) is
//! removed — blanked inside the crop of the main text ([`strip_ruby`]), and
//! dropped when it was detected as a thin line of its own
//! ([`drop_ruby_lines`]).

use super::{OcrEngine, OcrLine, OcrPage, OcrWord, MIN_LINE_CONF};
use crate::model::Bounds;
use image::{imageops, imageops::FilterType, RgbImage};
use rten::Model;
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, Tensor};
use std::path::Path;
use std::time::Instant;


const DB_THRESH: f32 = 0.3;
const DB_BOX_THRESH: f32 = 0.6;
const DB_UNCLIP: f32 = 1.5;
const DET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const DET_STD: [f32; 3] = [0.229, 0.224, 0.225];
const REC_HEIGHT: u32 = 48;
const REC_MAX_WIDTH: u32 = 3200;
const REC_BATCH: usize = 8;
/// A box this much taller than wide is also tried as vertical text.
const VERTICAL_RATIO: f32 = 1.5;

#[derive(Debug, Clone)]
pub struct PaddleParams {
    /// Longer side of the detection input, in pixels.
    pub det_max_side: u32,
}

impl Default for PaddleParams {
    fn default() -> Self {
        PaddleParams { det_max_side: 1600 }
    }
}

pub struct PaddleOcr {
    /// Model set id (`paddleocr-v5-latin`, `paddleocr-v5-cjk`).
    id: String,
    /// Japanese / Chinese set: furigana handling.
    cjk: bool,
    det: Model,
    rec: Model,
    dict: Vec<String>,
    pub params: PaddleParams,
}

/// Read the character list (`PostProcess.character_dict`) from the
/// recognition model's `inference.yml`. Only the subset of YAML used there
/// is understood: a block sequence of plain, single- or double-quoted scalars.
pub fn parse_dict(yml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut indent: Option<usize> = None;
    let mut on = false;
    for l in yml.lines() {
        let t = l.trim_start();
        if !on {
            if t.starts_with("character_dict:") {
                on = true;
            }
            continue;
        }
        if t.is_empty() {
            continue;
        }
        let ind = l.len() - t.len();
        if !t.starts_with("- ") && t != "-" {
            break;
        }
        match indent {
            None => indent = Some(ind),
            Some(i) if i != ind => break,
            _ => {}
        }
        let v = t.strip_prefix('-').unwrap_or("").strip_prefix(' ').unwrap_or("");
        out.push(unquote(v));
    }
    out
}

fn unquote(v: &str) -> String {
    let n = v.chars().count();
    if n >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].replace("''", "'");
    }
    if n >= 2 && v.starts_with('"') && v.ends_with('"') {
        let inner = &v[1..v.len() - 1];
        let mut s = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                s.push(c);
                continue;
            }
            match it.next() {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some('u') => {
                    let hex: String = it.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        s.push(ch);
                    }
                }
                Some(o) => s.push(o),
                None => {}
            }
        }
        return s;
    }
    v.to_string()
}

impl PaddleOcr {
    pub fn load(id: &str, det: &Path, rec: &Path, dict_yml: &Path) -> Result<PaddleOcr, String> {
        let det = Model::load_file(det).map_err(|e| format!("{}: {e}", det.display()))?;
        let rec = Model::load_file(rec).map_err(|e| format!("{}: {e}", rec.display()))?;
        let yml = std::fs::read_to_string(dict_yml).map_err(|e| format!("{}: {e}", dict_yml.display()))?;
        let dict = parse_dict(&yml);
        if dict.len() < 50 {
            return Err(format!("{}: character dictionary not found", dict_yml.display()));
        }
        Ok(PaddleOcr { id: id.to_string(), cjk: id.contains("cjk"), det, rec, dict, params: PaddleParams::default() })
    }

    /// Detected text boxes in image pixels, with their mean probability.
    fn detect(&self, img: &RgbImage) -> Result<Vec<(Bounds, f32)>, String> {
        let (w, h) = img.dimensions();
        let s = (self.params.det_max_side as f32 / w.max(h) as f32).min(1.0);
        let rw = (((w as f32 * s) / 32.0).round() as u32 * 32).max(32);
        let rh = (((h as f32 * s) / 32.0).round() as u32 * 32).max(32);
        let r = imageops::resize(img, rw, rh, FilterType::Triangle);
        let mut t = NdTensor::<f32, 4>::zeros([1, 3, rh as usize, rw as usize]);
        for (x, y, p) in r.enumerate_pixels() {
            let bgr = [p[2], p[1], p[0]];
            for c in 0..3 {
                t[[0, c, y as usize, x as usize]] = (bgr[c] as f32 / 255.0 - DET_MEAN[c]) / DET_STD[c];
            }
        }
        let out: Tensor<f32> = self
            .det
            .run_one(t.view().into(), None)
            .map_err(|e| format!("detection: {e}"))?
            .try_into()
            .map_err(|e| format!("detection output: {e:?}"))?;
        let sh = out.shape();
        if sh.len() != 4 {
            return Err(format!("detection output shape {:?}", sh));
        }
        let (oh, ow) = (sh[2], sh[3]);
        let prob = out.reshaped([oh, ow]).to_tensor();
        let sx = w as f32 / ow as f32;
        let sy = h as f32 / oh as f32;

        // connected regions of the thresholded map (4-connectivity)
        let mut seen = vec![false; oh * ow];
        let mut stack = Vec::new();
        let mut boxes = Vec::new();
        for y0 in 0..oh {
            for x0 in 0..ow {
                if seen[y0 * ow + x0] || prob[[y0, x0]] <= DB_THRESH {
                    continue;
                }
                seen[y0 * ow + x0] = true;
                stack.push((x0, y0));
                let (mut bx0, mut by0, mut bx1, mut by1) = (x0, y0, x0, y0);
                let (mut sum, mut n) = (0f32, 0usize);
                while let Some((x, y)) = stack.pop() {
                    bx0 = bx0.min(x);
                    by0 = by0.min(y);
                    bx1 = bx1.max(x);
                    by1 = by1.max(y);
                    sum += prob[[y, x]];
                    n += 1;
                    let nb = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
                    for (nx, ny) in nb {
                        if nx < ow && ny < oh && !seen[ny * ow + nx] && prob[[ny, nx]] > DB_THRESH {
                            seen[ny * ow + nx] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
                let (bw, bh) = ((bx1 - bx0 + 1) as f32, (by1 - by0 + 1) as f32);
                if bw.min(bh) < 3.0 {
                    continue;
                }
                let score = sum / n as f32;
                if score < DB_BOX_THRESH {
                    continue;
                }
                let d = bw * bh * DB_UNCLIP / (2.0 * (bw + bh));
                let x = ((bx0 as f32 - d) * sx).max(0.0);
                let y = ((by0 as f32 - d) * sy).max(0.0);
                let x1 = ((bx1 as f32 + 1.0 + d) * sx).min(w as f32);
                let y1 = ((by1 as f32 + 1.0 + d) * sy).min(h as f32);
                if x1 - x >= 2.0 && y1 - y >= 2.0 {
                    boxes.push((Bounds { x, y, w: x1 - x, h: y1 - y }, score));
                }
            }
        }
        Ok(boxes)
    }

    /// Recognise a list of crops (already upright). Returns, per crop, the
    /// text, its confidence and the time-step span of every character in
    /// crop pixels.
    fn recognize_crops(&self, crops: &[RgbImage]) -> Result<Vec<Decoded>, String> {
        let mut order: Vec<usize> = (0..crops.len()).collect();
        let ratio = |c: &RgbImage| c.width() as f32 / c.height().max(1) as f32;
        order.sort_by(|&a, &b| ratio(&crops[a]).total_cmp(&ratio(&crops[b])));
        let mut res: Vec<Decoded> = vec![Decoded::default(); crops.len()];
        for chunk in order.chunks(REC_BATCH) {
            let widths: Vec<u32> = chunk
                .iter()
                .map(|&i| ((REC_HEIGHT as f32 * ratio(&crops[i])).ceil() as u32).clamp(8, REC_MAX_WIDTH))
                .collect();
            let maxw = (*widths.iter().max().unwrap_or(&8)).max(REC_HEIGHT) as usize;
            let mut t = NdTensor::<f32, 4>::zeros([chunk.len(), 3, REC_HEIGHT as usize, maxw]);
            for (k, &i) in chunk.iter().enumerate() {
                let r = imageops::resize(&crops[i], widths[k], REC_HEIGHT, FilterType::Triangle);
                for (x, y, p) in r.enumerate_pixels() {
                    let bgr = [p[2], p[1], p[0]];
                    for c in 0..3 {
                        t[[k, c, y as usize, x as usize]] = (bgr[c] as f32 / 255.0 - 0.5) / 0.5;
                    }
                }
            }
            let out: Tensor<f32> = self
                .rec
                .run_one(t.view().into(), None)
                .map_err(|e| format!("recognition: {e}"))?
                .try_into()
                .map_err(|e| format!("recognition output: {e:?}"))?;
            let sh = out.shape();
            if sh.len() != 3 {
                return Err(format!("recognition output shape {:?}", sh));
            }
            let (steps, classes) = (sh[1], sh[2]);
            // input pixels per time step, mapped back to crop pixels
            let step_px = maxw as f32 / steps as f32;
            for (k, &i) in chunk.iter().enumerate() {
                let to_crop = crops[i].width() as f32 / widths[k] as f32;
                let mut d = Decoded::default();
                let (mut conf, mut prev) = (0f32, 0usize);
                for ti in 0..steps {
                    let (mut best, mut bp) = (0usize, f32::MIN);
                    for c in 0..classes {
                        let v = out[[k, ti, c]];
                        if v > bp {
                            bp = v;
                            best = c;
                        }
                    }
                    if best != 0 && best != prev {
                        let ch = if best <= self.dict.len() { self.dict[best - 1].as_str() } else { " " };
                        let x0 = ti as f32 * step_px * to_crop;
                        d.chars.push((ch.to_string(), x0, x0 + step_px * to_crop));
                        conf += bp;
                    } else if best != 0 && best == prev {
                        if let Some(last) = d.chars.last_mut() {
                            last.2 = (ti + 1) as f32 * step_px * to_crop;
                        }
                    }
                    prev = best;
                }
                d.conf = if d.chars.is_empty() { 0.0 } else { conf / d.chars.len() as f32 };
                res[i] = d;
            }
        }
        Ok(res)
    }
}

#[derive(Debug, Clone, Default)]
struct Decoded {
    /// (character, x0, x1) in crop pixels
    chars: Vec<(String, f32, f32)>,
    conf: f32,
}

impl Decoded {
    fn text(&self) -> String {
        self.chars.iter().map(|c| c.0.as_str()).collect::<String>().trim().to_string()
    }
}

fn crop(img: &RgbImage, b: &Bounds) -> RgbImage {
    let x = b.x.floor().max(0.0) as u32;
    let y = b.y.floor().max(0.0) as u32;
    let w = (b.x1().ceil() as u32).min(img.width()).saturating_sub(x).max(1);
    let h = (b.y1().ceil() as u32).min(img.height()).saturating_sub(y).max(1);
    imageops::crop_imm(img, x, y, w, h).to_image()
}

/// Blank the furigana inside the crop of a line of Japanese text.
///
/// The ink profile across the line (columns of a vertical crop, rows of a
/// horizontal one) shows the main text as the band with the most ink; the
/// ruby is a separate, much thinner band beside it (right of a column, above
/// a line), parted by at least one empty step. Such bands are painted with
/// the crop's background. Dark text on a light background is assumed.
pub fn strip_ruby(c: &mut RgbImage, vertical: bool) {
    let (w, h) = c.dimensions();
    let (n, m) = if vertical { (w, h) } else { (h, w) };
    if n < 6 {
        return;
    }
    let lum = |p: &image::Rgb<u8>| (p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10;
    let dark = |a: u32, b: u32| {
        let p = if vertical { c.get_pixel(a, b) } else { c.get_pixel(b, a) };
        lum(p) < 128
    };
    let profile: Vec<u32> = (0..n).map(|a| (0..m).filter(|&b| dark(a, b)).count() as u32).collect();
    let min_ink = ((m as f32) * 0.02).max(1.0) as u32;
    // bands of inked steps
    let mut bands: Vec<(u32, u32, u32)> = Vec::new(); // (start, end, ink)
    let mut a = 0;
    while a < n {
        if profile[a as usize] >= min_ink {
            let s = a;
            let mut ink = 0;
            while a < n && profile[a as usize] >= min_ink {
                ink += profile[a as usize];
                a += 1;
            }
            bands.push((s, a, ink));
        } else {
            a += 1;
        }
    }
    if bands.len() < 2 {
        return;
    }
    // bands one step apart belong together (strokes of the same glyphs)
    let mut merged: Vec<(u32, u32, u32)> = Vec::new();
    for b in bands {
        match merged.last_mut() {
            Some(l) if b.0 <= l.1 + 1 => {
                l.1 = b.1;
                l.2 += b.2;
            }
            _ => merged.push(b),
        }
    }
    let Some(&main) = merged.iter().max_by_key(|b| b.2) else { return };
    let main_w = main.1 - main.0;
    // background: the most common light value of the crop is close to white
    // in balloons; use the lightest pixel to repaint
    let bg = c.pixels().max_by_key(|p| lum(p)).copied().unwrap_or(image::Rgb([255, 255, 255]));
    for b in merged {
        if b == main {
            continue;
        }
        // furigana: right of a column / above a line, thin
        let beside = if vertical { b.0 >= main.1 } else { b.1 <= main.0 };
        if !beside || (b.1 - b.0) as f32 > main_w as f32 * 0.6 {
            continue;
        }
        for s in b.0..b.1 {
            for t in 0..m {
                if vertical {
                    c.put_pixel(s, t, bg);
                } else {
                    c.put_pixel(t, s, bg);
                }
            }
        }
    }
}

/// Drop furigana detected as lines of their own: a thin vertical line just
/// right of a column at least 1.6× thicker that spans it, or a thin
/// horizontal line just above a thicker line.
pub fn drop_ruby_lines(lines: Vec<OcrLine>) -> Vec<OcrLine> {
    let thick = |l: &OcrLine| if l.rot == 90 || l.rot == 270 { l.bounds.w } else { l.bounds.h };
    let keep: Vec<bool> = lines
        .iter()
        .map(|a| {
            !lines.iter().any(|b| {
                if std::ptr::eq(a, b) || (a.rot != 0) != (b.rot != 0) || thick(b) < thick(a) * 1.6 {
                    return false;
                }
                let (ta, tb) = (thick(a), thick(b));
                let (ab, bb) = (&a.bounds, &b.bounds);
                if a.rot != 0 {
                    // ruby right of the column, within its height
                    let gap = ab.x - bb.x1();
                    gap > -ta * 0.5 && gap < tb * 0.4 && ab.y >= bb.y - tb && ab.y1() <= bb.y1() + tb
                } else {
                    // ruby above the line, within its width
                    let gap = bb.y - ab.y1();
                    gap > -ta * 0.5 && gap < tb * 0.4 && ab.x >= bb.x - tb && ab.x1() <= bb.x1() + tb
                }
            })
        })
        .collect();
    lines.into_iter().zip(keep).filter(|(_, k)| *k).map(|(l, _)| l).collect()
}

/// Word boxes of a horizontal line from the decoded character spans.
fn words_of(d: &Decoded, line: &Bounds) -> Vec<OcrWord> {
    let mut out = Vec::new();
    let mut cur: Option<(String, f32, f32)> = None;
    for (ch, x0, x1) in &d.chars {
        if ch.trim().is_empty() {
            if let Some(w) = cur.take() {
                out.push(w);
            }
            continue;
        }
        match cur.as_mut() {
            Some(w) => {
                w.0.push_str(ch);
                w.2 = *x1;
            }
            None => cur = Some((ch.clone(), *x0, *x1)),
        }
    }
    if let Some(w) = cur {
        out.push(w);
    }
    out.into_iter()
        .map(|(text, x0, x1)| OcrWord { text, bounds: Bounds { x: line.x + x0, y: line.y, w: (x1 - x0).max(1.0), h: line.h } })
        .collect()
}

impl OcrEngine for PaddleOcr {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn recognize(&self, img: &RgbImage) -> Result<OcrPage, String> {
        let t0 = Instant::now();
        let boxes = self.detect(img)?;
        let detect_ms = t0.elapsed().as_millis() as u64;
        let t1 = Instant::now();

        // upright crops first; tall boxes also as ±90° rotations
        let mut crops: Vec<RgbImage> = Vec::with_capacity(boxes.len());
        let mut variants: Vec<Vec<(usize, u16)>> = Vec::with_capacity(boxes.len());
        for (b, _) in &boxes {
            let mut c = crop(img, b);
            if self.cjk {
                strip_ruby(&mut c, b.h > b.w * VERTICAL_RATIO);
            }
            let mut v = vec![(crops.len(), 0u16)];
            if b.h > b.w * VERTICAL_RATIO {
                // reading top→bottom (rot 90): turn the crop counter-clockwise
                v.push((crops.len() + 1, 90));
                v.push((crops.len() + 2, 270));
                crops.push(c.clone());
                crops.push(imageops::rotate270(&c));
                crops.push(imageops::rotate90(&c));
            } else {
                crops.push(c);
            }
            variants.push(v);
        }
        let decoded = self.recognize_crops(&crops)?;

        let mut lines = Vec::new();
        for ((b, score), v) in boxes.iter().zip(&variants) {
            let (best, rot) = v
                .iter()
                .map(|&(i, r)| (&decoded[i], r))
                .max_by(|a, b| {
                    // prefer the reading with more characters when the confidence is equal
                    (a.0.conf * (a.0.chars.len().min(3) as f32)).total_cmp(&(b.0.conf * (b.0.chars.len().min(3) as f32)))
                })
                .unwrap();
            let text = best.text();
            if text.is_empty() || best.conf < MIN_LINE_CONF {
                continue;
            }
            let words = if rot == 0 { words_of(best, b) } else { Vec::new() };
            lines.push(OcrLine { bounds: *b, text, conf: best.conf, det_score: *score, rot, words });
        }
        if self.cjk {
            lines = drop_ruby_lines(lines);
        }
        lines.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y).then(a.bounds.x.total_cmp(&b.bounds.x)));
        Ok(OcrPage {
            engine: self.id.clone(),
            width: img.width(),
            height: img.height(),
            lines,
            detect_ms,
            recognize_ms: t1.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_yaml_dictionary() {
        let yml = "PostProcess:\n  name: CTCLabelDecode\n  character_dict:\n  - '0'\n  - A\n  - à\n  - ''''\n  - '\"'\n  - \"\\\\\"\n  - '-'\n  - ':'\nOther: 1\n";
        assert_eq!(parse_dict(yml), vec!["0", "A", "à", "'", "\"", "\\", "-", ":"]);
    }

    #[test]
    fn words_follow_spaces() {
        let d = Decoded {
            chars: vec![("a".into(), 0.0, 5.0), ("b".into(), 5.0, 10.0), (" ".into(), 10.0, 15.0), ("c".into(), 20.0, 25.0)],
            conf: 0.9,
        };
        let w = words_of(&d, &Bounds { x: 100.0, y: 10.0, w: 30.0, h: 10.0 });
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].text, "ab");
        assert_eq!(w[1].bounds.x, 120.0);
        assert_eq!(d.text(), "ab c");
    }

    fn column_with_ruby() -> RgbImage {
        // a 40 px wide column of "glyphs" (x 0..40) and a 12 px ruby strip
        // (x 46..58), 200 px tall
        let mut img = RgbImage::from_pixel(60, 200, image::Rgb([255, 255, 255]));
        for y in (10..190).step_by(30) {
            for yy in y..y + 20 {
                for x in 4..36 {
                    img.put_pixel(x, yy, image::Rgb([0, 0, 0]));
                }
            }
        }
        for y in (20..80).step_by(12) {
            for yy in y..y + 8 {
                for x in 47..57 {
                    img.put_pixel(x, yy, image::Rgb([0, 0, 0]));
                }
            }
        }
        img
    }

    #[test]
    fn ruby_strip_is_blanked() {
        let mut img = column_with_ruby();
        strip_ruby(&mut img, true);
        assert!((0..200).all(|y| (46..60).all(|x| img.get_pixel(x, y)[0] == 255)), "ruby left");
        assert_eq!(img.get_pixel(10, 15)[0], 0, "main column erased");
        // horizontal text: the same picture transposed, ruby above
        let v = column_with_ruby();
        let mut hz = RgbImage::new(200, 60);
        for (x, y, p) in v.enumerate_pixels() {
            hz.put_pixel(y, 59 - x, *p);
        }
        strip_ruby(&mut hz, false);
        assert!((0..200).all(|x| (0..14).all(|y| hz.get_pixel(x, y)[0] == 255)), "ruby above left");
    }

    #[test]
    fn ruby_lines_are_dropped() {
        let l = |x: f32, y: f32, w: f32, h: f32, rot: u16, t: &str| OcrLine {
            bounds: Bounds { x, y, w, h },
            text: t.into(),
            conf: 1.0,
            det_score: 1.0,
            rot,
            words: vec![],
        };
        let lines = vec![
            l(100.0, 100.0, 40.0, 300.0, 90, "column"),
            l(142.0, 150.0, 14.0, 40.0, 90, "ruby"),
            l(40.0, 100.0, 40.0, 300.0, 90, "next column"),
            l(300.0, 100.0, 14.0, 60.0, 90, "small but alone"),
        ];
        let kept: Vec<String> = drop_ruby_lines(lines).into_iter().map(|l| l.text).collect();
        assert_eq!(kept, vec!["column", "next column", "small but alone"]);
    }
}
