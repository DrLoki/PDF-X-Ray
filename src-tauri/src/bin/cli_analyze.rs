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
            "--priority" => priority = it.next(),
            "--strategy" => strategy = it.next(),
            "--no-tags" => use_tags = false,
            "--no-stitch" => stitch = false,
            "-h" | "--help" => {
                eprintln!("cli_analyze [--text] [--priority X|Y|max-gap] [--strategy combined|delta-x|zero-run|dominant-font] [--no-tags] [--no-stitch] [FILE]");
                return;
            }
            _ => file = Some(a),
        }
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
