//! Seção Tools: cada módulo aqui é uma capacidade isolada com entrada e saída
//! serializáveis, para a UI Svelte e (mais tarde) um servidor MCP chamarem o
//! mesmo código. Nada de estado de janela; progresso sai por callback.

pub mod ai_keys;
pub mod aria2;
pub mod autoclick;
pub mod calameo;
pub mod dictation;
pub mod disk;
pub mod dub;
pub mod dupes;
pub mod edge_tts;
pub mod file_search;
pub mod gallery;
pub mod gdocs;
pub mod humanize;
pub mod github;
pub mod image_resize;
pub mod instagram;
pub mod jpeg_pdf;
pub mod kdeconnect;
pub mod manifest_dl;
pub mod ocr;
pub mod ollama;
pub mod pdf;
pub mod pinterest;
pub mod pricing;
pub mod rename;
pub mod ryd;
pub mod screen_record;
pub mod slides;
pub mod sponsorblock;
pub mod srt_translate;
pub mod startup;
pub mod sysclean;
pub mod uninstall;
pub mod upscale;
pub mod usage;
pub mod voicestudio;
pub mod whisper;
pub mod win_apps;
pub mod win_registry;
pub mod win_tweaks;
pub mod win_updater;
pub mod x;

use std::sync::Arc;

use serde::Serialize;

/// Progresso genérico de uma ferramenta. `id` identifica a operação na UI
/// (a mesma tela pode ter vários downloads de modelo ao mesmo tempo).
#[derive(Debug, Clone, Serialize)]
pub struct ToolProgress {
    pub id: String,
    /// "started" | "progress" | "done" | "error" | texto livre da etapa
    pub stage: String,
    pub done: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
}

pub type ProgressFn = Arc<dyn Fn(ToolProgress) + Send + Sync>;

pub fn noop_progress() -> ProgressFn {
    Arc::new(|_| {})
}

pub fn report(p: &ProgressFn, id: &str, stage: &str, done: u64, total: Option<u64>, message: Option<String>) {
    p(ToolProgress {
        id: id.to_string(),
        stage: stage.to_string(),
        done,
        total,
        message,
    });
}

/// Pasta de dados das ferramentas (`<app_data>/tools`).
pub fn tools_dir() -> Option<std::path::PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("tools"))
}

/// Pasta temporária própria, para não deixar lixo no `/tmp` do sistema.
pub fn temp_dir() -> std::path::PathBuf {
    let base = tools_dir().unwrap_or_else(std::env::temp_dir).join("tmp");
    let _ = std::fs::create_dir_all(&base);
    base
}

pub fn client() -> anyhow::Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        ),
    );
    Ok(crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(600))
        .build()?)
}

/// Baixa uma URL para um arquivo, em streaming, reportando bytes.
pub async fn download_to(
    client: &reqwest::Client,
    url: &str,
    dest: &std::path::Path,
    progress: &ProgressFn,
    id: &str,
) -> anyhow::Result<u64> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("download de {} falhou: HTTP {}", url, resp.status());
    }
    let total = resp.content_length();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await?;
    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    let mut last = std::time::Instant::now();
    report(progress, id, "started", 0, total, None);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if last.elapsed() > std::time::Duration::from_millis(200) {
            report(progress, id, "progress", done, total, None);
            last = std::time::Instant::now();
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, dest).await?;
    report(progress, id, "done", done, total, None);
    Ok(done)
}

pub fn sanitize_name(name: &str) -> String {
    let s = sanitize_filename::sanitize(name.trim());
    if s.is_empty() {
        "arquivo".to_string()
    } else {
        s
    }
}
