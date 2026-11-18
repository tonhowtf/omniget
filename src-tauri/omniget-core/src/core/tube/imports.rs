use std::io::{Cursor, Read};

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

use super::model::{ImportedSub, ImportedWatch, LocalSub};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Imported {
    pub format: String,
    pub subs: Vec<ImportedSub>,
    pub history: Vec<ImportedWatch>,
    pub playlists: Vec<(String, Vec<String>)>,
}

static CHANNEL_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"UC[\w-]{22}").unwrap());
static CHANNEL_ID_EXACT: Lazy<Regex> = Lazy::new(|| Regex::new(r"^UC[\w-]{22}$").unwrap());
static VIDEO_ID_EXACT: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[\w-]{11}$").unwrap());
static VIDEO_IN_URL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:[?&]v=|youtu\.be/|/shorts/|/embed/|/live/)([\w-]{11})(?:[^\w-]|$)").unwrap()
});

const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";
const MAX_ZIP_ENTRY: u64 = 256 * 1024 * 1024;

pub fn video_id_from(s: &str) -> Option<String> {
    let s = s.trim();
    if VIDEO_ID_EXACT.is_match(s) {
        return Some(s.to_string());
    }
    VIDEO_IN_URL.captures(s).map(|c| c[1].to_string())
}

pub fn channel_id_from(s: &str) -> Option<String> {
    CHANNEL_ID.find(s).map(|m| m.as_str().to_string())
}

impl Imported {
    fn is_empty(&self) -> bool {
        self.subs.is_empty() && self.history.is_empty() && self.playlists.is_empty()
    }

    fn add_sub(&mut self, id: &str, name: &str) {
        if !CHANNEL_ID_EXACT.is_match(id) || self.subs.iter().any(|s| s.channel_id == id) {
            return;
        }
        self.subs.push(ImportedSub {
            channel_id: id.to_string(),
            name: name.trim().to_string(),
        });
    }

    fn merge(&mut self, other: Imported) {
        if self.format.is_empty() || self.format == "unknown" {
            self.format = other.format.clone();
        }
        for s in other.subs {
            self.add_sub(&s.channel_id, &s.name);
        }
        self.history.extend(other.history);
        self.playlists.extend(other.playlists);
    }
}

fn file_stem(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    for suffix in ["-videos", "-vídeos", "-filmy"] {
        if let Some(s) = stem.strip_suffix(suffix) {
            return s.trim().to_string();
        }
    }
    stem.trim().to_string()
}

fn str_of<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| v[*k].as_str())
        .filter(|s| !s.is_empty())
}

fn num_of(v: &Value, key: &str) -> Option<f64> {
    v[key]
        .as_f64()
        .or_else(|| v[key].as_str().and_then(|s| s.parse().ok()))
}

fn freetube_watch(v: &Value) -> Option<ImportedWatch> {
    let video_id = str_of(v, &["videoId"])?.to_string();
    Some(ImportedWatch {
        title: str_of(v, &["title"]).unwrap_or_default().to_string(),
        channel_name: str_of(v, &["author"]).map(str::to_string),
        channel_id: str_of(v, &["authorId"]).map(str::to_string),
        watched_ms: num_of(v, "timeWatched").map(|t| t as i64),
        position_s: num_of(v, "watchProgress").map(|p| p.max(0.0) as u32),
        duration_s: num_of(v, "lengthSeconds").map(|d| d.max(0.0) as u32),
        video_id,
    })
}

const WATCHED_PREFIXES: &[&str] = &[
    "Watched ",
    "Assistiu a ",
    "Assistiu ao ",
    "Assistiu à ",
    "Assistiu ",
    "Has visto ",
    "Viste ",
    "Visto ",
    "A regardé ",
    "Hai guardato ",
];

fn takeout_watch(v: &Value) -> Option<ImportedWatch> {
    if v["details"].as_array().is_some_and(|d| !d.is_empty()) {
        return None;
    }
    let video_id = video_id_from(str_of(v, &["titleUrl"])?)?;
    let raw_title = str_of(v, &["title"]).unwrap_or_default();
    let title = WATCHED_PREFIXES
        .iter()
        .find_map(|p| raw_title.strip_prefix(p))
        .unwrap_or(raw_title);
    let sub = &v["subtitles"][0];
    Some(ImportedWatch {
        video_id,
        title: title.trim().to_string(),
        channel_name: str_of(sub, &["name"]).map(str::to_string),
        channel_id: str_of(sub, &["url"]).and_then(channel_id_from),
        watched_ms: str_of(v, &["time"])
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|d| d.timestamp_millis()),
        position_s: None,
        duration_s: None,
    })
}

fn absorb_json(v: &Value, out: &mut Imported, fmt: &mut Vec<&'static str>) -> bool {
    match v {
        Value::Object(o) => {
            if let Some(subs) = o.get("subscriptions").and_then(Value::as_array) {
                let is_profile = o.contains_key("_id") || o.contains_key("bgColor");
                let mut kind = if is_profile { "freetube" } else { "newpipe" };
                for s in subs {
                    match s {
                        Value::String(id) => {
                            kind = "invidious";
                            out.add_sub(id, "");
                        }
                        _ => {
                            if s["service_id"].as_i64().is_some_and(|id| id != 0) {
                                continue;
                            }
                            let name = str_of(s, &["name", "title"]).unwrap_or_default();
                            if let Some(id) = str_of(s, &["id", "channelId"]) {
                                if !is_profile {
                                    kind = "freetube";
                                }
                                out.add_sub(id, name);
                            } else if let Some(id) = str_of(s, &["url"]).and_then(channel_id_from) {
                                out.add_sub(&id, name);
                            }
                        }
                    }
                }
                if let Some(hist) = o.get("watch_history").and_then(Value::as_array) {
                    for id in hist.iter().filter_map(Value::as_str) {
                        out.history.push(ImportedWatch {
                            video_id: id.to_string(),
                            ..Default::default()
                        });
                    }
                }
                if let Some(pls) = o.get("playlists").and_then(Value::as_array) {
                    for pl in pls {
                        let ids: Vec<String> = pl["videos"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default();
                        if let (Some(t), false) = (str_of(pl, &["title"]), ids.is_empty()) {
                            out.playlists.push((t.to_string(), ids));
                        }
                    }
                }
                fmt.push(kind);
                return true;
            }
            if o.contains_key("videoId")
                && (o.contains_key("timeWatched") || o.contains_key("watchProgress"))
            {
                if let Some(w) = freetube_watch(v) {
                    out.history.push(w);
                }
                fmt.push("freetube_history");
                return true;
            }
            if let (Some(name), Some(videos)) = (
                str_of(v, &["playlistName"]),
                o.get("videos").and_then(Value::as_array),
            ) {
                let ids: Vec<String> = videos
                    .iter()
                    .filter_map(|x| str_of(x, &["videoId"]).map(str::to_string))
                    .collect();
                out.playlists.push((name.to_string(), ids));
                fmt.push("freetube");
                return true;
            }
            false
        }
        Value::Array(items) => {
            let mut known = false;
            for it in items {
                if it.get("titleUrl").is_some() || it.get("header").is_some() {
                    if let Some(w) = takeout_watch(it) {
                        out.history.push(w);
                    }
                    if !known {
                        fmt.push("takeout_history");
                    }
                    known = true;
                } else if let Some(id) = it["snippet"]["resourceId"]["channelId"].as_str() {
                    out.add_sub(id, str_of(&it["snippet"], &["title"]).unwrap_or_default());
                    if !known {
                        fmt.push("youtube_api");
                    }
                    known = true;
                }
            }
            known
        }
        _ => false,
    }
}

fn parse_json_text(text: &str) -> Option<Imported> {
    let mut out = Imported::default();
    let mut fmt = Vec::new();
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if absorb_json(&v, &mut out, &mut fmt) {
            out.format = fmt.first().copied().unwrap_or("unknown").to_string();
            return Some(out);
        }
        return None;
    }
    let mut any = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            any |= absorb_json(&v, &mut out, &mut fmt);
        }
    }
    if !any {
        return None;
    }
    out.format = if fmt.contains(&"freetube") {
        "freetube"
    } else {
        fmt.first().copied().unwrap_or("unknown")
    }
    .to_string();
    Some(out)
}

fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

fn parse_csv(text: &str, file_name: &str) -> Option<Imported> {
    let rows = csv_rows(text);
    let mut out = Imported::default();
    for r in &rows {
        let id = r[0].trim();
        if CHANNEL_ID_EXACT.is_match(id) {
            let name = r.get(2).map(String::as_str).unwrap_or_default();
            out.add_sub(id, name);
        }
    }
    if !out.subs.is_empty() {
        out.format = "takeout_csv".into();
        return Some(out);
    }
    let header = rows
        .iter()
        .position(|r| r.iter().any(|c| c.trim().eq_ignore_ascii_case("video id")))?;
    let col = rows[header]
        .iter()
        .position(|c| c.trim().eq_ignore_ascii_case("video id"))?;
    let ids: Vec<String> = rows[header + 1..]
        .iter()
        .filter_map(|r| r.get(col))
        .map(|c| c.trim())
        .filter(|c| VIDEO_ID_EXACT.is_match(c))
        .map(str::to_string)
        .collect();
    out.playlists.push((file_stem(file_name), ids));
    out.format = "takeout_csv".into();
    Some(out)
}

fn parse_opml(text: &str) -> Option<Imported> {
    let mut out = Imported {
        format: "opml".into(),
        ..Default::default()
    };
    match roxmltree::Document::parse(text) {
        Ok(doc) => {
            if !doc.root_element().has_tag_name("opml") {
                return None;
            }
            for o in doc.descendants().filter(|n| n.has_tag_name("outline")) {
                let Some(id) = o
                    .attribute("xmlUrl")
                    .or_else(|| o.attribute("xmlurl"))
                    .or_else(|| o.attribute("htmlUrl"))
                    .and_then(channel_id_from)
                else {
                    continue;
                };
                let name = o
                    .attribute("title")
                    .or_else(|| o.attribute("text"))
                    .unwrap_or("");
                out.add_sub(&id, name);
            }
        }
        Err(_) => {
            static OUTLINE: Lazy<Regex> =
                Lazy::new(|| Regex::new(r"(?is)<outline\b[^>]*>").unwrap());
            static TITLE: Lazy<Regex> =
                Lazy::new(|| Regex::new(r#"(?i)\b(?:title|text)\s*=\s*"([^"]*)""#).unwrap());
            for m in OUTLINE.find_iter(text) {
                if let Some(id) = channel_id_from(m.as_str()) {
                    let name = TITLE
                        .captures(m.as_str())
                        .map(|c| c[1].to_string())
                        .unwrap_or_default();
                    out.add_sub(&id, &unescape(&name));
                }
            }
        }
    }
    Some(out)
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_history_html(text: &str) -> Option<Imported> {
    static LINK: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"<a href="([^"]+)">([^<]*)</a>"#).unwrap());
    let mut out = Imported {
        format: "takeout_history".into(),
        ..Default::default()
    };
    let mut last: Option<usize> = None;
    for c in LINK.captures_iter(text) {
        let href = unescape(&c[1]);
        let label = unescape(&c[2]);
        let video_id = if href.contains("watch?v=") {
            video_id_from(&href)
        } else {
            None
        };
        if let Some(video_id) = video_id {
            out.history.push(ImportedWatch {
                video_id,
                title: label,
                ..Default::default()
            });
            last = Some(out.history.len() - 1);
        } else if let (Some(i), Some(ch)) = (last.take(), channel_id_from(&href)) {
            out.history[i].channel_id = Some(ch);
            out.history[i].channel_name = Some(label);
        }
    }
    (!out.history.is_empty()).then_some(out)
}

fn parse_links(text: &str, file_name: &str) -> Option<Imported> {
    let mut out = Imported {
        format: "links".into(),
        ..Default::default()
    };
    let mut videos = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some(id) = channel_id_from(line) {
            out.add_sub(&id, "");
        } else if let Some(v) = video_id_from(line) {
            if !videos.contains(&v) {
                videos.push(v);
            }
        }
    }
    if !videos.is_empty() {
        out.playlists.push((file_stem(file_name), videos));
    }
    (!out.is_empty()).then_some(out)
}

fn parse_text(bytes: &[u8], file_name: &str) -> Option<Imported> {
    if bytes.starts_with(SQLITE_MAGIC) {
        return Some(Imported {
            format: "newpipe_db".into(),
            ..Default::default()
        });
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with('{') || text.starts_with('[') {
        if let Some(r) = parse_json_text(text) {
            return Some(r);
        }
    }
    if text.starts_with('<') {
        let head: String = text.chars().take(2048).collect::<String>().to_lowercase();
        if head.contains("<opml") {
            return parse_opml(text);
        }
        return parse_history_html(text);
    }
    let first = text.lines().next().unwrap_or("");
    if first.contains(',') {
        if let Some(r) = parse_csv(text, file_name) {
            return Some(r);
        }
    }
    parse_links(text, file_name)
}

fn parse_zip(bytes: &[u8]) -> Result<Imported, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("bad zip: {e}"))?;
    let mut out = Imported::default();
    let mut has_newpipe_db = false;
    let mut takeout = false;
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        if entry.is_dir() || entry.size() > MAX_ZIP_ENTRY {
            continue;
        }
        let Ok(name) = entry.name().map(|n| n.to_string()) else {
            continue;
        };
        let lower = name.to_lowercase();
        takeout |= lower.starts_with("takeout/");
        let wanted = [".json", ".csv", ".opml", ".xml", ".html", ".txt", ".db"]
            .iter()
            .any(|ext| lower.ends_with(ext));
        if !wanted {
            continue;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        if entry.read_to_end(&mut buf).is_err() {
            continue;
        }
        if buf.starts_with(SQLITE_MAGIC) {
            has_newpipe_db |= lower.ends_with("newpipe.db");
            continue;
        }
        if let Some(part) = parse_text(&buf, &name) {
            out.merge(part);
        }
    }
    if out.is_empty() && has_newpipe_db {
        out.format = "newpipe_db".into();
    } else if takeout && !out.is_empty() {
        out.format = "takeout".into();
    } else if out.format.is_empty() {
        return Err("no importable data in the zip".into());
    }
    Ok(out)
}

pub fn parse_import(bytes: &[u8], file_name: &str) -> Result<Imported, String> {
    if bytes.starts_with(b"PK\x03\x04") {
        return parse_zip(bytes);
    }
    parse_text(bytes, file_name).ok_or_else(|| "unrecognized import file".to_string())
}

pub fn newpipe_db_from_zip(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let idx = (0..zip.len()).find(|i| {
        zip.by_index(*i)
            .map(|e| e.name().to_lowercase().ends_with("newpipe.db"))
            .unwrap_or(false)
    })?;
    let mut entry = zip.by_index(idx).ok()?;
    if entry.size() > MAX_ZIP_ENTRY {
        return None;
    }
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;
    buf.starts_with(SQLITE_MAGIC).then_some(buf)
}

fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn to_opml(subs: &[LocalSub]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"1.1\">\n<head><title>OmniTube subscriptions</title></head>\n<body>\n<outline text=\"YouTube Subscriptions\" title=\"YouTube Subscriptions\">\n",
    );
    for s in subs {
        let name = xml_attr(&s.name);
        out.push_str(&format!(
            "<outline text=\"{name}\" title=\"{name}\" type=\"rss\" xmlUrl=\"https://www.youtube.com/feeds/videos.xml?channel_id={}\"/>\n",
            xml_attr(&s.channel_id)
        ));
    }
    out.push_str("</outline>\n</body>\n</opml>\n");
    out
}

pub fn to_newpipe_json(subs: &[LocalSub]) -> String {
    let list: Vec<Value> = subs
        .iter()
        .map(|s| {
            json!({
                "service_id": 0,
                "url": format!("https://www.youtube.com/channel/{}", s.channel_id),
                "name": s.name,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({
        "app_version": "0.19.8",
        "app_version_int": 953,
        "subscriptions": list,
    }))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const A: &str = "UCaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "UCbbbbbbbbbbbbbbbbbbbbbb";

    fn ids(r: &Imported) -> Vec<&str> {
        r.subs.iter().map(|s| s.channel_id.as_str()).collect()
    }

    #[test]
    fn newpipe_json() {
        let body = format!(
            r#"{{"app_version":"0.27.0","app_version_int":1000,"subscriptions":[
            {{"service_id":0,"url":"https://www.youtube.com/channel/{A}","name":"Alpha"}},
            {{"service_id":1,"url":"https://soundcloud.com/x","name":"SC"}},
            {{"service_id":0,"url":"https://www.youtube.com/channel/{B}","name":"Beta"}}]}}"#
        );
        let r = parse_import(body.as_bytes(), "whatever.bin").unwrap();
        assert_eq!(r.format, "newpipe");
        assert_eq!(ids(&r), vec![A, B]);
        assert_eq!(r.subs[0].name, "Alpha");
    }

    #[test]
    fn freetube_profiles_history_playlists_ndjson() {
        let profiles = format!(
            "{{\"_id\":\"allChannels\",\"name\":\"All Channels\",\"bgColor\":\"#000\",\"textColor\":\"#fff\",\"subscriptions\":[{{\"id\":\"{A}\",\"name\":\"Alpha\",\"thumbnail\":\"x\"}},{{\"id\":\"{B}\",\"name\":\"Beta\"}}]}}\n\
             {{\"_id\":\"p2\",\"name\":\"Music\",\"bgColor\":\"#000\",\"textColor\":\"#fff\",\"subscriptions\":[{{\"id\":\"{A}\",\"name\":\"Alpha\"}}]}}\n"
        );
        let r = parse_import(profiles.as_bytes(), "profiles.db").unwrap();
        assert_eq!(r.format, "freetube");
        assert_eq!(ids(&r), vec![A, B]);

        let history = format!(
            "{{\"videoId\":\"dQw4w9WgXcQ\",\"title\":\"Song\",\"author\":\"Rick\",\"authorId\":\"{A}\",\"lengthSeconds\":212,\"watchProgress\":35.7,\"timeWatched\":1700000000000,\"type\":\"video\",\"_id\":\"dQw4w9WgXcQ\"}}\n\
             {{\"videoId\":\"abcdefghijk\",\"title\":\"Other\",\"timeWatched\":1700000001000,\"watchProgress\":0}}\n"
        );
        let r = parse_import(history.as_bytes(), "history.db").unwrap();
        assert_eq!(r.format, "freetube_history");
        assert_eq!(r.history.len(), 2);
        let h = &r.history[0];
        assert_eq!(h.video_id, "dQw4w9WgXcQ");
        assert_eq!(h.channel_id.as_deref(), Some(A));
        assert_eq!(h.position_s, Some(35));
        assert_eq!(h.duration_s, Some(212));
        assert_eq!(h.watched_ms, Some(1_700_000_000_000));

        let playlists = "{\"playlistName\":\"Favorites\",\"protected\":true,\"videos\":[{\"videoId\":\"dQw4w9WgXcQ\",\"title\":\"x\"}],\"_id\":\"favorites\"}\n";
        let r = parse_import(playlists.as_bytes(), "playlists.db").unwrap();
        assert_eq!(
            r.playlists,
            vec![("Favorites".to_string(), vec!["dQw4w9WgXcQ".to_string()])]
        );
    }

    #[test]
    fn freetube_old_json_and_invidious_and_api() {
        let ft = format!(r#"{{"subscriptions":[{{"id":"{A}","name":"Alpha","thumbnail":null}}]}}"#);
        let r = parse_import(ft.as_bytes(), "subs.json").unwrap();
        assert_eq!(r.format, "freetube");
        assert_eq!(ids(&r), vec![A]);

        let inv = format!(
            r#"{{"subscriptions":["{A}","{B}"],"watch_history":["dQw4w9WgXcQ"],"playlists":[{{"title":"Mine","videos":["abcdefghijk"]}}]}}"#
        );
        let r = parse_import(inv.as_bytes(), "subscription_manager.json").unwrap();
        assert_eq!(r.format, "invidious");
        assert_eq!(ids(&r), vec![A, B]);
        assert_eq!(r.history[0].video_id, "dQw4w9WgXcQ");
        assert_eq!(r.playlists[0].0, "Mine");

        let api = format!(
            r#"[{{"kind":"youtube#subscription","snippet":{{"title":"Alpha","resourceId":{{"kind":"youtube#channel","channelId":"{A}"}}}}}}]"#
        );
        let r = parse_import(api.as_bytes(), "subscriptions.json").unwrap();
        assert_eq!(r.format, "youtube_api");
        assert_eq!(r.subs[0].name, "Alpha");
    }

    #[test]
    fn takeout_subscriptions_csv_by_content() {
        let csv = format!(
            "Id do canal,URL do canal,Título do canal\n{A},http://www.youtube.com/channel/{A},\"Alpha, the \"\"best\"\"\"\n{B},http://www.youtube.com/channel/{B},Beta\n"
        );
        let r = parse_import(csv.as_bytes(), "inscrições.csv").unwrap();
        assert_eq!(r.format, "takeout_csv");
        assert_eq!(ids(&r), vec![A, B]);
        assert_eq!(r.subs[0].name, "Alpha, the \"best\"");
    }

    #[test]
    fn takeout_playlist_csv() {
        let csv = "Video ID,Playlist Video Creation Timestamp\r\ndQw4w9WgXcQ,2024-01-01T00:00:00+00:00\r\nabcdefghijk,2024-01-02T00:00:00+00:00\r\n";
        let r = parse_import(
            csv.as_bytes(),
            "Takeout/YouTube/playlists/Watch later-videos.csv",
        )
        .unwrap();
        assert_eq!(
            r.playlists,
            vec![(
                "Watch later".to_string(),
                vec!["dQw4w9WgXcQ".to_string(), "abcdefghijk".to_string()]
            )]
        );
    }

    #[test]
    fn takeout_watch_history_json() {
        let body = format!(
            r#"[
            {{"header":"YouTube","title":"Watched Never Gonna Give You Up","titleUrl":"https://www.youtube.com/watch?v=dQw4w9WgXcQ","subtitles":[{{"name":"Rick Astley","url":"https://www.youtube.com/channel/{A}"}}],"time":"2024-05-01T10:00:00.000Z","products":["YouTube"]}},
            {{"header":"YouTube","title":"Assistiu a Vídeo","titleUrl":"https://www.youtube.com/watch?v=abcdefghijk","time":"2024-05-02T10:00:00Z"}},
            {{"header":"YouTube","title":"Watched an ad","titleUrl":"https://www.youtube.com/watch?v=zzzzzzzzzzz","details":[{{"name":"From Google Ads"}}],"time":"2024-05-02T10:00:00Z"}},
            {{"header":"YouTube","title":"Watched a video that has been removed","time":"2024-05-03T10:00:00Z"}}
        ]"#
        );
        let r = parse_import(body.as_bytes(), "histórico.json").unwrap();
        assert_eq!(r.format, "takeout_history");
        assert_eq!(r.history.len(), 2);
        assert_eq!(r.history[0].title, "Never Gonna Give You Up");
        assert_eq!(r.history[0].channel_id.as_deref(), Some(A));
        assert_eq!(r.history[0].channel_name.as_deref(), Some("Rick Astley"));
        assert_eq!(r.history[0].watched_ms, Some(1_714_557_600_000));
        assert_eq!(r.history[1].title, "Vídeo");
    }

    #[test]
    fn takeout_watch_history_html() {
        let html = format!(
            r#"<html><body><div class="outer-cell"><div class="content-cell">Watched&nbsp;<a href="https://www.youtube.com/watch?v=dQw4w9WgXcQ">Rick &amp; Roll</a><br><a href="https://www.youtube.com/channel/{A}">Rick</a><br>May 1, 2024</div></div></body></html>"#
        );
        let r = parse_import(html.as_bytes(), "watch-history.html").unwrap();
        assert_eq!(r.history.len(), 1);
        assert_eq!(r.history[0].title, "Rick & Roll");
        assert_eq!(r.history[0].channel_id.as_deref(), Some(A));
    }

    #[test]
    fn opml_well_formed_and_broken() {
        let opml = format!(
            r#"<?xml version="1.0"?><opml version="1.1"><body><outline text="YouTube Subscriptions"><outline text="Alpha &amp; Co" title="Alpha &amp; Co" type="rss" xmlUrl="https://www.youtube.com/feeds/videos.xml?channel_id={A}"/></outline></body></opml>"#
        );
        let r = parse_import(opml.as_bytes(), "subs.xml").unwrap();
        assert_eq!(r.format, "opml");
        assert_eq!(r.subs[0].name, "Alpha & Co");
        let broken = format!(
            "<opml><body><outline title=\"Beta\" xmlUrl=\"https://www.youtube.com/feeds/videos.xml?channel_id={B}\"></body>"
        );
        let r = parse_import(broken.as_bytes(), "x.opml").unwrap();
        assert_eq!(ids(&r), vec![B]);
        assert_eq!(r.subs[0].name, "Beta");
    }

    #[test]
    fn links_list() {
        let text = format!(
            "# my list\nhttps://www.youtube.com/channel/{A}\nhttps://youtu.be/dQw4w9WgXcQ?t=1\nhttps://www.youtube.com/watch?v=abcdefghijk&list=PL1\nabcdefghijk\nhttps://www.youtube.com/shorts/short-id_01\n"
        );
        let r = parse_import(text.as_bytes(), "later.txt").unwrap();
        assert_eq!(r.format, "links");
        assert_eq!(ids(&r), vec![A]);
        assert_eq!(r.playlists[0].0, "later");
        assert_eq!(
            r.playlists[0].1,
            vec!["dQw4w9WgXcQ", "abcdefghijk", "short-id_01"]
        );
        assert!(parse_import(b"hello world", "x.txt").is_err());
        assert!(parse_import(b"", "x.txt").is_err());
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in files {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn takeout_zip_finds_files_inside() {
        let subs = format!("Channel Id,Channel Url,Channel Title\n{A},x,Alpha\n");
        let hist = r#"[{"header":"YouTube","title":"Watched X","titleUrl":"https://www.youtube.com/watch?v=dQw4w9WgXcQ","time":"2024-05-01T10:00:00Z"}]"#;
        let data = zip_of(&[
            (
                "Takeout/YouTube e YouTube Music/inscrições/inscrições.csv",
                subs.as_bytes(),
            ),
            (
                "Takeout/YouTube e YouTube Music/histórico/histórico-de-visualização.json",
                hist.as_bytes(),
            ),
            (
                "Takeout/YouTube e YouTube Music/vídeos/clip.mp4",
                b"\x00\x00",
            ),
        ]);
        let r = parse_import(&data, "takeout-2026.zip").unwrap();
        assert_eq!(r.format, "takeout");
        assert_eq!(ids(&r), vec![A]);
        assert_eq!(r.history.len(), 1);
    }

    #[test]
    fn newpipe_zip_hands_sqlite_over() {
        let mut db = SQLITE_MAGIC.to_vec();
        db.extend_from_slice(&[0u8; 32]);
        let data = zip_of(&[("newpipe.db", &db), ("newpipe.settings", b"\xac\xed")]);
        let r = parse_import(&data, "NewPipeData.zip").unwrap();
        assert_eq!(r.format, "newpipe_db");
        assert!(r.subs.is_empty());
        assert_eq!(newpipe_db_from_zip(&data).unwrap(), db);
        assert!(parse_import(&zip_of(&[("readme.md", b"hi")]), "x.zip").is_err());
    }

    #[test]
    fn exports_round_trip() {
        let subs = vec![
            LocalSub {
                channel_id: A.into(),
                name: "Alpha & \"Co\"".into(),
                added_ms: 1,
                ..Default::default()
            },
            LocalSub {
                channel_id: B.into(),
                name: "Beta".into(),
                added_ms: 2,
                ..Default::default()
            },
        ];
        let opml = to_opml(&subs);
        let back = parse_import(opml.as_bytes(), "out.opml").unwrap();
        assert_eq!(ids(&back), vec![A, B]);
        assert_eq!(back.subs[0].name, "Alpha & \"Co\"");
        let np = to_newpipe_json(&subs);
        let back = parse_import(np.as_bytes(), "out.json").unwrap();
        assert_eq!(back.format, "newpipe");
        assert_eq!(ids(&back), vec![A, B]);
    }

    #[test]
    fn id_helpers() {
        assert_eq!(
            video_id_from("https://m.youtube.com/watch?feature=x&v=dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            video_id_from("https://www.youtube.com/embed/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert!(video_id_from("https://www.youtube.com/watch?v=short").is_none());
        assert_eq!(
            channel_id_from(&format!("https://youtube.com/channel/{A}/videos")).as_deref(),
            Some(A)
        );
    }
}
