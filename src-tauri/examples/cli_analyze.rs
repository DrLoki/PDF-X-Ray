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
            "-h" | "--help" => {
                eprintln!("cli_analyze [--text] [--priority X|Y|max-gap] [--strategy combined|delta-x|zero-run|dominant-font] [--no-tags] [--no-stitch] [FILE]");
                eprintln!("cli_analyze --distribute [FILE]");
                eprintln!("cli_analyze --eval [--json] SNAPSHOT_OR_DIR...");
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
