//! Downloads › Tools › aria2 server: send links to an aria2 that runs on
//! another machine (a NAS, a seedbox, a Raspberry Pi) and watch its
//! downloads, without opening a web page.
//!
//! The idea comes from AriaNg, a web front end that drives aria2 over
//! JSON-RPC with a `token:<secret>` first parameter
//! (`src/scripts/services/aria2RpcService.js` at 1.3.14). Reimplemented as a
//! small client with a fixed list of methods; no code was copied.

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

/// The only aria2 methods the panel may call: reading state, adding links,
/// and pause/resume/remove of a single task. Nothing that changes the
/// server's options or shuts it down.
const ALLOWED: &[&str] = &[
    "aria2.getVersion",
    "aria2.getGlobalStat",
    "aria2.addUri",
    "aria2.tellActive",
    "aria2.tellWaiting",
    "aria2.tellStopped",
    "aria2.pause",
    "aria2.unpause",
    "aria2.remove",
    "aria2.removeDownloadResult",
];

const TASK_KEYS: &[&str] = &[
    "gid",
    "status",
    "totalLength",
    "completedLength",
    "downloadSpeed",
    "errorMessage",
    "files",
    "bittorrent",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RemoteTask {
    pub gid: String,
    pub status: String,
    pub name: String,
    pub total: u64,
    pub done: u64,
    pub speed: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RemoteOverview {
    pub version: String,
    pub download_speed: u64,
    pub tasks: Vec<RemoteTask>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AddResult {
    pub added: usize,
    pub failed: Vec<String>,
}

/// Accepts `host:port`, `http(s)://host:port` with or without `/jsonrpc`.
pub fn normalize_endpoint(input: &str) -> Result<String, String> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err("ARIA2_NO_ADDRESS".into());
    }
    let had_scheme = s.starts_with("http://") || s.starts_with("https://");
    let with_scheme = if had_scheme {
        s.to_string()
    } else if s.contains("://") {
        return Err("ARIA2_BAD_ADDRESS".into());
    } else {
        format!("http://{s}")
    };
    let mut url = url::Url::parse(&with_scheme).map_err(|_| "ARIA2_BAD_ADDRESS".to_string())?;
    if url.host_str().is_none() {
        return Err("ARIA2_BAD_ADDRESS".into());
    }
    if url.path() == "/" || url.path().is_empty() {
        url.set_path("/jsonrpc");
    }
    if !had_scheme && url.port().is_none() {
        // A bare host gets aria2's default RPC port, like AriaNg's default profile.
        let _ = url.set_port(Some(6800));
    }
    Ok(url.to_string())
}

/// Links to send, one per line: http(s), ftp, sftp and magnet.
pub fn parse_links(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            ["http://", "https://", "ftp://", "sftp://", "magnet:?"]
                .iter()
                .any(|p| l.starts_with(p))
        })
        .map(str::to_string)
        .collect()
}

fn params_with_token(secret: &str, params: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::with_capacity(params.len() + 1);
    if !secret.is_empty() {
        out.push(Value::String(format!("token:{secret}")));
    }
    out.extend(params);
    out
}

async fn call(
    endpoint: &str,
    secret: &str,
    method: &str,
    params: Vec<Value>,
) -> Result<Value, String> {
    if !ALLOWED.contains(&method) {
        return Err("ARIA2_METHOD_NOT_ALLOWED".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let body = json!({
        "jsonrpc": "2.0",
        "id": "omniget",
        "method": method,
        "params": params_with_token(secret, params),
    });
    let res = client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|_| "ARIA2_UNREACHABLE".to_string())?;
    let status = res.status();
    let value: Value = res.json().await.map_err(|_| {
        if status.is_success() {
            "ARIA2_NOT_ARIA2".to_string()
        } else {
            format!("ARIA2_HTTP_{}", status.as_u16())
        }
    })?;
    if let Some(err) = value.get("error") {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or("");
        if msg.eq_ignore_ascii_case("unauthorized") {
            return Err("ARIA2_UNAUTHORIZED".into());
        }
        return Err(format!("ARIA2_ERROR|{msg}"));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

fn num(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn task_name(v: &Value) -> String {
    if let Some(name) = v
        .pointer("/bittorrent/info/name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return name.to_string();
    }
    let first = v.pointer("/files/0");
    let path = first
        .and_then(|f| f.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !path.is_empty() {
        return path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
    }
    first
        .and_then(|f| f.pointer("/uris/0/uri"))
        .and_then(Value::as_str)
        .map(crate::core::flight_recorder::redact_url)
        .unwrap_or_default()
}

pub fn to_task(v: &Value) -> RemoteTask {
    RemoteTask {
        gid: v
            .get("gid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        status: v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name: task_name(v),
        total: num(v, "totalLength"),
        done: num(v, "completedLength"),
        speed: num(v, "downloadSpeed"),
        error: v
            .get("errorMessage")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }
}

#[tauri::command]
pub async fn aria2_remote_overview(
    address: String,
    secret: String,
) -> Result<RemoteOverview, String> {
    let endpoint = normalize_endpoint(&address)?;
    let version = call(&endpoint, &secret, "aria2.getVersion", vec![]).await?;
    let stat = call(&endpoint, &secret, "aria2.getGlobalStat", vec![]).await?;
    let keys = json!(TASK_KEYS);
    let mut tasks = Vec::new();
    for (method, extra) in [
        ("aria2.tellActive", vec![keys.clone()]),
        ("aria2.tellWaiting", vec![json!(0), json!(50), keys.clone()]),
        ("aria2.tellStopped", vec![json!(0), json!(20), keys.clone()]),
    ] {
        if let Value::Array(list) = call(&endpoint, &secret, method, extra).await? {
            tasks.extend(list.iter().map(to_task));
        }
    }
    Ok(RemoteOverview {
        version: version
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        download_speed: num(&stat, "downloadSpeed"),
        tasks,
    })
}

#[tauri::command]
pub async fn aria2_remote_add(
    address: String,
    secret: String,
    links: String,
) -> Result<AddResult, String> {
    let endpoint = normalize_endpoint(&address)?;
    let links = parse_links(&links);
    if links.is_empty() {
        return Err("ARIA2_NO_LINKS".into());
    }
    let mut added = 0;
    let mut failed = Vec::new();
    for link in links {
        match call(&endpoint, &secret, "aria2.addUri", vec![json!([link])]).await {
            Ok(_) => added += 1,
            // Wrong secret or server gone: every other link would fail the same way.
            Err(e) if e == "ARIA2_UNAUTHORIZED" || e == "ARIA2_UNREACHABLE" => return Err(e),
            Err(_) => failed.push(crate::core::flight_recorder::redact_url(&link)),
        }
    }
    Ok(AddResult { added, failed })
}

/// `action`: "pause", "resume" or "remove" for one task.
#[tauri::command]
pub async fn aria2_remote_task(
    address: String,
    secret: String,
    gid: String,
    action: String,
) -> Result<(), String> {
    let endpoint = normalize_endpoint(&address)?;
    if gid.is_empty() || !gid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("ARIA2_BAD_TASK".into());
    }
    let method = match action.as_str() {
        "pause" => "aria2.pause",
        "resume" => "aria2.unpause",
        "remove" => "aria2.remove",
        "clear" => "aria2.removeDownloadResult",
        _ => return Err("ARIA2_BAD_ACTION".into()),
    };
    call(&endpoint, &secret, method, vec![json!(gid)])
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints() {
        assert_eq!(
            normalize_endpoint("192.168.1.5").unwrap(),
            "http://192.168.1.5:6800/jsonrpc"
        );
        assert_eq!(
            normalize_endpoint("nas.local:6801").unwrap(),
            "http://nas.local:6801/jsonrpc"
        );
        assert_eq!(
            normalize_endpoint("https://seedbox.example.com/jsonrpc/").unwrap(),
            "https://seedbox.example.com/jsonrpc"
        );
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:16800/jsonrpc").unwrap(),
            "http://127.0.0.1:16800/jsonrpc"
        );
        assert_eq!(normalize_endpoint("  ").unwrap_err(), "ARIA2_NO_ADDRESS");
        assert_eq!(
            normalize_endpoint("ws://x:6800").unwrap_err(),
            "ARIA2_BAD_ADDRESS"
        );
    }

    #[test]
    fn links_one_per_line() {
        let got = parse_links("https://a/b.iso\n\n  magnet:?xt=urn:btih:abc  \nnot a link\nftp://c/d\nfile:///etc/passwd");
        assert_eq!(
            got,
            vec!["https://a/b.iso", "magnet:?xt=urn:btih:abc", "ftp://c/d"]
        );
    }

    #[test]
    fn token_goes_first() {
        let p = params_with_token("s3cret", vec![json!(["https://a"])]);
        assert_eq!(p[0], json!("token:s3cret"));
        assert_eq!(params_with_token("", vec![json!(1)]), vec![json!(1)]);
    }

    #[test]
    fn tasks_from_aria2() {
        let t = to_task(&json!({
            "gid": "2089b05ecca3d829", "status": "active", "totalLength": "1000",
            "completedLength": "250", "downloadSpeed": "100",
            "files": [{ "path": "/data/dl/ubuntu.iso", "uris": [] }]
        }));
        assert_eq!(
            (t.name.as_str(), t.total, t.done, t.speed),
            ("ubuntu.iso", 1000, 250, 100)
        );
        let bt = to_task(
            &json!({ "gid": "1", "bittorrent": { "info": { "name": "Big Buck Bunny" } }, "files": [] }),
        );
        assert_eq!(bt.name, "Big Buck Bunny");
    }
}
