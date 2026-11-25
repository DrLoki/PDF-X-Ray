//! Snapshots and evaluation.
//!
//! A snapshot (saved from the app with "📸 Snapshot") contains the extracted
//! input of a page (plus its neighbours), the settings the user was using,
//! the user's verdict and — when the user validated the result — a
//! *reference* segmentation (the leaf blocks of the page). Evaluation
//! re-runs the engine on the snapshot and measures how well the automatic
//! settings reproduce the reference, so heuristic changes can be scored
//! against a growing set of annotated pages.
//!
//! Metric: pairwise F1 over lines. Two lines are "together" when they fall
//! in the same block; precision/recall compare the pairs of the prediction
//! with the pairs of the reference (1.0 = identical grouping). Lines are
//! matched to blocks geometrically (centre in the smallest containing box),
//! so the reference stays valid when the engine's internals change.
//!
//! OCR pages also carry an `ocr` section: the exact image given to the OCR
//! (a PNG saved next to the JSON), the lines recognised at the time and a
//! reference transcription (corrected by the user, confirmed as correct, or
//! taken from the text layer of a digital page rasterised on purpose).
//! [`evaluate_ocr`] runs the current OCR engine on that image again and
//! scores both results with [`crate::ocr::metrics::score`], so changes to the
//! engine are measured the same way as changes to the layout heuristics.
//!
//! Comic pages carry their panels and containers in the page input
//! (`comic`); the reference leaves are then the validated containers, in
//! reading order. [`evaluate_comic`] runs the current comic detection on the
//! stored image and the stored OCR lines (so OCR changes do not interfere)
//! and scores grouping (pairwise F1) and reading order.

use crate::model::{AnalysisOptions, Bounds, DocumentResult, LayoutNode, PageInput, PageResult};
use crate::ocr::metrics::{score as ocr_score, OcrScore};
use crate::ocr::{OcrEngine, OcrLine};
use crate::pipeline::analyze_document;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default)]
    pub file: Option<String>,
    pub page_num: u32,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    pub settings: SnapshotSettings,
    pub pages: Vec<PageInput>,
    #[serde(default)]
    pub reference: Option<Reference>,
    /// Regression guard: the evaluation test fails if the default automatic
    /// settings score below this value.
    #[serde(default)]
    pub min_auto_score: Option<f32>,
    /// Present when the page was recognised with OCR.
    #[serde(default)]
    pub ocr: Option<OcrSnapshot>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrSnapshot {
    /// Engine and model set that produced `lines`.
    pub engine: String,
    /// Resolution of the image (pixels per inch; pixels per point = dpi / 72).
    pub dpi: f32,
    /// File name of the PNG given to the OCR, next to the snapshot JSON.
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Lines recognised when the snapshot was taken (image pixels).
    #[serde(default)]
    pub lines: Vec<OcrLine>,
    #[serde(default)]
    pub reference: Option<OcrReference>,
    /// Regression guard: the evaluation test fails if the current engine's
    /// CER on this page is above this value.
    #[serde(default)]
    pub max_ocr_cer: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrReference {
    /// `user` (corrected transcription), `ocr` (the OCR output confirmed as
    /// correct) or `text-layer` (text layer of a digital page).
    pub source: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrEvalResult {
    pub file: Option<String>,
    pub page_num: u32,
    pub reference_source: String,
    /// The lines stored in the snapshot.
    pub stored_engine: String,
    pub stored: OcrScore,
    /// The current engine run on the stored image (absent without image).
    pub current_engine: Option<String>,
    pub current: Option<OcrScore>,
    pub current_ms: Option<u64>,
    pub max_ocr_cer: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSettings {
    pub options: AnalysisOptions,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub leaves: Vec<RefLeaf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefLeaf {
    pub bounds: Bounds,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// Position in the reading order when the snapshot was taken.
    #[serde(default)]
    pub reading_order: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub f1: f32,
    pub precision: f32,
    pub recall: f32,
    pub blocks: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalResult {
    pub file: Option<String>,
    pub page_num: u32,
    pub verdict: Option<String>,
    pub note: Option<String>,
    pub reference_blocks: usize,
    /// Settings stored in the snapshot (manual or automatic).
    pub snapshot_settings: Score,
    /// Automatic thresholds with the snapshot's strategy and priority.
    pub auto_same_priority: Score,
    /// Default automatic settings (`combined`, rows first).
    pub auto_default: Score,
    pub min_auto_score: Option<f32>,
}

/// Leaf blocks of a page result (bounds, in reading order of the tree walk).
pub fn page_leaves(page: &PageResult) -> Vec<Bounds> {
    fn walk(n: &LayoutNode, out: &mut Vec<Bounds>) {
        if n.node_type == "leaf" {
            out.push(n.bounds);
        }
        for c in n.children.iter().flatten() {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    walk(&page.root, &mut out);
    out
}

/// Index of the smallest box containing the point, if any.
fn owner(boxes: &[Bounds], x: f32, y: f32) -> Option<usize> {
    let grow = |b: &Bounds| Bounds { x: b.x - 0.5, y: b.y - 0.5, w: b.w + 1.0, h: b.h + 1.0 };
    boxes
        .iter()
        .enumerate()
        .filter(|(_, b)| grow(b).contains_point(x, y))
        .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
        .map(|(i, _)| i)
}

/// Pairwise F1 of the grouping of `lines` induced by `predicted` vs `reference` boxes.
pub fn pair_score(lines: &[Bounds], reference: &[Bounds], predicted: &[Bounds]) -> Score {
    let assigned: Vec<(usize, Option<usize>)> = lines
        .iter()
        .filter_map(|l| owner(reference, l.cx(), l.cy()).map(|r| (r, owner(predicted, l.cx(), l.cy()))))
        .collect();
    let (mut both, mut same_ref, mut same_pred) = (0u64, 0u64, 0u64);
    for i in 0..assigned.len() {
        for j in i + 1..assigned.len() {
            let r = assigned[i].0 == assigned[j].0;
            let p = assigned[i].1.is_some() && assigned[i].1 == assigned[j].1;
            if r {
                same_ref += 1;
            }
            if p {
                same_pred += 1;
            }
            if r && p {
                both += 1;
            }
        }
    }
    let precision = if same_pred == 0 { 1.0 } else { both as f32 / same_pred as f32 };
    let recall = if same_ref == 0 { 1.0 } else { both as f32 / same_ref as f32 };
    let f1 = if precision + recall == 0.0 { 0.0 } else { 2.0 * precision * recall / (precision + recall) };
    Score { f1, precision, recall, blocks: predicted.len() }
}

fn result_page(r: &DocumentResult, page_num: u32) -> Option<&PageResult> {
    r.pages.iter().find(|p| p.page_num == page_num)
}

fn score_with(snap: &Snapshot, reference: &[Bounds], opts: &AnalysisOptions) -> Score {
    let r = analyze_document(&snap.pages, opts);
    match result_page(&r, snap.page_num) {
        Some(p) => {
            let lines: Vec<Bounds> = p.lines.iter().map(|l| l.bounds).collect();
            pair_score(&lines, reference, &page_leaves(p))
        }
        None => Score { f1: 0.0, precision: 0.0, recall: 0.0, blocks: 0 },
    }
}

/// Evaluate a snapshot; `None` when it carries no reference segmentation.
pub fn evaluate(snap: &Snapshot) -> Option<EvalResult> {
    let reference: Vec<Bounds> = snap.reference.as_ref()?.leaves.iter().map(|l| l.bounds).collect();
    let current = snap.settings.options.clone();
    let auto_same = AnalysisOptions { manual: None, ..current.clone() };
    let auto_default = AnalysisOptions { use_tags: current.use_tags, stitch: current.stitch, ..AnalysisOptions::default() };
    Some(EvalResult {
        file: snap.file.clone(),
        page_num: snap.page_num,
        verdict: snap.verdict.clone(),
        note: snap.note.clone(),
        reference_blocks: reference.len(),
        snapshot_settings: score_with(snap, &reference, &current),
        auto_same_priority: score_with(snap, &reference, &auto_same),
        auto_default: score_with(snap, &reference, &auto_default),
        min_auto_score: snap.min_auto_score,
    })
}

/// The OCR image next to a snapshot: `ocr.image`, or `<snapshot name>_ocr.png`.
fn ocr_image(ocr: &OcrSnapshot, snapshot_path: &Path) -> Option<PathBuf> {
    let dir = snapshot_path.parent().unwrap_or(Path::new("."));
    let stem = snapshot_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    ocr.image.as_ref().map(|n| dir.join(n)).filter(|p| p.is_file()).or_else(|| Some(dir.join(format!("{stem}_ocr.png"))).filter(|p| p.is_file()))
}

/// Score the OCR of a snapshot: the stored lines and, when `engine` is given
/// and the image is next to the snapshot file (`ocr.image`, or
/// `<snapshot name>_ocr.png`), a fresh run of the engine.
/// `Ok(None)` when the snapshot has no OCR reference.
pub fn evaluate_ocr(snap: &Snapshot, snapshot_path: &Path, engine: Option<&dyn OcrEngine>) -> Result<Option<OcrEvalResult>, String> {
    let Some(ocr) = snap.ocr.as_ref() else { return Ok(None) };
    let Some(reference) = ocr.reference.as_ref() else { return Ok(None) };
    let stored_text: Vec<String> = ocr.lines.iter().map(|l| l.text.clone()).collect();
    let stored = ocr_score(&reference.lines, &stored_text);
    let (mut current, mut current_engine, mut current_ms) = (None, None, None);
    if let (Some(engine), Some(path)) = (engine, ocr_image(ocr, snapshot_path)) {
        let img = image::open(&path).map_err(|e| format!("{}: {e}", path.display()))?.into_rgb8();
        let t0 = std::time::Instant::now();
        let page = engine.recognize(&img)?;
        current_ms = Some(t0.elapsed().as_millis() as u64);
        let text: Vec<String> = page.lines.iter().map(|l| l.text.clone()).collect();
        current = Some(ocr_score(&reference.lines, &text));
        current_engine = Some(engine.id());
    }
    Ok(Some(OcrEvalResult {
        file: snap.file.clone(),
        page_num: snap.page_num,
        reference_source: reference.source.clone(),
        stored_engine: ocr.engine.clone(),
        stored,
        current_engine,
        current,
        current_ms,
        max_ocr_cer: ocr.max_ocr_cer,
    }))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicScore {
    /// Grouping of the lines into containers (pairwise F1, see [`pair_score`]).
    pub grouping: Score,
    /// Share of the pairs of reference containers read in the same order
    /// (1.0 = same reading order).
    pub order: f32,
    pub panels: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicEvalResult {
    pub file: Option<String>,
    pub page_num: u32,
    pub reference_blocks: usize,
    /// The panels and containers stored in the snapshot.
    pub stored: ComicScore,
    /// The current detection run on the stored image (absent without image).
    pub current: Option<ComicScore>,
    pub current_ms: Option<u64>,
    pub min_auto_score: Option<f32>,
}

/// Share of the reference pairs (in reference reading order) whose
/// predicted blocks come in the same order. Each reference block is mapped
/// to the predicted block holding most of its lines.
pub fn order_score(lines: &[Bounds], reference: &[Bounds], predicted: &[Bounds]) -> f32 {
    let mut votes: Vec<std::collections::HashMap<usize, usize>> = vec![Default::default(); reference.len()];
    for l in lines {
        if let (Some(r), Some(p)) = (owner(reference, l.cx(), l.cy()), owner(predicted, l.cx(), l.cy())) {
            *votes[r].entry(p).or_insert(0) += 1;
        }
    }
    let mapped: Vec<Option<usize>> = votes.iter().map(|v| v.iter().max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k))).map(|(k, _)| *k)).collect();
    let (mut agree, mut total) = (0usize, 0usize);
    for i in 0..mapped.len() {
        for j in i + 1..mapped.len() {
            if let (Some(a), Some(b)) = (mapped[i], mapped[j]) {
                if a != b {
                    total += 1;
                    agree += (a < b) as usize;
                }
            }
        }
    }
    if total == 0 {
        1.0
    } else {
        agree as f32 / total as f32
    }
}

fn comic_score(snap: &Snapshot, page: &PageInput, reference: &[Bounds]) -> ComicScore {
    let pages: Vec<PageInput> = snap.pages.iter().map(|p| if p.page_num == page.page_num { page.clone() } else { p.clone() }).collect();
    let r = analyze_document(&pages, &snap.settings.options);
    match result_page(&r, snap.page_num) {
        Some(p) => {
            let lines: Vec<Bounds> = p.lines.iter().map(|l| l.bounds).collect();
            let leaves = page_leaves(p);
            ComicScore {
                grouping: pair_score(&lines, reference, &leaves),
                order: order_score(&lines, reference, &leaves),
                panels: page.comic.as_ref().map_or(0, |c| c.panels.len()),
            }
        }
        None => ComicScore { grouping: Score { f1: 0.0, precision: 0.0, recall: 0.0, blocks: 0 }, order: 0.0, panels: 0 },
    }
}

/// Evaluate the comic layout of a snapshot: the stored panels and
/// containers and, when the OCR image is next to the snapshot, a fresh
/// detection on it (with the stored OCR lines). `Ok(None)` when the page is
/// not a comic page or has no reference.
pub fn evaluate_comic(snap: &Snapshot, snapshot_path: &Path) -> Result<Option<ComicEvalResult>, String> {
    let Some(reference) = snap.reference.as_ref() else { return Ok(None) };
    let Some(page) = snap.pages.iter().find(|p| p.page_num == snap.page_num) else { return Ok(None) };
    if page.comic.is_none() {
        return Ok(None);
    }
    let mut leaves = reference.leaves.clone();
    leaves.sort_by_key(|l| l.reading_order.unwrap_or(usize::MAX));
    let ref_bounds: Vec<Bounds> = leaves.iter().map(|l| l.bounds).collect();
    let stored = comic_score(snap, page, &ref_bounds);
    let (mut current, mut current_ms) = (None, None);
    if let Some(ocr) = snap.ocr.as_ref() {
        if let Some(path) = ocr_image(ocr, snapshot_path) {
            let img = image::open(&path).map_err(|e| format!("{}: {e}", path.display()))?.into_rgb8();
            let t0 = std::time::Instant::now();
            let layout = crate::comic::analyze(&img, &ocr.lines).scaled(72.0 / ocr.dpi);
            current_ms = Some(t0.elapsed().as_millis() as u64);
            let fresh = PageInput { comic: Some(layout), ..page.clone() };
            current = Some(comic_score(snap, &fresh, &ref_bounds));
        }
    }
    Ok(Some(ComicEvalResult {
        file: snap.file.clone(),
        page_num: snap.page_num,
        reference_blocks: ref_bounds.len(),
        stored,
        current,
        current_ms,
        min_auto_score: snap.min_auto_score,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds { x, y, w, h }
    }

    #[test]
    fn pair_score_identical_and_merged() {
        let lines: Vec<Bounds> = (0..4).map(|k| b(10.0 + 200.0 * (k / 2) as f32, 10.0 + 20.0 * (k % 2) as f32, 100.0, 10.0)).collect();
        let cols = [b(0.0, 0.0, 150.0, 100.0), b(200.0, 0.0, 150.0, 100.0)];
        let s = pair_score(&lines, &cols, &cols);
        assert!((s.f1 - 1.0).abs() < 1e-6);
        // predicting one big block merges the columns: recall 1, precision 2/6
        let merged = [b(0.0, 0.0, 400.0, 100.0)];
        let s = pair_score(&lines, &cols, &merged);
        assert!((s.recall - 1.0).abs() < 1e-6);
        assert!((s.precision - 2.0 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn order_score_counts_swapped_pairs() {
        let lines: Vec<Bounds> = (0..3).map(|k| b(10.0 + 100.0 * k as f32, 10.0, 50.0, 10.0)).collect();
        let blocks: Vec<Bounds> = (0..3).map(|k| b(100.0 * k as f32, 0.0, 90.0, 30.0)).collect();
        assert!((order_score(&lines, &blocks, &blocks) - 1.0).abs() < 1e-6);
        let swapped = vec![blocks[1], blocks[0], blocks[2]];
        // reference order 0,1,2 -> predicted positions 1,0,2: one pair of three swapped
        assert!((order_score(&lines, &blocks, &swapped) - 2.0 / 3.0).abs() < 1e-6);
    }
}
