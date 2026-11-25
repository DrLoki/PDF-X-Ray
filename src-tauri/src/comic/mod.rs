//! Comic pages: panels, text containers (balloons, narration boxes, free
//! text, sound effects) and their reading order.
//!
//! Detection runs on the page raster together with the OCR (see
//! `ocr_page`): [`analyze`] returns a [`ComicLayout`] in image pixels, which
//! the frontend stores, scaled to PDF points, in [`crate::model::PageInput`].
//! The pipeline then builds the page from it instead of running XY-Cut
//! ([`layout::plan_comic_page`]): one block per container, grouped by
//! panel, in comic reading order ([`order`]), one translation unit each.
//!
//! - [`panels`]: gutter flood fill + recursive XY-cut on the raster.
//! - [`balloons`]: text blocks → flood fill of the background behind the
//!   letters → balloon / narration box / free text / sound effect.
//! - [`order`]: panels in tiers (left→right or right→left), then the
//!   containers inside each panel, top first.

pub mod balloons;
pub mod layout;
pub mod order;
pub mod panels;

use crate::model::Bounds;
use crate::ocr::OcrLine;
use image::RgbImage;
use serde::{Deserialize, Serialize};

/// Speech or thought balloon (closed, non-rectangular container).
pub const KIND_BALLOON: &str = "balloon";
/// Narration box (closed, rectangular container).
pub const KIND_NARRATION: &str = "narration";
/// Text without a container (on the artwork or in the gutter).
pub const KIND_TEXT: &str = "comic-text";
/// Sound effect: free text much bigger than the dialogue.
pub const KIND_SFX: &str = "sfx";

#[derive(Debug, Clone)]
pub struct BalloonDraft {
    pub bounds: Bounds,
    pub kind: &'static str,
    pub lines: Vec<usize>,
    pub shape: Vec<[f32; 3]>,
    pub fill: Option<[u8; 3]>,
}

/// A text container found on a comic page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComicBalloon {
    /// The container (inside of the balloon / box); for free text, the
    /// extent of its lines.
    pub bounds: Bounds,
    /// One of the `KIND_*` constants.
    pub kind: String,
    /// Indices of the OCR lines it contains (diagnostics; the pipeline
    /// assigns text runs geometrically).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<usize>,
    /// Inside of a balloon / narration box, as horizontal rows `[y, x0, x1]`
    /// top to bottom (the extent of the container's background on that
    /// row, letters included). Used to erase the original lettering and to
    /// wrap a translation inside the shape. Empty for free text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shape: Vec<[f32; 3]>,
    /// Background colour of the container, `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
}

/// Panels and text containers of a page (unordered: the pipeline orders
/// them, so the reading direction can change without a new detection).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComicLayout {
    pub panels: Vec<Bounds>,
    pub balloons: Vec<ComicBalloon>,
    /// Verdict of [`looks_like_comic`]: used by the automatic page mode.
    #[serde(default)]
    pub is_comic: bool,
}

impl ComicLayout {
    /// The same layout with every box multiplied by `s` (pixels → points).
    pub fn scaled(&self, s: f32) -> ComicLayout {
        let sc = |b: &Bounds| Bounds { x: b.x * s, y: b.y * s, w: b.w * s, h: b.h * s };
        ComicLayout {
            panels: self.panels.iter().map(sc).collect(),
            balloons: self
                .balloons
                .iter()
                .map(|b| ComicBalloon { bounds: sc(&b.bounds), shape: b.shape.iter().map(|r| [r[0] * s, r[1] * s, r[2] * s]).collect(), ..b.clone() })
                .collect(),
            is_comic: self.is_comic,
        }
    }
}

/// Detect panels and text containers of a page image (pixels).
pub fn analyze(img: &RgbImage, lines: &[OcrLine]) -> ComicLayout {
    let panels = panels::detect_panels(img);
    let drafts = balloons::detect_balloons(img, lines, &panels);
    let is_comic = looks_like_comic(&panels, &drafts, lines.len(), img.width() as f32 * img.height() as f32);
    ComicLayout {
        panels,
        balloons: drafts
            .into_iter()
            .map(|d| ComicBalloon {
                bounds: d.bounds,
                kind: d.kind.to_string(),
                lines: d.lines,
                shape: d.shape,
                fill: d.fill.map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])),
            })
            .collect(),
        is_comic,
    }
}

/// A page is a comic page when its text lives mostly in containers, or
/// when it has several panels and at least one balloon. A page of prose has
/// no panel (its lines are cut apart by the white leading) and its text
/// fills leak into the margins.
fn looks_like_comic(panels: &[Bounds], drafts: &[BalloonDraft], n_lines: usize, page_area: f32) -> bool {
    let contained: Vec<&BalloonDraft> = drafts.iter().filter(|d| d.kind == KIND_BALLOON || d.kind == KIND_NARRATION).collect();
    let balloons = drafts.iter().filter(|d| d.kind == KIND_BALLOON).count();
    let contained_lines: usize = contained.iter().map(|d| d.lines.len()).sum();
    let panel_area: f32 = panels.iter().map(|p| p.area()).sum();
    if n_lines == 0 {
        // wordless comic: several panels covering most of the page
        return panels.len() >= 2 && panel_area >= page_area * 0.5;
    }
    (balloons >= 2 && contained_lines * 2 >= n_lines) || (panels.len() >= 2 && balloons >= 1 && panel_area >= page_area * 0.4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(kind: &'static str, lines: usize) -> BalloonDraft {
        BalloonDraft { bounds: Bounds::default(), kind, lines: (0..lines).collect(), shape: vec![], fill: None }
    }

    #[test]
    fn comic_verdict() {
        let p = Bounds { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        // prose: no panels, a bordered sidebar
        assert!(!looks_like_comic(&[], &[d(KIND_NARRATION, 6), d(KIND_TEXT, 40)], 46, 50_000.0));
        // balloons holding most of the text
        assert!(looks_like_comic(&[], &[d(KIND_BALLOON, 3), d(KIND_BALLOON, 2), d(KIND_TEXT, 1)], 6, 50_000.0));
        // panels and one balloon
        assert!(looks_like_comic(&[p, p, p], &[d(KIND_BALLOON, 2), d(KIND_TEXT, 4)], 6, 50_000.0));
        // a page with two photos and a boxed caption is not
        assert!(!looks_like_comic(&[p, p], &[d(KIND_NARRATION, 2), d(KIND_TEXT, 30)], 32, 50_000.0));
    }

    #[test]
    fn scaled_layout() {
        let l = ComicLayout { panels: vec![Bounds { x: 10.0, y: 20.0, w: 30.0, h: 40.0 }], balloons: vec![], is_comic: true };
        let s = l.scaled(0.5);
        assert_eq!(s.panels[0], Bounds { x: 5.0, y: 10.0, w: 15.0, h: 20.0 });
        let l = ComicLayout { balloons: vec![ComicBalloon { shape: vec![[10.0, 20.0, 40.0]], ..Default::default() }], ..Default::default() };
        assert_eq!(l.scaled(0.5).balloons[0].shape, vec![[5.0, 10.0, 20.0]]);
    }
}
