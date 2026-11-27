use std::collections::HashMap;

use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Download, Hit, Page, RemoteChapter, RemotePage};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeclarativeSource {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub lang: String,
    pub kind: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub mirrors: Vec<String>,
    pub icon: Option<String>,
    pub nsfw: bool,
    pub template: Option<String>,
    pub headers: HashMap<String, String>,
    pub rate_limit_ms: u64,
    pub popular: Option<ListSpec>,
    pub latest: Option<ListSpec>,
    pub search: Option<ListSpec>,
    pub details: Option<DetailsSpec>,
    pub chapters: Option<ChaptersSpec>,
    pub pages: Option<PagesSpec>,
    pub filters: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Request {
    pub url: String,
    pub method: Option<String>,
    pub body: Option<String>,
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadsSpec {
    pub list: String,
    pub url: Extractor,
    pub format: Option<Extractor>,
    pub label: Option<Extractor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ListSpec {
    #[serde(flatten)]
    pub request: Request,
    pub list: String,
    pub key: Extractor,
    pub title: Extractor,
    pub authors: Option<Extractor>,
    pub cover: Option<Extractor>,
    pub has_next: Option<Extractor>,
    pub downloads: Option<DownloadsSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DetailsSpec {
    #[serde(flatten)]
    pub request: Request,
    pub title: Option<Extractor>,
    pub description: Option<Extractor>,
    pub authors: Option<Extractor>,
    pub genres: Option<Extractor>,
    pub status: Option<Extractor>,
    pub cover: Option<Extractor>,
    pub downloads: Option<DownloadsSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ChaptersSpec {
    #[serde(flatten)]
    pub request: Request,
    pub list: String,
    pub key: Extractor,
    pub title: Extractor,
    pub number: Option<Extractor>,
    pub date: Option<Extractor>,
    pub scanlator: Option<Extractor>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PagesSpec {
    #[serde(flatten)]
    pub request: Request,
    pub list: String,
    pub image: Extractor,
    pub script_json: Option<String>,
    pub referer: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Extractor {
    pub css: Option<String>,
    pub attr: Option<Vec<String>>,
    pub json: Option<String>,
    pub value: Option<String>,
    pub regex: Option<String>,
    pub replace: Option<(String, String)>,
    pub prefix: Option<String>,
    pub all: bool,
    pub abs: bool,
}

const DOWNLOAD_KEYS: &[&str] = &["url", "format", "label"];

const EXTRACTOR_KEYS: &[&str] = &[
    "key",
    "title",
    "cover",
    "hasNext",
    "description",
    "authors",
    "genres",
    "status",
    "number",
    "date",
    "scanlator",
    "image",
];

fn normalize_keys(v: &mut Value, keys: &[&str]) {
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if k == "downloads" {
                    normalize_keys(child, DOWNLOAD_KEYS);
                    continue;
                }
                if keys.contains(&k.as_str()) {
                    if let Value::String(s) = child {
                        *child = short_extractor(s);
                        continue;
                    }
                    if let Some(Value::String(a)) = child.get("attr").cloned() {
                        child["attr"] =
                            Value::Array(a.split('|').map(|x| Value::String(x.into())).collect());
                    }
                }
                normalize_keys(child, EXTRACTOR_KEYS);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| normalize_keys(x, keys)),
        _ => {}
    }
}

fn short_extractor(s: &str) -> Value {
    if let Some(j) = s.strip_prefix("$.") {
        return serde_json::json!({ "json": j });
    }
    match s.rsplit_once('@') {
        Some((css, attrs)) if !attrs.contains(' ') => {
            let mut v = serde_json::json!({ "attr": attrs.split('|').collect::<Vec<_>>() });
            if !css.trim().is_empty() {
                v["css"] = Value::String(css.trim().to_string());
            }
            v
        }
        _ => serde_json::json!({ "css": s }),
    }
}

fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(bv) if bv.is_object() && v.is_object() => merge(bv, v),
                    _ => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

pub fn template(name: &str) -> Option<Value> {
    Some(match name {
        "madara" => serde_json::json!({
            "popular": { "url": "{baseUrl}/manga/page/{page}/?m_orderby=views",
                "list": "div.page-item-detail, div.manga__item",
                "key": "h3 a@href", "title": "h3 a", "cover": "img@data-src|data-lazy-src|srcset|src",
                "hasNext": "a.nextpostslink, div.nav-previous a, a.next" },
            "latest": { "url": "{baseUrl}/manga/page/{page}/?m_orderby=latest",
                "list": "div.page-item-detail, div.manga__item",
                "key": "h3 a@href", "title": "h3 a", "cover": "img@data-src|data-lazy-src|srcset|src",
                "hasNext": "a.nextpostslink, div.nav-previous a, a.next" },
            "search": { "url": "{baseUrl}/page/{page}/?s={query}&post_type=wp-manga",
                "list": "div.c-tabs-item__content, div.page-item-detail",
                "key": "div.post-title a@href", "title": "div.post-title a",
                "cover": "img@data-src|data-lazy-src|srcset|src",
                "hasNext": "a.nextpostslink, div.nav-previous a" },
            "details": { "url": "{key}",
                "title": "div.post-title h1, div.post-title h3",
                "description": "div.description-summary div.summary__content, div.summary_content div.post-content_item > h5 + div, div.manga-excerpt",
                "authors": { "css": "div.author-content a", "all": true },
                "genres": { "css": "div.genres-content a", "all": true },
                "status": "div.post-status div.summary-content",
                "cover": "div.summary_image img@data-src|data-lazy-src|srcset|src" },
            "chapters": { "url": "{key}ajax/chapters/", "method": "POST",
                "list": "li.wp-manga-chapter", "key": "a@href", "title": "a",
                "date": "span.chapter-release-date", "order": "desc" },
            "pages": { "url": "{key}?style=list", "list": "div.page-break img, div.reading-content img",
                "image": "@data-src|data-lazy-src|srcset|src", "referer": true }
        }),
        "mangathemesia" => serde_json::json!({
            "popular": { "url": "{baseUrl}/manga/?page={page}&order=popular",
                "list": "div.bs div.bsx", "key": "a@href", "title": "a@title",
                "cover": "img@data-src|src", "hasNext": "a.r, a.next.page-numbers" },
            "latest": { "url": "{baseUrl}/manga/?page={page}&order=update",
                "list": "div.bs div.bsx", "key": "a@href", "title": "a@title",
                "cover": "img@data-src|src", "hasNext": "a.r, a.next.page-numbers" },
            "search": { "url": "{baseUrl}/page/{page}/?s={query}",
                "list": "div.bs div.bsx", "key": "a@href", "title": "a@title",
                "cover": "img@data-src|src", "hasNext": "a.r, a.next.page-numbers" },
            "details": { "url": "{key}", "title": "h1.entry-title",
                "description": "div.entry-content[itemprop=description], div.desc",
                "authors": { "css": "div.imptdt:contains(Author) i, div.fmed:nth-child(2) span", "all": true },
                "genres": { "css": "div.wd-full span.mgen a, span.mgen a", "all": true },
                "status": "div.imptdt i, div.tsinfo div:nth-child(1) i",
                "cover": "div.thumb img@data-src|src" },
            "chapters": { "url": "{key}", "list": "div#chapterlist li, div.eplister li",
                "key": "a@href", "title": "span.chapternum, div.epl-num",
                "date": "span.chapterdate, div.epl-date", "order": "desc" },
            "pages": { "url": "{key}", "list": "div#readerarea img",
                "image": "@data-src|src",
                "scriptJson": "\"images\"\\s*:\\s*(\\[[^\\]]*\\])", "referer": true }
        }),
        _ => return None,
    })
}

pub fn parse_source(v: Value) -> Result<DeclarativeSource, String> {
    let mut v = v;
    if let Some(t) = v.get("template").and_then(|t| t.as_str()) {
        let mut base = template(t).ok_or_else(|| format!("unknown template {t}"))?;
        merge(&mut base, v.clone());
        v = base;
    }
    normalize_keys(&mut v, EXTRACTOR_KEYS);
    let s: DeclarativeSource = serde_json::from_value(v).map_err(|e| format!("bad source: {e}"))?;
    if s.id.is_empty() || s.name.is_empty() || s.base_url.is_empty() {
        return Err("a source needs id, name and baseUrl".into());
    }
    if s.popular.is_none() && s.search.is_none() {
        return Err("a source needs popular or search".into());
    }
    Ok(s)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoIndex {
    #[serde(default)]
    sources: Vec<RepoEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoEntry {
    url: String,
    sha256: Option<String>,
    #[serde(default)]
    kind: Option<String>,
}

pub async fn fetch_repo(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<DeclarativeSource>, String> {
    let body = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&body).map_err(|_| {
        "not an OmniGet source or repository (Mihon repositories need the Suwayomi bridge)"
            .to_string()
    })?;
    if v.get("baseUrl").is_some() {
        return Ok(vec![parse_source(v)?]);
    }
    let idx: RepoIndex = serde_json::from_value(v).map_err(|e| e.to_string())?;
    if idx.sources.is_empty() {
        return Err("empty repository".into());
    }
    let base = url::Url::parse(url).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for e in idx.sources {
        if e.kind.as_deref().is_some_and(|k| k != "declarative") {
            continue;
        }
        let file_url = base.join(&e.url).map_err(|x| x.to_string())?;
        let bytes = client
            .get(file_url.as_str())
            .send()
            .await
            .map_err(|x| x.to_string())?
            .bytes()
            .await
            .map_err(|x| x.to_string())?;
        let want = e.sha256.ok_or_else(|| format!("{} has no sha256", e.url))?;
        let got = hex::encode(Sha256::digest(&bytes));
        if !got.eq_ignore_ascii_case(want.trim()) {
            return Err(format!("{}: checksum mismatch", e.url));
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|x| x.to_string())?;
        out.push(parse_source(v)?);
    }
    Ok(out)
}

enum Doc {
    Html(Html),
    Json(Value),
}

fn fill(template: &str, s: &DeclarativeSource, vars: &[(&str, &str)]) -> String {
    let mut out = template.replace("{baseUrl}", s.base_url.trim_end_matches('/'));
    for (k, v) in vars {
        let val = if *k == "query" {
            urlencoding::encode(v).to_string()
        } else {
            v.to_string()
        };
        out = out.replace(&format!("{{{k}}}"), &val);
    }
    out
}

async fn fetch_text_once(
    client: &reqwest::Client,
    s: &DeclarativeSource,
    req: &Request,
    url: &str,
    referer_base: &str,
) -> Result<(String, String), String> {
    if s.rate_limit_ms > 0 {
        throttle(&s.id, s.rate_limit_ms).await;
    }
    let method = req.method.as_deref().unwrap_or("GET").to_ascii_uppercase();
    let mut r = if method == "POST" {
        client.post(url).body(req.body.clone().unwrap_or_default())
    } else {
        client.get(url)
    };
    r = r.header(
        "Referer",
        format!("{}/", referer_base.trim_end_matches('/')),
    );
    for (k, v) in &s.headers {
        r = r.header(k.as_str(), fill(v, s, &[]));
    }
    if method == "POST" && req.body.is_some() {
        r = r.header("Content-Type", "application/x-www-form-urlencoded");
    }
    let resp = r.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let final_url = resp.url().to_string();
    if !status.is_success() {
        let hint = if status.as_u16() == 403 || status.as_u16() == 503 {
            " (the site may be behind a bot check)"
        } else {
            ""
        };
        return Err(format!("{status} from {url}{hint}"));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    Ok((text, final_url))
}

async fn fetch(
    client: &reqwest::Client,
    s: &DeclarativeSource,
    req: &Request,
    url: &str,
) -> Result<(Doc, String), String> {
    let (text, final_url) = match fetch_text_once(client, s, req, url, &s.base_url).await {
        Ok(res) => res,
        Err(first_err) => {
            let base = s.base_url.trim_end_matches('/');
            if s.mirrors.is_empty() || base.is_empty() || !url.starts_with(base) {
                return Err(first_err);
            }
            let suffix = &url[base.len()..];
            let mut last_err = first_err;
            let mut found = None;
            for mirror in &s.mirrors {
                let m = mirror.trim().trim_end_matches('/');
                if m.is_empty() || m == base {
                    continue;
                }
                let alt_url = format!("{m}{suffix}");
                match fetch_text_once(client, s, req, &alt_url, m).await {
                    Ok(res) => {
                        found = Some(res);
                        break;
                    }
                    Err(e) => last_err = e,
                }
            }
            match found {
                Some(res) => res,
                None => return Err(last_err),
            }
        }
    };
    let doc = if req.json {
        Doc::Json(serde_json::from_str(&text).map_err(|e| e.to_string())?)
    } else {
        Doc::Html(Html::parse_document(&text))
    };
    Ok((doc, final_url))
}

async fn throttle(id: &str, ms: u64) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<HashMap<String, std::time::Instant>>> = Mutex::new(None);
    let wait = {
        let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let map = g.get_or_insert_with(HashMap::new);
        let now = std::time::Instant::now();
        let next = map
            .get(id)
            .map(|t| *t + std::time::Duration::from_millis(ms))
            .unwrap_or(now);
        let slot = next.max(now);
        map.insert(id.to_string(), slot);
        slot.saturating_duration_since(now)
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

fn sel(css: &str) -> Option<Selector> {
    let re = regex::Regex::new(r":contains\([^)]*\)").ok()?;
    Selector::parse(&re.replace_all(css, "")).ok()
}

pub fn json_path<'a>(v: &'a Value, path: &str) -> Vec<&'a Value> {
    let mut cur = vec![v];
    for seg in path
        .trim_start_matches("$.")
        .split('.')
        .filter(|s| !s.is_empty())
    {
        let mut next = Vec::new();
        for c in cur {
            if seg == "*" {
                match c {
                    Value::Array(a) => next.extend(a.iter()),
                    Value::Object(o) => next.extend(o.values()),
                    _ => {}
                }
            } else if let Ok(i) = seg.parse::<usize>() {
                if let Some(x) = c.get(i) {
                    next.push(x);
                }
            } else if let Some(x) = c.get(seg) {
                next.push(x);
            }
        }
        cur = next;
    }
    cur
}

fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn post(ex: &Extractor, raw: String, page_url: &str) -> String {
    let mut s = raw.trim().to_string();
    if s.contains(", ") && s.split(", ").all(|p| p.contains(' ')) {
        s = s.split(' ').next().unwrap_or("").to_string();
    }
    if let Some(r) = &ex.regex {
        if let Ok(re) = regex::Regex::new(r) {
            s = re
                .captures(&s)
                .map(|c| {
                    c.get(1)
                        .or(c.get(0))
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
        }
    }
    if let Some((a, b)) = &ex.replace {
        s = s.replace(a.as_str(), b.as_str());
    }
    if let Some(p) = &ex.prefix {
        if !s.is_empty() {
            s = format!("{p}{s}");
        }
    }
    if ex.abs && !s.is_empty() {
        if let Ok(u) = url::Url::parse(page_url).and_then(|b| b.join(&s)) {
            s = u.to_string();
        }
    }
    s
}

fn from_element(ex: &Extractor, el: ElementRef<'_>, page_url: &str) -> Vec<String> {
    if let Some(v) = &ex.value {
        return vec![v.clone()];
    }
    let targets: Vec<ElementRef<'_>> = match ex.css.as_deref().and_then(sel) {
        Some(s) => el.select(&s).collect(),
        None => vec![el],
    };
    let read = |e: ElementRef<'_>| -> String {
        match &ex.attr {
            Some(attrs) => attrs
                .iter()
                .find_map(|a| e.value().attr(a).filter(|v| !v.trim().is_empty()))
                .unwrap_or("")
                .to_string(),
            None => e
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        }
    };
    let take = if ex.all { usize::MAX } else { 1 };
    targets
        .into_iter()
        .take(take)
        .map(|e| post(ex, read(e), page_url))
        .filter(|s| !s.is_empty())
        .collect()
}

fn from_json(ex: &Extractor, v: &Value, page_url: &str) -> Vec<String> {
    if let Some(c) = &ex.value {
        return vec![c.clone()];
    }
    let found = match &ex.json {
        Some(p) => json_path(v, p),
        None => vec![v],
    };
    let take = if ex.all { usize::MAX } else { 1 };
    found
        .into_iter()
        .flat_map(|x| match x {
            Value::Array(a) if ex.all => a.iter().map(value_str).collect::<Vec<_>>(),
            other => vec![value_str(other)],
        })
        .take(take)
        .map(|s| post(ex, s, page_url))
        .filter(|s| !s.is_empty())
        .collect()
}

fn one(ex: &Extractor, el: ElementRef<'_>, url: &str) -> String {
    from_element(ex, el, url)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn one_json(ex: &Extractor, v: &Value, url: &str) -> String {
    from_json(ex, v, url).into_iter().next().unwrap_or_default()
}

fn abs_key(k: String, page_url: &str) -> String {
    if k.starts_with('/') || k.starts_with("./") || k.starts_with("../") {
        url::Url::parse(page_url)
            .and_then(|b| b.join(&k))
            .map(|u| u.to_string())
            .unwrap_or(k)
    } else {
        k
    }
}

fn image_abs(v: String, page_url: &str) -> Option<String> {
    if v.is_empty() || v.starts_with("data:") {
        return None;
    }
    url::Url::parse(page_url)
        .and_then(|b| b.join(v.trim()))
        .map(|u| u.to_string())
        .ok()
}

fn infer_book_format(url: &str, label: &str) -> String {
    let u = url.to_ascii_lowercase();
    let l = label.to_ascii_lowercase();
    for ext in [
        "epub", "pdf", "mobi", "azw3", "cbz", "cbr", "cb7", "djvu", "fb2", "fbz", "txt",
    ] {
        let dot = format!(".{ext}");
        if u.contains(&dot) || l.contains(ext) {
            return ext.to_string();
        }
    }
    "file".to_string()
}

fn extract_downloads_html(
    spec: &DownloadsSpec,
    el: ElementRef<'_>,
    page_url: &str,
) -> Vec<Download> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let nodes: Vec<ElementRef<'_>> = if spec.list.trim().is_empty() {
        vec![el]
    } else if let Some(s) = sel(&spec.list) {
        el.select(&s).collect()
    } else {
        vec![]
    };
    for node in nodes {
        let raw_url = one(&spec.url, node, page_url);
        let url = abs_key(raw_url, page_url);
        if url.is_empty() || !seen.insert(url.clone()) {
            continue;
        }
        let label = spec
            .label
            .as_ref()
            .map(|x| one(x, node, page_url))
            .unwrap_or_default();
        let fmt = spec
            .format
            .as_ref()
            .map(|x| one(x, node, page_url).to_ascii_lowercase())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| infer_book_format(&url, &label));
        let final_label = if label.is_empty() {
            fmt.to_uppercase()
        } else {
            label
        };
        out.push(Download {
            url,
            format: fmt,
            label: final_label,
        });
    }
    out
}

fn extract_downloads_json(spec: &DownloadsSpec, v: &Value, page_url: &str) -> Vec<Download> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let nodes: Vec<&Value> = if spec.list.trim().is_empty() {
        vec![v]
    } else {
        json_path(v, &spec.list)
    };
    for node in nodes {
        let raw_url = one_json(&spec.url, node, page_url);
        let url = abs_key(raw_url, page_url);
        if url.is_empty() || !seen.insert(url.clone()) {
            continue;
        }
        let label = spec
            .label
            .as_ref()
            .map(|x| one_json(x, node, page_url))
            .unwrap_or_default();
        let fmt = spec
            .format
            .as_ref()
            .map(|x| one_json(x, node, page_url).to_ascii_lowercase())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| infer_book_format(&url, &label));
        let final_label = if label.is_empty() {
            fmt.to_uppercase()
        } else {
            label
        };
        out.push(Download {
            url,
            format: fmt,
            label: final_label,
        });
    }
    out
}

pub async fn browse(
    client: &reqwest::Client,
    s: &DeclarativeSource,
    mode: &str,
    query: &str,
    page: u32,
    filters: &Value,
) -> Result<Page, String> {
    let spec = match mode {
        "search" => s.search.as_ref().or(s.popular.as_ref()),
        "latest" => s.latest.as_ref().or(s.popular.as_ref()),
        _ => s.popular.as_ref().or(s.search.as_ref()),
    }
    .ok_or("this source can't list")?;
    let pg = page.max(1).to_string();
    let mut filter_owned: Vec<(String, String)> = Vec::new();
    if let Some(obj) = filters.as_object() {
        for (k, v) in obj {
            let s_val = value_str(v);
            if !s_val.is_empty() {
                filter_owned.push((format!("filter.{k}"), s_val));
            }
        }
    }
    let mut vars: Vec<(&str, &str)> = vec![("page", &pg), ("query", query)];
    for (k, v) in &filter_owned {
        vars.push((k.as_str(), v.as_str()));
    }
    let url = fill(&spec.request.url, s, &vars);
    let (doc, final_url) = fetch(client, s, &spec.request, &url).await?;
    let mut hits = Vec::new();
    let mut has_next = false;
    match &doc {
        Doc::Html(h) => {
            let root = h.root_element();
            if let Some(ls) = sel(&spec.list) {
                for el in root.select(&ls) {
                    let key = abs_key(one(&spec.key, el, &final_url), &final_url);
                    let title = one(&spec.title, el, &final_url);
                    if key.is_empty() || title.is_empty() {
                        continue;
                    }
                    let cover = spec
                        .cover
                        .as_ref()
                        .map(|c| one(c, el, &final_url))
                        .and_then(|c| image_abs(c, &final_url));
                    let mut h = hit(s, key, title, cover);
                    if let Some(a) = &spec.authors {
                        let mut ex = a.clone();
                        ex.all = true;
                        h.authors = from_element(&ex, el, &final_url);
                    }
                    if let Some(dl) = &spec.downloads {
                        h.downloads = extract_downloads_html(dl, el, &final_url);
                    }
                    hits.push(h);
                }
            }
            if let Some(n) = &spec.has_next {
                has_next = !from_element(n, root, &final_url).is_empty()
                    || n.css
                        .as_deref()
                        .and_then(sel)
                        .is_some_and(|x| root.select(&x).next().is_some());
            }
        }
        Doc::Json(v) => {
            for item in json_path(v, &spec.list) {
                let key = one_json(&spec.key, item, &final_url);
                let title = one_json(&spec.title, item, &final_url);
                if key.is_empty() || title.is_empty() {
                    continue;
                }
                let cover = spec
                    .cover
                    .as_ref()
                    .map(|c| one_json(c, item, &final_url))
                    .and_then(|c| image_abs(c, &final_url));
                let mut h = hit(s, key, title, cover);
                if let Some(a) = &spec.authors {
                    let mut ex = a.clone();
                    ex.all = true;
                    h.authors = from_json(&ex, item, &final_url);
                }
                if let Some(dl) = &spec.downloads {
                    h.downloads = extract_downloads_json(dl, item, &final_url);
                }
                hits.push(h);
            }
            if let Some(n) = &spec.has_next {
                let v = one_json(n, v, &final_url);
                has_next = !v.is_empty() && v != "false" && v != "0";
            }
        }
    }
    Ok(Page {
        total: None,
        next: (has_next && !hits.is_empty()).then(|| (page.max(1) + 1).to_string()),
        hits,
    })
}

fn hit(s: &DeclarativeSource, key: String, title: String, cover: Option<String>) -> Hit {
    Hit {
        source: s.id.clone(),
        url: if key.starts_with("http") {
            key.clone()
        } else {
            String::new()
        },
        key,
        title,
        cover,
        kind: s
            .kind
            .clone()
            .unwrap_or_else(|| "series".into())
            .replace("manga", "series"),
        language: s.lang.clone(),
        ..Default::default()
    }
}

pub async fn details(
    client: &reqwest::Client,
    s: &DeclarativeSource,
    key: &str,
) -> Result<(Hit, Vec<RemoteChapter>), String> {
    let d = s.details.clone().unwrap_or_default();
    let url = fill(
        if d.request.url.is_empty() {
            "{key}"
        } else {
            &d.request.url
        },
        s,
        &[("key", key)],
    );
    let (h, same_page) = {
        let (doc, final_url) = fetch(client, s, &d.request, &url).await?;
        let mut h = hit(s, key.to_string(), String::new(), None);
        h.url = final_url.clone();
        let get_list = |ex: &Option<Extractor>| -> Vec<String> {
            let Some(ex) = ex else { return vec![] };
            let mut ex = ex.clone();
            ex.all = true;
            match &doc {
                Doc::Html(x) => from_element(&ex, x.root_element(), &final_url),
                Doc::Json(v) => from_json(&ex, v, &final_url),
            }
        };
        let get = |ex: &Option<Extractor>| -> String {
            let Some(ex) = ex else { return String::new() };
            match &doc {
                Doc::Html(x) => one(ex, x.root_element(), &final_url),
                Doc::Json(v) => one_json(ex, v, &final_url),
            }
        };
        h.title = get(&d.title);
        h.summary = get(&d.description);
        h.authors = get_list(&d.authors);
        h.subjects = get_list(&d.genres);
        h.cover = image_abs(get(&d.cover), &final_url);
        if let Some(dl) = &d.downloads {
            h.downloads = match &doc {
                Doc::Html(x) => extract_downloads_html(dl, x.root_element(), &final_url),
                Doc::Json(v) => extract_downloads_json(dl, v, &final_url),
            };
        }
        h.extra = serde_json::json!({ "status": get(&d.status) });
        let same_page = match &s.chapters {
            Some(c) if c.request.url.is_empty() || c.request.url == "{key}" => {
                Some(parse_chapters(s, c, &doc, &final_url))
            }
            _ => None,
        };
        (h, same_page)
    };
    let chapters = match (same_page, &s.chapters) {
        (Some(list), _) => list,
        (None, Some(c)) => fetch_chapters(client, s, c, key).await?,
        (None, None) => vec![],
    };
    Ok((h, chapters))
}

async fn fetch_chapters(
    client: &reqwest::Client,
    s: &DeclarativeSource,
    c: &ChaptersSpec,
    key: &str,
) -> Result<Vec<RemoteChapter>, String> {
    let key_slash = if key.ends_with('/') {
        key.to_string()
    } else {
        format!("{key}/")
    };
    let url = fill(&c.request.url, s, &[("key", &key_slash)]);
    let err = match fetch(client, s, &c.request, &url).await {
        Ok((doc, page_url)) => return Ok(parse_chapters(s, c, &doc, &page_url)),
        Err(e) => e,
    };
    if s.template.as_deref() != Some("madara") {
        return Err(err);
    }
    let fallback = Request {
        url: String::new(),
        method: Some("POST".into()),
        body: None,
        json: false,
    };
    let ajax = format!(
        "{}/wp-admin/admin-ajax.php",
        s.base_url.trim_end_matches('/')
    );
    match fetch(client, s, &fallback, &ajax).await {
        Ok((doc, page_url)) => Ok(parse_chapters(s, c, &doc, &page_url)),
        Err(_) => Err(err),
    }
}

fn parse_chapters(
    s: &DeclarativeSource,
    c: &ChaptersSpec,
    doc: &Doc,
    page_url: &str,
) -> Vec<RemoteChapter> {
    let mut out = Vec::new();
    let mut push = |key: String, title: String, number: String, date: String, scan: String| {
        if key.is_empty() {
            return;
        }
        let num = number
            .parse::<f64>()
            .ok()
            .or_else(|| crate::core::read::comic::parse_chapter_number(&title).0);
        out.push(RemoteChapter {
            key,
            title,
            number: num,
            volume: None,
            language: s.lang.clone(),
            scanlator: scan,
            published: parse_date(&date),
            pages: None,
            external_url: None,
        });
    };
    match doc {
        Doc::Html(h) => {
            if let Some(ls) = sel(&c.list) {
                for el in h.root_element().select(&ls) {
                    push(
                        abs_key(one(&c.key, el, page_url), page_url),
                        one(&c.title, el, page_url),
                        c.number
                            .as_ref()
                            .map(|n| one(n, el, page_url))
                            .unwrap_or_default(),
                        c.date
                            .as_ref()
                            .map(|n| one(n, el, page_url))
                            .unwrap_or_default(),
                        c.scanlator
                            .as_ref()
                            .map(|n| one(n, el, page_url))
                            .unwrap_or_default(),
                    );
                }
            }
        }
        Doc::Json(v) => {
            for item in json_path(v, &c.list) {
                push(
                    one_json(&c.key, item, page_url),
                    one_json(&c.title, item, page_url),
                    c.number
                        .as_ref()
                        .map(|n| one_json(n, item, page_url))
                        .unwrap_or_default(),
                    c.date
                        .as_ref()
                        .map(|n| one_json(n, item, page_url))
                        .unwrap_or_default(),
                    c.scanlator
                        .as_ref()
                        .map(|n| one_json(n, item, page_url))
                        .unwrap_or_default(),
                );
            }
        }
    }
    if c.order.as_deref().unwrap_or("desc") == "desc" {
        out.reverse();
    }
    out
}

pub fn parse_date(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(n) = t.parse::<i64>() {
        return Some(if n < 10_000_000_000 { n * 1000 } else { n });
    }
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(d.timestamp_millis());
    }
    for f in [
        "%B %d, %Y",
        "%b %d, %Y",
        "%d/%m/%Y",
        "%Y-%m-%d",
        "%m/%d/%Y",
        "%d %B %Y",
        "%d.%m.%Y",
    ] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(t, f) {
            return d
                .and_hms_opt(0, 0, 0)
                .map(|x| x.and_utc().timestamp_millis());
        }
    }
    let lower = t.to_lowercase();
    let n: i64 = lower
        .split_whitespace()
        .find_map(|w| w.parse().ok())
        .unwrap_or(1);
    let unit = [
        ("sec", 1_000),
        ("min", 60_000),
        ("hour", 3_600_000),
        ("hora", 3_600_000),
        ("day", 86_400_000),
        ("dia", 86_400_000),
        ("week", 604_800_000),
        ("semana", 604_800_000),
        ("month", 2_592_000_000),
        ("mes", 2_592_000_000),
        ("mês", 2_592_000_000),
        ("year", 31_536_000_000),
        ("ano", 31_536_000_000),
    ]
    .iter()
    .find(|(k, _)| lower.contains(k))
    .map(|(_, v)| *v)?;
    Some(chrono::Utc::now().timestamp_millis() - n * unit)
}

pub async fn pages(
    client: &reqwest::Client,
    s: &DeclarativeSource,
    chapter_key: &str,
) -> Result<Vec<RemotePage>, String> {
    let p = s.pages.clone().ok_or("this source has no page rules")?;
    let url = fill(
        if p.request.url.is_empty() {
            "{key}"
        } else {
            &p.request.url
        },
        s,
        &[("key", chapter_key)],
    );
    let (doc, final_url) = fetch(client, s, &p.request, &url).await?;
    let mut urls: Vec<String> = Vec::new();
    match &doc {
        Doc::Html(h) => {
            if let Some(ls) = sel(&p.list) {
                for el in h.root_element().select(&ls) {
                    if let Some(u) = image_abs(one(&p.image, el, &final_url), &final_url) {
                        urls.push(u);
                    }
                }
            }
            if urls.is_empty() {
                if let Some(re) = p
                    .script_json
                    .as_deref()
                    .and_then(|r| regex::Regex::new(r).ok())
                {
                    let html = h.html();
                    if let Some(c) = re.captures(&html) {
                        let raw = c
                            .get(1)
                            .map(|m| m.as_str())
                            .unwrap_or("[]")
                            .replace("\\/", "/");
                        if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
                            urls.extend(list.into_iter().filter_map(|u| image_abs(u, &final_url)));
                        }
                    }
                }
            }
        }
        Doc::Json(v) => {
            for item in json_path(v, &p.list) {
                if let Some(u) = image_abs(one_json(&p.image, item, &final_url), &final_url) {
                    urls.push(u);
                }
            }
        }
    }
    if urls.is_empty() {
        return Err("no pages found".into());
    }
    let referer = p.referer.unwrap_or(true).then(|| final_url.clone());
    Ok(urls
        .into_iter()
        .map(|u| RemotePage {
            url: u,
            headers: referer
                .iter()
                .map(|r| ("Referer".to_string(), r.clone()))
                .collect(),
            xor_key: None,
        })
        .collect())
}

#[derive(Debug, Serialize, Default)]
pub struct HealthReport {
    pub list: Option<String>,
    pub details: Option<String>,
    pub chapters: Option<String>,
    pub pages: Option<String>,
    pub ok: bool,
    pub sample: Option<String>,
}

pub async fn health(client: &reqwest::Client, s: &DeclarativeSource) -> HealthReport {
    let mut r = HealthReport::default();
    let list = browse(client, s, "popular", "", 1, &Value::Null).await;
    let first = match list {
        Ok(p) if !p.hits.is_empty() => p.hits.into_iter().next(),
        Ok(_) => {
            r.list = Some("no items".into());
            return r;
        }
        Err(e) => {
            r.list = Some(e);
            return r;
        }
    };
    let Some(first) = first else { return r };
    r.sample = Some(first.title.clone());
    let (_, chs) = match details(client, s, &first.key).await {
        Ok(x) => x,
        Err(e) => {
            r.details = Some(e);
            return r;
        }
    };
    let Some(ch) = chs.last() else {
        r.chapters = Some("no chapters".into());
        return r;
    };
    if let Err(e) = pages(client, s, &ch.key).await {
        r.pages = Some(e);
        return r;
    }
    r.ok = true;
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_url_stays_a_string_while_download_url_becomes_an_extractor() {
        let s = parse_source(serde_json::json!({
            "id": "t",
            "name": "T",
            "baseUrl": "https://books.example",
            "popular": {
                "url": "{baseUrl}/books",
                "list": "li",
                "key": "a@href",
                "title": "a",
                "downloads": { "list": "a.dl", "url": "@href", "format": "span.fmt", "label": "span.lbl" }
            }
        }))
        .expect("source parses");
        let popular = s.popular.expect("popular");
        assert_eq!(popular.request.url, "{baseUrl}/books");
        let dl = popular.downloads.expect("downloads");
        assert_eq!(dl.url.attr, Some(vec!["href".to_string()]));
        assert_eq!(dl.format.and_then(|f| f.css), Some("span.fmt".to_string()));
        assert_eq!(dl.label.and_then(|l| l.css), Some("span.lbl".to_string()));
    }
}
