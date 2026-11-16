//! Scores every snapshot in `tests/snapshots/` (saved from the app with
//! "📸 Snapshot" and copied here) against its reference segmentation.
//!
//! The scores are printed (`cargo test --test snapshots -- --nocapture`).
//! A snapshot fails the test only when it declares `"minAutoScore"` and the
//! default automatic settings score below it: that is how a page, once the
//! heuristics handle it, is locked against regressions.

use pdf_xray_lib::eval::{evaluate, Snapshot};
use std::fs;
use std::path::PathBuf;

#[test]
fn snapshots_do_not_regress() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("snapshots");
    let Ok(rd) = fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect();
    files.sort();
    let mut failures = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).unwrap();
        let snap: Snapshot = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {}", f.display(), e));
        let Some(r) = evaluate(&snap) else { continue };
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        println!(
            "{name}: page {} ref {} blocks | snapshot settings {:.3} | auto same priority {:.3} | auto default {:.3}",
            r.page_num, r.reference_blocks, r.snapshot_settings.f1, r.auto_same_priority.f1, r.auto_default.f1
        );
        if let Some(min) = r.min_auto_score {
            if r.auto_default.f1 + 1e-6 < min {
                failures.push(format!("{name}: auto default {:.3} < minAutoScore {:.3}", r.auto_default.f1, min));
            }
        }
    }
    assert!(failures.is_empty(), "regressions:\n{}", failures.join("\n"));
}
