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

use crate::model::{AnalysisOptions, Bounds, DocumentResult, LayoutNode, PageInput, PageResult};
use crate::pipeline::analyze_document;
use serde::{Deserialize, Serialize};

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
}
