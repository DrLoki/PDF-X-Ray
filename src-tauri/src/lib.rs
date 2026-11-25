pub mod comic;
pub mod eval;
pub mod flow;
pub mod lines;
pub mod model;
pub mod ocr;
pub mod order;
pub mod pipeline;
pub mod reflow;
pub mod roles;
pub mod text;
pub mod xycut;

use crate::model::{AnalysisOptions, DocumentResult, PageInput, TextElement};
use crate::ocr::classify::{classify_page, PageKind};
use crate::ocr::models::ModelsStatus;
use crate::ocr::paddle::PaddleOcr;
use crate::ocr::{OcrEngine, OcrPage};
use crate::reflow::DistUnit;
use base64::Engine;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

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

fn decode_data_url(b64: &str) -> Result<Vec<u8>, String> {
    let data = b64.split_once(',').map(|(_, d)| d).unwrap_or(b64);
    base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())
}

/// Save a settings/layout snapshot (JSON), an optional PNG of the view and,
/// for OCR pages, the image given to the OCR (`<stem>_ocr.png`, referenced
/// by the JSON). Returns the path of the JSON file.
#[tauri::command]
fn save_snapshot(app: tauri::AppHandle, file_stem: String, json: String, png_base64: Option<String>, ocr_png_base64: Option<String>) -> Result<String, String> {
    let dir = snapshot_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    let stem: String = file_stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let json_path = dir.join(format!("{}.json", stem));
    std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
    if let Some(b64) = png_base64 {
        std::fs::write(dir.join(format!("{}.png", stem)), decode_data_url(&b64)?).map_err(|e| e.to_string())?;
    }
    if let Some(b64) = ocr_png_base64 {
        std::fs::write(dir.join(format!("{}_ocr.png", stem)), decode_data_url(&b64)?).map_err(|e| e.to_string())?;
    }
    Ok(json_path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// OCR
// ---------------------------------------------------------------------------

/// The OCR engine, loaded on first use and kept for the session.
#[derive(Default)]
struct OcrState {
    engine: Mutex<Option<Arc<PaddleOcr>>>,
}

/// `$PDFXRAY_MODEL_DIR`, or `<app data>/models/<set>`.
fn model_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("PDFXRAY_MODEL_DIR") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(ocr::models::set_dir(&data))
}

/// Which pages need OCR (see `ocr::classify`).
#[tauri::command]
fn classify_pages(pages: Vec<PageInput>) -> Vec<PageKind> {
    pages.iter().map(classify_page).collect()
}

/// Whether the OCR models are installed, and what a download would fetch.
#[tauri::command]
fn ocr_models_status(app: tauri::AppHandle) -> Result<ModelsStatus, String> {
    Ok(ocr::models::status(&model_dir(&app)?))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    file: String,
    total: u64,
    done: u64,
}

/// Download the missing model files (the UI asks the user first). Emits
/// `ocr-download-progress` events.
#[tauri::command]
async fn ocr_download_models(app: tauri::AppHandle) -> Result<ModelsStatus, String> {
    let dir = model_dir(&app)?;
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        ocr::models::download(&dir, &mut |file, total, done| {
            // at most ~10 events per second, plus the end of each file
            if last.elapsed().as_millis() >= 100 || done == total {
                last = std::time::Instant::now();
                let _ = handle.emit("ocr-download-progress", DownloadProgress { file: file.into(), total, done });
            }
        })?;
        Ok(ocr::models::status(&dir))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrPageResult {
    /// Lines in image pixels.
    page: OcrPage,
    /// The same lines as text runs in PDF points, ready for `analyze_document`.
    items: Vec<TextElement>,
    /// Image pixels per PDF point.
    scale: f32,
    /// Panels and text containers (PDF points); `isComic` is the verdict
    /// used by the automatic page mode.
    comic: comic::ComicLayout,
    comic_ms: u64,
}

fn header<T: std::str::FromStr>(req: &tauri::ipc::Request<'_>, name: &str) -> Result<T, String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("header {name} mancante o non valido"))
}

/// Recognise one rasterised page. Body: raw RGBA pixels; headers `x-width`,
/// `x-height` (pixels) and `x-scale` (pixels per PDF point).
#[tauri::command]
async fn ocr_page(app: tauri::AppHandle, state: tauri::State<'_, OcrState>, request: tauri::ipc::Request<'_>) -> Result<OcrPageResult, String> {
    let w: u32 = header(&request, "x-width")?;
    let h: u32 = header(&request, "x-height")?;
    let scale: f32 = header(&request, "x-scale")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("corpo della richiesta: attesi i pixel RGBA".into());
    };
    if bytes.len() != w as usize * h as usize * 4 || w == 0 || h == 0 || !(scale > 0.0) {
        return Err(format!("immagine non valida: {} byte per {w}×{h}", bytes.len()));
    }
    let rgba = bytes.clone();
    let engine = {
        let mut slot = state.engine.lock().map_err(|e| e.to_string())?;
        match slot.as_ref() {
            Some(e) => e.clone(),
            None => {
                let e = Arc::new(ocr::load_engine(&model_dir(&app)?)?);
                *slot = Some(e.clone());
                e
            }
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut rgb = image::RgbImage::new(w, h);
        for (i, p) in rgb.pixels_mut().enumerate() {
            // composite on white: transparent areas of the canvas are paper
            let a = rgba[i * 4 + 3] as u32;
            for c in 0..3 {
                p[c] = ((rgba[i * 4 + c] as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
        }
        let page = engine.recognize(&rgb)?;
        let items = ocr::to_text_elements(&page, scale);
        let t = std::time::Instant::now();
        let comic = comic::analyze(&rgb, &page.lines).scaled(1.0 / scale);
        let comic_ms = t.elapsed().as_millis() as u64;
        Ok(OcrPageResult { page, items, scale, comic, comic_ms })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(OcrState::default())
        .invoke_handler(tauri::generate_handler![
            analyze_document,
            distribute_translations,
            save_snapshot,
            classify_pages,
            ocr_models_status,
            ocr_download_models,
            ocr_page
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
