use std::env;
use std::fs;
use std::io::{self, Read};
use pdf_xray_lib::gutter_detection::{TextElement, Bounds, XYCutResult, perform_auto_xycut};

fn main() {
    // Read JSON from stdin or from first argument file
    let args: Vec<String> = env::args().collect();
    let json = if args.len() > 1 {
        fs::read_to_string(&args[1]).expect("Failed to read input JSON file")
    } else {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s).expect("Failed to read stdin");
        s
    };

    #[derive(serde::Deserialize)]
    struct Input {
        items: Vec<TextElement>,
        pageBounds: Bounds,
        borderedBoxes: Vec<Bounds>,
        strategy: Option<String>,
    }

    let parsed: Input = match serde_json::from_str(&json) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to parse JSON input: {}", e);
            std::process::exit(2);
        }
    };

    let strategy = parsed.strategy.unwrap_or_else(|| "combined".to_string());

    let result: XYCutResult = perform_auto_xycut(&parsed.items, parsed.pageBounds, &parsed.borderedBoxes, &strategy);

    match serde_json::to_string_pretty(&result) {
        Ok(out) => println!("{}", out),
        Err(e) => {
            eprintln!("Failed to serialize result: {}", e);
            std::process::exit(3);
        }
    }
}
