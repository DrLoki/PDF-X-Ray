//! OCR for pages without a usable text layer (scans, image-only pages).
//!
//! The page is rasterised by the frontend (pdf.js) and recognised here; the
//! recognised lines become ordinary [`TextElement`]s in PDF points, so the
//! rest of the pipeline (lines → XY-Cut → roles → paragraphs) is unchanged.
//!
//! - [`classify`]: decides which pages need OCR (`text` / `scan`).
//! - [`paddle`]: the PaddleOCR PP-OCRv5 engine (DBNet detection + Latin CTC
//!   recognition) running on `rten`, a pure-Rust ONNX runtime.
//! - [`models`]: the model files, their download (on first use, after the
//!   user's confirmation) and verification. Recognition itself is offline.
//! - [`metrics`]: character / word error rates used by the OCR snapshots.

pub mod classify;
pub mod metrics;
pub mod models;
pub mod paddle;

use crate::model::{Bounds, TextElement};
use image::RgbImage;
use serde::{Deserialize, Serialize};

/// One word inside a recognised line, in image pixels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrWord {
    pub text: String,
    pub bounds: Bounds,
}

/// A recognised text line, in image pixels (origin top-left).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub bounds: Bounds,
    pub text: String,
    /// Mean probability of the recognised characters (0–1).
    pub conf: f32,
    /// Mean text probability of the detected region (0–1).
    pub det_score: f32,
    /// Reading direction, degrees clockwise (0, 90, 270), as in `TextElement::rot`.
    pub rot: u16,
    /// Word boxes (horizontal lines only; estimated from the CTC time steps).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<OcrWord>,
}

/// Result of recognising one page image.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrPage {
    /// Engine and model set, e.g. `paddleocr-v5-latin`.
    pub engine: String,
    pub width: u32,
    pub height: u32,
    pub lines: Vec<OcrLine>,
    pub detect_ms: u64,
    pub recognize_ms: u64,
}

/// An OCR engine: image in, text lines out.
pub trait OcrEngine: Send + Sync {
    fn id(&self) -> String;
    fn recognize(&self, img: &RgbImage) -> Result<OcrPage, String>;
}

/// Load the default engine from a model folder (see [`models`]).
pub fn load_engine(dir: &std::path::Path) -> Result<paddle::PaddleOcr, String> {
    let st = models::status(dir);
    if !st.installed {
        return Err(format!("modelli OCR non installati in {}", st.dir));
    }
    paddle::PaddleOcr::load(&models::det_path(dir), &models::rec_path(dir), &models::dict_path(dir))
}

/// Lines whose recognition confidence is below this are dropped.
pub const MIN_LINE_CONF: f32 = 0.5;

/// Ratio between the height of a detected line box and the font size.
/// The DBNet box spans ascenders to descenders plus a little padding;
/// calibrated against the text layer of digital pages (see tests).
pub const BOX_HEIGHT_PER_EM: f32 = 1.3;

/// Convert recognised lines into text runs in PDF points.
///
/// `scale` is image pixels per PDF point (DPI / 72). A horizontal line is
/// split into several runs where the gap between two words is wider than
/// the line height, so a line box that crosses a column gutter does not glue
/// the two columns together (the layout engine cuts between runs).
///
/// The height of a detected box depends on the letters of the line (a line
/// without ascenders or descenders gets a lower box), which would look like
/// changes of type size and of leading to the paragraph heuristics. Lines of
/// similar thickness are therefore grouped (within 25% of the group's
/// thinnest line) and get the group's median thickness, centred on the
/// line: one size per group, as in a text layer.
pub fn to_text_elements(page: &OcrPage, scale: f32) -> Vec<TextElement> {
    let s = if scale > 0.0 { 1.0 / scale } else { 1.0 };
    let pt = |b: &Bounds| Bounds { x: b.x * s, y: b.y * s, w: b.w * s, h: b.h * s };
    let thickness: Vec<f32> = page.lines.iter().map(|l| if l.rot == 0 { l.bounds.h } else { l.bounds.w }).collect();
    let weights: Vec<usize> = page.lines.iter().map(|l| l.text.chars().count()).collect();
    let norm = size_groups(&thickness, &weights);
    let mut out = Vec::new();
    for (li, line) in page.lines.iter().enumerate() {
        let thick = norm[li];
        let fs = thick * s / BOX_HEIGHT_PER_EM;
        let mut push = |text: String, b: Bounds| {
            // the normalised thickness, centred on the detected box
            let b = if line.rot == 0 {
                Bounds { y: b.cy() - thick * 0.5, h: thick, ..b }
            } else {
                Bounds { x: b.cx() - thick * 0.5, w: thick, ..b }
            };
            let b = pt(&b);
            out.push(TextElement {
                text,
                x: b.x,
                y: b.y,
                width: b.w,
                height: b.h,
                font_size: fs,
                font_name: Some("OCR".into()),
                rot: Some(line.rot as f32),
                source: Some("ocr".into()),
                conf: Some(line.conf),
                ..Default::default()
            });
        };
        if line.rot != 0 || line.words.len() < 2 {
            push(line.text.clone(), line.bounds);
            continue;
        }
        // split at gutter-wide gaps
        let mut start = 0;
        for i in 1..=line.words.len() {
            let split = i == line.words.len() || line.words[i].bounds.x - line.words[i - 1].bounds.x1() > line.bounds.h;
            if split {
                let ws = &line.words[start..i];
                let text = ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                let x0 = ws[0].bounds.x.max(line.bounds.x);
                let x1 = ws[ws.len() - 1].bounds.x1().min(line.bounds.x1());
                push(text, Bounds { x: x0, y: line.bounds.y, w: (x1 - x0).max(1.0), h: line.bounds.h });
                start = i;
            }
        }
    }
    out
}

/// Thickness of the size group of each line. Groups are found around modes:
/// the value whose ±`SIZE_TOLERANCE` window holds the most weight (characters)
/// among the lines not yet grouped seeds a group with every line in that
/// window; repeat. Each line gets its group's weighted median.
fn size_groups(values: &[f32], weights: &[usize]) -> Vec<f32> {
    const SIZE_TOLERANCE: f32 = 1.2;
    let n = values.len();
    let mut out = vec![0.0; n];
    let mut free: Vec<usize> = (0..n).collect();
    while !free.is_empty() {
        let in_window = |c: f32, v: f32| v >= c / SIZE_TOLERANCE && v <= c * SIZE_TOLERANCE;
        let centre = free
            .iter()
            .map(|&i| values[i])
            .max_by(|&a, &b| {
                let w = |c: f32| free.iter().filter(|&&j| in_window(c, values[j])).map(|&j| weights[j].max(1)).sum::<usize>();
                w(a).cmp(&w(b)).then(b.total_cmp(&a))
            })
            .unwrap();
        let (mut group, rest): (Vec<usize>, Vec<usize>) = free.iter().partition(|&&i| in_window(centre, values[i]));
        group.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
        let total: usize = group.iter().map(|&i| weights[i].max(1)).sum();
        let mut acc = 0;
        let mut median = values[group[0]];
        for &i in &group {
            acc += weights[i].max(1);
            if acc * 2 >= total {
                median = values[i];
                break;
            }
        }
        for &i in &group {
            out[i] = median;
        }
        free = rest;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_groups_snap_similar_lines() {
        // the body (45–56, many characters) stays one group even though
        // 56 > 1.25 × 45; the headings and the folio get their own
        let g = size_groups(&[45.0, 50.0, 56.0, 49.0, 120.0, 110.0, 20.0], &[60, 60, 60, 60, 10, 10, 2]);
        assert_eq!(g, vec![49.0, 49.0, 49.0, 49.0, 110.0, 110.0, 20.0]);
    }

    fn word(text: &str, x: f32, w: f32) -> OcrWord {
        OcrWord { text: text.into(), bounds: Bounds { x, y: 100.0, w, h: 40.0 } }
    }

    #[test]
    fn splits_lines_at_gutter_gaps() {
        let page = OcrPage {
            engine: "t".into(),
            width: 1000,
            height: 1000,
            lines: vec![OcrLine {
                bounds: Bounds { x: 0.0, y: 100.0, w: 900.0, h: 40.0 },
                text: "left side right".into(),
                conf: 0.9,
                det_score: 0.9,
                rot: 0,
                words: vec![word("left", 0.0, 80.0), word("side", 95.0, 80.0), word("right", 400.0, 100.0)],
            }],
            detect_ms: 0,
            recognize_ms: 0,
        };
        let els = to_text_elements(&page, 2.0);
        assert_eq!(els.len(), 2);
        assert_eq!(els[0].text, "left side");
        assert_eq!(els[1].text, "right");
        assert!((els[1].x - 200.0).abs() < 0.01);
        assert!((els[0].font_size - 40.0 / 2.0 / BOX_HEIGHT_PER_EM).abs() < 0.01);
        assert_eq!(els[0].source.as_deref(), Some("ocr"));
    }
}
