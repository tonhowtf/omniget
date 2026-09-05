//! Grok (estudo 67). Dois caminhos:
//! 1. API oficial da xAI (`/v1/responses`) com a chave do usuario e as
//!    tools `x_search` / `web_search` — o que grok-mcp e grok-cli fazem.
//! 2. O Grok dentro do X pela sessao: `CreateGrokConversation` +
//!    `add_response.json` em NDJSON (GrokAiChat / Grok-Wrapper, MIT).
//!
//! grok.com ficou de fora: exige `x-statsig-id` de um assinador externo.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const CONFIG_FILE: &str = "grok.json";
pub const DEFAULT_XAI_MODEL: &str = "grok-4.6";
pub const DEFAULT_X_MODEL: &str = "grok-3";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrokConfig {
    #[serde(default)]
    pub xai_key: String,
    #[serde(default)]
    pub xai_model: String,
    #[serde(default)]
    pub x_model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GrokConfigView {
    pub has_xai_key: bool,
    pub xai_model: String,
    pub x_model: String,
}

fn path() -> std::path::PathBuf {
    super::x_dir().join(CONFIG_FILE)
}

pub fn config() -> GrokConfig {
    let mut c: GrokConfig = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if c.xai_model.is_empty() {
        c.xai_model = DEFAULT_XAI_MODEL.into();
    }
    if c.x_model.is_empty() {
        c.x_model = DEFAULT_X_MODEL.into();
    }
    c
}

pub fn view() -> GrokConfigView {
    let c = config();
    GrokConfigView { has_xai_key: !c.xai_key.is_empty(), xai_model: c.xai_model, x_model: c.x_model }
}

/// `xai_key`: `None` mantem, `Some("")` apaga.
pub fn set(xai_key: Option<String>, xai_model: Option<String>, x_model: Option<String>) -> anyhow::Result<GrokConfigView> {
    let mut c = config();
    if let Some(k) = xai_key {
        c.xai_key = k.trim().to_string();
    }
    if let Some(m) = xai_model {
        c.xai_model = m.trim().to_string();
    }
    if let Some(m) = x_model {
        c.x_model = m.trim().to_string();
    }
    std::fs::write(path(), serde_json::to_string_pretty(&c)?)?;
    Ok(view())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GrokRequest {
    pub prompt: String,
    #[serde(default)]
    pub system: String,
    /// "xai" | "x" | "auto"
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub x_search: bool,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default)]
    pub handles: Vec<String>,
    #[serde(default)]
    pub from_date: String,
    #[serde(default)]
    pub to_date: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Citation {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrokAnswer {
    pub text: String,
    pub citations: Vec<Citation>,
    pub model: String,
    pub backend: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub async fn ask(req: GrokRequest) -> anyhow::Result<GrokAnswer> {
    let cfg = config();
    let backend = match req.backend.as_str() {
        "xai" => "xai",
        "x" => "x",
        _ => {
            if !cfg.xai_key.is_empty() {
                "xai"
            } else {
                "x"
            }
        }
    };
    if backend == "xai" {
        ask_xai(&cfg, req).await
    } else {
        ask_x(&cfg, req).await
    }
}

async fn ask_xai(cfg: &GrokConfig, req: GrokRequest) -> anyhow::Result<GrokAnswer> {
    if cfg.xai_key.is_empty() {
        anyhow::bail!("GROK_NO_KEY");
    }
    let model = if req.model.trim().is_empty() { cfg.xai_model.clone() } else { req.model.trim().to_string() };
    let mut tools = Vec::new();
    if req.x_search {
        let mut t = json!({ "type": "x_search" });
        let handles: Vec<String> = req.handles.iter().map(|h| h.trim().trim_start_matches('@').to_string()).filter(|h| !h.is_empty()).take(20).collect();
        if !handles.is_empty() {
            t["allowed_x_handles"] = json!(handles);
        }
        if !req.from_date.trim().is_empty() {
            t["from_date"] = json!(req.from_date.trim());
        }
        if !req.to_date.trim().is_empty() {
            t["to_date"] = json!(req.to_date.trim());
        }
        tools.push(t);
    }
    if req.web_search {
        tools.push(json!({ "type": "web_search" }));
    }
    let mut body = json!({ "model": model, "input": [{ "role": "user", "content": req.prompt }] });
    if !req.system.trim().is_empty() {
        body["instructions"] = json!(req.system.trim());
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder()).timeout(std::time::Duration::from_secs(180)).build()?;
    let resp = client.post("https://api.x.ai/v1/responses").bearer_auth(&cfg.xai_key).json(&body).send().await?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| anyhow!("xAI: resposta invalida ({})", e))?;
    if !status.is_success() {
        let msg = v.pointer("/error/message").or_else(|| v.get("error")).and_then(|m| m.as_str()).unwrap_or("erro").to_string();
        return Err(anyhow!("xAI HTTP {}: {}", status, msg));
    }
    let mut text = String::new();
    let mut citations: Vec<Citation> = Vec::new();
    for item in v.get("output").and_then(|o| o.as_array()).into_iter().flatten() {
        if item.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        for c in item.get("content").and_then(|c| c.as_array()).into_iter().flatten() {
            if c.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                text.push_str(c.get("text").and_then(|t| t.as_str()).unwrap_or(""));
                for a in c.get("annotations").and_then(|a| a.as_array()).into_iter().flatten() {
                    if let Some(url) = a.get("url").and_then(|u| u.as_str()) {
                        citations.push(Citation { url: url.to_string(), title: a.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string() });
                    }
                }
            }
        }
    }
    if text.is_empty() {
        text = v.get("output_text").and_then(|t| t.as_str()).unwrap_or("").to_string();
    }
    for c in v.get("citations").and_then(|c| c.as_array()).into_iter().flatten() {
        if let Some(url) = c.as_str() {
            if !citations.iter().any(|x| x.url == url) {
                citations.push(Citation { url: url.to_string(), title: String::new() });
            }
        }
    }
    let input_tokens = v.pointer("/usage/input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
    let output_tokens = v.pointer("/usage/output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
    crate::core::tools::usage::record("grok", "xai", &model, input_tokens, output_tokens, None);
    Ok(GrokAnswer { text, citations, model, backend: "xai".into(), input_tokens, output_tokens })
}

async fn ask_x(cfg: &GrokConfig, req: GrokRequest) -> anyhow::Result<GrokAnswer> {
    let client = super::client::XClient::new()?;
    client.require_login()?;
    let model = if req.model.trim().is_empty() { cfg.x_model.clone() } else { req.model.trim().to_string() };
    let conv = client.gql_post("CreateGrokConversation", json!({}), None).await?;
    let conversation_id = conv
        .pointer("/data/create_grok_conversation/conversation_id")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("Grok: nao consegui abrir uma conversa"))?
        .to_string();
    let mut message = req.prompt.clone();
    if !req.system.trim().is_empty() {
        message = format!("{}\n\n{}", req.system.trim(), req.prompt);
    }
    let body = json!({
        "responses": [{ "message": message, "sender": 1, "promptSource": "", "fileAttachments": [] }],
        "systemPromptName": "",
        "grokModelOptionId": model,
        "conversationId": conversation_id,
        "returnSearchResults": true,
        "returnCitations": true,
        "promptMetadata": { "promptSource": "NATURAL", "action": "INPUT" },
        "imageGenerationCount": 4,
        "requestFeatures": { "eagerTweets": true, "serverHistory": true },
        "enableSideBySide": true,
        "toolOverrides": {},
        "isDeepsearch": false,
        "isReasoning": false
    });
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    let extra = [("x-client-uuid", uuid.as_str()), ("Referer", "https://x.com/i/grok")];
    let resp = match client.post_json_raw("https://api.x.com/2/grok/add_response.json", &body, &extra).await {
        Ok(r) => r,
        Err(first) => client
            .post_json_raw("https://x.com/i/api/2/grok/add_response.json", &body, &extra)
            .await
            .map_err(|e| anyhow!("{} / {}", first, e))?,
    };
    let raw = resp.text().await?;
    let mut text = String::new();
    let mut citations: Vec<Citation> = Vec::new();
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let Some(r) = v.get("result") else { continue };
        if let Some(m) = r.get("message").and_then(|m| m.as_str()) {
            if r.get("sender").and_then(|s| s.as_str()).map(|s| s != "USER").unwrap_or(true) {
                text.push_str(m);
            }
        }
        if let Some(tok) = r.pointer("/response/token").and_then(|t| t.as_str()) {
            text.push_str(tok);
        }
        for key in ["cited_web_results", "webResults", "citedWebResults"] {
            for w in r.get(key).and_then(|a| a.as_array()).into_iter().flatten() {
                if let Some(url) = w.get("url").and_then(|u| u.as_str()) {
                    if !citations.iter().any(|c| c.url == url) {
                        citations.push(Citation { url: url.to_string(), title: w.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string() });
                    }
                }
            }
        }
        if let Some(err) = r.get("error").and_then(|e| e.as_str()) {
            if text.is_empty() {
                return Err(anyhow!("Grok: {}", err));
            }
        }
    }
    if text.trim().is_empty() {
        return Err(anyhow!("Grok nao respondeu (modelo `{}` pode nao existir mais; troque nas opcoes)", model));
    }
    Ok(GrokAnswer { text, citations, model, backend: "x".into(), input_tokens: 0, output_tokens: 0 })
}
