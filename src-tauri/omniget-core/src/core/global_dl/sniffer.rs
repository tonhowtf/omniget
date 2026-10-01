//! What the tabs load. Every response lands in a bounded request log (for
//! connector authors and the `gd_network_log` tool); responses that look like
//! media also land in the sniffed list the Global Downloader screen shows.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
    /// CDP resource type (`Media`, `XHR`, `Fetch`, `Document`…): a player
    /// fetching through MSE asks with XHR/Fetch, a `<video src>` with Media.
    pub resource_type: String,
    pub at: i64,
    /// An HLS playlist with no end (`EXT-X-ENDLIST` missing): a live stream.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub live: bool,
}

#[derive(Default)]
struct State {
    log: VecDeque<NetEntry>,
    sniffed: VecDeque<Sniffed>,
    next_id: u64,
    /// (session, requestId) → what requestWillBeSent / ExtraInfo told us.
    requests: HashMap<(String, String), Pending>,
    /// Dedupe keys of files a sniffed playlist already covers: the variants
    /// of a master and the segments of a media playlist. They never show up
    /// on their own.
    covered: HashSet<String>,
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

    // 204 is an empty answer (ad pixels on a `.m3u8` path among them).
    if !(200..400).contains(&status) || status == 204 {
        return None;
    }
    // Pre-rolls and outstream ads stay in the log, never in the media list.
    if super::blocklist::is_ad_media(&url) {
        return None;
    }
    let kind = classify(&url, &mime, &rtype)?;
    let key = dedupe_key(&url, kind);
    if st.covered.contains(&key)
        || st
            .sniffed
            .iter()
            .any(|s| s.tab_id == tab_id && dedupe_key(&s.url, &s.kind) == key)
    {
        return None;
    }
    if matches!(kind, "video" | "audio") {
        // A player that streams a manifest through MSE fetches its fragments
        // with XHR/Fetch (`bbb_0.m4v`, `seg-12.mp4`); the manifest is the
        // download. A `<video src>` file still counts.
        let streamed = matches!(rtype.as_str(), "XHR" | "Fetch");
        let has_manifest = st
            .sniffed
            .iter()
            .any(|s| s.tab_id == tab_id && (s.kind == "hls" || s.kind == "dash"));
        if streamed && has_manifest {
            return None;
        }
        // The same file from another mirror (PeerTube redundancy, CDN
        // fallbacks) is one download.
        if let Some(name) = distinctive_name(&url) {
            if st.sniffed.iter().any(|s| {
                s.tab_id == tab_id && s.kind == kind && distinctive_name(&s.url) == Some(name)
            }) {
                return None;
            }
        }
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
        resource_type: rtype.clone(),
        at: now,
        live: false,
    };
    if kind == "hls" || kind == "dash" {
        // Fragments sniffed before the manifest showed up.
        st.sniffed.retain(|s| {
            !(s.tab_id == tab_id
                && matches!(s.kind.as_str(), "video" | "audio")
                && matches!(s.resource_type.as_str(), "XHR" | "Fetch"))
        });
    }
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
    // Scripts, styles, fonts and images are never the media, whatever their
    // path says; neither is a page, JSON or XML answer on a media-looking
    // path (a wiki page named `File:x.webm`, a VAST ad XML on `.m3u8`). The
    // extension decides only for types servers get wrong (`text/plain`,
    // `application/octet-stream`).
    if matches!(rtype, "Script" | "Stylesheet" | "Font" | "Image") {
        return None;
    }
    let textual = mime.starts_with("text/html")
        || mime.contains("json")
        || mime.contains("javascript")
        || (mime.contains("xml") && !mime.contains("dash+xml"));
    if mime.contains("mpegurl") || (ext == "m3u8" && !textual) {
        return Some("hls");
    }
    if mime.contains("dash+xml") || (ext == "mpd" && !mime.starts_with("text/html")) {
        return Some("dash");
    }
    if textual {
        return None;
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
            "pdf" | "epub" | "docx" | "pptx" | "xlsx" | "doc" | "ppt" | "xls"
        )
    {
        return Some("document");
    }
    // An archive a script fetched is app data (a player's bundle), not a
    // file the page offers.
    if matches!(ext.as_str(), "zip" | "rar" | "7z") && !matches!(rtype, "XHR" | "Fetch") {
        return Some("document");
    }
    None
}

/// The file name when it is specific enough to identify one file across
/// hosts (`9c9de5e8-…-720-fragmented.mp4`), not a generic one (`index.mp4`,
/// `video.mp4`).
fn distinctive_name(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("");
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    if stem.len() < 16 {
        return None;
    }
    Some(name)
}

/// Reads a sniffed HLS playlist (fetched again by the browser module, the
/// way cat-catch parses what it catches). A master hides its variants, a
/// media playlist hides its segment files (single-file playlists list one
/// `fragmented.mp4` many times), and an answer that is not a playlist at all
/// drops the entry. Returns the ids that left the list.
/// Marks a sniffed playlist as live, once its media playlist showed no end.
pub fn mark_live(id: u64) {
    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = st.sniffed.iter_mut().find(|s| s.id == id) {
        s.live = true;
    }
}

pub fn absorb_playlist(id: u64, body: &[u8]) -> Vec<u64> {
    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(item) = st.sniffed.iter().find(|s| s.id == id).cloned() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(body);
    let mut covered = Vec::new();
    if !text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("#EXTM3U")
    {
        st.sniffed.retain(|s| s.id != id);
        return vec![id];
    }
    match m3u8_rs::parse_playlist_res(body) {
        Ok(m3u8_rs::Playlist::MasterPlaylist(m)) => {
            for v in &m.variants {
                covered.push(join(&item.url, &v.uri));
            }
            for a in &m.alternatives {
                if let Some(u) = &a.uri {
                    covered.push(join(&item.url, u));
                }
            }
        }
        Ok(m3u8_rs::Playlist::MediaPlaylist(p)) => {
            for seg in &p.segments {
                covered.push(join(&item.url, &seg.uri));
                if let Some(m) = &seg.map {
                    covered.push(join(&item.url, &m.uri));
                }
            }
        }
        Err(_) => {
            st.sniffed.retain(|s| s.id != id);
            return vec![id];
        }
    }
    let keys: HashSet<String> = covered
        .iter()
        .flat_map(|u| {
            ["hls", "video", "audio"]
                .into_iter()
                .map(move |k| dedupe_key(u, k))
        })
        .collect();
    let mut removed = Vec::new();
    st.sniffed.retain(|s| {
        let gone =
            s.id != id && s.tab_id == item.tab_id && keys.contains(&dedupe_key(&s.url, &s.kind));
        if gone {
            removed.push(s.id);
        }
        !gone
    });
    if st.covered.len() > 50_000 {
        st.covered.clear();
    }
    st.covered.extend(keys);
    removed
}

fn join(base: &str, rel: &str) -> String {
    url::Url::parse(base)
        .and_then(|b| b.join(rel))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| rel.to_string())
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
    use super::{absorb_playlist, classify, on_response, sniffed};
    use serde_json::json;

    fn respond(tab: &str, rid: &str, url: &str, mime: &str, rtype: &str) -> Option<u64> {
        let p = json!({
            "requestId": rid,
            "type": rtype,
            "response": { "url": url, "mimeType": mime, "status": 200, "headers": {} },
        });
        on_response("s-test", tab, ("https://page/", "Page"), &p).map(|s| s.id)
    }

    #[test]
    fn media_paths_with_page_or_data_answers_are_not_media() {
        // A wiki page named after the file, a VAST ad XML on a playlist path.
        assert_eq!(
            classify(
                "https://commons.wikimedia.org/wiki/File:BBB.webm",
                "text/html",
                "Document"
            ),
            None
        );
        assert_eq!(
            classify("https://ads/x.m3u8?ad=1", "application/xml", "XHR"),
            None
        );
        assert_eq!(
            classify("https://a/x.m3u8", "text/plain", "XHR"),
            Some("hls")
        );
        assert_eq!(
            classify("https://a/v.mpd", "application/xml", "XHR"),
            Some("dash")
        );
        assert_eq!(classify("https://a/x.m3u8", "", "Script"), None);
        // A player's bundle is not a download; a zip the page links is.
        assert_eq!(
            classify("https://a/web_main.zip", "application/zip", "XHR"),
            None
        );
        assert_eq!(
            classify("https://a/files.zip", "application/zip", "Document"),
            Some("document")
        );
    }

    #[test]
    fn fragments_mirrors_and_variants_collapse() {
        let tab = "t-collapse";
        // MSE fragments before the manifest go when it shows up.
        assert!(respond(tab, "1", "https://cdn/bbb/bbb_0.m4v", "video/mp4", "XHR").is_some());
        let mpd = respond(
            tab,
            "2",
            "https://cdn/bbb/bbb.mpd",
            "application/dash+xml",
            "XHR",
        );
        assert!(mpd.is_some());
        assert!(respond(tab, "3", "https://cdn/bbb/bbb_1.m4v", "video/mp4", "XHR").is_none());
        assert_eq!(sniffed(Some(tab)).len(), 1);

        let tab = "t-hls";
        let master = respond(
            tab,
            "1",
            "https://f.org/hls/abc/master.m3u8",
            "application/vnd.apple.mpegurl",
            "XHR",
        )
        .unwrap();
        let variant = respond(
            tab,
            "2",
            "https://f.org/hls/abc/abc-720.m3u8",
            "application/vnd.apple.mpegurl",
            "XHR",
        )
        .unwrap();
        let removed = absorb_playlist(
            master,
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=1280x720\nabc-720.m3u8\n",
        );
        assert_eq!(removed, vec![variant]);
        // The single file the media playlist ranges over, and its mirrors.
        let removed = absorb_playlist(master, b"not a playlist");
        assert_eq!(removed, vec![master]);
        let a = respond(
            tab,
            "3",
            "https://one.org/r/9c9de5e8-0a1e-484a-b099-720-fragmented.mp4",
            "video/mp4",
            "Media",
        );
        let b = respond(
            tab,
            "4",
            "https://two.org/r/9c9de5e8-0a1e-484a-b099-720-fragmented.mp4",
            "video/mp4",
            "Media",
        );
        assert!(a.is_some());
        assert!(b.is_none());
    }

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
