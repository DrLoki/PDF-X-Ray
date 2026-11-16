pub mod eval;
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
use base64::Engine;
use std::path::PathBuf;
use tauri::Manager;

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

/// Folder where snapshots are written: `$PDFXRAY_SNAPSHOT_DIR`, or
/// `Documents/PDF-X-Ray/snapshots`.
fn snapshot_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("PDFXRAY_SNAPSHOT_DIR") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let docs = app.path().document_dir().map_err(|e| e.to_string())?;
    Ok(docs.join("PDF-X-Ray").join("snapshots"))
}

/// Save a settings/layout snapshot (JSON) and an optional PNG of the view.
/// Returns the path of the JSON file.
#[tauri::command]
fn save_snapshot(app: tauri::AppHandle, file_stem: String, json: String, png_base64: Option<String>) -> Result<String, String> {
    let dir = snapshot_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    let stem: String = file_stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let json_path = dir.join(format!("{}.json", stem));
    std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
    if let Some(b64) = png_base64 {
        let data = b64.split_once(',').map(|(_, d)| d).unwrap_or(&b64);
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{}.png", stem)), bytes).map_err(|e| e.to_string())?;
    }
    Ok(json_path.to_string_lossy().into_owned())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![analyze_document, distribute_translations, save_snapshot])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
