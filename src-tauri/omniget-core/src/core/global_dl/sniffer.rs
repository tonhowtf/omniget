//! What the tabs load. Every response lands in a bounded request log (for
//! connector authors and the `gd_network_log` tool); responses that look like
//! media also land in the sniffed list the Global Downloader screen shows.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

use serde::Serialize;
use serde_json::{json, Value};

const LOG_CAP: usize = 4000;
const SNIFF_CAP: usize = 600;

#[derive(Debug, Clone, Serialize)]
pub struct NetEntry {
    pub request_id: String,
    pub session: String,
    pub tab_id: String,
    pub url: String,
    pub method: String,
    pub status: u16,
    pub mime: String,
    pub resource_type: String,
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub request_headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_data: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sniffed {
    pub id: u64,
    pub tab_id: String,
    pub page_url: String,
    pub page_title: String,
    pub url: String,
    /// hls | dash | video | audio | subtitle | document | image
    pub kind: String,
    pub mime: String,
    pub size: Option<u64>,
    /// Headers the page sent with the request (Referer, Origin, auth), so the
    /// download looks like the page.
    pub headers: BTreeMap<String, String>,
    pub at: i64,
}

#[derive(Default)]
struct State {
    log: VecDeque<NetEntry>,
    sniffed: VecDeque<Sniffed>,
    next_id: u64,
    /// (session, requestId) → what requestWillBeSent / ExtraInfo told us.
    requests: HashMap<(String, String), Pending>,
}

#[derive(Default, Clone)]
struct Pending {
    url: String,
    method: String,
    resource_type: String,
    headers: BTreeMap<String, String>,
    post_data: Option<String>,
}

fn state() -> &'static Mutex<State> {
    static S: std::sync::OnceLock<Mutex<State>> = std::sync::OnceLock::new();
    S.get_or_init(Mutex::default)
}

fn headers_of(v: &Value) -> BTreeMap<String, String> {
    v.as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

pub fn on_request(session: &str, p: &Value) {
    let Some(rid) = p.get("requestId").and_then(Value::as_str) else {
        return;
    };
    let req = &p["request"];
    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    let entry = st
        .requests
        .entry((session.to_string(), rid.to_string()))
        .or_default();
    entry.url = req["url"].as_str().unwrap_or_default().to_string();
    entry.method = req["method"].as_str().unwrap_or("GET").to_string();
    entry.resource_type = p["type"].as_str().unwrap_or_default().to_string();
    entry.post_data = req["postData"].as_str().map(String::from);
    for (k, v) in headers_of(&req["headers"]) {
        entry.headers.entry(k).or_insert(v);
    }
    if st.requests.len() > 20_000 {
        st.requests.clear();
    }
}

/// `requestWillBeSentExtraInfo` carries the headers Chrome really sent,
/// Cookie included.
pub fn on_request_extra(session: &str, p: &Value) {
    let Some(rid) = p.get("requestId").and_then(Value::as_str) else {
        return;
    };
    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    let entry = st
        .requests
        .entry((session.to_string(), rid.to_string()))
        .or_default();
    for (k, v) in headers_of(&p["headers"]) {
        entry.headers.insert(k, v);
    }
}

/// Returns the sniffed item when this response is media, so the caller can
/// emit it.
pub fn on_response(session: &str, tab_id: &str, page: (&str, &str), p: &Value) -> Option<Sniffed> {
    let rid = p.get("requestId").and_then(Value::as_str)?;
    let resp = &p["response"];
    let url = resp["url"].as_str().unwrap_or_default().to_string();
    if url.starts_with("data:") || url.starts_with("blob:") || url.starts_with("chrome") {
        return None;
    }
    let mime = resp["mimeType"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let status = resp["status"].as_u64().unwrap_or(0) as u16;
    let rtype = p["type"].as_str().unwrap_or_default().to_string();
    let resp_headers = headers_of(&resp["headers"]);
    let size = resp_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<u64>().ok());
    let now = chrono::Utc::now().timestamp_millis();

    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    let pending = st
        .requests
        .get(&(session.to_string(), rid.to_string()))
        .cloned()
        .unwrap_or_default();
    st.log.push_back(NetEntry {
        request_id: rid.to_string(),
        session: session.to_string(),
        tab_id: tab_id.to_string(),
        url: url.clone(),
        method: if pending.method.is_empty() {
            "GET".into()
        } else {
            pending.method.clone()
        },
        status,
        mime: mime.clone(),
        resource_type: rtype.clone(),
        size,
        request_headers: pending.headers.clone(),
        post_data: pending.post_data.clone(),
        at: now,
    });
    while st.log.len() > LOG_CAP {
        st.log.pop_front();
    }

    if !(200..400).contains(&status) && status != 206 {
        return None;
    }
    let kind = classify(&url, &mime, &rtype)?;
    let key = dedupe_key(&url, kind);
    if st
        .sniffed
        .iter()
        .any(|s| s.tab_id == tab_id && dedupe_key(&s.url, &s.kind) == key)
    {
        return None;
    }
    st.next_id += 1;
    let mut headers = BTreeMap::new();
    for (k, v) in &pending.headers {
        let lk = k.to_ascii_lowercase();
        if matches!(
            lk.as_str(),
            "referer" | "origin" | "authorization" | "cookie" | "user-agent"
        ) || lk.starts_with("x-")
        {
            headers.insert(k.clone(), v.clone());
        }
    }
    let item = Sniffed {
        id: st.next_id,
        tab_id: tab_id.to_string(),
        page_url: page.0.to_string(),
        page_title: page.1.to_string(),
        url,
        kind: kind.to_string(),
        mime,
        size,
        headers,
        at: now,
    };
    st.sniffed.push_back(item.clone());
    while st.sniffed.len() > SNIFF_CAP {
        st.sniffed.pop_front();
    }
    Some(item)
}

fn path_ext(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    last.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// What kind of media a response is, or `None` for page plumbing. Segments of
/// a stream (`.ts`, `.m4s`, `video/mp2t`) are skipped: the playlist is what
/// gets downloaded.
pub fn classify(url: &str, mime: &str, rtype: &str) -> Option<&'static str> {
    let ext = path_ext(url);
    if mime.contains("mpegurl") || ext == "m3u8" {
        return Some("hls");
    }
    if mime.contains("dash+xml") || ext == "mpd" {
        return Some("dash");
    }
    if matches!(
        ext.as_str(),
        "ts" | "m4s" | "cmfv" | "cmfa" | "m4f" | "m4v_seg"
    ) || mime == "video/mp2t"
        || mime == "video/iso.segment"
    {
        return None;
    }
    if ext == "vtt" || ext == "srt" || mime == "text/vtt" || mime == "application/x-subrip" {
        return Some("subtitle");
    }
    if mime.starts_with("video/")
        || matches!(
            ext.as_str(),
            "mp4" | "webm" | "mov" | "mkv" | "m4v" | "avi" | "flv"
        )
    {
        return Some("video");
    }
    if mime.starts_with("audio/")
        || matches!(
            ext.as_str(),
            "mp3" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "flac"
        )
    {
        return Some("audio");
    }
    if mime == "application/pdf"
        || mime == "application/epub+zip"
        || matches!(
            ext.as_str(),
            "pdf"
                | "epub"
                | "zip"
                | "rar"
                | "7z"
                | "docx"
                | "pptx"
                | "xlsx"
                | "doc"
                | "ppt"
                | "xls"
        ) && rtype != "Script"
    {
        return Some("document");
    }
    None
}

/// Range requests and signed-URL refreshes of the same file collapse into one
/// entry: host + path, plus the query for playlists (their query often picks
/// the rendition).
fn dedupe_key(url: &str, kind: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            let base = format!("{}{}", u.host_str().unwrap_or(""), u.path());
            if kind == "hls" || kind == "dash" {
                base
            } else if u.host_str().is_some_and(|h| h.ends_with("googlevideo.com")) {
                // YouTube streams: one entry per itag.
                let itag = u
                    .query_pairs()
                    .find(|(k, _)| k == "itag")
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_default();
                format!("{base}#{itag}")
            } else {
                base
            }
        }
        Err(_) => url.to_string(),
    }
}

pub fn sniffed(tab_id: Option<&str>) -> Vec<Sniffed> {
    let st = state().lock().unwrap_or_else(|e| e.into_inner());
    st.sniffed
        .iter()
        .filter(|s| tab_id.is_none_or(|t| s.tab_id == t))
        .cloned()
        .collect()
}

pub fn sniffed_by_id(id: u64) -> Option<Sniffed> {
    let st = state().lock().unwrap_or_else(|e| e.into_inner());
    st.sniffed.iter().find(|s| s.id == id).cloned()
}

pub fn clear(tab_id: Option<&str>) {
    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    match tab_id {
        Some(t) => {
            st.sniffed.retain(|s| s.tab_id != t);
            st.log.retain(|s| s.tab_id != t);
        }
        None => {
            st.sniffed.clear();
            st.log.clear();
        }
    }
}

/// The request log, newest last, filtered for the connector tools.
pub fn log(
    tab_id: Option<&str>,
    contains: Option<&str>,
    types: &[String],
    limit: usize,
) -> Vec<NetEntry> {
    let st = state().lock().unwrap_or_else(|e| e.into_inner());
    let needle = contains.map(|c| c.to_ascii_lowercase());
    let mut out: Vec<NetEntry> = st
        .log
        .iter()
        .filter(|e| tab_id.is_none_or(|t| e.tab_id == t))
        .filter(|e| {
            needle
                .as_deref()
                .is_none_or(|n| e.url.to_ascii_lowercase().contains(n))
        })
        .filter(|e| {
            types.is_empty()
                || types
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(&e.resource_type))
        })
        .cloned()
        .collect();
    if out.len() > limit {
        out.drain(..out.len() - limit);
    }
    out
}

pub fn log_entry(request_id: &str) -> Option<NetEntry> {
    let st = state().lock().unwrap_or_else(|e| e.into_inner());
    st.log
        .iter()
        .rev()
        .find(|e| e.request_id == request_id)
        .cloned()
}

pub fn summary() -> Value {
    let st = state().lock().unwrap_or_else(|e| e.into_inner());
    json!({ "requests": st.log.len(), "media": st.sniffed.len() })
}

#[cfg(test)]
mod tests {
    use super::classify;

    #[test]
    fn classifies_media_and_skips_segments() {
        assert_eq!(
            classify("https://a/x/master.m3u8?t=1", "", "XHR"),
            Some("hls")
        );
        assert_eq!(classify("https://a/seg-1.ts", "video/mp2t", "Media"), None);
        assert_eq!(
            classify("https://a/v.mp4", "video/mp4", "Media"),
            Some("video")
        );
        assert_eq!(
            classify("https://a/s.vtt", "text/vtt", "Other"),
            Some("subtitle")
        );
        assert_eq!(
            classify("https://a/app.js", "application/javascript", "Script"),
            None
        );
        assert_eq!(
            classify("https://a/f.pdf", "application/pdf", "Document"),
            Some("document")
        );
    }
}
