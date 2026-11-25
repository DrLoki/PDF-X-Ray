//! Headless analyzer.
//!
//! Usage:
//!   cli_analyze [--text] [--priority X|Y|max-gap] [--strategy NAME] [--no-tags] [--no-stitch] [FILE]
//!
//! FILE (or stdin) is either a single page `{ "items": [...], "pageBounds": {...}, ... }`
//! or a document `{ "pages": [ ... ], "options": { ... } }`.
//! Default output: the full `DocumentResult` as JSON. With `--text`: the
//! logical paragraphs in reading order, one per block, prefixed by role.
//!
//!   cli_analyze --distribute [FILE]
//!
//! reads `{ "units": [ { "text": "...", "weights": [..] } ] }` and prints the
//! translated text split across segments (same as the `distribute_translations`
//! Tauri command).
//!
//!   cli_analyze --eval PATH... [--json]
//!
//! scores snapshots (files or folders of `*.json` saved with "📸 Snapshot")
//! against their reference segmentation: the stored settings, automatic
//! thresholds with the snapshot's strategy/priority, and default automatic
//! settings. See `pdf_xray_lib::eval`.
//!
//!   cli_analyze --ocr IMAGE [--models DIR] [--det-max-side N] [--ref FILE] [--dpi N [--text]] [--json]
//!
//! runs the OCR engine on an image and prints the recognised lines (box in
//! pixels, detection score / recognition confidence, text). `--ref` scores
//! them against a reference transcription (one line per line); `--dpi` turns
//! them into text runs and runs the layout pipeline (`--text` prints the
//! paragraphs, `--json` the page input). Models: `--models`,
//! `$PDFXRAY_MODEL_DIR` or the app's folder.
//!
//!   cli_analyze --eval-ocr PATH... [--models DIR] [--json]
//!
//! scores the OCR snapshots against their reference transcription: the lines
//! stored in the snapshot and a fresh run of the current engine on the saved
//! image. See `pdf_xray_lib::eval::evaluate_ocr`.

use pdf_xray_lib::model::{AnalysisOptions, PageInput};
use pdf_xray_lib::pipeline::analyze_document;
use std::env;
use std::fs;
use std::io::{self, Read};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocInput {
    pages: Vec<PageInput>,
    #[serde(default)]
    options: Option<AnalysisOptions>,
}

#[derive(serde::Deserialize)]
struct LegacyPage {
    #[serde(flatten)]
    page: PageInput,
    strategy: Option<String>,
    priority: Option<String>,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut text_mode = false;
    let mut distribute = false;
    let mut eval_mode = false;
    let mut json_out = false;
    let mut paths: Vec<String> = Vec::new();
    let mut file: Option<String> = None;
    let mut priority: Option<String> = None;
    let mut strategy: Option<String> = None;
    let mut use_tags = true;
    let mut stitch = true;
    let mut ocr_image: Option<String> = None;
    let mut eval_ocr = false;
    let mut models: Option<String> = None;
    let mut det_max_side: Option<u32> = None;
    let mut ref_file: Option<String> = None;
    let mut dpi: Option<f32> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--text" => text_mode = true,
            "--distribute" => distribute = true,
            "--eval" => eval_mode = true,
            "--json" => json_out = true,
            "--priority" => priority = it.next(),
            "--strategy" => strategy = it.next(),
            "--no-tags" => use_tags = false,
            "--no-stitch" => stitch = false,
            "--ocr" => ocr_image = it.next(),
            "--eval-ocr" => eval_ocr = true,
            "--models" => models = it.next(),
            "--det-max-side" => det_max_side = it.next().and_then(|v| v.parse().ok()),
            "--ref" => ref_file = it.next(),
            "--dpi" => dpi = it.next().and_then(|v| v.parse().ok()),
            "-h" | "--help" => {
                eprintln!("cli_analyze [--text] [--priority X|Y|max-gap] [--strategy combined|delta-x|zero-run|dominant-font] [--no-tags] [--no-stitch] [FILE]");
                eprintln!("cli_analyze --distribute [FILE]");
                eprintln!("cli_analyze --eval [--json] SNAPSHOT_OR_DIR...");
                eprintln!("cli_analyze --ocr IMAGE [--models DIR] [--det-max-side N] [--ref FILE] [--dpi N [--text]] [--json]");
                eprintln!("cli_analyze --eval-ocr [--models DIR] [--json] SNAPSHOT_OR_DIR...");
                return;
            }
            _ => {
                paths.push(a.clone());
                file = Some(a);
            }
        }
    }

    if eval_mode {
        run_eval(&paths, json_out);
        return;
    }
    if eval_ocr {
        run_eval_ocr(&paths, models.as_deref(), det_max_side, json_out);
        return;
    }
    if let Some(img) = ocr_image {
        run_ocr(&img, models.as_deref(), det_max_side, ref_file.as_deref(), dpi, text_mode, json_out);
        return;
    }

    let json = match &file {
        Some(f) => fs::read_to_string(f).unwrap_or_else(|e| {
            eprintln!("Failed to read {}: {}", f, e);
            std::process::exit(1)
        }),
        None => {
            let mut s = String::new();
            io::stdin().read_to_string(&mut s).expect("Failed to read stdin");
            s
        }
    };

    if distribute {
        #[derive(serde::Deserialize)]
        struct Units {
            units: Vec<pdf_xray_lib::reflow::DistUnit>,
        }
        let u: Units = serde_json::from_str(&json).unwrap_or_else(|e| {
            eprintln!("Failed to parse units: {}", e);
            std::process::exit(2)
        });
        let out: Vec<Vec<String>> = u.units.iter().map(|x| pdf_xray_lib::reflow::distribute(&x.text, &x.weights)).collect();
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }

    let (pages, mut opts) = if let Ok(doc) = serde_json::from_str::<DocInput>(&json) {
        (doc.pages, doc.options.unwrap_or_default())
    } else {
        match serde_json::from_str::<LegacyPage>(&json) {
            Ok(p) => {
                let mut o = AnalysisOptions::default();
                if let Some(s) = p.strategy {
                    o.strategy = s;
                }
                if let Some(pr) = p.priority {
                    o.priority = pr;
                }
                (vec![p.page], o)
            }
            Err(e) => {
                eprintln!("Failed to parse JSON input: {}", e);
                std::process::exit(2);
            }
        }
    };
    if let Some(p) = priority {
        opts.priority = p;
    }
    if let Some(s) = strategy {
        opts.strategy = s;
    }
    opts.use_tags = use_tags && opts.use_tags;
    opts.stitch = stitch && opts.stitch;

    let result = analyze_document(&pages, &opts);

    if text_mode {
        for p in &result.paragraphs {
            let pages: Vec<String> = {
                let mut v: Vec<u32> = p.segments.iter().map(|s| s.page_num).collect();
                v.dedup();
                v.iter().map(|n| n.to_string()).collect()
            };
            let joins = if p.joins.is_empty() {
                String::new()
            } else {
                format!(" ⟲{}", p.joins.iter().map(|j| j.kind.as_str()).collect::<Vec<_>>().join(","))
            };
            println!("[{} p.{} {}{}] {}\n", p.id, pages.join("-"), p.role, joins, p.text);
        }
        let s = &result.stats;
        eprintln!(
            "pages={} paragraphs={} stitched={} body_fs={} chars in/out={}/{}",
            s.page_count, s.paragraph_count, s.stitched_joins, s.body_font_size, s.input_chars, s.output_chars
        );
        return;
    }

    match serde_json::to_string_pretty(&result) {
        Ok(out) => println!("{}", out),
        Err(e) => {
            eprintln!("Failed to serialize result: {}", e);
            std::process::exit(3);
        }
    }
}

fn collect_snapshots(paths: &[String]) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let path = std::path::PathBuf::from(p);
        if path.is_dir() {
            if let Ok(rd) = fs::read_dir(&path) {
                let mut v: Vec<_> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|f| f.extension().map(|x| x == "json").unwrap_or(false))
                    .collect();
                v.sort();
                out.extend(v);
            }
        } else {
            out.push(path);
        }
    }
    out
}

fn run_eval(paths: &[String], json_out: bool) {
    use pdf_xray_lib::eval::{evaluate, Snapshot};
    let files = collect_snapshots(paths);
    if files.is_empty() {
        eprintln!("No snapshot files given.");
        std::process::exit(1);
    }
    let mut results = Vec::new();
    let mut skipped = 0;
    for f in &files {
        let snap: Snapshot = match fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip {}: {}", f.display(), e);
                skipped += 1;
                continue;
            }
        };
        match evaluate(&snap) {
            Some(r) => results.push((f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), r)),
            None => {
                eprintln!("skip {}: no reference segmentation (verdict {:?})", f.display(), snap.verdict);
                skipped += 1;
            }
        }
    }
    if json_out {
        let v: Vec<_> = results.iter().map(|(name, r)| serde_json::json!({ "snapshot": name, "result": r })).collect();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return;
    }
    println!("{:<44} {:>5} {:>4} | {:>14} | {:>14} | {:>14}", "snapshot", "page", "ref", "snapshot sett.", "auto (same pr.)", "auto default");
    let fmt = |s: &pdf_xray_lib::eval::Score| format!("{:.3} ({:>3} bl)", s.f1, s.blocks);
    let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
    for (name, r) in &results {
        let short: String = name.chars().take(44).collect();
        println!(
            "{:<44} {:>5} {:>4} | {:>14} | {:>14} | {:>14}{}",
            short,
            r.page_num,
            r.reference_blocks,
            fmt(&r.snapshot_settings),
            fmt(&r.auto_same_priority),
            fmt(&r.auto_default),
            r.min_auto_score.map(|m| if r.auto_default.f1 + 1e-6 < m { format!("  ✗ < {:.3}", m) } else { String::new() }).unwrap_or_default()
        );
        a += r.snapshot_settings.f1;
        b += r.auto_same_priority.f1;
        c += r.auto_default.f1;
    }
    if !results.is_empty() {
        let n = results.len() as f32;
        println!("{:<44} {:>5} {:>4} | {:>14.3} | {:>14.3} | {:>14.3}", "MEAN F1", "", "", a / n, b / n, c / n);
    }
    if skipped > 0 {
        eprintln!("{} file(s) skipped", skipped);
    }
}

fn load_engine(models: Option<&str>, det_max_side: Option<u32>) -> pdf_xray_lib::ocr::paddle::PaddleOcr {
    let dir = models.map(std::path::PathBuf::from).or_else(pdf_xray_lib::ocr::models::default_model_dir).unwrap_or_else(|| {
        eprintln!("No model folder: pass --models DIR or set PDFXRAY_MODEL_DIR");
        std::process::exit(1)
    });
    let mut engine = pdf_xray_lib::ocr::load_engine(&dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    if let Some(n) = det_max_side {
        engine.params.det_max_side = n;
    }
    engine
}

fn run_ocr(img: &str, models: Option<&str>, det_max_side: Option<u32>, ref_file: Option<&str>, dpi: Option<f32>, text_mode: bool, json_out: bool) {
    use pdf_xray_lib::ocr::OcrEngine;
    let engine = load_engine(models, det_max_side);
    let rgb = image::open(img)
        .unwrap_or_else(|e| {
            eprintln!("{img}: {e}");
            std::process::exit(1)
        })
        .into_rgb8();
    let page = engine.recognize(&rgb).unwrap_or_else(|e| {
        eprintln!("OCR failed: {e}");
        std::process::exit(1)
    });
    if let Some(dpi) = dpi {
        let scale = dpi / 72.0;
        let items = pdf_xray_lib::ocr::to_text_elements(&page, scale);
        let bounds = pdf_xray_lib::model::Bounds { x: 0.0, y: 0.0, w: rgb.width() as f32 / scale, h: rgb.height() as f32 / scale };
        let input = PageInput { page_num: 1, page_bounds: bounds, items, bordered_boxes: vec![], images: vec![bounds] };
        if json_out && !text_mode {
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "pages": [input] })).unwrap());
            return;
        }
        let r = analyze_document(&[input], &AnalysisOptions::default());
        for p in &r.paragraphs {
            println!("[{} {}] {}\n", p.id, p.role, p.text);
        }
        eprintln!("paragraphs={} body_fs={:.1}", r.stats.paragraph_count, r.stats.body_font_size);
        return;
    }
    if json_out {
        println!("{}", serde_json::to_string_pretty(&page).unwrap());
    } else {
        for l in &page.lines {
            let b = l.bounds;
            println!("[{:5.0},{:5.0} {:5.0}x{:4.0}] {:.2}/{:.2} r{:<3} {}", b.x, b.y, b.w, b.h, l.det_score, l.conf, l.rot, l.text);
        }
    }
    eprintln!("{} lines · detect {} ms · recognize {} ms · {}x{} px", page.lines.len(), page.detect_ms, page.recognize_ms, page.width, page.height);
    if let Some(rf) = ref_file {
        let reference: Vec<String> = fs::read_to_string(rf)
            .unwrap_or_else(|e| {
                eprintln!("{rf}: {e}");
                std::process::exit(1)
            })
            .lines()
            .map(str::to_string)
            .collect();
        let text: Vec<String> = page.lines.iter().map(|l| l.text.clone()).collect();
        let s = pdf_xray_lib::ocr::metrics::score(&reference, &text);
        eprintln!(
            "CER {:.2}% · WER {:.2}% · lines ref {} / ocr {} · paired {} · missed {} · extra {}",
            s.cer * 100.0,
            s.wer * 100.0,
            s.reference_lines,
            s.recognised_lines,
            s.paired_lines,
            s.missed_lines,
            s.extra_lines
        );
    }
}

fn run_eval_ocr(paths: &[String], models: Option<&str>, det_max_side: Option<u32>, json_out: bool) {
    use pdf_xray_lib::eval::{evaluate_ocr, Snapshot};
    let files = collect_snapshots(paths);
    if files.is_empty() {
        eprintln!("No snapshot files given.");
        std::process::exit(1);
    }
    let engine = load_engine(models, det_max_side);
    let mut results = Vec::new();
    for f in &files {
        let snap: Snapshot = match fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip {}: {}", f.display(), e);
                continue;
            }
        };
        match evaluate_ocr(&snap, f, Some(&engine)) {
            Ok(Some(r)) => results.push((f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), r)),
            Ok(None) => {}
            Err(e) => eprintln!("skip {}: {}", f.display(), e),
        }
    }
    if json_out {
        let v: Vec<_> = results.iter().map(|(name, r)| serde_json::json!({ "snapshot": name, "result": r })).collect();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return;
    }
    println!("{:<44} {:>5} {:<10} | {:>17} | {:>17} | {:>7}", "snapshot", "page", "reference", "stored CER/WER", "current CER/WER", "ms");
    let pct = |s: &pdf_xray_lib::ocr::metrics::OcrScore| format!("{:5.2}% / {:5.2}%", s.cer * 100.0, s.wer * 100.0);
    let (mut a, mut b, mut n) = (0.0, 0.0, 0usize);
    for (name, r) in &results {
        let short: String = name.chars().take(44).collect();
        let cur = r.current.as_ref().map(pct).unwrap_or_else(|| "no image".into());
        let flag = match (r.max_ocr_cer, r.current.as_ref()) {
            (Some(m), Some(c)) if c.cer > m + 1e-6 => format!("  ✗ > {:.2}%", m * 100.0),
            _ => String::new(),
        };
        let ms = r.current_ms.map(|m| m.to_string()).unwrap_or_default();
        println!("{:<44} {:>5} {:<10} | {:>17} | {:>17} | {:>7}{}", short, r.page_num, r.reference_source, pct(&r.stored), cur, ms, flag);
        if let Some(c) = &r.current {
            a += r.stored.cer;
            b += c.cer;
            n += 1;
        }
    }
    if n > 0 {
        println!("{:<44} {:>5} {:<10} | {:>16.2}% | {:>16.2}% |", "MEAN CER", "", "", a / n as f32 * 100.0, b / n as f32 * 100.0);
    } else if results.is_empty() {
        eprintln!("No OCR snapshot with a reference transcription.");
    }
}
