//! Local inference servers: detect what is already running on the machine
//! (Ollama, LM Studio, llama-server) and manage a `llama-server` of our own,
//! in the mould of `core/tools/whisper.rs` (pinned release + GGUF catalogue +
//! managed binary). Owned by f2-llm-commands.
//!
//! Nothing here runs on its own: every probe, download and spawn happens on an
//! explicit user action, and the managed `llama-server` dies with the app.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::core::dependencies::bin_name;
use crate::core::tools::ProgressFn;

pub const OLLAMA_URL: &str = "http://127.0.0.1:11434";
pub const LMSTUDIO_URL: &str = "http://127.0.0.1:1234";
pub const LLAMA_SERVER_URL: &str = "http://127.0.0.1:8080";
pub const LLAMA_SERVER_PORT: u16 = 8080;

const LLAMA_REPO: &str = "ggml-org/llama.cpp";
const PROBE_TIMEOUT_MS: u64 = 1200;

pub const ERR_LOCAL_BINARY: &str = "ERR_LLM_LOCAL_BINARY";
pub const ERR_LOCAL_MODEL: &str = "ERR_LLM_LOCAL_MODEL";
pub const ERR_LOCAL_SPAWN: &str = "ERR_LLM_LOCAL_SPAWN";
pub const ERR_LOCAL_HASH: &str = "ERR_LLM_LOCAL_HASH";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalKind {
    Ollama,
    LmStudio,
    LlamaServer,
}

impl LocalKind {
    pub fn id(self) -> &'static str {
        match self {
            LocalKind::Ollama => "ollama",
            LocalKind::LmStudio => "lmstudio",
            LocalKind::LlamaServer => "llama-server",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LocalKind::Ollama => "Ollama",
            LocalKind::LmStudio => "LM Studio",
            LocalKind::LlamaServer => "llama-server",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            LocalKind::Ollama => OLLAMA_URL,
            LocalKind::LmStudio => LMSTUDIO_URL,
            LocalKind::LlamaServer => LLAMA_SERVER_URL,
        }
    }

    /// Every one of them speaks the OpenAI-compatible surface at `/v1`.
    pub fn openai_base(self, host: &str) -> String {
        format!("{}/v1", normalize_base(host, self.default_url()))
    }

    pub fn all() -> [LocalKind; 3] {
        [
            LocalKind::Ollama,
            LocalKind::LmStudio,
            LocalKind::LlamaServer,
        ]
    }
}

/// Trims a host down to `scheme://host:port`, falling back to the default.
pub fn normalize_base(host: &str, default: &str) -> String {
    let h = host.trim().trim_end_matches('/');
    if h.is_empty() {
        return default.trim_end_matches('/').to_string();
    }
    if h.starts_with("http://") || h.starts_with("https://") {
        h.to_string()
    } else {
        format!("http://{h}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalServer {
    pub kind: LocalKind,
    pub label: String,
    pub base_url: String,
    pub running: bool,
    #[serde(default)]
    pub models: Vec<String>,
    pub version: Option<String>,
    /// Where to get it when it is not running.
    pub download_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlamaBinary {
    pub installed: bool,
    pub path: Option<String>,
    /// "custom" | "managed" | "path"
    pub source: Option<String>,
    pub managed_running: bool,
    pub can_install: bool,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalStatus {
    pub servers: Vec<LocalServer>,
    pub llama: LlamaBinary,
    pub models_dir: Option<String>,
    pub models: Vec<GgufModel>,
}

// ── GGUF catalogue ────────────────────────────────────────────────────

/// A small pinned catalogue. `sha256` is `None` where the hash has not been
/// measured against the real file yet: the download then records the computed
/// hash next to the model and warns, instead of pretending it verified one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GgufModel {
    pub id: String,
    pub label: String,
    pub url: String,
    pub file: String,
    pub size_mb: u64,
    pub note: String,
    pub sha256: Option<String>,
    pub installed: bool,
    pub path: Option<String>,
    pub size_bytes: u64,
}

/// (id, label, repo path on Hugging Face, file, size MB, note, sha256)
type CatalogRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u64,
    &'static str,
    Option<&'static str>,
);

const CATALOG: &[CatalogRow] = &[
    (
        "qwen2.5-1.5b-instruct-q4",
        "Qwen2.5 1.5B Instruct (Q4_K_M)",
        "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        1120,
        "cabe em qualquer maquina; respostas curtas",
        None,
    ),
    (
        "qwen2.5-7b-instruct-q4",
        "Qwen2.5 7B Instruct (Q4_K_M)",
        "Qwen/Qwen2.5-7B-Instruct-GGUF",
        "qwen2.5-7b-instruct-q4_k_m.gguf",
        4680,
        "equilibrio; precisa de ~6 GB livres",
        None,
    ),
    (
        "llama-3.2-3b-instruct-q4",
        "Llama 3.2 3B Instruct (Q4_K_M)",
        "bartowski/Llama-3.2-3B-Instruct-GGUF",
        "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        2020,
        "rapido, bom em ingles",
        None,
    ),
];

pub fn models_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("models").join("llm"))
}

fn model_url(repo: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/main/{file}?download=true")
}

pub fn catalog() -> Vec<GgufModel> {
    let dir = models_dir();
    CATALOG
        .iter()
        .map(|(id, label, repo, file, size_mb, note, sha)| {
            let path = dir.as_ref().map(|d| d.join(file));
            let size = path
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);
            GgufModel {
                id: (*id).to_string(),
                label: (*label).to_string(),
                url: model_url(repo, file),
                file: (*file).to_string(),
                size_mb: *size_mb,
                note: (*note).to_string(),
                sha256: sha.map(str::to_string),
                installed: size > 0,
                path: path
                    .filter(|_| size > 0)
                    .map(|p| p.to_string_lossy().to_string()),
                size_bytes: size,
            }
        })
        .collect()
}

// ── Detection ─────────────────────────────────────────────────────────

/// Model ids out of an OpenAI `/v1/models` body. Pure; no network.
pub fn parse_openai_models(body: &serde_json::Value) -> Vec<String> {
    body.get("data")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Model names out of Ollama's `/api/tags` body. Pure; no network.
pub fn parse_ollama_tags(body: &serde_json::Value) -> Vec<String> {
    body.get("models")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m.get("name").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

async fn probe(client: &reqwest::Client, kind: LocalKind, host: &str) -> LocalServer {
    let base = normalize_base(host, kind.default_url());
    let mut server = LocalServer {
        kind,
        label: kind.label().to_string(),
        base_url: base.clone(),
        running: false,
        models: Vec::new(),
        version: None,
        download_url: match kind {
            LocalKind::Ollama => "https://ollama.com/download",
            LocalKind::LmStudio => "https://lmstudio.ai",
            LocalKind::LlamaServer => "https://github.com/ggml-org/llama.cpp/releases",
        }
        .to_string(),
    };
    let url = match kind {
        LocalKind::Ollama => format!("{base}/api/tags"),
        _ => format!("{base}/v1/models"),
    };
    let response = client
        .get(&url)
        .timeout(std::time::Duration::from_millis(PROBE_TIMEOUT_MS))
        .send()
        .await;
    let Ok(response) = response else {
        return server;
    };
    if !response.status().is_success() {
        return server;
    }
    server.running = true;
    if let Ok(body) = response.json::<serde_json::Value>().await {
        server.models = match kind {
            LocalKind::Ollama => parse_ollama_tags(&body),
            _ => parse_openai_models(&body),
        };
    }
    server
}

/// Probes the three servers in parallel. Only ever called from a command.
pub async fn detect(hosts: Option<&[(LocalKind, String)]>) -> LocalStatus {
    let client = crate::core::tools::client().ok();
    let mut servers = Vec::new();
    if let Some(client) = client {
        let host_for = |kind: LocalKind| -> String {
            hosts
                .and_then(|h| h.iter().find(|(k, _)| *k == kind))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| kind.default_url().to_string())
        };
        let futures = LocalKind::all()
            .into_iter()
            .map(|kind| {
                let host = host_for(kind);
                let client = client.clone();
                async move { probe(&client, kind, &host).await }
            })
            .collect::<Vec<_>>();
        servers = futures::future::join_all(futures).await;
    }
    LocalStatus {
        servers,
        llama: llama_binary().await,
        models_dir: models_dir().map(|d| d.to_string_lossy().to_string()),
        models: catalog(),
    }
}

// ── Managed llama-server ──────────────────────────────────────────────

fn managed_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("bin").join("llama-cpp"))
}

/// Release asset variants this system can install, most wanted first.
pub fn variants() -> Vec<&'static str> {
    if cfg!(target_os = "windows") {
        vec!["cpu", "vulkan", "cuda"]
    } else if cfg!(target_os = "macos") {
        vec!["metal"]
    } else {
        vec!["cpu", "vulkan"]
    }
}

/// Does this asset name belong to `variant` on this system? Pure, so it is
/// testable without hitting the GitHub API.
pub fn asset_matches(name: &str, variant: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if !n.starts_with("llama-") || !(n.ends_with(".zip") || n.ends_with(".tar.gz")) {
        return false;
    }
    let os_ok = if cfg!(target_os = "windows") {
        n.contains("win")
    } else if cfg!(target_os = "macos") {
        n.contains("macos")
    } else {
        n.contains("ubuntu") || n.contains("linux")
    };
    if !os_ok {
        return false;
    }
    let arch_ok = if cfg!(target_arch = "aarch64") {
        n.contains("arm64") || n.contains("aarch64")
    } else {
        n.contains("x64") || n.contains("x86_64") || n.contains("amd64")
    };
    if !arch_ok {
        return false;
    }
    match variant {
        "cpu" => !n.contains("cuda") && !n.contains("vulkan") && !n.contains("hip"),
        other => n.contains(other),
    }
}

/// Where `llama-server` lives, and how we found it.
pub fn locate() -> Option<(PathBuf, &'static str)> {
    if let Some(dir) = managed_dir() {
        let p = dir.join(bin_name("llama-server"));
        if p.exists() {
            return Some((p, "managed"));
        }
    }
    which_in_path(&bin_name("llama-server")).map(|p| (p, "path"))
}

fn which_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

pub async fn llama_binary() -> LlamaBinary {
    let found = locate();
    LlamaBinary {
        installed: found.is_some(),
        path: found.as_ref().map(|(p, _)| p.to_string_lossy().to_string()),
        source: found.as_ref().map(|(_, s)| (*s).to_string()),
        managed_running: SERVER.lock().await.is_some(),
        can_install: true,
        variants: variants().into_iter().map(String::from).collect(),
    }
}

/// Downloads and unpacks the `llama-server` release asset for `variant`.
pub async fn install(variant: &str, progress: ProgressFn) -> Result<PathBuf, String> {
    if !variants().contains(&variant) {
        return Err(format!(
            "{ERR_LOCAL_BINARY}: unknown variant {variant} for this system"
        ));
    }
    let dir = managed_dir().ok_or_else(|| format!("{ERR_LOCAL_BINARY}: no data directory"))?;
    let client =
        crate::core::tools::github::client().map_err(|e| format!("{ERR_LOCAL_BINARY}: {e}"))?;
    let variant_owned = variant.to_string();
    let asset = crate::core::tools::github::asset(&client, LLAMA_REPO, None, move |name| {
        asset_matches(name, &variant_owned)
    })
    .await
    .map_err(|e| format!("{ERR_LOCAL_BINARY}: {e}"))?;
    let bytes =
        crate::core::tools::github::download(&client, &asset, false, &progress, "llama-cpp")
            .await
            .map_err(|e| format!("{ERR_LOCAL_BINARY}: {e}"))?;
    let staging = dir.with_extension("staging");
    let _ = std::fs::remove_dir_all(&staging);
    crate::core::tools::github::unpack(&bytes, &asset.name, &staging)
        .map_err(|e| format!("{ERR_LOCAL_BINARY}: {e}"))?;
    let exe = crate::core::tools::github::find_file(&staging, &bin_name("llama-server"))
        .ok_or_else(|| format!("{ERR_LOCAL_BINARY}: llama-server not in {}", asset.name))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{ERR_LOCAL_BINARY}: {e}"))?;
    // Keep the whole unpacked tree: llama-server needs its shared libraries.
    let root = exe.parent().unwrap_or(&staging).to_path_buf();
    crate::core::tools::github::swap_dir(&root, &dir)
        .map_err(|e| format!("{ERR_LOCAL_BINARY}: {e}"))?;
    let _ = std::fs::remove_dir_all(&staging);
    let final_exe = dir.join(bin_name("llama-server"));
    crate::core::tools::github::make_executable(&final_exe);
    crate::core::tools::github::strip_quarantine(&dir).await;
    Ok(final_exe)
}

/// The managed child, if one is alive. `Mutex` because start/stop are commands.
static SERVER: once_cell::sync::Lazy<Arc<Mutex<Option<tokio::process::Child>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));

/// Command line the managed server runs with. Pure, for the test.
pub fn server_args(model: &Path, port: u16, ctx: u32) -> Vec<String> {
    vec![
        "-m".into(),
        model.to_string_lossy().to_string(),
        "--port".into(),
        port.to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "-c".into(),
        ctx.to_string(),
    ]
}

/// Starts the managed `llama-server` on a model file. Idempotent: a server
/// already running is left alone.
pub async fn start(model: &Path, port: u16) -> Result<String, String> {
    let mut guard = SERVER.lock().await;
    if guard.is_some() {
        return Ok(format!("http://127.0.0.1:{port}"));
    }
    if !model.exists() {
        return Err(format!("{ERR_LOCAL_MODEL}: {} not found", model.display()));
    }
    let (exe, _) =
        locate().ok_or_else(|| format!("{ERR_LOCAL_BINARY}: llama-server not installed"))?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(server_args(model, port, 8192));
    cmd.kill_on_drop(true);
    // tokio's `Command` has `creation_flags` itself on Windows.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let child = cmd.spawn().map_err(|e| format!("{ERR_LOCAL_SPAWN}: {e}"))?;
    *guard = Some(child);
    Ok(format!("http://127.0.0.1:{port}"))
}

/// Kills the managed server. Called by the command and on app shutdown.
pub async fn stop() -> bool {
    let mut guard = SERVER.lock().await;
    match guard.take() {
        Some(mut child) => {
            let _ = child.kill().await;
            true
        }
        None => false,
    }
}

pub async fn managed_running() -> bool {
    SERVER.lock().await.is_some()
}
