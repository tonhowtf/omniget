use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{browser, data_dir, host_matches, host_of, sniffer, Node, Rendition, Source};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub description: String,
    pub domains: Vec<String>,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    #[serde(default)]
    pub login: Option<Value>,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub min_omniget: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Value>,
}

fn default_entry() -> String {
    "connector.js".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct Connector {
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub dev: bool,
}

fn installed_dir() -> PathBuf {
    let d = data_dir().join("connectors");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn dev_list_file() -> PathBuf {
    data_dir().join("connectors.json")
}

fn dev_dirs() -> Vec<PathBuf> {
    std::fs::read_to_string(dev_list_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["dev"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(PathBuf::from))
        .collect()
}

fn save_dev_dirs(dirs: &[PathBuf]) -> Result<(), String> {
    let v = json!({ "dev": dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>() });
    std::fs::write(dev_list_file(), v.to_string()).map_err(|e| e.to_string())
}

pub fn load(dir: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(dir.join("manifest.json"))
        .map_err(|e| format!("{}: no manifest.json ({e})", dir.display()))?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("manifest.json: {e}"))?;
    validate_manifest(&m)?;
    if !dir.join(&m.entry).is_file() {
        return Err(format!("entry file {} is missing", m.entry));
    }
    Ok(m)
}

pub fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if m.id.trim().is_empty()
        || !m
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("id must be letters, digits, '-', '_' or '.'".into());
    }
    if m.domains.is_empty() {
        return Err("domains must list at least one host".into());
    }
    if m.matches.is_empty() {
        return Err("match must list at least one URL pattern".into());
    }
    if m.entry.contains("..") || m.entry.starts_with('/') {
        return Err("entry must be a file inside the connector folder".into());
    }
    for d in &m.domains {
        if crate::core::platform_optout::is_opted_out_host(d.trim_start_matches("*.")) {
            return Err(format!(
                "{d}: {}",
                crate::core::platform_optout::OPTED_OUT_ERROR
            ));
        }
    }
    for p in &m.matches {
        pattern_regex(p)?;
    }
    Ok(())
}

pub fn list() -> Vec<Connector> {
    let mut out = Vec::new();
    for d in dev_dirs() {
        if let Ok(m) = load(&d) {
            out.push(Connector {
                manifest: m,
                dir: d,
                dev: true,
            });
        }
    }
    if let Ok(rd) = std::fs::read_dir(installed_dir()) {
        for e in rd.flatten() {
            let d = e.path();
            if let Ok(m) = load(&d) {
                if !out.iter().any(|c| c.manifest.id == m.id) {
                    out.push(Connector {
                        manifest: m,
                        dir: d,
                        dev: false,
                    });
                }
            }
        }
    }
    out
}

pub fn install_dev(dir: &Path) -> Result<Manifest, String> {
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let m = load(&dir)?;
    let mut dirs = dev_dirs();
    dirs.retain(|d| d != &dir);
    dirs.push(dir);
    save_dev_dirs(&dirs)?;
    Ok(m)
}

pub fn remove(id: &str) -> Result<(), String> {
    let mut dirs = dev_dirs();
    dirs.retain(|d| load(d).map(|m| m.id != id).unwrap_or(true));
    save_dev_dirs(&dirs)?;
    let p = installed_dir().join(id);
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn options_file(id: &str) -> PathBuf {
    let d = data_dir().join("connector-options");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{id}.json"))
}

pub fn option_values(m: &Manifest, overrides: Option<&Value>) -> Value {
    let mut out = serde_json::Map::new();
    for o in &m.options {
        if let Some(id) = o["id"].as_str() {
            out.insert(id.into(), o.get("default").cloned().unwrap_or(Value::Null));
        }
    }
    let saved = std::fs::read_to_string(options_file(&m.id))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    for src in [saved.as_ref(), overrides].into_iter().flatten() {
        if let Some(o) = src.as_object() {
            for (k, v) in o {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

pub fn find_by_id(id: &str) -> Option<Connector> {
    list().into_iter().find(|c| c.manifest.id == id)
}

pub fn get_options(id: &str) -> Result<Value, String> {
    let c = find_by_id(id).ok_or_else(|| format!("no connector {id}"))?;
    Ok(json!({ "defs": c.manifest.options, "values": option_values(&c.manifest, None) }))
}

pub fn set_options(id: &str, values: &Value) -> Result<Value, String> {
    let c = find_by_id(id).ok_or_else(|| format!("no connector {id}"))?;
    let known: Vec<&str> = c
        .manifest
        .options
        .iter()
        .filter_map(|o| o["id"].as_str())
        .collect();
    let mut keep = serde_json::Map::new();
    for (k, v) in values.as_object().ok_or("options must be an object")? {
        if known.contains(&k.as_str()) {
            keep.insert(k.clone(), v.clone());
        }
    }
    std::fs::write(options_file(id), Value::Object(keep).to_string()).map_err(|e| e.to_string())?;
    get_options(id)
}

fn pattern_regex(p: &str) -> Result<regex::Regex, String> {
    let src = if p.starts_with('^') {
        p.to_string()
    } else {
        let mut s = String::from("^");
        for ch in p.chars() {
            if ch == '*' {
                s.push_str(".*");
            } else {
                s.push_str(&regex::escape(&ch.to_string()));
            }
        }
        s.push('$');
        s
    };
    regex::Regex::new(&src).map_err(|e| format!("match pattern {p}: {e}"))
}

pub fn find_for(url: &str) -> Option<Connector> {
    list().into_iter().find(|c| {
        c.manifest
            .matches
            .iter()
            .any(|p| pattern_regex(p).map(|r| r.is_match(url)).unwrap_or(false))
    })
}

pub fn scaffold(
    dir: &Path,
    id: &str,
    name: &str,
    domains: &[String],
    matches: &[String],
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let m = Manifest {
        id: id.into(),
        name: name.into(),
        version: "0.1.0".into(),
        author: String::new(),
        license: "MIT".into(),
        homepage: String::new(),
        description: String::new(),
        domains: domains.to_vec(),
        matches: matches.to_vec(),
        login: None,
        entry: "connector.js".into(),
        min_omniget: env!("CARGO_PKG_VERSION").into(),
        options: Vec::new(),
    };
    validate_manifest(&m)?;
    let write = |f: &str, body: String| -> Result<(), String> {
        let p = dir.join(f);
        if p.exists() {
            return Ok(());
        }
        std::fs::write(p, body).map_err(|e| e.to_string())
    };
    write(
        "manifest.json",
        serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?,
    )?;
    write("connector.js", TEMPLATE.to_string())?;
    write(
        "GUIDE.md",
        format!("# {name}\n\nWhat it downloads, what needs a login, known limits.\n"),
    )?;
    Ok(())
}

const TEMPLATE: &str = r#"// resolve(url, ctx) returns the tree OmniGet downloads.
// ctx.fetch(url, {method, headers, body}) -> {status, ok, headers, text(), json()}
// ctx.cookies(domain) -> {name: value}   ctx.page.eval(js) -> value (the open tab)
// ctx.sniffed() -> media the tab already loaded   ctx.log(...)
function resolve(url, ctx) {
  const page = ctx.fetch(url).text();
  const title = (page.match(/<title>([^<]*)<\/title>/i) || [, "Untitled"])[1].trim();
  return {
    title,
    children: ctx.sniffed().map((m) => ({
      title: m.page_title || title,
      kind: m.kind === "hls" || m.kind === "dash" ? "video" : m.kind,
      source: { format: m.kind === "hls" ? "hls" : m.kind === "dash" ? "dash" : "direct", url: m.url, headers: m.headers },
    })),
  };
}
"#;

pub const SPEC: &str = include_str!("SPEC.md");

const PRELUDE: &str = r#"
var ctx = (function () {
  function wrap(r) {
    if (r.error) throw new Error(r.error);
    return {
      status: r.status, ok: r.status >= 200 && r.status < 300, url: r.url, headers: r.headers,
      text: function () { return r.body; },
      json: function () { return JSON.parse(r.body); },
    };
  }
  return {
    url: __target_url,
    tabId: __tab_id,
    options: JSON.parse(__options),
    fetch: function (url, init) {
      init = init || {};
      return wrap(JSON.parse(__host_fetch(JSON.stringify({
        url: String(url), method: init.method || "GET", headers: init.headers || {},
        body: init.body === undefined ? null : (typeof init.body === "string" ? init.body : JSON.stringify(init.body)),
      }))));
    },
    fetchJson: function (url, init) {
      var r = this.fetch(url, init);
      if (!r.ok) throw new Error("HTTP " + r.status + " for " + url);
      return r.json();
    },
    cookies: function (domain) { return JSON.parse(__host_cookies(String(domain || ""))); },
    sniffed: function () { return JSON.parse(__host_sniffed()); },
    page: { eval: function (js) { var r = JSON.parse(__host_eval(String(js))); if (r.error) throw new Error(r.error); return r.value; } },
    log: function () { __host_log(Array.prototype.map.call(arguments, function (a) { return typeof a === "string" ? a : JSON.stringify(a); }).join(" ")); },
    query: function (o) { return Object.keys(o).map(function (k) { return encodeURIComponent(k) + "=" + encodeURIComponent(o[k]); }).join("&"); },
  };
})();
"#;

#[derive(Debug, Clone, Serialize)]
pub struct ResolveResult {
    pub connector: Option<String>,
    pub tree: Node,
    pub logs: Vec<String>,
    pub items: usize,
    pub drm: usize,
    pub ms: u128,
}

pub async fn resolve(
    url: &str,
    dir: Option<PathBuf>,
    tab_id: Option<String>,
    options: Option<Value>,
) -> Result<ResolveResult, String> {
    crate::core::platform_optout::ensure_allowed(url)?;
    let started = Instant::now();
    let conn = match dir {
        Some(d) => Some(Connector {
            manifest: load(&d)?,
            dir: d,
            dev: true,
        }),
        None => find_for(url),
    };
    let tab_id = tab_id.or_else(|| {
        browser::current().and_then(|b| {
            b.tabs()
                .into_iter()
                .rev()
                .find(|t| t.url == url)
                .or_else(|| b.tab_for_origin(url))
                .map(|t| t.id)
        })
    });
    let (mut tree, logs, id) = match conn {
        Some(c) => {
            let opts = option_values(&c.manifest, options.as_ref());
            let (out, logs) =
                run_script(c.clone(), url.to_string(), tab_id, Call::Resolve, opts).await?;
            let mut tree: Node = serde_json::from_str(&out).map_err(|e| format!("tree: {e}"))?;
            if tree.kind.is_empty() {
                tree.kind = "folder".into();
            }
            tree.connector = Some(c.manifest.id.clone());
            tree.connector_dir = Some(c.dir.display().to_string());
            tree.origin = Some(url.to_string());
            (tree, logs, Some(c.manifest.id))
        }
        None => {
            if let Some(t) = tab_id.as_deref() {
                wake_media(t).await;
            }
            let title = page_title(url, tab_id.as_deref()).await;
            let links = match tab_id.as_deref() {
                Some(t) => page_files(t).await,
                None => Vec::new(),
            };
            (
                generic_tree(url, tab_id.as_deref(), &title, links),
                Vec::new(),
                None,
            )
        }
    };
    tree.assign_ids("0");
    let (items, drm, _) = tree.counts();
    Ok(ResolveResult {
        connector: id,
        tree,
        logs,
        items,
        drm,
        ms: started.elapsed().as_millis(),
    })
}

async fn page_title(url: &str, tab_id: Option<&str>) -> String {
    let Some(b) = browser::current() else {
        return url.to_string();
    };
    let Some(tab) = tab_id.and_then(|t| b.tab(t)) else {
        return url.to_string();
    };
    if !looks_like_address(&tab.title) {
        return tab.title;
    }
    match b
        .eval(&tab.id, "document.title", Duration::from_secs(3))
        .await
    {
        Ok(Value::String(t)) if !t.trim().is_empty() => t.trim().to_string(),
        _ if !tab.title.is_empty() => tab.title,
        _ => url.to_string(),
    }
}

fn looks_like_address(title: &str) -> bool {
    let t = title.trim();
    t.is_empty() || (!t.contains(' ') && (t.contains('/') || t.contains("://")))
}

const GENERIC_STEMS: &[&str] = &[
    "index",
    "master",
    "playlist",
    "manifest",
    "chunklist",
    "video",
    "audio",
    "media",
    "stream",
    "full",
    "default",
    "main",
    "output",
    "prog_index",
    "source",
    "file",
    "download",
    "play",
    "hls",
    "dash",
    "api",
];

fn url_file(url: &str) -> (String, String) {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    let decoded = urlencoding::decode(last)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| last.to_string());
    let name = decoded.rsplit('/').next().unwrap_or(&decoded).to_string();
    match name.rsplit_once('.') {
        Some((stem, ext)) if (1..=5).contains(&ext.len()) => (stem.to_string(), ext.to_string()),
        _ => (name, String::new()),
    }
}

pub fn media_title(url: &str, page_title: &str) -> String {
    let (stem, _) = url_file(url);
    let lower = stem.to_ascii_lowercase();
    let generic = stem.is_empty() || GENERIC_STEMS.contains(&lower.as_str());
    if (generic || id_like(&lower)) && !page_title.trim().is_empty() {
        page_title.trim().to_string()
    } else if stem.is_empty() {
        "media".into()
    } else {
        stem
    }
}

fn id_like(stem: &str) -> bool {
    const NOISE: &[&str] = &[
        "mp4",
        "h264",
        "h265",
        "hevc",
        "avc",
        "aac",
        "vp9",
        "av1",
        "opus",
        "hd",
        "sd",
        "hq",
        "fhd",
        "uhd",
        "fragmented",
        "frag",
        "video",
        "audio",
        "master",
        "index",
        "playlist",
        "manifest",
    ];
    let tokens: Vec<&str> = stem
        .split(['-', '_', '.', ' '])
        .filter(|t| !t.is_empty())
        .collect();
    let mixed = |t: &str| {
        t.len() >= 6
            && t.chars().any(|c| c.is_ascii_digit())
            && t.chars().any(|c| c.is_ascii_alphabetic())
            && t.chars().all(|c| c.is_ascii_alphanumeric())
    };
    if tokens.len() == 1 {
        return mixed(tokens[0])
            || (tokens[0].len() >= 6 && tokens[0].chars().all(|c| c.is_ascii_digit()))
            || (tokens[0].len() >= 16 && tokens[0].chars().all(|c| c.is_ascii_hexdigit()));
    }
    !tokens.is_empty()
        && tokens.iter().all(|t| {
            NOISE.contains(t)
                || t.chars().all(|c| c.is_ascii_digit())
                || (t.len() >= 4 && t.chars().all(|c| c.is_ascii_hexdigit()))
                || t.strip_suffix('p')
                    .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                || t.strip_suffix('k')
                    .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
}

fn ext_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime.split(';').next().unwrap_or("").trim() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" => "flac",
        "application/pdf" => "pdf",
        "application/epub+zip" => "epub",
        "text/vtt" => "vtt",
        "application/x-subrip" => "srt",
        _ => return None,
    })
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "video" => 0,
        "audio" => 1,
        "subtitle" => 2,
        _ => 3,
    }
}

const WAKE_WAIT: Duration = Duration::from_secs(6);

fn has_media(tab: &str) -> bool {
    sniffer::sniffed(Some(tab))
        .iter()
        .any(|m| matches!(m.kind.as_str(), "hls" | "dash" | "video" | "audio"))
}

async fn wake_media(tab: &str) {
    if !browser::autoplay() || has_media(tab) {
        return;
    }
    let Some(b) = browser::current() else {
        return;
    };
    match super::page::start_playback(&b, tab).await {
        Ok(r) => tracing::info!("[global-dl] started the player of {tab}: {r}"),
        Err(e) => {
            tracing::info!("[global-dl] could not start the player of {tab}: {e}");
            return;
        }
    }
    let deadline = Instant::now() + WAKE_WAIT;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if has_media(tab) {
            tokio::time::sleep(Duration::from_millis(1200)).await;
            return;
        }
    }
}

const PAGE_FILES_JS: &str = r#"(() => {
  const exts = /\.(jpe?g|png|gif|webp|avif|heic|bmp|tiff?|mp4|m4v|webm|mov|mkv|avi|mp3|m4a|aac|wav|flac|ogg|opus|pdf|epub|mobi|cbz|cbr|zip|rar|7z|docx?|xlsx?|pptx?|csv|txt|srt|vtt)$/i;
  const seen = new Set();
  const out = [];
  const add = (raw, title) => {
    if (!raw || out.length >= 300) return;
    let u;
    try { u = new URL(raw, location.href); } catch { return; }
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return;
    u.hash = '';
    const key = u.href;
    if (seen.has(key)) return;
    seen.add(key);
    out.push({ url: key, title: (title || '').trim().slice(0, 140) });
  };
  for (const img of document.images) {
    if ((img.naturalWidth || img.width) >= 200 && (img.naturalHeight || img.height) >= 200) add(img.currentSrc || img.src, img.alt);
  }
  for (const a of document.querySelectorAll('a[href]')) {
    let path = '';
    try { path = new URL(a.href, location.href).pathname; } catch { continue; }
    if (exts.test(path) || a.hasAttribute('download')) add(a.href, a.getAttribute('download') || a.textContent);
  }
  for (const m of document.querySelectorAll('video[src],audio[src],video source[src],audio source[src]')) add(m.getAttribute('src'), '');
  return out;
})()"#;

#[derive(Debug, Clone, PartialEq)]
struct PageFile {
    url: String,
    title: String,
}

async fn page_files(tab: &str) -> Vec<PageFile> {
    let Some(b) = browser::current() else {
        return Vec::new();
    };
    match b.eval(tab, PAGE_FILES_JS, Duration::from_secs(4)).await {
        Ok(Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| {
                Some(PageFile {
                    url: v.get("url")?.as_str()?.to_string(),
                    title: v
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect(),
        Ok(_) => Vec::new(),
        Err(e) => {
            tracing::info!("[global-dl] could not list the files of {tab}: {e}");
            Vec::new()
        }
    }
}

fn kind_for_ext(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "bmp" | "tif" | "tiff" => {
            "image"
        }
        "mp4" | "m4v" | "webm" | "mov" | "mkv" | "avi" => "video",
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" => "audio",
        "srt" | "vtt" => "subtitle",
        "pdf" | "epub" | "mobi" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "csv"
        | "txt" => "document",
        _ => "file",
    }
}

fn page_file_nodes(files: Vec<PageFile>, known: &[String], page_url: &str) -> Vec<Node> {
    files
        .into_iter()
        .filter(|f| !known.iter().any(|k| k == &f.url) && f.url != page_url)
        .map(|f| {
            let (stem, ext) = url_file(&f.url);
            let ext = ext.to_ascii_lowercase();
            let title = if !f.title.trim().is_empty() && !looks_like_address(&f.title) {
                f.title.trim().to_string()
            } else if !stem.is_empty() {
                stem
            } else {
                f.url.clone()
            };
            let mut headers = BTreeMap::new();
            headers.insert("Referer".to_string(), page_url.to_string());
            Node {
                title,
                kind: kind_for_ext(&ext).into(),
                source: Some(Source {
                    format: "direct".into(),
                    url: Some(f.url),
                    headers,
                    ext: (!ext.is_empty()).then_some(ext),
                    ..Default::default()
                }),
                ..Default::default()
            }
        })
        .collect()
}

fn generic_tree(url: &str, tab_id: Option<&str>, page_title: &str, links: Vec<PageFile>) -> Node {
    let page_title = if looks_like_address(page_title) {
        let (stem, _) = url_file(url);
        if stem.is_empty() {
            url.to_string()
        } else {
            stem
        }
    } else {
        page_title.to_string()
    };
    let mut children = vec![Node {
        title: format!("{page_title} (yt-dlp)"),
        kind: "video".into(),
        source: Some(Source {
            format: "ytdlp".into(),
            url: Some(url.to_string()),
            ..Default::default()
        }),
        note: Some("sent to the Downloads queue".into()),
        ..Default::default()
    }];
    let mut media: Vec<Node> = Vec::new();
    if let Some(t) = tab_id {
        for m in sniffer::sniffed(Some(t)) {
            let format = match m.kind.as_str() {
                "hls" => "hls",
                "dash" => "dash",
                _ => "direct",
            };
            let (_, ext) = url_file(&m.url);
            let script = matches!(
                ext.to_ascii_lowercase().as_str(),
                "php" | "asp" | "aspx" | "jsp" | "cgi" | "ashx" | "pl"
            );
            let ext = if format != "direct" {
                None
            } else if ext.is_empty() || script {
                ext_for_mime(&m.mime)
                    .or(match m.kind.as_str() {
                        _ if !script => None,
                        "subtitle" => Some("vtt"),
                        "video" => Some("mp4"),
                        "audio" => Some("m4a"),
                        _ => None,
                    })
                    .map(String::from)
            } else {
                Some(ext.to_ascii_lowercase())
            };
            let mut title = media_title(&m.url, &page_title);
            if m.kind == "subtitle" {
                if let Some(lang) = query_lang(&m.url) {
                    title = format!("{title} ({lang})");
                }
            }
            media.push(Node {
                note: m.live.then(|| {
                    format!(
                        "live: saves the latest {} segments",
                        crate::core::hls_downloader::LIVE_TAKE
                    )
                }),
                title,
                kind: match m.kind.as_str() {
                    "hls" | "dash" => "video".into(),
                    "document" => "file".into(),
                    k => k.into(),
                },
                source: Some(Source {
                    format: format.into(),
                    url: Some(m.url),
                    headers: m.headers,
                    ext,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
    }
    let known: Vec<String> = media
        .iter()
        .filter_map(|n| n.source.as_ref().and_then(|s| s.url.clone()))
        .collect();
    media.extend(page_file_nodes(links, &known, url));
    let mut media = merge_renditions(media);
    media.sort_by_key(|n| kind_rank(&n.kind));
    disambiguate(&mut media);
    children.extend(media);
    Node {
        title: page_title,
        kind: "folder".into(),
        children,
        ..Default::default()
    }
}

fn query_lang(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    u.query_pairs()
        .find(|(k, _)| matches!(k.as_ref(), "lang" | "language" | "tlang" | "hl"))
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty() && v.len() <= 16)
}

pub fn height_of(url: &str) -> Option<u32> {
    let (stem, _) = url_file(url);
    let tokens: Vec<&str> = stem
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '×')
        .filter(|t| !t.is_empty())
        .collect();
    let num = |s: &str| s.parse::<u32>().ok().filter(|n| (100..=8640).contains(n));
    for t in &tokens {
        if let Some((w, h)) = t.split_once(['x', 'X', '×']) {
            if w.chars().all(|c| c.is_ascii_digit()) && !w.is_empty() {
                if let Some(h) = num(h) {
                    return Some(h);
                }
            }
        }
    }
    for t in &tokens {
        let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = &t[digits.len()..];
        let mut rest = rest.chars();
        if matches!(rest.next(), Some('p' | 'P' | 'i' | 'I')) {
            let fps: String = rest.collect();
            if fps.is_empty() || (fps.len() <= 3 && fps.chars().all(|c| c.is_ascii_digit())) {
                if let Some(h) = num(&digits) {
                    return Some(h);
                }
            }
        }
    }
    tokens
        .iter()
        .find_map(|t| match t.to_ascii_lowercase().as_str() {
            "4k" => Some(2160),
            "8k" => Some(4320),
            _ => None,
        })
}

fn rendition_key(url: &str, height: u32) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let (dir, last) = path.rsplit_once('/').unwrap_or(("", path));
    let mut out = String::new();
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        let hit = height_of(&format!("a/{token}")) == Some(height);
        out.push_str(if hit { "*" } else { token.as_str() });
        token.clear();
    };
    for c in last.chars() {
        if c.is_ascii_alphanumeric() || c == '×' {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    format!("{dir}/{out}")
}

fn strip_rendition(title: &str, heights: &[u32]) -> String {
    const DROP: &[&str] = &[
        "webm", "mp4", "mkv", "mov", "vp8", "vp9", "av1", "h264", "h265", "hevc", "avc",
    ];
    let mut parts: Vec<String> = Vec::new();
    let mut token = String::new();
    let push = |token: &mut String, sep: Option<char>, parts: &mut Vec<String>| {
        let lower = token.to_ascii_lowercase();
        let height = height_of(&format!("a/{token}")).is_some_and(|h| heights.contains(&h));
        if !token.is_empty() && !height && !DROP.contains(&lower.as_str()) {
            parts.push(std::mem::take(token));
        } else {
            token.clear();
            if parts
                .last()
                .is_some_and(|p| p.len() == 1 && !p.chars().all(|c| c.is_ascii_alphanumeric()))
            {
                parts.pop();
            }
        }
        if let Some(c) = sep {
            parts.push(c.to_string());
        }
    };
    for c in title.chars() {
        if c.is_alphanumeric() {
            token.push(c);
        } else {
            push(&mut token, Some(c), &mut parts);
        }
    }
    push(&mut token, None, &mut parts);
    let out = parts.concat();
    let out = out.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    if out.is_empty() {
        title.to_string()
    } else {
        out
    }
}

fn merge_renditions(media: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(media.len());
    let mut groups: Vec<(String, usize)> = Vec::new();
    for n in media {
        let direct = n.kind == "video" && n.source.as_ref().is_some_and(|s| s.format == "direct");
        let url = n
            .source
            .as_ref()
            .and_then(|s| s.url.clone())
            .unwrap_or_default();
        let height = if direct { height_of(&url) } else { None };
        let Some(h) = height else {
            out.push(n);
            continue;
        };
        let key = rendition_key(&url, h);
        match groups.iter().find(|(k, _)| *k == key) {
            Some((_, i)) => {
                let first = &mut out[*i];
                if let Some(s) = first.source.as_mut() {
                    if !s.renditions.iter().any(|r| r.url == url) {
                        s.renditions.push(Rendition { height: h, url });
                    }
                }
            }
            None => {
                let mut n = n;
                if let Some(s) = n.source.as_mut() {
                    s.renditions = vec![Rendition { height: h, url }];
                }
                groups.push((key, out.len()));
                out.push(n);
            }
        }
    }
    for n in out.iter_mut() {
        let Some(s) = n.source.as_mut() else { continue };
        if s.renditions.len() < 2 {
            s.renditions.clear();
            continue;
        }
        s.renditions.sort_by_key(|r| r.height);
        s.url = s.renditions.last().map(|r| r.url.clone());
        let heights: Vec<String> = s
            .renditions
            .iter()
            .map(|r| format!("{}p", r.height))
            .collect();
        let hs: Vec<u32> = s.renditions.iter().map(|r| r.height).collect();
        n.title = strip_rendition(&n.title, &hs);
        n.note = Some(format!(
            "{} (best within the max height)",
            heights.join(", ")
        ));
    }
    out
}

fn disambiguate(items: &mut [Node]) {
    use std::collections::HashMap;
    let mut count: HashMap<(String, String), usize> = HashMap::new();
    for n in items.iter() {
        *count.entry((n.kind.clone(), n.title.clone())).or_default() += 1;
    }
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    for n in items.iter_mut() {
        let key = (n.kind.clone(), n.title.clone());
        if count.get(&key).copied().unwrap_or(0) < 2 {
            continue;
        }
        let url = n
            .source
            .as_ref()
            .and_then(|s| s.url.clone())
            .unwrap_or_default();
        let path = url.split(['?', '#']).next().unwrap_or("");
        let segs: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
        let hint = segs
            .iter()
            .rev()
            .skip(1)
            .find(|x| !x.contains('.') && x.len() <= 24 && !GENERIC_STEMS.contains(x))
            .map(|x| x.to_string());
        let i = seen.entry(key).or_default();
        *i += 1;
        n.title = match hint {
            Some(h) => format!("{} ({h})", n.title),
            None => format!("{} ({})", n.title, i),
        };
    }
}

fn domain_allowed(domains: &[String], url: &str) -> Result<(), String> {
    let host = host_of(url).ok_or_else(|| format!("bad url: {url}"))?;
    if crate::core::platform_optout::is_opted_out_host(&host) {
        return Err(crate::core::platform_optout::OPTED_OUT_ERROR.into());
    }
    if domains.iter().any(|d| host_matches(&host, d)) {
        Ok(())
    } else {
        Err(format!("{host} is not in this connector's domains"))
    }
}

#[derive(Clone)]
enum Call {
    Resolve,
    Item(String),
}

pub async fn resolve_item(
    connector_id: &str,
    connector_dir: Option<&str>,
    origin: &str,
    key: &str,
) -> Result<Node, String> {
    let c = match connector_dir.map(PathBuf::from).filter(|d| d.is_dir()) {
        Some(d) => Connector {
            manifest: load(&d)?,
            dir: d,
            dev: true,
        },
        None => find_by_id(connector_id)
            .ok_or_else(|| format!("connector {connector_id} is not installed"))?,
    };
    let tab_id = browser::current()
        .and_then(|b| b.tab_for_origin(origin))
        .map(|t| t.id);
    let opts = option_values(&c.manifest, None);
    let (out, _) = run_script(
        c,
        origin.to_string(),
        tab_id,
        Call::Item(key.to_string()),
        opts,
    )
    .await?;
    let v: Value = serde_json::from_str(&out).map_err(|e| format!("resolveItem: {e}"))?;
    let node = if v.get("source").is_some() || v.get("drm").is_some() {
        serde_json::from_value::<Node>(v).map_err(|e| format!("resolveItem: {e}"))?
    } else {
        Node {
            source: Some(
                serde_json::from_value::<Source>(v).map_err(|e| format!("resolveItem: {e}"))?,
            ),
            ..Default::default()
        }
    };
    Ok(node)
}

async fn run_script(
    c: Connector,
    url: String,
    tab_id: Option<String>,
    call: Call,
    options: Value,
) -> Result<(String, Vec<String>), String> {
    let source =
        std::fs::read_to_string(c.dir.join(&c.manifest.entry)).map_err(|e| e.to_string())?;
    let source = source
        .replace("export async function", "async function")
        .replace("export function", "function")
        .replace("export default ", "");
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        run_blocking(handle, c.manifest, source, url, tab_id, call, options)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn run_blocking(
    handle: tokio::runtime::Handle,
    manifest: Manifest,
    source: String,
    url: String,
    tab_id: Option<String>,
    call: Call,
    options: Value,
) -> Result<(String, Vec<String>), String> {
    use rquickjs::{CatchResultExt, Context, Function, Runtime};
    use std::sync::{Arc, Mutex};

    let rt = Runtime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(512 << 20);
    rt.set_max_stack_size(4 << 20);
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
    let logs: Arc<Mutex<Vec<String>>> = Arc::default();
    let domains = Arc::new(manifest.domains.clone());

    let result: Result<String, String> = ctx.with(|ctx| {
        let g = ctx.globals();
        let js_err = |e: rquickjs::Error| e.to_string();
        g.set("__target_url", url.clone()).map_err(js_err)?;
        g.set("__tab_id", tab_id.clone().unwrap_or_default())
            .map_err(js_err)?;
        g.set("__options", options.to_string()).map_err(js_err)?;

        let (h, d) = (handle.clone(), domains.clone());
        g.set(
            "__host_fetch",
            Function::new(ctx.clone(), move |req: String| -> String {
                let out = (|| -> Result<Value, String> {
                    let r: Value = serde_json::from_str(&req).map_err(|e| e.to_string())?;
                    let u = r["url"].as_str().unwrap_or_default().to_string();
                    domain_allowed(&d, &u)?;
                    let headers = r["headers"].as_object().cloned().unwrap_or_default();
                    let method = r["method"].as_str().unwrap_or("GET").to_string();
                    let body = r["body"].as_str().map(String::from);
                    h.block_on(browser::fetch(&u, &method, &headers, body.as_deref()))
                })();
                out.unwrap_or_else(|e| json!({ "error": e })).to_string()
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let (h, d) = (handle.clone(), domains.clone());
        g.set(
            "__host_cookies",
            Function::new(ctx.clone(), move |domain: String| -> String {
                let host = domain.trim_start_matches('.').to_ascii_lowercase();
                if !d.iter().any(|x| host_matches(&host, x)) {
                    return "{}".into();
                }
                let Some(b) = browser::current() else {
                    return "{}".into();
                };
                let cookies = h.block_on(b.cookies_for(&host)).unwrap_or_default();
                let mut o = serde_json::Map::new();
                for c in cookies {
                    if let (Some(n), Some(v)) = (c["name"].as_str(), c["value"].as_str()) {
                        o.insert(n.into(), Value::String(v.into()));
                    }
                }
                Value::Object(o).to_string()
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let tab = tab_id.clone();
        g.set(
            "__host_sniffed",
            Function::new(ctx.clone(), move || -> String {
                serde_json::to_string(&sniffer::sniffed(tab.as_deref()))
                    .unwrap_or_else(|_| "[]".into())
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let (h, d, tab) = (handle.clone(), domains.clone(), tab_id.clone());
        g.set(
            "__host_eval",
            Function::new(ctx.clone(), move |js: String| -> String {
                let out = (|| -> Result<Value, String> {
                    let b = browser::require()?;
                    let t = tab
                        .as_deref()
                        .and_then(|t| b.tab(t))
                        .ok_or("no tab is open for this page")?;
                    domain_allowed(&d, &t.url)?;
                    h.block_on(b.eval(&t.id, &js, Duration::from_secs(60)))
                })();
                match out {
                    Ok(v) => json!({ "value": v }).to_string(),
                    Err(e) => json!({ "error": e }).to_string(),
                }
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        let l = logs.clone();
        g.set(
            "__host_log",
            Function::new(ctx.clone(), move |msg: String| {
                tracing::info!("[global-dl] connector: {msg}");
                if let Ok(mut l) = l.lock() {
                    if l.len() < 500 {
                        l.push(msg);
                    }
                }
            })
            .map_err(js_err)?,
        )
        .map_err(js_err)?;

        ctx.eval::<(), _>(PRELUDE)
            .catch(&ctx)
            .map_err(|e| format!("prelude: {e}"))?;
        ctx.eval::<(), _>(source)
            .catch(&ctx)
            .map_err(|e| format!("connector: {e}"))?;
        let (fname, arg) = match &call {
            Call::Resolve => ("resolve", url.clone()),
            Call::Item(k) => ("resolveItem", k.clone()),
        };
        let f: Function = g
            .get(fname)
            .map_err(|_| format!("the connector defines no {fname}(…, ctx)"))?;
        let c: rquickjs::Value = g.get("ctx").map_err(js_err)?;
        let mut out: rquickjs::Value = f
            .call((arg, c))
            .catch(&ctx)
            .map_err(|e| format!("{fname}: {e}"))?;
        if let Some(p) = out.as_promise().cloned() {
            out = p
                .finish::<rquickjs::Value>()
                .catch(&ctx)
                .map_err(|e| format!("{fname}: {e}"))?;
        }
        ctx.json_stringify(out)
            .catch(&ctx)
            .map_err(|e| format!("{fname} returned something that is not JSON: {e}"))?
            .map(|s| s.to_string().unwrap_or_default())
            .ok_or_else(|| format!("{fname} returned nothing"))
    });
    let text = result?;
    let logs = logs.lock().map(|l| l.clone()).unwrap_or_default();
    Ok((text, logs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(url: &str, title: &str) -> PageFile {
        PageFile {
            url: url.into(),
            title: title.into(),
        }
    }

    #[test]
    fn page_files_become_direct_nodes_with_kind_and_referer() {
        let page = "https://site.test/gallery";
        let nodes = page_file_nodes(
            vec![
                file("https://site.test/media/photo.JPG", "A sunset"),
                file(
                    "https://site.test/docs/manual.pdf",
                    "https://site.test/docs/manual.pdf",
                ),
                file("https://cdn.test/v.mp4", ""),
                file("https://cdn.test/sniffed.mp4", ""),
                file(page, "self"),
            ],
            &["https://cdn.test/sniffed.mp4".to_string()],
            page,
        );
        let got: Vec<(&str, &str, Option<&str>)> = nodes
            .iter()
            .map(|n| {
                let s = n.source.as_ref().unwrap();
                (n.title.as_str(), n.kind.as_str(), s.ext.as_deref())
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("A sunset", "image", Some("jpg")),
                ("manual", "document", Some("pdf")),
                ("v", "video", Some("mp4")),
            ]
        );
        let s = nodes[0].source.as_ref().unwrap();
        assert_eq!(s.format, "direct");
        assert_eq!(s.headers.get("Referer").map(String::as_str), Some(page));
    }
}
