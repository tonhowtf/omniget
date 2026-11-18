use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const OLLAMA_URL: &str = "http://127.0.0.1:11434";
pub const LMSTUDIO_URL: &str = "http://127.0.0.1:1234";
pub const LLAMA_SERVER_URL: &str = "http://127.0.0.1:8080";

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

    pub fn default_url(self) -> &'static str {
        match self {
            LocalKind::Ollama => OLLAMA_URL,
            LocalKind::LmStudio => LMSTUDIO_URL,
            LocalKind::LlamaServer => LLAMA_SERVER_URL,
        }
    }

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

static SERVER: once_cell::sync::Lazy<Arc<Mutex<Option<tokio::process::Child>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));

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
