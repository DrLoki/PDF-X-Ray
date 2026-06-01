pub mod gutter_detection;

use crate::gutter_detection::{TextElement, Bounds, perform_auto_xycut};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn analyze_layout_auto_gutter(
    items: Vec<TextElement>,
    page_bounds: Bounds,
    bordered_boxes: Vec<Bounds>,
    strategy: String,
) -> Result<String, String> {
    let result = perform_auto_xycut(&items, page_bounds, &bordered_boxes, &strategy);
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, analyze_layout_auto_gutter])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

