//! OCR model files: where they live, whether they are installed, and their
//! download.
//!
//! The models are not shipped in the installer. On first use the app asks
//! the user, then downloads them once into `<app data>/models/<set>/`; every
//! file is checked against its SHA-256 before it is used. After that the OCR
//! works offline. Each file has a list of mirrors tried in order: the
//! project's own GitHub release first, the official PaddlePaddle repository
//! on Hugging Face as fallback.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub struct ModelFile {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub urls: &'static [&'static str],
    pub role: &'static str,
}

/// A model set: detection, recognition and its character dictionary, in
/// this order. Each set has its own folder (the detection model is the same
/// file in both sets).
pub struct ModelSet {
    /// Engine id, also the folder name and the `engine` of OCR snapshots.
    pub id: &'static str,
    /// Language code used by the UI and the IPC (`latin`, `ja`).
    pub lang: &'static str,
    /// Shown in the download confirmation.
    pub label: &'static str,
    pub files: [ModelFile; 3],
}

/// `RELEASE` + file name, as a `&'static str`.
macro_rules! const_format_url {
    ($name:literal) => {
        concat!("https://github.com/DrLoki/PDF-X-Ray/releases/download/models-v1/", $name)
    };
}
#[cfg(test)]
const RELEASE: &str = "https://github.com/DrLoki/PDF-X-Ray/releases/download/models-v1/";

const DET: ModelFile = ModelFile {
    name: "PP-OCRv5_mobile_det.onnx",
    sha256: "a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d",
    size: 4_826_518,
    urls: &[
        const_format_url!("PP-OCRv5_mobile_det.onnx"),
        "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/main/inference.onnx",
    ],
    role: "rilevamento del testo",
};

/// Latin script (accented letters included).
pub const LATIN: ModelSet = ModelSet {
    id: "paddleocr-v5-latin",
    lang: "latin",
    label: "alfabeto latino",
    files: [
        DET,
        ModelFile {
            name: "latin_PP-OCRv5_mobile_rec.onnx",
            sha256: "7888113072263cb471b93f66dd5e2ad70548dc526fa1ace760d0d973dd121498",
            size: 8_042_023,
            urls: &[
                const_format_url!("latin_PP-OCRv5_mobile_rec.onnx"),
                "https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx/resolve/main/inference.onnx",
            ],
            role: "riconoscimento (alfabeto latino)",
        },
        ModelFile {
            name: "latin_PP-OCRv5_mobile_rec.yml",
            sha256: "0bbe984570f597af3638e50bdf2e8276f3ab26a61966096538b3b0d1849f5c84",
            size: 6_817,
            urls: &[
                const_format_url!("latin_PP-OCRv5_mobile_rec.yml"),
                "https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx/resolve/main/inference.yml",
            ],
            role: "dizionario dei caratteri",
        },
    ],
};

/// Japanese (kana, kanji, vertical text), with Chinese and English: the
/// PP-OCRv5 main recognition model, 18 383 characters.
pub const JAPANESE: ModelSet = ModelSet {
    id: "paddleocr-v5-cjk",
    lang: "ja",
    label: "giapponese (anche cinese e inglese)",
    files: [
        DET,
        ModelFile {
            name: "PP-OCRv5_mobile_rec.onnx",
            sha256: "da72dc72ca4dc220df0dfde68c1dedc31c58d3e76a25871122e5056227d50092",
            size: 16_534_782,
            urls: &[
                const_format_url!("PP-OCRv5_mobile_rec.onnx"),
                "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_rec_onnx/resolve/main/inference.onnx",
            ],
            role: "riconoscimento (giapponese, cinese, inglese)",
        },
        ModelFile {
            name: "PP-OCRv5_mobile_rec.yml",
            sha256: "5dfeb2777f6d0db8177d8128a8acfcf6e6276dc4ac73ea3bf0dc06d6a5e85d8e",
            size: 148_345,
            urls: &[
                const_format_url!("PP-OCRv5_mobile_rec.yml"),
                "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_rec_onnx/resolve/main/inference.yml",
            ],
            role: "dizionario dei caratteri",
        },
    ],
};

pub const SETS: [&ModelSet; 2] = [&LATIN, &JAPANESE];

/// The set for a UI language code (Latin for anything unknown).
pub fn set_for_lang(lang: &str) -> &'static ModelSet {
    SETS.iter().copied().find(|s| s.lang == lang).unwrap_or(&LATIN)
}

/// The set that produced an OCR result (`OcrPage::engine`).
pub fn set_for_engine(id: &str) -> Option<&'static ModelSet> {
    SETS.iter().copied().find(|s| s.id == id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStatus {
    pub name: String,
    pub role: String,
    pub size: u64,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsStatus {
    pub set: String,
    pub label: String,
    pub dir: String,
    pub installed: bool,
    /// Bytes still to download.
    pub missing_bytes: u64,
    pub files: Vec<FileStatus>,
    /// Hosts the files are downloaded from (shown in the confirmation).
    pub sources: Vec<String>,
}

/// Model folder for a data directory: `<data>/models/<set id>`.
pub fn set_dir(data_dir: &Path, set: &ModelSet) -> PathBuf {
    data_dir.join("models").join(set.id)
}

/// `$PDFXRAY_MODEL_DIR` for a set: its `<set id>` subfolder when present,
/// otherwise the folder itself (a folder holding one set's files).
pub fn env_model_dir(set: &ModelSet) -> Option<PathBuf> {
    let d = std::env::var("PDFXRAY_MODEL_DIR").ok().filter(|d| !d.trim().is_empty())?;
    let base = PathBuf::from(d);
    let sub = base.join(set.id);
    Some(if sub.is_dir() { sub } else { base })
}

/// Model folder of a set without Tauri (for the CLI and the tests):
/// `$PDFXRAY_MODEL_DIR` (see [`env_model_dir`]); otherwise the same folder
/// Tauri uses for the identifier `com.drloki.pdfxray`.
pub fn default_model_dir(set: &ModelSet) -> Option<PathBuf> {
    if let Some(d) = env_model_dir(set) {
        return Some(d);
    }
    let id = "com.drloki.pdfxray";
    let data = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
    }?;
    Some(set_dir(&data.join(id), set))
}

fn present(dir: &Path, f: &ModelFile) -> bool {
    fs::metadata(dir.join(f.name)).map(|m| m.len() == f.size).unwrap_or(false)
}

pub fn status(dir: &Path, set: &ModelSet) -> ModelsStatus {
    let files: Vec<FileStatus> = set
        .files
        .iter()
        .map(|f| FileStatus { name: f.name.into(), role: f.role.into(), size: f.size, present: present(dir, f) })
        .collect();
    let missing_bytes = files.iter().filter(|f| !f.present).map(|f| f.size).sum();
    let mut sources: Vec<String> = Vec::new();
    for f in &set.files {
        for u in f.urls {
            let host = u.split('/').nth(2).unwrap_or(u).to_string();
            if !sources.contains(&host) {
                sources.push(host);
            }
        }
    }
    ModelsStatus {
        set: set.id.into(),
        label: set.label.into(),
        dir: dir.to_string_lossy().into_owned(),
        installed: files.iter().all(|f| f.present),
        missing_bytes,
        files,
        sources,
    }
}

pub fn det_path(dir: &Path, set: &ModelSet) -> PathBuf {
    dir.join(set.files[0].name)
}
pub fn rec_path(dir: &Path, set: &ModelSet) -> PathBuf {
    dir.join(set.files[1].name)
}
pub fn dict_path(dir: &Path, set: &ModelSet) -> PathBuf {
    dir.join(set.files[2].name)
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Check the installed files against their SHA-256 (slow-ish: reads them).
pub fn verify(dir: &Path, set: &ModelSet) -> Result<(), String> {
    for f in &set.files {
        let p = dir.join(f.name);
        let got = sha256_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if got != f.sha256 {
            return Err(format!("{}: checksum non valido", f.name));
        }
    }
    Ok(())
}

/// Download progress: (file name, bytes of this file, total bytes of all
/// missing files, bytes done overall).
pub type Progress<'a> = dyn FnMut(&str, u64, u64) + 'a;

/// Download the missing files. Each file goes to `<name>.part`, is checked
/// against its SHA-256 and only then renamed; a failed mirror falls back to
/// the next one.
pub fn download(dir: &Path, set: &ModelSet, progress: &mut Progress) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let total: u64 = set.files.iter().filter(|f| !present(dir, f)).map(|f| f.size).sum();
    let mut done = 0u64;
    for f in set.files.iter().filter(|f| !present(dir, f)) {
        let mut errors = Vec::new();
        let mut ok = false;
        for url in f.urls {
            match fetch(url, dir, f, &mut |n| progress(f.name, total, done + n)) {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(e) => errors.push(format!("{url}: {e}")),
            }
        }
        if !ok {
            return Err(format!("download di {} fallito:\n{}", f.name, errors.join("\n")));
        }
        done += f.size;
        progress(f.name, total, done);
    }
    Ok(())
}

fn fetch(url: &str, dir: &Path, f: &ModelFile, progress: &mut dyn FnMut(u64)) -> Result<(), String> {
    let part = dir.join(format!("{}.part", f.name));
    let res = (|| {
        let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
        let mut reader = resp.into_body().into_reader();
        let mut out = fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut n = 0u64;
        loop {
            let k = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if k == 0 {
                break;
            }
            n += k as u64;
            if n > f.size {
                return Err("file più grande del previsto".to_string());
            }
            h.update(&buf[..k]);
            out.write_all(&buf[..k]).map_err(|e| e.to_string())?;
            progress(n);
        }
        out.flush().map_err(|e| e.to_string())?;
        drop(out);
        let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != f.sha256 {
            return Err("checksum non valido".to_string());
        }
        fs::rename(&part, dir.join(f.name)).map_err(|e| e.to_string())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&part);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_start_with_the_project_release() {
        for set in SETS {
            for f in &set.files {
                assert!(f.urls[0].starts_with(RELEASE), "{}", f.urls[0]);
                assert!(f.urls[0].ends_with(f.name));
            }
        }
    }

    #[test]
    fn status_of_an_empty_folder() {
        for set in SETS {
            let s = status(Path::new("this/folder/does/not/exist"), set);
            assert!(!s.installed);
            assert_eq!(s.set, set.id);
            assert_eq!(s.missing_bytes, set.files.iter().map(|f| f.size).sum::<u64>());
            assert_eq!(s.sources, vec!["github.com".to_string(), "huggingface.co".to_string()]);
        }
    }

    #[test]
    fn sets_by_language_and_engine() {
        assert_eq!(set_for_lang("ja").id, "paddleocr-v5-cjk");
        assert_eq!(set_for_lang("xx").id, "paddleocr-v5-latin");
        assert_eq!(set_for_engine("paddleocr-v5-cjk").map(|s| s.lang), Some("ja"));
        // file names are unique within a set, the detection model is shared
        assert_eq!(LATIN.files[0].sha256, JAPANESE.files[0].sha256);
        assert_ne!(LATIN.files[1].name, JAPANESE.files[1].name);
    }
}
