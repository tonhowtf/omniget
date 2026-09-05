//! Ollama (estudo 18): detectar o daemon em `127.0.0.1:11434`, listar,
//! baixar (stream NDJSON de `/api/pull`) e remover modelos.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

pub const DEFAULT_HOST: &str = "http://127.0.0.1:11434";

fn base(host: &str) -> String {
    let h = if host.trim().is_empty() { DEFAULT_HOST } else { host.trim() };
    h.trim_end_matches('/').to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelDetails {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub parameter_size: String,
    #[serde(default)]
    pub quantization_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub details: ModelDetails,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaStatus {
    pub running: bool,
    pub version: Option<String>,
    pub host: String,
    pub models: Vec<OllamaModel>,
    pub loaded: Vec<String>,
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommended {
    pub name: String,
    pub size_gb: f32,
    pub use_case: String,
}

pub fn recommended() -> Vec<Recommended> {
    vec![
        Recommended { name: "qwen3:8b".into(), size_gb: 5.2, use_case: "tradução e resumo, boa em português".into() },
        Recommended { name: "gemma3:12b".into(), size_gb: 8.1, use_case: "qualidade alta, precisa de 12 GB+ de RAM".into() },
        Recommended { name: "llama3.2:3b".into(), size_gb: 2.0, use_case: "máquinas fracas, rápido".into() },
        Recommended { name: "gemma3:4b".into(), size_gb: 3.3, use_case: "equilíbrio, aceita imagens".into() },
        Recommended { name: "qwen2.5-coder:7b".into(), size_gb: 4.7, use_case: "código".into() },
        Recommended { name: "nomic-embed-text".into(), size_gb: 0.3, use_case: "busca semântica (embeddings)".into() },
    ]
}

pub async fn status(host: &str) -> OllamaStatus {
    let b = base(host);
    let client = super::client().ok();
    let mut st = OllamaStatus {
        running: false,
        version: None,
        host: b.clone(),
        models: vec![],
        loaded: vec![],
        download_url: "https://ollama.com/download".into(),
    };
    let Some(client) = client else { return st };
    let short = client.get(format!("{}/api/version", b)).timeout(std::time::Duration::from_secs(3)).send().await;
    if let Ok(r) = short {
        if r.status().is_success() {
            st.running = true;
            st.version = r.json::<serde_json::Value>().await.ok().and_then(|v| v["version"].as_str().map(|s| s.to_string()));
        }
    }
    if !st.running {
        return st;
    }
    if let Ok(r) = client.get(format!("{}/api/tags", b)).send().await {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(arr) = v["models"].as_array() {
                st.models = arr.iter().filter_map(|m| serde_json::from_value(m.clone()).ok()).collect();
            }
        }
    }
    if let Ok(r) = client.get(format!("{}/api/ps", b)).send().await {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(arr) = v["models"].as_array() {
                st.loaded = arr.iter().filter_map(|m| m["name"].as_str().map(|s| s.to_string())).collect();
            }
        }
    }
    st
}

pub async fn pull(host: &str, name: &str, progress: super::ProgressFn) -> anyhow::Result<()> {
    use futures::StreamExt;
    let b = base(host);
    let client = super::client()?;
    let id = format!("ollama-pull:{}", name);
    let resp = client
        .post(format!("{}/api/pull", b))
        .json(&serde_json::json!({ "model": name, "stream": true }))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("ollama pull: HTTP {}", resp.status()));
    }
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim().to_string();
            buf.drain(..=pos);
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(err) = v["error"].as_str() {
                return Err(anyhow!("ollama: {}", err));
            }
            let status = v["status"].as_str().unwrap_or("").to_string();
            let done = v["completed"].as_u64().unwrap_or(0);
            let total = v["total"].as_u64();
            let stage = if status == "success" { "done" } else { "progress" };
            super::report(&progress, &id, stage, done, total, Some(status));
        }
    }
    Ok(())
}

pub async fn delete(host: &str, name: &str) -> anyhow::Result<()> {
    let client = super::client()?;
    let resp = client
        .delete(format!("{}/api/delete", base(host)))
        .json(&serde_json::json!({ "model": name }))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("ollama delete: HTTP {}", resp.status()));
    }
    Ok(())
}
