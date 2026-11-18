//! `crates/innertube/clients.json` (both GPL-3.0), checked live on

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};

const EMBEDDED_CLIENTS: &str = include_str!("clients.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Www,
    Music,
}

impl Host {
    pub fn origin(self) -> &'static str {
        match self {
            Host::Www => "https://www.youtube.com",
            Host::Music => "https://music.youtube.com",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientDef {
    pub name: String,
    pub id: u32,
    pub version: String,
    pub user_agent: String,
    pub host: String,
    #[serde(default)]
    pub context: Value,
    #[serde(default)]
    pub supports_cookies: bool,
    #[serde(default)]
    pub embed: bool,
}

impl ClientDef {
    pub fn origin(&self) -> String {
        format!("https://{}", self.host)
    }
}

pub const PLAYER_LADDER: &[&str] = &["VISIONOS", "ANDROID_VR"];
pub const MUSIC_PLAYER_LADDER: &[&str] = &["VISIONOS", "ANDROID_VR"];

fn parse_clients(text: &str) -> Result<Vec<ClientDef>, String> {
    serde_json::from_str::<Vec<ClientDef>>(text).map_err(|e| e.to_string())
}

static EMBEDDED: Lazy<Vec<ClientDef>> =
    Lazy::new(|| parse_clients(EMBEDDED_CLIENTS).expect("embedded clients.json is valid"));

static OVERRIDE: Lazy<RwLock<HashMap<String, ClientDef>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub fn set_clients_override(path: PathBuf) {
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    match parse_clients(&text) {
        Ok(list) => {
            let map = list
                .into_iter()
                .map(|c| (c.name.to_ascii_uppercase(), c))
                .collect();
            if let Ok(mut guard) = OVERRIDE.write() {
                *guard = map;
            }
        }
        Err(e) => tracing::warn!("[tube] ignoring clients override {}: {e}", path.display()),
    }
}

pub fn client(name: &str) -> Option<ClientDef> {
    let key = name.to_ascii_uppercase();
    if let Ok(guard) = OVERRIDE.read() {
        if let Some(c) = guard.get(&key) {
            return Some(c.clone());
        }
    }
    EMBEDDED
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .cloned()
}

pub fn context(c: &ClientDef, visitor_data: Option<&str>, hl: &str, gl: &str) -> Value {
    let mut client = Map::new();
    client.insert("clientName".into(), json!(c.name));
    client.insert("clientVersion".into(), json!(c.version));
    client.insert("hl".into(), json!(hl));
    client.insert("gl".into(), json!(gl));
    client.insert("utcOffsetMinutes".into(), json!(0));
    if let Some(extra) = c.context.as_object() {
        for (k, v) in extra {
            client.insert(k.clone(), v.clone());
        }
    }
    if let Some(v) = visitor_data.filter(|v| !v.is_empty()) {
        client.insert("visitorData".into(), json!(v));
    }
    let mut ctx = json!({
        "client": Value::Object(client),
        "user": { "lockedSafetyMode": false },
        "request": { "useSsl": true, "internalExperimentFlags": [] },
    });
    if c.embed {
        ctx["thirdParty"] = json!({ "embedUrl": "https://www.youtube.com/" });
    }
    json!({ "context": ctx })
}

fn with(mut body: Value, fields: &[(&str, Option<Value>)]) -> Value {
    if let Some(obj) = body.as_object_mut() {
        for (k, v) in fields {
            if let Some(v) = v {
                obj.insert((*k).to_string(), v.clone());
            }
        }
    }
    body
}

pub fn browse_body(
    c: &ClientDef,
    visitor: Option<&str>,
    hl: &str,
    gl: &str,
    browse_id: &str,
    params: Option<&str>,
) -> Value {
    with(
        context(c, visitor, hl, gl),
        &[
            ("browseId", Some(json!(browse_id))),
            ("params", params.map(|p| json!(p))),
        ],
    )
}

pub fn continuation_body(
    c: &ClientDef,
    visitor: Option<&str>,
    hl: &str,
    gl: &str,
    token: &str,
) -> Value {
    with(
        context(c, visitor, hl, gl),
        &[("continuation", Some(json!(token)))],
    )
}

pub fn search_body(
    c: &ClientDef,
    visitor: Option<&str>,
    hl: &str,
    gl: &str,
    query: &str,
    params: Option<&str>,
) -> Value {
    with(
        context(c, visitor, hl, gl),
        &[
            ("query", Some(json!(query))),
            ("params", params.map(|p| json!(p))),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn next_body(
    c: &ClientDef,
    visitor: Option<&str>,
    hl: &str,
    gl: &str,
    video_id: Option<&str>,
    playlist_id: Option<&str>,
    index: Option<u32>,
    params: Option<&str>,
) -> Value {
    with(
        context(c, visitor, hl, gl),
        &[
            ("videoId", video_id.map(|v| json!(v))),
            ("playlistId", playlist_id.map(|v| json!(v))),
            ("index", index.map(|v| json!(v))),
            ("params", params.map(|v| json!(v))),
            ("contentCheckOk", Some(json!(true))),
            ("racyCheckOk", Some(json!(true))),
        ],
    )
}

pub fn player_body(
    c: &ClientDef,
    visitor: Option<&str>,
    hl: &str,
    gl: &str,
    video_id: &str,
    sts: Option<u32>,
) -> Value {
    let mut playback = json!({ "html5Preference": "HTML5_PREF_WANTS" });
    if let Some(sts) = sts {
        playback["signatureTimestamp"] = json!(sts);
    }
    with(
        context(c, visitor, hl, gl),
        &[
            ("videoId", Some(json!(video_id))),
            ("contentCheckOk", Some(json!(true))),
            ("racyCheckOk", Some(json!(true))),
            (
                "playbackContext",
                Some(json!({ "contentPlaybackContext": playback })),
            ),
        ],
    )
}

pub fn headers(c: &ClientDef, visitor: Option<&str>) -> Vec<(String, String)> {
    let origin = c.origin();
    let mut out = vec![
        ("User-Agent".to_string(), c.user_agent.clone()),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("X-YouTube-Client-Name".to_string(), c.id.to_string()),
        ("X-YouTube-Client-Version".to_string(), c.version.clone()),
        ("Origin".to_string(), origin.clone()),
        ("X-Origin".to_string(), origin.clone()),
        ("Referer".to_string(), format!("{origin}/")),
    ];
    if let Some(v) = visitor.filter(|v| !v.is_empty()) {
        out.push(("X-Goog-Visitor-Id".to_string(), v.to_string()));
    }
    out
}

pub fn endpoint_url(c: &ClientDef, endpoint: &str) -> String {
    format!(
        "https://{}/youtubei/v1/{}?prettyPrint=false",
        c.host,
        endpoint.trim_start_matches('/')
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct CookieEntry {
    pub domain: String,
    pub include_sub: bool,
    pub path: String,
    pub secure: bool,
    pub expires: i64,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CookieJar {
    pub entries: Vec<CookieEntry>,
}

const PROTECTED_COOKIES: [&str; 9] = [
    "SID",
    "HSID",
    "SSID",
    "APISID",
    "SAPISID",
    "__Secure-1PSID",
    "__Secure-3PSID",
    "__Secure-1PAPISID",
    "__Secure-3PAPISID",
];

fn allowed_domain(bare: &str) -> bool {
    bare == "youtube.com"
        || bare.ends_with(".youtube.com")
        || bare == "google.com"
        || bare.ends_with(".google.com")
}

fn bare_domain(d: &str) -> String {
    d.trim_start_matches('.').to_ascii_lowercase()
}

impl CookieJar {
    pub fn from_netscape(text: &str) -> Self {
        let mut entries = Vec::new();
        for raw in text.lines() {
            let line = raw.trim_end_matches('\r');
            let line = match line.strip_prefix("#HttpOnly_") {
                Some(rest) => rest,
                None if line.starts_with('#') || line.trim().is_empty() => continue,
                None => line,
            };
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 7 {
                continue;
            }
            let bare = bare_domain(f[0]);
            if !allowed_domain(&bare) {
                continue;
            }
            entries.push(CookieEntry {
                domain: f[0].to_string(),
                include_sub: f[1].eq_ignore_ascii_case("TRUE"),
                path: f[2].to_string(),
                secure: f[3].eq_ignore_ascii_case("TRUE"),
                expires: f[4].trim().parse().unwrap_or(0),
                name: f[5].to_string(),
                value: f[6..].join("\t"),
            });
        }
        Self { entries }
    }
    fn entry(&self, name: &str) -> Option<&CookieEntry> {
        let mut best: Option<&CookieEntry> = None;
        for e in self.entries.iter().filter(|e| e.name == name) {
            let yt = bare_domain(&e.domain).ends_with("youtube.com");
            match best {
                Some(b) if bare_domain(&b.domain).ends_with("youtube.com") || !yt => {}
                _ => best = Some(e),
            }
        }
        best
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entry(name).map(|e| e.value.as_str())
    }

    pub fn header(&self) -> String {
        let mut seen: Vec<&str> = Vec::new();
        let mut parts = Vec::new();
        for e in &self.entries {
            if !bare_domain(&e.domain).ends_with("youtube.com") || seen.contains(&e.name.as_str()) {
                continue;
            }
            seen.push(&e.name);
            if let Some(best) = self.entry(&e.name) {
                parts.push(format!("{}={}", best.name, best.value));
            }
        }
        parts.join("; ")
    }

    pub fn is_logged_in(&self) -> bool {
        self.get("SAPISID").is_some() || self.get("__Secure-3PAPISID").is_some()
    }

    pub fn merge_set_cookie_from(&mut self, host: &str, values: &[String], now_s: i64) -> bool {
        let host_bare = bare_domain(host);
        let mut changed = false;
        for raw in values {
            let Some(sc) = parse_set_cookie(raw, now_s) else {
                continue;
            };
            let bare = sc
                .domain
                .as_deref()
                .map(bare_domain)
                .unwrap_or_else(|| host_bare.clone());
            if !allowed_domain(&bare)
                || !(host_bare == bare || host_bare.ends_with(&format!(".{bare}")))
            {
                continue;
            }
            let pos = self
                .entries
                .iter()
                .position(|e| e.name == sc.name && bare_domain(&e.domain) == bare);
            if sc.delete {
                if PROTECTED_COOKIES.contains(&sc.name.as_str()) {
                    continue;
                }
                if let Some(i) = pos {
                    self.entries.remove(i);
                    changed = true;
                }
                continue;
            }
            match pos {
                Some(i) => {
                    let e = &mut self.entries[i];
                    if e.value != sc.value {
                        e.value = sc.value;
                        changed = true;
                    }
                    e.expires = sc.expires;
                }
                None => {
                    self.entries.push(CookieEntry {
                        domain: format!(".{bare}"),
                        include_sub: true,
                        path: sc.path.unwrap_or_else(|| "/".into()),
                        secure: sc.secure,
                        expires: sc.expires,
                        name: sc.name,
                        value: sc.value,
                    });
                    changed = true;
                }
            }
        }
        changed
    }
}

struct SetCookie {
    name: String,
    value: String,
    domain: Option<String>,
    path: Option<String>,
    secure: bool,
    expires: i64,
    delete: bool,
}

fn parse_cookie_date(s: &str) -> Option<i64> {
    let cleaned = s.trim().replace('-', " ");
    let cleaned = cleaned
        .trim_end_matches("GMT")
        .trim_end_matches("UTC")
        .trim();
    let without_day = cleaned
        .split_once(',')
        .map(|(_, r)| r.trim())
        .unwrap_or(cleaned);
    chrono::NaiveDateTime::parse_from_str(without_day, "%d %b %Y %H:%M:%S")
        .ok()
        .map(|d| d.and_utc().timestamp())
}

fn parse_set_cookie(raw: &str, now_s: i64) -> Option<SetCookie> {
    let mut parts = raw.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut sc = SetCookie {
        name: name.to_string(),
        value: value.trim().trim_matches('"').to_string(),
        domain: None,
        path: None,
        secure: false,
        expires: 0,
        delete: false,
    };
    let mut max_age: Option<i64> = None;
    let mut expires: Option<i64> = None;
    for attr in parts {
        let (k, v) = attr.split_once('=').unwrap_or((attr, ""));
        match k.trim().to_ascii_lowercase().as_str() {
            "domain" if !v.trim().is_empty() => sc.domain = Some(v.trim().to_string()),
            "path" => sc.path = Some(v.trim().to_string()),
            "secure" => sc.secure = true,
            "max-age" => max_age = v.trim().parse().ok(),
            "expires" => expires = parse_cookie_date(v),
            _ => {}
        }
    }
    if let Some(ma) = max_age {
        sc.delete = ma <= 0;
        sc.expires = now_s.saturating_add(ma);
    } else if let Some(ts) = expires {
        sc.delete = ts <= now_s;
        sc.expires = ts;
    }
    Some(sc)
}

fn sid_hash(scheme: &str, sid: &str, origin: &str, now_s: u64) -> String {
    let mut h = Sha1::new();
    h.update(format!("{now_s} {sid} {origin}").as_bytes());
    format!("{scheme} {now_s}_{}", hex::encode(h.finalize()))
}

pub fn auth_header(jar: &CookieJar, origin: &str, now_s: u64) -> Option<String> {
    let sapisid = jar
        .get("SAPISID")
        .or_else(|| jar.get("__Secure-3PAPISID"))?;
    let mut parts = vec![sid_hash("SAPISIDHASH", sapisid, origin, now_s)];
    if let Some(p1) = jar.get("__Secure-1PAPISID") {
        parts.push(sid_hash("SAPISID1PHASH", p1, origin, now_s));
    }
    if let Some(p3) = jar.get("__Secure-3PAPISID") {
        parts.push(sid_hash("SAPISID3PHASH", p3, origin, now_s));
    }
    Some(parts.join(" "))
}

fn find_visitor(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if s.starts_with("Cg") && s.len() >= 16 && !s.contains(' ') => {
            Some(s.clone())
        }
        Value::Array(items) => items.iter().find_map(find_visitor),
        _ => None,
    }
}

pub fn parse_visitor_data_swjs(body: &str) -> Option<String> {
    let json_start = body.find('[')?;
    let v: Value = serde_json::from_str(body[json_start..].trim()).ok()?;
    v.get(0)
        .and_then(|x| x.get(2))
        .and_then(find_visitor)
        .or_else(|| find_visitor(&v))
}

pub fn search_params(filter: &str) -> Option<&'static str> {
    match filter {
        "video" | "videos" => Some("EgIQAQ=="),
        "short" | "shorts" => Some("EgIQCQ=="),
        "channel" | "channels" => Some("EgIQAg=="),
        "playlist" | "playlists" => Some("EgIQAw=="),
        "live" => Some("EgJAAQ=="),
        "movie" | "movies" => Some("EgIQBA=="),
        _ => None,
    }
}

pub fn music_search_params(filter: &str) -> Option<&'static str> {
    match filter {
        "songs" | "song" => Some("EgWKAQIIAWoKEAkQBRAKEAMQBA=="),
        "videos" | "video" => Some("EgWKAQIQAWoKEAkQChAFEAMQBA=="),
        "albums" | "album" => Some("EgWKAQIYAWoKEAkQChAFEAMQBA=="),
        "artists" | "artist" => Some("EgWKAQIgAWoKEAkQChAFEAMQBA=="),
        "playlists" | "featured_playlists" => Some("EgeKAQQoADgBagwQDhAKEAMQBRAJEAQ="),
        "community_playlists" => Some("EgeKAQQoAEABagoQAxAEEAoQCRAF"),
        "podcasts" | "podcast" => Some("EgWKAQJQAWoIEBAQERADEBU="),
        _ => None,
    }
}

pub fn channel_tab_params(tab: &str) -> Option<&'static str> {
    match tab {
        "videos" => Some("EgZ2aWRlb3PyBgQKAjoA"),
        "shorts" => Some("EgZzaG9ydHPyBgUKA5oBAA=="),
        "streams" | "live" => Some("EgdzdHJlYW1z8gYECgJ6AA=="),
        "playlists" => Some("EglwbGF5bGlzdHPyBgQKAkIA"),
        "podcasts" => Some("Eghwb2RjYXN0c_IGBQoDugEA"),
        "releases" => Some("EghyZWxlYXNlc_IGBQoDsgEA"),
        "community" | "posts" => Some("EgVwb3N0c_IGBAoCSgA="),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_clients_parse_and_resolve() {
        for name in [
            "VISIONOS",
            "ANDROID_VR",
            "WEB",
            "WEB_REMIX",
            "ANDROID_MUSIC",
            "IOS_MUSIC",
            "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
        ] {
            let c = client(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(c.name, name);
            assert!(!c.version.is_empty());
        }
        assert_eq!(client("web_remix").unwrap().id, 67);
        assert_eq!(client("VISIONOS").unwrap().id, 101);
        assert!(client("WEB").unwrap().supports_cookies);
        assert!(!client("VISIONOS").unwrap().supports_cookies);
        for name in PLAYER_LADDER.iter().chain(MUSIC_PLAYER_LADDER) {
            assert!(client(name).is_some());
        }
    }

    #[test]
    fn override_replaces_by_name() {
        let dir = std::env::temp_dir().join(format!("tube-clients-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clients.json");
        std::fs::write(
            &path,
            r#"[{"name":"IOS_MUSIC","id":26,"version":"9.99","user_agent":"x","host":"music.youtube.com"}]"#,
        )
        .unwrap();
        set_clients_override(path);
        assert_eq!(client("IOS_MUSIC").unwrap().version, "9.99");
        assert_eq!(client("WEB").unwrap().id, 1);
        OVERRIDE.write().unwrap().clear();
        assert_eq!(client("IOS_MUSIC").unwrap().version, "7.01.05");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn player_body_has_context_and_checks() {
        let c = client("VISIONOS").unwrap();
        let b = player_body(&c, Some("CgtVISITOR"), "en", "US", "dQw4w9WgXcQ", None);
        let cl = &b["context"]["client"];
        assert_eq!(cl["clientName"], "VISIONOS");
        assert_eq!(cl["clientVersion"], "1.02");
        assert_eq!(cl["deviceModel"], "RealityDevice17,1");
        assert_eq!(cl["visitorData"], "CgtVISITOR");
        assert_eq!(cl["hl"], "en");
        assert_eq!(b["videoId"], "dQw4w9WgXcQ");
        assert_eq!(b["contentCheckOk"], true);
        assert_eq!(b["racyCheckOk"], true);
        assert!(b["playbackContext"]["contentPlaybackContext"]
            .get("signatureTimestamp")
            .is_none());
        let with_sts = player_body(&c, None, "en", "US", "x", Some(20000));
        assert_eq!(
            with_sts["playbackContext"]["contentPlaybackContext"]["signatureTimestamp"],
            20000
        );
        assert!(with_sts["context"]["client"].get("visitorData").is_none());
    }

    #[test]
    fn embedded_client_sends_third_party() {
        let c = client("TVHTML5_SIMPLY_EMBEDDED_PLAYER").unwrap();
        let b = context(&c, None, "en", "US");
        assert_eq!(
            b["context"]["thirdParty"]["embedUrl"],
            "https://www.youtube.com/"
        );
        let web = client("WEB").unwrap();
        assert!(context(&web, None, "en", "US")["context"]
            .get("thirdParty")
            .is_none());
    }

    #[test]
    fn other_bodies() {
        let c = client("WEB").unwrap();
        let b = browse_body(&c, None, "pt", "BR", "FEwhat_to_watch", None);
        assert_eq!(b["browseId"], "FEwhat_to_watch");
        assert!(b.get("params").is_none());
        assert_eq!(b["context"]["client"]["gl"], "BR");
        let s = search_body(&c, None, "en", "US", "cats", search_params("video"));
        assert_eq!(s["query"], "cats");
        assert_eq!(s["params"], "EgIQAQ==");
        let n = next_body(
            &c,
            None,
            "en",
            "US",
            Some("abc"),
            Some("PL1"),
            Some(3),
            None,
        );
        assert_eq!(n["videoId"], "abc");
        assert_eq!(n["playlistId"], "PL1");
        assert_eq!(n["index"], 3);
        let k = continuation_body(&c, None, "en", "US", "TOKEN");
        assert_eq!(k["continuation"], "TOKEN");
    }

    #[test]
    fn headers_and_url() {
        let c = client("WEB_REMIX").unwrap();
        let h = headers(&c, Some("Cgxyz"));
        let get = |k: &str| h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("X-YouTube-Client-Name"), Some("67"));
        assert_eq!(get("Origin"), Some("https://music.youtube.com"));
        assert_eq!(get("Referer"), Some("https://music.youtube.com/"));
        assert_eq!(get("X-Goog-Visitor-Id"), Some("Cgxyz"));
        assert!(headers(&c, None)
            .iter()
            .all(|(k, _)| k != "X-Goog-Visitor-Id"));
        assert_eq!(
            endpoint_url(&c, "browse"),
            "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false"
        );
        assert_eq!(Host::Music.origin(), "https://music.youtube.com");
    }

    const JAR: &str = "# Netscape HTTP Cookie File\n\
.youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tabc/def\n\
.youtube.com\tTRUE\t/\tTRUE\t1900000000\t__Secure-1PAPISID\tone\n\
.youtube.com\tTRUE\t/\tTRUE\t1900000000\t__Secure-3PAPISID\tthree\n\
#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1900000000\tSID\tsid-yt\n\
.google.com\tTRUE\t/\tTRUE\t1900000000\tSID\tsid-google\n\
.example.com\tTRUE\t/\tFALSE\t0\tX\ty\n";

    #[test]
    fn jar_parses_and_filters_domains() {
        let jar = CookieJar::from_netscape(JAR);
        assert_eq!(jar.entries.len(), 5);
        assert_eq!(jar.get("SID"), Some("sid-yt"));
        assert!(jar.get("X").is_none());
        assert!(jar.is_logged_in());
        let header = jar.header();
        assert!(header.contains("SAPISID=abc/def"));
        assert!(header.contains("SID=sid-yt"));
        assert!(!header.contains("sid-google"));
        assert!(!CookieJar::default().is_logged_in());
    }

    #[test]
    fn sapisidhash_known_vector() {
        let jar = CookieJar::from_netscape(JAR);
        let h = auth_header(&jar, "https://www.youtube.com", 1_700_000_000).unwrap();
        let expect = |sid: &str| {
            let mut s = Sha1::new();
            s.update(format!("1700000000 {sid} https://www.youtube.com").as_bytes());
            hex::encode(s.finalize())
        };
        assert_eq!(
            h,
            format!(
                "SAPISIDHASH 1700000000_{} SAPISID1PHASH 1700000000_{} SAPISID3PHASH 1700000000_{}",
                expect("abc/def"),
                expect("one"),
                expect("three")
            )
        );
        let only = CookieJar::from_netscape(".youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tabc\n");
        assert_eq!(
            auth_header(&only, "https://www.youtube.com", 1).unwrap(),
            "SAPISIDHASH 1_76945995212dc850298a9feccea18a02639c6f64"
        );
        assert!(auth_header(&CookieJar::default(), "https://www.youtube.com", 1).is_none());
        let three_only =
            CookieJar::from_netscape(".youtube.com\tTRUE\t/\tTRUE\t0\t__Secure-3PAPISID\tq\n");
        let h = auth_header(&three_only, "https://music.youtube.com", 5).unwrap();
        assert!(h.starts_with("SAPISIDHASH 5_"));
        assert!(h.contains(" SAPISID3PHASH 5_"));
        assert!(!h.contains("SAPISID1PHASH"));
    }

    #[test]
    fn merge_set_cookie_rotates_and_protects() {
        let mut jar = CookieJar::from_netscape(JAR);
        let changed = jar.merge_set_cookie_from(
            "www.youtube.com",
            &[
                "SID=new; Domain=.youtube.com; Path=/; Secure; Max-Age=3600".into(),
                "SAPISID=; Domain=.youtube.com; Max-Age=0".into(),
                "__Secure-1PSIDTS=ts1; Domain=.youtube.com; Path=/; Expires=Wed, 30-Sep-2037 12:00:00 GMT; Secure".into(),
                "EVIL=1; Domain=.example.com".into(),
                "GOOGLE=1; Domain=.google.com".into(),
                "VISITOR_INFO1_LIVE=v; Path=/".into(),
            ],
            1_000,
        );
        assert!(changed);
        assert_eq!(jar.get("SID"), Some("new"));
        assert_eq!(jar.get("SAPISID"), Some("abc/def"));
        let ts = jar
            .entries
            .iter()
            .find(|e| e.name == "__Secure-1PSIDTS")
            .unwrap();
        assert_eq!(ts.expires, 2_137_924_800);
        assert!(jar.get("EVIL").is_none());
        assert!(jar.entries.iter().all(|e| e.name != "GOOGLE"));
        let vis = jar
            .entries
            .iter()
            .find(|e| e.name == "VISITOR_INFO1_LIVE")
            .unwrap();
        assert_eq!(vis.domain, ".www.youtube.com");
        assert!(jar.merge_set_cookie_from(
            "www.youtube.com",
            &[
                "__Secure-1PSIDTS=x; Domain=.youtube.com; Expires=Thu, 01 Jan 1970 00:00:01 GMT"
                    .into()
            ],
            1_000,
        ));
        assert!(jar.get("__Secure-1PSIDTS").is_none());
        assert_eq!(jar.get("SID"), Some("new"));
        assert!(!jar.merge_set_cookie_from("www.youtube.com", &["SSID=; Max-Age=0".into()], 1));
    }

    #[test]
    fn visitor_from_swjs() {
        let body = ")]}'\n[[\"yt.sw.adr\",null,[[[\"pt\",\"BR\",null,\"1.2.3.4\",null,null,null,null,null,null,null,\"\",\"\",\"CgtnMFhMLW16bXFDdyjfj_LVBjIKCgJCUhIEGgAgPQ%3D%3D\"]]]]]";
        assert_eq!(
            parse_visitor_data_swjs(body).as_deref(),
            Some("CgtnMFhMLW16bXFDdyjfj_LVBjIKCgJCUhIEGgAgPQ%3D%3D")
        );
        assert!(parse_visitor_data_swjs(")]}'\n[[]]").is_none());
        assert!(parse_visitor_data_swjs("garbage").is_none());
    }

    #[test]
    fn param_tables() {
        assert_eq!(search_params("channel"), Some("EgIQAg=="));
        assert!(search_params("all").is_none());
        assert!(music_search_params("songs")
            .unwrap()
            .starts_with("EgWKAQIIAW"));
        assert_eq!(channel_tab_params("videos"), Some("EgZ2aWRlb3PyBgQKAjoA"));
        assert!(channel_tab_params("home").is_none());
    }
}
