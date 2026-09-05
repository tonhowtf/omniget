//! OCR de imagens com o Tesseract do sistema (estudo 29, Text Extractor do
//! PowerToys). `brew install tesseract tesseract-lang`, `apt install
//! tesseract-ocr tesseract-ocr-por`, `winget install UB-Mannheim.TesseractOCR`.

use std::path::PathBuf;

use anyhow::anyhow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct OcrStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub languages: Vec<String>,
    pub install_hint: String,
}

pub async fn locate() -> Option<PathBuf> {
    if let Some(p) = crate::core::dependencies::find_tool("tesseract").await {
        return Some(p);
    }
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[r"C:\Program Files\Tesseract-OCR\tesseract.exe", r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe"]
    } else {
        &["/opt/homebrew/bin/tesseract", "/usr/local/bin/tesseract", "/usr/bin/tesseract"]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

pub async fn status() -> OcrStatus {
    let hint = if cfg!(target_os = "windows") {
        "winget install UB-Mannheim.TesseractOCR"
    } else if cfg!(target_os = "macos") {
        "brew install tesseract tesseract-lang"
    } else {
        "sudo apt install tesseract-ocr tesseract-ocr-por"
    }
    .to_string();
    let Some(bin) = locate().await else {
        return OcrStatus { installed: false, path: None, version: None, languages: vec![], install_hint: hint };
    };
    let version = crate::core::process::command(&bin)
        .arg("--version")
        .output()
        .await
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(|l| l.trim().to_string()));
    let languages = crate::core::process::command(&bin)
        .arg("--list-langs")
        .output()
        .await
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.contains(':') && l != "osd")
                .collect()
        })
        .unwrap_or_default();
    OcrStatus { installed: true, path: Some(bin.to_string_lossy().to_string()), version, languages, install_hint: hint }
}

#[derive(Debug, Clone, Serialize)]
pub struct OcrResult {
    pub path: String,
    pub text: String,
}

pub async fn run(inputs: &[String], langs: &str, progress: super::ProgressFn) -> anyhow::Result<Vec<OcrResult>> {
    let bin = locate().await.ok_or_else(|| anyhow!("tesseract nao esta instalado"))?;
    let langs = if langs.trim().is_empty() { "eng" } else { langs.trim() };
    let mut out = Vec::new();
    let total = inputs.len() as u64;
    for (i, input) in inputs.iter().enumerate() {
        super::report(&progress, "ocr", "progress", i as u64, Some(total), Some(input.clone()));
        let o = crate::core::process::command(&bin)
            .arg(input)
            .args(["stdout", "-l", langs, "--psm", "3"])
            .output()
            .await?;
        if !o.status.success() {
            return Err(anyhow!("tesseract falhou em {}: {}", input, String::from_utf8_lossy(&o.stderr).trim()));
        }
        out.push(OcrResult { path: input.clone(), text: String::from_utf8_lossy(&o.stdout).trim().to_string() });
    }
    super::report(&progress, "ocr", "done", total, Some(total), None);
    Ok(out)
}
