//! Servidor MCP embutido (estudos 22 e 23): as tools da seção Tools expostas
//! como ferramentas MCP em `POST /mcp` no bridge local, com o mesmo bearer
//! da extensão. Transporte "Streamable HTTP" só com respostas JSON (sem SSE),
//! que é o que Claude Code, Cursor, Goose e o `mcp-remote` do Claude Desktop
//! aceitam. Sem crate de MCP: o protocolo aqui é JSON-RPC com quatro métodos.

use omniget_core::core::tools::{self as tools, ai_keys, disk, dupes, edge_tts, file_search, humanize, image_resize, ocr, pdf, pricing, ryd, sponsorblock, startup, sysclean, uninstall, whisper, x};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

pub const PROTOCOL: &str = "2025-06-18";

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

pub fn tools() -> Vec<ToolDef> {
    let t = |name, description, input_schema| ToolDef { name, description, input_schema };
    vec![
        t("download_url", "Queue a URL (video, audio, playlist, course, image) in the OmniGet Downloads panel. Same as pasting it in the app.", obj(json!({ "url": { "type": "string" } }), &["url"])),
        t("youtube_sponsorblock", "SponsorBlock segments (sponsor, intro, outro, selfpromo…) of a YouTube video.", obj(json!({ "url": { "type": "string" }, "categories": { "type": "array", "items": { "type": "string" } } }), &["url"])),
        t("youtube_dislikes", "Return YouTube Dislike estimates for a video.", obj(json!({ "url": { "type": "string" } }), &["url"])),
        t("pdf_info", "Pages, size, title, author and whether the PDF has a text layer.", obj(json!({ "path": { "type": "string" } }), &["path"])),
        t("pdf_merge", "Merge PDFs into one file, in the given order.", obj(json!({ "inputs": { "type": "array", "items": { "type": "string" } }, "output": { "type": "string" } }), &["inputs", "output"])),
        t("pdf_split", "Split a PDF: mode each | every | ranges (\"1-3; 4-10\") | extract (\"1,3,5-7\").", obj(json!({ "input": { "type": "string" }, "mode": { "type": "string" }, "every": { "type": "integer" }, "ranges": { "type": "string" }, "output_dir": { "type": "string" } }), &["input", "mode"])),
        t("pdf_text", "Extract the text of a PDF (optionally a page range like \"1-3, 5\").", obj(json!({ "path": { "type": "string" }, "pages": { "type": "string" } }), &["path"])),
        t("pdf_render", "Render PDF pages to PNG or JPG files.", obj(json!({ "input": { "type": "string" }, "pages": { "type": "string" }, "dpi": { "type": "integer" }, "format": { "type": "string", "enum": ["png", "jpg"] }, "output_dir": { "type": "string" } }), &["input"])),
        t("pdf_sanitize", "Rebuild a PDF from pixels (Dangerzone-style) so scripts, forms and attachments are dropped.", obj(json!({ "input": { "type": "string" }, "output_dir": { "type": "string" } }), &["input"])),
        t("tts_speak", "Text to speech with Microsoft Edge neural voices; writes an MP3.", obj(json!({ "text": { "type": "string" }, "voice": { "type": "string", "description": "e.g. pt-BR-AntonioNeural, en-US-AriaNeural" }, "output": { "type": "string" } }), &["text", "output"])),
        t("transcribe", "Transcribe audio or video locally with whisper.cpp; returns text and SRT path.", obj(json!({ "input": { "type": "string" }, "model": { "type": "string", "description": "GGML model id, default base" }, "language": { "type": "string", "description": "auto | pt | en …" } }), &["input"])),
        t("image_resize", "Resize images in batch. mode width | height | fit | percent.", obj(json!({ "inputs": { "type": "array", "items": { "type": "string" } }, "mode": { "type": "string" }, "value": { "type": "integer" }, "value2": { "type": "integer" }, "format": { "type": "string" }, "output_dir": { "type": "string" } }), &["inputs", "mode", "value"])),
        t("ocr", "Extract text from images with Tesseract.", obj(json!({ "inputs": { "type": "array", "items": { "type": "string" } }, "langs": { "type": "string", "description": "por+eng" } }), &["inputs"])),
        t("find_duplicates", "Find duplicate files (same content) under folders.", obj(json!({ "dirs": { "type": "array", "items": { "type": "string" } }, "min_size": { "type": "integer" } }), &["dirs"])),
        t("file_search", "Search files by name (Everything, Spotlight or locate/find).", obj(json!({ "query": { "type": "string" }, "folder": { "type": "string" }, "limit": { "type": "integer" } }), &["query"])),
        t("ai_prices", "Search LLM prices per million tokens (LiteLLM + models.dev).", obj(json!({ "query": { "type": "string" }, "limit": { "type": "integer" } }), &["query"])),
        t("humanize", "Rewrite AI-sounding text so it reads like a person wrote it, using the app's configured AI.", obj(json!({ "text": { "type": "string" } }), &["text"])),
        t("x_post", "Fetch an X/Twitter post (text, author, media) by URL or id.", obj(json!({ "url": { "type": "string" } }), &["url"])),
        t("x_thread", "Unroll an X/Twitter thread from any post in it.", obj(json!({ "url": { "type": "string" } }), &["url"])),
        t("x_profile", "Profile analytics for an X/Twitter user (engagement, best hours, top posts).", obj(json!({ "handle": { "type": "string" }, "limit": { "type": "integer" } }), &["handle"])),
        t("x_search", "Search X/Twitter posts (advanced operators supported).", obj(json!({ "query": { "type": "string" }, "feed": { "type": "string", "enum": ["latest", "top"] } }), &["query"])),
        t("x_trends", "Current X/Twitter trends.", obj(json!({}), &[])),
        t("instagram_profile", "Public info of an Instagram profile, using the cookies captured by the OmniGet extension.", obj(json!({ "username": { "type": "string" }, "account": { "type": "string", "description": "cookie slot, default _default" } }), &["username"])),
        t("gallery_download", "Download a gallery/profile with gallery-dl (Pinterest, ArtStation, DeviantArt, Reddit…).", obj(json!({ "url": { "type": "string" }, "dest": { "type": "string" } }), &["url", "dest"])),
        t("aria2_download", "Download a large file with aria2 (multi-connection).", obj(json!({ "url": { "type": "string" }, "dest_dir": { "type": "string" }, "connections": { "type": "integer" } }), &["url", "dest_dir"])),
        t("disk_volumes", "Mounted volumes with total and free space.", obj(json!({}), &[])),
        t("disk_scan", "Folder sizes tree and largest files under a path.", obj(json!({ "path": { "type": "string" }, "depth": { "type": "integer" } }), &["path"])),
        t("clean_scan", "What the cache cleaner would remove (rule, size, files). Does not delete anything.", obj(json!({}), &[])),
        t("startup_items", "Programs that start with the system.", obj(json!({}), &[])),
        t("installed_apps", "Installed applications with version and size.", obj(json!({}), &[])),
        t("ai_keys", "Saved AI API accounts (names, providers, balances). Keys are never returned.", obj(json!({}), &[])),
    ]
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}
fn list(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}
fn num(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64())
}
fn to_json<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}
fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub async fn call(app: &AppHandle, name: &str, a: Value) -> Result<Value, String> {
    let p = tools::noop_progress();
    match name {
        "download_url" => {
            let url = s(&a, "url");
            let action = crate::external_url::handle_external_url(app, url.clone(), "mcp").await?;
            Ok(json!({ "url": url, "action": format!("{:?}", action).to_lowercase() }))
        }
        "youtube_sponsorblock" => to_json(sponsorblock::segments(&s(&a, "url"), &list(&a, "categories")).await.map_err(err)?),
        "youtube_dislikes" => to_json(ryd::votes(&s(&a, "url")).await.map_err(err)?),
        "pdf_info" => {
            let path = s(&a, "path");
            to_json(tokio::task::spawn_blocking(move || pdf::info(&path, None)).await.map_err(err)?.map_err(err)?)
        }
        "pdf_merge" => {
            let opts = pdf::MergeOptions { inputs: list(&a, "inputs"), output: s(&a, "output") };
            to_json(tokio::task::spawn_blocking(move || pdf::merge(&opts, &p)).await.map_err(err)?.map_err(err)?)
        }
        "pdf_split" => {
            let opts = pdf::SplitOptions { input: s(&a, "input"), mode: s(&a, "mode"), every: num(&a, "every").unwrap_or(0) as usize, ranges: s(&a, "ranges"), output_dir: s(&a, "output_dir") };
            to_json(tokio::task::spawn_blocking(move || pdf::split(&opts, &p)).await.map_err(err)?.map_err(err)?)
        }
        "pdf_text" => {
            let (path, pages) = (s(&a, "path"), s(&a, "pages"));
            to_json(tokio::task::spawn_blocking(move || pdf::to_text(&path, &pages, false, "")).await.map_err(err)?.map_err(err)?)
        }
        "pdf_render" => {
            let opts = pdf::RenderOptions { input: s(&a, "input"), pages: s(&a, "pages"), dpi: num(&a, "dpi").unwrap_or(0) as u32, format: s(&a, "format"), quality: 0, output_dir: s(&a, "output_dir") };
            to_json(tokio::task::spawn_blocking(move || pdf::render(&opts, &p)).await.map_err(err)?.map_err(err)?)
        }
        "pdf_sanitize" => {
            let (input, dir) = (s(&a, "input"), s(&a, "output_dir"));
            to_json(tokio::task::spawn_blocking(move || pdf::sanitize(&input, &dir, 0, 0, &p)).await.map_err(err)?.map_err(err)?)
        }
        "tts_speak" => {
            let voice = { let v = s(&a, "voice"); if v.is_empty() { "pt-BR-AntonioNeural".to_string() } else { v } };
            let opts: edge_tts::TtsOptions = serde_json::from_value(json!({ "text": s(&a, "text"), "voice": voice })).map_err(err)?;
            to_json(edge_tts::synthesize(opts, std::path::Path::new(&s(&a, "output")), p).await.map_err(err)?)
        }
        "transcribe" => {
            let model = { let m = s(&a, "model"); if m.is_empty() { "base".to_string() } else { m } };
            let language = { let l = s(&a, "language"); if l.is_empty() { "auto".to_string() } else { l } };
            let opts: whisper::TranscribeOptions = serde_json::from_value(json!({ "input": s(&a, "input"), "model": model, "language": language })).map_err(err)?;
            let r = whisper::transcribe(opts, p).await.map_err(err)?;
            Ok(json!({ "language": r.language, "text": r.text, "srt": r.srt_path, "vtt": r.vtt_path, "txt": r.txt_path, "seconds": r.seconds }))
        }
        "image_resize" => {
            let opts: image_resize::ResizeOptions = serde_json::from_value(json!({ "inputs": list(&a, "inputs"), "mode": s(&a, "mode"), "value": num(&a, "value").unwrap_or(1024), "value2": num(&a, "value2").unwrap_or(0), "format": s(&a, "format"), "output_dir": s(&a, "output_dir") })).map_err(err)?;
            to_json(image_resize::run(opts, p).await.map_err(err)?)
        }
        "ocr" => to_json(ocr::run(&list(&a, "inputs"), &s(&a, "langs"), p).await.map_err(err)?),
        "find_duplicates" => {
            let opts: dupes::DupesOptions = serde_json::from_value(json!({ "dirs": list(&a, "dirs"), "min_size": num(&a, "min_size").unwrap_or(1024) })).map_err(err)?;
            to_json(tokio::task::spawn_blocking(move || dupes::scan(&opts, &p)).await.map_err(err)?)
        }
        "file_search" => to_json(file_search::search(&s(&a, "query"), &s(&a, "folder"), num(&a, "limit").unwrap_or(100) as usize).await.map_err(err)?),
        "ai_prices" => to_json(pricing::search(&s(&a, "query"), "", num(&a, "limit").unwrap_or(30) as usize).await.map_err(err)?),
        "humanize" => Ok(json!({ "text": humanize::humanize(&s(&a, "text"), None).await? })),
        "x_post" => {
            let input = s(&a, "url");
            let id = x::post_id_from(&input).ok_or_else(|| format!("not an X post: {}", input))?;
            to_json(x::fx::status(&id).await.map_err(err)?)
        }
        "x_thread" => to_json(x::thread::unroll(&s(&a, "url")).await.map_err(err)?),
        "x_profile" => to_json(x::profile::analyze(&s(&a, "handle"), num(&a, "limit").unwrap_or(100) as usize, false).await.map_err(err)?),
        "x_search" => {
            let feed = { let f = s(&a, "feed"); if f.is_empty() { "latest".to_string() } else { f } };
            to_json(x::search::search(&s(&a, "query"), &feed, None).await.map_err(err)?)
        }
        "x_trends" => to_json(x::search::trends().await.map_err(err)?),
        "instagram_profile" => {
            let account = { let v = s(&a, "account"); if v.is_empty() { None } else { Some(v) } };
            let client = crate::commands::tools::instagram::load_client(account.as_deref())?;
            to_json(tools::instagram::profile::resolve_user(&client, &s(&a, "username")).await.map_err(err)?)
        }
        "gallery_download" => to_json(tools::gallery::download(&s(&a, "url"), &s(&a, "dest"), None, p).await.map_err(err)?),
        "aria2_download" => {
            let opts: tools::aria2::Aria2Options = serde_json::from_value(json!({ "url": s(&a, "url"), "dest_dir": s(&a, "dest_dir"), "connections": num(&a, "connections").unwrap_or(16) })).map_err(err)?;
            to_json(tools::aria2::download(opts, p).await.map_err(err)?)
        }
        "disk_volumes" => to_json(disk::volumes()),
        "disk_scan" => {
            let (path, depth) = (s(&a, "path"), num(&a, "depth").unwrap_or(2) as usize);
            to_json(tokio::task::spawn_blocking(move || disk::scan(&path, depth, 25, &p)).await.map_err(err)?.map_err(err)?)
        }
        "clean_scan" => to_json(tokio::task::spawn_blocking(move || sysclean::scan(&p)).await.map_err(err)?),
        "startup_items" => to_json(startup::list().await),
        "installed_apps" => to_json(uninstall::list(p).await),
        "ai_keys" => to_json(ai_keys::list()),
        _ => Err(format!("unknown tool: {}", name)),
    }
}

// ── Estado (ligado/desligado) ──────────────────────────────────────────

fn config_file() -> Option<std::path::PathBuf> {
    tools::tools_dir().map(|d| d.join("mcp.json"))
}

pub fn enabled() -> bool {
    config_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
        .unwrap_or(false)
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let p = config_file().ok_or("no data dir")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(err)?;
    std::fs::write(&p, serde_json::to_string_pretty(&json!({ "enabled": on })).unwrap()).map_err(err)
}

// ── JSON-RPC ───────────────────────────────────────────────────────────

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Trata uma mensagem. `None` = notificação (sem resposta).
pub async fn handle(app: &AppHandle, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    if method.starts_with("notifications/") {
        return None;
    }
    let id = id?;
    Some(match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(PROTOCOL);
            let version = if matches!(requested, "2024-11-05" | "2025-03-26" | "2025-06-18") { requested } else { PROTOCOL };
            rpc_ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "OmniGet", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "OmniGet desktop tools: downloads, PDF, speech, images, files, X/Twitter, Instagram, system. Paths are local to this machine."
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call(app, name, args).await {
                Ok(v) => {
                    let text = if let Some(t) = v.as_str() { t.to_string() } else { serde_json::to_string_pretty(&v).unwrap_or_default() };
                    rpc_ok(id, json!({ "content": [{ "type": "text", "text": text }], "structuredContent": v, "isError": false }))
                }
                Err(e) => rpc_ok(id, json!({ "content": [{ "type": "text", "text": e }], "isError": true })),
            }
        }
        "resources/list" => rpc_ok(id, json!({ "resources": [] })),
        "prompts/list" => rpc_ok(id, json!({ "prompts": [] })),
        _ => rpc_error(id, -32601, format!("method not found: {}", method)),
    })
}

/// Corpo inteiro (mensagem única ou lote) → resposta pronta para o HTTP.
pub async fn handle_body(app: &AppHandle, body: &Value) -> Option<Value> {
    match body {
        Value::Array(items) => {
            let mut out = Vec::new();
            for m in items {
                if let Some(r) = handle(app, m).await {
                    out.push(r);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Array(out))
            }
        }
        m => handle(app, m).await,
    }
}

/// Trechos de configuração para os clientes, com a URL e o token deste app.
pub fn client_snippets(url: &str, token: &str) -> Vec<(String, String)> {
    let auth = format!("Authorization: Bearer {}", token);
    vec![
        ("Claude Code".into(), format!("claude mcp add --transport http omniget {} --header \"{}\"", url, auth)),
        (
            "Cursor".into(),
            serde_json::to_string_pretty(&json!({ "mcpServers": { "omniget": { "url": url, "headers": { "Authorization": format!("Bearer {}", token) } } } })).unwrap_or_default(),
        ),
        (
            "VS Code".into(),
            serde_json::to_string_pretty(&json!({ "servers": { "omniget": { "type": "http", "url": url, "headers": { "Authorization": format!("Bearer {}", token) } } } })).unwrap_or_default(),
        ),
        (
            "Goose".into(),
            format!("extensions:\n  omniget:\n    enabled: true\n    type: streamable_http\n    name: omniget\n    uri: {}\n    headers:\n      Authorization: Bearer {}\n    timeout: 300", url, token),
        ),
        (
            "Claude Desktop".into(),
            serde_json::to_string_pretty(&json!({ "mcpServers": { "omniget": { "command": "npx", "args": ["-y", "mcp-remote", url, "--header", auth] } } })).unwrap_or_default(),
        ),
        (
            "Codex".into(),
            format!("[mcp_servers.omniget]\nurl = \"{}\"\nhttp_headers = {{ Authorization = \"Bearer {}\" }}", url, token),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_objects() {
        let list = tools();
        assert!(list.len() > 20);
        for t in &list {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert!(!t.description.is_empty());
        }
        let names: std::collections::HashSet<_> = list.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), list.len(), "nomes repetidos");
    }

    #[test]
    fn snippets_carry_token() {
        for (_, s) in client_snippets("http://127.0.0.1:47720/mcp", "tok123") {
            assert!(s.contains("tok123"));
            assert!(s.contains("47720"));
        }
    }
}
