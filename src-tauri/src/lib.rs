pub mod flow;
pub mod lines;
pub mod model;
pub mod order;
pub mod pipeline;
pub mod reflow;
pub mod roles;
pub mod text;
pub mod xycut;

use crate::model::{AnalysisOptions, DocumentResult, PageInput};
use crate::reflow::DistUnit;

/// Analyse a whole document (or any subset of pages, in order).
/// Runs on Tauri's async runtime so the UI thread is never blocked.
#[tauri::command]
async fn analyze_document(pages: Vec<PageInput>, options: Option<AnalysisOptions>) -> Result<DocumentResult, String> {
    let opts = options.unwrap_or_default();
    Ok(pipeline::analyze_document(&pages, &opts))
}

/// Split translated paragraphs across their segments (see `reflow`).
#[tauri::command]
fn distribute_translations(units: Vec<DistUnit>) -> Vec<Vec<String>> {
    units.iter().map(|u| reflow::distribute(&u.text, &u.weights)).collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![analyze_document, distribute_translations])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
