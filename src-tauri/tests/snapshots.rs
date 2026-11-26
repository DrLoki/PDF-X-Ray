//! Scores every snapshot in `tests/snapshots/` (saved from the app with
//! "📸 Snapshot" and copied here) against its reference segmentation.
//!
//! The scores are printed (`cargo test --test snapshots -- --nocapture`).
//! A snapshot fails the test only when it declares `"minAutoScore"` and the
//! default automatic settings score below it: that is how a page, once the
//! heuristics handle it, is locked against regressions.
//!
//! OCR snapshots (with their `_ocr.png`) are also run through the current OCR
//! engine when the models are installed (`$PDFXRAY_MODEL_DIR` or the app's
//! folder); `"ocr": { "maxOcrCer": 0.02 }` turns one into a regression test.
//! Without the models (CI) the OCR part is skipped.
//!
//! Comic snapshots are re-detected on their stored image with the stored OCR
//! lines (no models needed); with `"minAutoScore"` the current detection
//! must group the lines at least that well.

use pdf_xray_lib::eval::{evaluate, evaluate_comic, evaluate_ocr, Snapshot};
use pdf_xray_lib::ocr::OcrEngine;
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
    // engines by model set, loaded on demand (absent when the models are not installed)
    let mut engines: std::collections::HashMap<&str, Option<pdf_xray_lib::ocr::paddle::PaddleOcr>> = std::collections::HashMap::new();
    let mut failures = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).unwrap();
        let snap: Snapshot = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {}", f.display(), e));
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(r) = evaluate(&snap) {
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
        if let Some(r) = evaluate_comic(&snap, &f).unwrap_or_else(|e| panic!("{name}: {e}")) {
            let line = |s: &pdf_xray_lib::eval::ComicScore| format!("F1 {:.3} order {:.3} panels {}", s.grouping.f1, s.order, s.panels);
            match &r.current {
                Some(c) => {
                    println!("{name}: comic ref {} blocks | stored {} | current {}", r.reference_blocks, line(&r.stored), line(c));
                    if let Some(min) = r.min_auto_score {
                        if c.grouping.f1 + 1e-6 < min {
                            failures.push(format!("{name}: comic F1 {:.3} < minAutoScore {:.3}", c.grouping.f1, min));
                        }
                    }
                }
                None => println!("{name}: comic stored {} (image not available: not re-run)", line(&r.stored)),
            }
        }
        let set = snap
            .ocr
            .as_ref()
            .and_then(|o| pdf_xray_lib::ocr::models::set_for_engine(&o.engine))
            .unwrap_or(&pdf_xray_lib::ocr::models::LATIN);
        let engine = engines
            .entry(set.id)
            .or_insert_with(|| pdf_xray_lib::ocr::models::default_model_dir(set).and_then(|d| pdf_xray_lib::ocr::load_engine(&d, set).ok()));
        let ocr = evaluate_ocr(&snap, &f, engine.as_ref().map(|e| e as &dyn OcrEngine)).unwrap_or_else(|e| panic!("{name}: {e}"));
        if let Some(r) = ocr {
            match &r.current {
                Some(c) => {
                    println!("{name}: OCR ({}) stored CER {:.2}% | current CER {:.2}% WER {:.2}%", r.reference_source, r.stored.cer * 100.0, c.cer * 100.0, c.wer * 100.0);
                    if let Some(max) = r.max_ocr_cer {
                        if c.cer > max + 1e-6 {
                            failures.push(format!("{name}: OCR CER {:.4} > maxOcrCer {:.4}", c.cer, max));
                        }
                    }
                }
                None => println!("{name}: OCR stored CER {:.2}% (engine or image not available: not re-run)", r.stored.cer * 100.0),
            }
        }
    }
    assert!(failures.is_empty(), "regressions:\n{}", failures.join("\n"));
}
