use std::collections::BTreeMap;
use std::time::Duration;

use scraper::{ElementRef, Html, Node, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{cap_chars, estimate_tokens, http, DevError};

pub const DEFAULT_MAX_TOKENS: usize = 4000;
pub const DEFAULT_SELECT_LIMIT: usize = 20;
const STRUCTURED_BLOCKS: usize = 5;
const STRUCTURED_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Markdown,
    Text,
    Html,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FetchRequest {
    pub url: String,
    pub urls: Vec<String>,
    pub format: Format,
    pub max_tokens: Option<usize>,
    pub raw: bool,
    pub if_hash: Option<String>,
}

pub const MAX_URLS: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PageOrError {
    Page(Box<Page>),
    Error { url: String, error: Value },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pages {
    pub pages: Vec<PageOrError>,
    pub total: usize,
    pub failed: usize,
    pub truncated: bool,
    pub untrusted: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SelectRequest {
    pub url: Option<String>,
    pub html: Option<String>,
    pub selector: String,
    pub attr: Option<String>,
    pub limit: Option<usize>,
    pub remember: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Structured {
    pub kind: &'static str,
    pub id: Option<String>,
    pub chars: usize,
    pub truncated: bool,
    pub data: Option<Value>,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub title: Option<String>,
    pub content: String,
    pub removed: BTreeMap<&'static str, usize>,
    pub structured: Vec<Structured>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub url: String,
    pub final_url: String,
    pub status: u16,
    pub via: &'static str,
    pub title: Option<String>,
    pub format: Format,
    pub content: String,
    pub tokens: usize,
    pub total: usize,
    pub truncated: bool,
    pub note: Option<&'static str>,
    pub removed: BTreeMap<&'static str, usize>,
    pub structured: Vec<Structured>,
    pub hash: String,
    pub unchanged: bool,
    pub untrusted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub matches: Vec<String>,
    pub total: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relocated: Option<String>,
    pub untrusted: bool,
}

const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "figure",
    "figcaption",
    "details",
    "summary",
    "dl",
    "dt",
    "dd",
    "address",
    "form",
    "fieldset",
    "body",
    "html",
    "li",
];
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];
const MATCH_CHARS: usize = 500;
const MAX_PAGE_BYTES: usize = 5 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

fn is_code(name: &str) -> bool {
    matches!(name, "script" | "style" | "noscript" | "template")
}

fn inside_content(el: ElementRef) -> bool {
    el.ancestors()
        .filter_map(ElementRef::wrap)
        .any(|a| matches!(a.value().name(), "article" | "main"))
}

fn chrome_kind(el: ElementRef) -> Option<&'static str> {
    let e = el.value();
    match e.name() {
        "script" => return Some("script"),
        "style" => return Some("style"),
        "noscript" | "template" | "iframe" | "svg" => return Some("other"),
        "nav" => return Some("nav"),
        "aside" => return Some("aside"),
        "header" if !inside_content(el) => return Some("header"),
        "footer" if !inside_content(el) => return Some("footer"),
        _ => {}
    }
    match e.attr("role") {
        Some("navigation") => return Some("nav"),
        Some("banner") => return Some("header"),
        Some("contentinfo") => return Some("footer"),
        Some("complementary") => return Some("aside"),
        _ => {}
    }
    if e.attr("hidden").is_some() || e.attr("aria-hidden") == Some("true") {
        return Some("hidden");
    }
    let mut names = e.classes().chain(e.id()).map(str::to_ascii_lowercase);
    names.find_map(|name| {
        if name.contains("cookie") || name.contains("consent") {
            Some("cookie")
        } else if matches!(name.as_str(), "ad" | "ads" | "sponsored")
            || name.starts_with("ad-")
            || name.starts_with("ads-")
            || name.starts_with("advert")
            || name.ends_with("-ad")
            || name.ends_with("-ads")
        {
            Some("ads")
        } else {
            None
        }
    })
}

fn count_removed(el: ElementRef, removed: &mut BTreeMap<&'static str, usize>) {
    if let Some(kind) = chrome_kind(el) {
        *removed.entry(kind).or_default() += 1;
        return;
    }
    for child in el.children().filter_map(ElementRef::wrap) {
        count_removed(child, removed);
    }
}

fn text_len(el: ElementRef) -> usize {
    el.text().map(str::len).sum()
}

fn content_root(doc: &Html) -> ElementRef<'_> {
    for css in ["main, [role=main]", "article", "body"] {
        let Ok(selector) = Selector::parse(css) else {
            continue;
        };
        let mut best: Option<ElementRef> = None;
        for el in doc.select(&selector) {
            if best.is_none_or(|b| text_len(el) > text_len(b)) {
                best = Some(el);
            }
        }
        if let Some(found) = best {
            return found;
        }
    }
    doc.root_element()
}

fn collapse(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

fn push(out: &mut String, piece: &str) {
    let piece = if out.is_empty() || out.ends_with('\n') || out.ends_with(' ') {
        piece.trim_start_matches(' ')
    } else {
        piece
    };
    out.push_str(piece);
}

fn finish(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

fn block(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!("\n\n{text}\n\n")
    }
}

struct Renderer<'a> {
    base: Option<&'a url::Url>,
    markdown: bool,
    raw: bool,
}

impl Renderer<'_> {
    fn skipped(&self, el: ElementRef) -> bool {
        if self.raw {
            is_code(el.value().name())
        } else {
            chrome_kind(el).is_some()
        }
    }

    fn link(&self, target: &str) -> String {
        self.base
            .and_then(|base| base.join(target).ok())
            .map_or_else(|| target.to_string(), |url| url.to_string())
    }

    fn children(&self, parent: ElementRef) -> String {
        let mut out = String::new();
        for child in parent.children() {
            match child.value() {
                Node::Text(text) => push(&mut out, &collapse(&text.text)),
                Node::Element(_) => {
                    if let Some(el) = ElementRef::wrap(child) {
                        push(&mut out, &self.element(el));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn wrapped(&self, el: ElementRef, mark: &str) -> String {
        let inner = self.children(el);
        if self.markdown && !inner.trim().is_empty() {
            format!("{mark}{}{mark}", inner.trim())
        } else {
            inner
        }
    }

    fn list(&self, el: ElementRef, ordered: bool) -> String {
        let mut lines = Vec::new();
        let items = el
            .children()
            .filter_map(ElementRef::wrap)
            .filter(|c| c.value().name() == "li" && !self.skipped(*c));
        for (index, item) in items.enumerate() {
            let body = finish(&self.children(item)).replace("\n\n", "\n");
            if body.is_empty() {
                continue;
            }
            let marker = if ordered {
                format!("{}. ", index + 1)
            } else {
                "- ".to_string()
            };
            lines.push(format!("{marker}{}", body.replace('\n', "\n  ")));
        }
        block(&lines.join("\n"))
    }

    fn table(&self, el: ElementRef) -> String {
        let Ok(rows) = Selector::parse("tr") else {
            return String::new();
        };
        let mut lines = Vec::new();
        for row in el.select(&rows) {
            let cells: Vec<String> = row
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|c| matches!(c.value().name(), "td" | "th"))
                .map(|c| {
                    finish(&self.children(c))
                        .replace('\n', " ")
                        .replace('|', "\\|")
                })
                .collect();
            if cells.is_empty() {
                continue;
            }
            if self.markdown {
                lines.push(format!("| {} |", cells.join(" | ")));
                if lines.len() == 1 {
                    lines.push(format!("|{}", " --- |".repeat(cells.len())));
                }
            } else {
                lines.push(cells.join(" | "));
            }
        }
        block(&lines.join("\n"))
    }

    fn element(&self, el: ElementRef) -> String {
        if self.skipped(el) {
            return String::new();
        }
        let e = el.value();
        let name = e.name();
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let title = finish(&self.children(el)).replace('\n', " ");
                let level = usize::from(name.as_bytes()[1] - b'0');
                if self.markdown && !title.is_empty() {
                    block(&format!("{} {title}", "#".repeat(level)))
                } else {
                    block(&title)
                }
            }
            "br" => "\n".to_string(),
            "hr" if self.markdown => block("---"),
            "a" => {
                let label = self.children(el);
                let target = e
                    .attr("href")
                    .filter(|h| !h.starts_with('#') && !h.starts_with("javascript:"));
                match target {
                    Some(target) if self.markdown && !label.trim().is_empty() => {
                        format!("[{}]({})", label.trim(), self.link(target))
                    }
                    _ => label,
                }
            }
            "img" => {
                let alt = e.attr("alt").unwrap_or_default().trim();
                match e.attr("src") {
                    Some(src) if self.markdown && !src.starts_with("data:") => {
                        format!("![{alt}]({})", self.link(src))
                    }
                    _ => alt.to_string(),
                }
            }
            "strong" | "b" => self.wrapped(el, "**"),
            "em" | "i" => self.wrapped(el, "*"),
            "code" => self.wrapped(el, "`"),
            "pre" => {
                let code: String = el.text().collect();
                let code = code.trim_matches('\n');
                if self.markdown {
                    block(&format!("```\n{code}\n```"))
                } else {
                    block(code)
                }
            }
            "blockquote" => {
                let inner = finish(&self.children(el));
                if self.markdown {
                    let quoted: Vec<String> = inner
                        .lines()
                        .map(|l| format!("> {l}").trim_end().to_string())
                        .collect();
                    block(&quoted.join("\n"))
                } else {
                    block(&inner)
                }
            }
            "ul" => self.list(el, false),
            "ol" => self.list(el, true),
            "table" => self.table(el),
            _ if BLOCKS.contains(&name) => block(&self.children(el)),
            _ => self.children(el),
        }
    }

    fn html(&self, el: ElementRef, out: &mut String) {
        if self.skipped(el) {
            return;
        }
        let e = el.value();
        out.push('<');
        out.push_str(e.name());
        for (name, value) in e.attrs() {
            out.push_str(&format!(" {name}=\"{}\"", escape(value, true)));
        }
        out.push('>');
        if VOID.contains(&e.name()) {
            return;
        }
        for child in el.children() {
            match child.value() {
                Node::Text(text) => out.push_str(&escape(&text.text, false)),
                Node::Element(_) => {
                    if let Some(inner) = ElementRef::wrap(child) {
                        self.html(inner, out);
                    }
                }
                _ => {}
            }
        }
        out.push_str(&format!("</{}>", e.name()));
    }
}

fn escape(text: &str, attribute: bool) -> String {
    let text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if attribute {
        text.replace('"', "&quot;")
    } else {
        text
    }
}

fn structured(doc: &Html) -> Vec<Structured> {
    let mut out = Vec::new();
    let Ok(scripts) = Selector::parse("script") else {
        return out;
    };
    for script in doc.select(&scripts) {
        let id = script.value().attr("id");
        let kind = match script.value().attr("type") {
            Some("application/ld+json") => "json_ld",
            Some("application/json") if id.is_some() => "embedded_state",
            _ => continue,
        };
        let raw: String = script.text().collect();
        let raw = raw.trim();
        let Ok(data) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let (preview, truncated, chars) = cap_chars(raw, STRUCTURED_CHARS);
        out.push(Structured {
            kind,
            id: id.map(str::to_string),
            chars,
            truncated,
            data: (!truncated).then_some(data),
            preview: truncated.then_some(preview),
        });
        if out.len() == STRUCTURED_BLOCKS {
            break;
        }
    }
    out
}

pub fn extract(html: &str, base: Option<&url::Url>, format: Format, raw: bool) -> Extracted {
    let doc = Html::parse_document(html);
    let title = Selector::parse("title")
        .ok()
        .and_then(|s| doc.select(&s).next())
        .map(|t| collapse(&t.text().collect::<String>()).trim().to_string())
        .filter(|t| !t.is_empty());
    let mut removed = BTreeMap::new();
    let root = if raw {
        doc.root_element()
    } else {
        count_removed(doc.root_element(), &mut removed);
        content_root(&doc)
    };
    let renderer = Renderer {
        base,
        markdown: format == Format::Markdown,
        raw,
    };
    let content = match format {
        Format::Html if raw => html.to_string(),
        Format::Html => {
            let mut out = String::new();
            renderer.html(root, &mut out);
            out
        }
        Format::Markdown | Format::Text => finish(&renderer.element(root)),
    };
    Extracted {
        title,
        content,
        removed,
        structured: structured(&doc),
    }
}

pub fn block_reason(status: u16, body: &str) -> Option<&'static str> {
    let has = |marks: &[&str]| marks.iter().any(|m| body.contains(m));
    let challenge = has(&["_cf_chl_opt"])
        || (status >= 400 && has(&["challenge-platform", "cf-chl-", "Just a moment..."]));
    if challenge {
        return Some("cloudflare_challenge");
    }
    match status {
        429 => Some("rate_limited"),
        401 => Some("login_required"),
        s if s >= 400 && has(&["g-recaptcha", "h-captcha", "captcha-delivery"]) => Some("captcha"),
        403 => Some("http_403"),
        _ => None,
    }
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let current = row[j + 1];
            row[j + 1] = (previous + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(current + 1);
            previous = current;
        }
    }
    row[b.len()]
}

fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    if a == b {
        return 1.0;
    }
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    if short.len() >= 3 && long.contains(short.as_str()) {
        return 0.6 + 0.4 * short.len() as f64 / long.len() as f64;
    }
    1.0 - distance(&a, &b) as f64 / long.chars().count().max(1) as f64
}

fn candidates(doc: &Html, selector: &str) -> Vec<String> {
    let Ok(token) = regex::Regex::new(r"([.#]?)([A-Za-z_][\w-]*)") else {
        return Vec::new();
    };
    let wanted: Vec<(bool, String)> = token
        .captures_iter(selector)
        .map(|c| (c[1].is_empty(), c[2].to_string()))
        .collect();
    let mut present: BTreeMap<String, bool> = BTreeMap::new();
    for el in doc
        .root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
    {
        let e = el.value();
        present.insert(e.name().to_string(), true);
        for class in e.classes() {
            present.insert(format!(".{class}"), false);
        }
        if let Some(id) = e.id() {
            present.insert(format!("#{id}"), false);
        }
    }
    let mut scored: Vec<(f64, String)> = present
        .into_iter()
        .filter_map(|(name, is_tag)| {
            let bare = name.trim_start_matches(['.', '#']);
            let best = wanted
                .iter()
                .filter(|(tag, _)| *tag == is_tag)
                .map(|(_, want)| similarity(want, bare))
                .fold(0.0, f64::max);
            (best >= 0.5).then_some((best, name))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(5).map(|(_, name)| name).collect()
}

pub fn select(
    html: &str,
    selector: &str,
    attr: Option<&str>,
    limit: usize,
) -> Result<Selection, DevError> {
    let parsed = Selector::parse(selector)
        .map_err(|e| DevError::InvalidInput(format!("invalid selector \"{selector}\": {e}")))?;
    let doc = Html::parse_document(html);
    let found: Vec<String> = doc
        .select(&parsed)
        .filter_map(|el| match attr {
            Some(name) => el.value().attr(name).map(str::to_string),
            None => Some(collapse(&el.text().collect::<String>()).trim().to_string()),
        })
        .map(|text| cap_chars(&text, MATCH_CHARS).0)
        .collect();
    if found.is_empty() {
        let (what, candidates) = match attr {
            Some(name) if doc.select(&parsed).next().is_some() => (
                format!("attribute {name} on selector {selector}"),
                Vec::new(),
            ),
            _ => (format!("selector {selector}"), candidates(&doc, selector)),
        };
        return Err(DevError::NoMatch { what, candidates });
    }
    let total = found.len();
    Ok(Selection {
        matches: found.into_iter().take(limit).collect(),
        total,
        truncated: total > limit,
        relocated: None,
        untrusted: true,
    })
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Fingerprint {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub text: String,
    pub parent: String,
}

const FINGERPRINT_TEXT: usize = 80;
const RELOCATE_SCORE: f64 = 0.6;

fn fingerprint(el: ElementRef) -> Fingerprint {
    let e = el.value();
    let parent = el
        .parent()
        .and_then(ElementRef::wrap)
        .map(|p| selector_for(p))
        .unwrap_or_default();
    Fingerprint {
        tag: e.name().to_string(),
        id: e.id().map(str::to_string),
        classes: e.classes().map(str::to_string).collect(),
        text: cap_chars(
            collapse(&el.text().collect::<String>()).trim(),
            FINGERPRINT_TEXT,
        )
        .0,
        parent,
    }
}

fn selector_for(el: ElementRef) -> String {
    let e = el.value();
    match e.id() {
        Some(id) => format!("{}#{id}", e.name()),
        None => e
            .classes()
            .fold(e.name().to_string(), |out, class| format!("{out}.{class}")),
    }
}

fn likeness(a: &Fingerprint, b: &Fingerprint) -> f64 {
    if a.tag != b.tag {
        return 0.0;
    }
    let id = match (&a.id, &b.id) {
        (Some(x), Some(y)) => similarity(x, y),
        (None, None) => 1.0,
        _ => 0.0,
    };
    let classes = if a.classes.is_empty() && b.classes.is_empty() {
        1.0
    } else {
        let best = |name: &String, others: &[String]| {
            others
                .iter()
                .map(|other| similarity(name, other))
                .fold(0.0, f64::max)
        };
        let sum: f64 = a.classes.iter().map(|c| best(c, &b.classes)).sum();
        sum / a.classes.len().max(b.classes.len()) as f64
    };
    let text = if a.text.is_empty() && b.text.is_empty() {
        1.0
    } else {
        similarity(&a.text, &b.text)
    };
    0.15 + 0.15 * id + 0.2 * classes + 0.4 * text + 0.1 * similarity(&a.parent, &b.parent)
}

fn relocate<'a>(doc: &'a Html, saved: &Fingerprint) -> Option<ElementRef<'a>> {
    doc.root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
        .map(|el| (likeness(saved, &fingerprint(el)), el))
        .filter(|(score, _)| *score >= RELOCATE_SCORE)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, el)| el)
}

pub fn select_adaptive(
    html: &str,
    selector: &str,
    attr: Option<&str>,
    limit: usize,
    saved: Option<&Fingerprint>,
) -> Result<(Selection, Option<Fingerprint>), DevError> {
    let no_match = match select(html, selector, attr, limit) {
        Ok(selection) => {
            let doc = Html::parse_document(html);
            let print = Selector::parse(selector)
                .ok()
                .and_then(|parsed| doc.select(&parsed).next().map(fingerprint));
            return Ok((selection, print));
        }
        Err(error @ DevError::NoMatch { .. }) => error,
        Err(other) => return Err(other),
    };
    let doc = Html::parse_document(html);
    let Some(el) = saved.and_then(|print| relocate(&doc, print)) else {
        return Err(no_match);
    };
    let value = match attr {
        Some(name) => el.value().attr(name).map(str::to_string),
        None => Some(collapse(&el.text().collect::<String>()).trim().to_string()),
    };
    let Some(value) = value else {
        return Err(no_match);
    };
    Ok((
        Selection {
            matches: vec![cap_chars(&value, MATCH_CHARS).0],
            total: 1,
            truncated: false,
            relocated: Some(selector_for(el)),
            untrusted: true,
        },
        Some(fingerprint(el)),
    ))
}

pub fn store_key(name: &str, url: Option<&str>) -> String {
    let host = url
        .and_then(|raw| url::Url::parse(raw).ok())
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_default();
    format!("{host}|{name}")
}

fn store_path() -> Option<std::path::PathBuf> {
    crate::core::paths::app_data_dir().map(|dir| dir.join("devtools").join("selectors.json"))
}

fn load_prints() -> BTreeMap<String, Fingerprint> {
    store_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_print(key: String, print: Fingerprint) {
    let Some(path) = store_path() else { return };
    let mut all = load_prints();
    if all.get(&key) == Some(&print) {
        return;
    }
    all.insert(key, print);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&all) {
        let _ = std::fs::write(path, bytes);
    }
}

pub fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(content.as_bytes())[..8])
}

pub fn retry_after_seconds(
    header: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<u64> {
    let value = header?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    let at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some((at.with_timezone(&chrono::Utc) - now).num_seconds().max(0) as u64)
}

pub fn checked_urls(urls: &[String]) -> Result<&[String], DevError> {
    if urls.is_empty() || urls.len() > MAX_URLS {
        return Err(DevError::InvalidInput(format!(
            "urls takes 1 to {MAX_URLS} addresses"
        )));
    }
    Ok(urls)
}

async fn get(raw: &str) -> Result<(u16, String, String), DevError> {
    let url = http::checked_url(raw)?;
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(http::USER_AGENT)
        .build()
        .map_err(http::network_error)?;
    let mut response = client.get(url).send().await.map_err(http::network_error)?;
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let retry_after = retry_after_seconds(
        response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        chrono::Utc::now(),
    );
    let (bytes, _) = http::read_capped(&mut response, MAX_PAGE_BYTES).await?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    if let Some(reason) = block_reason(status, &body) {
        return Err(DevError::Blocked {
            reason,
            status,
            retry_after,
        });
    }
    if status >= 400 {
        return Err(DevError::HttpStatus(status));
    }
    Ok((status, final_url, body))
}

#[allow(clippy::too_many_arguments)]
pub fn finish_page(
    url: &str,
    final_url: &str,
    status: u16,
    html: &str,
    format: Format,
    raw: bool,
    max_tokens: Option<usize>,
    if_hash: Option<&str>,
) -> Page {
    let base = url::Url::parse(final_url).ok();
    let extracted = extract(html, base.as_ref(), format, raw);
    let hash = content_hash(&extracted.content);
    let unchanged = if_hash == Some(hash.as_str());
    let max_chars = max_tokens.unwrap_or(DEFAULT_MAX_TOKENS) * 4;
    let (content, truncated, total_chars) = if unchanged {
        (String::new(), false, extracted.content.chars().count())
    } else {
        cap_chars(&extracted.content, max_chars)
    };
    let note = (content.is_empty() && !unchanged).then_some(
        "no text content was found; the page may render with JavaScript, so read it with gd_browser_open and gd_snapshot",
    );
    Page {
        url: url.to_string(),
        final_url: final_url.to_string(),
        status,
        via: "http",
        title: extracted.title,
        format,
        tokens: estimate_tokens(&content),
        content,
        total: total_chars.div_ceil(4),
        truncated,
        note,
        removed: extracted.removed,
        structured: if unchanged {
            Vec::new()
        } else {
            extracted.structured
        },
        hash,
        unchanged,
        untrusted: true,
    }
}

pub async fn fetch(request: FetchRequest) -> Result<Page, DevError> {
    let (status, final_url, html) = get(&request.url).await?;
    Ok(finish_page(
        &request.url,
        &final_url,
        status,
        &html,
        request.format,
        request.raw,
        request.max_tokens,
        request.if_hash.as_deref(),
    ))
}

pub async fn fetch_many(request: FetchRequest) -> Result<Pages, DevError> {
    let urls = checked_urls(&request.urls)?.to_vec();
    let jobs = urls.into_iter().map(|url| {
        let one = FetchRequest {
            url: url.clone(),
            urls: Vec::new(),
            if_hash: None,
            ..request.clone()
        };
        async move {
            match fetch(one).await {
                Ok(page) => PageOrError::Page(Box::new(page)),
                Err(e) => {
                    let text = e.wire();
                    let (code, detail) = text.split_once(": ").unwrap_or(("ERROR", "{}"));
                    let mut error: Value = serde_json::from_str(detail).unwrap_or_default();
                    error["code"] = Value::from(code);
                    PageOrError::Error { url, error }
                }
            }
        }
    });
    let pages = futures::future::join_all(jobs).await;
    let failed = pages
        .iter()
        .filter(|p| matches!(p, PageOrError::Error { .. }))
        .count();
    Ok(Pages {
        total: pages.len(),
        failed,
        truncated: pages
            .iter()
            .any(|p| matches!(p, PageOrError::Page(page) if page.truncated)),
        pages,
        untrusted: true,
    })
}

pub async fn select_from(request: SelectRequest) -> Result<Selection, DevError> {
    let html = match (&request.html, &request.url) {
        (Some(html), _) => html.clone(),
        (None, Some(url)) => get(url).await?.2,
        (None, None) => return Err(DevError::InvalidInput("pass url or html".into())),
    };
    let limit = request.limit.unwrap_or(DEFAULT_SELECT_LIMIT);
    let attr = request.attr.as_deref();
    let Some(name) = request.remember.as_deref().filter(|n| !n.is_empty()) else {
        return select(&html, &request.selector, attr, limit);
    };
    let key = store_key(name, request.url.as_deref());
    let saved = load_prints().remove(&key);
    let (selection, print) =
        select_adaptive(&html, &request.selector, attr, limit, saved.as_ref())?;
    if let Some(print) = print {
        save_print(key, print);
    }
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remembered_element_is_found_again_after_its_class_changes() {
        let before = r#"<main><p>intro</p><div class="product-price">$42.00</div></main>"#;
        let (first, print) = select_adaptive(before, ".product-price", None, 5, None).unwrap();
        assert_eq!(first.matches, vec!["$42.00"]);
        assert_eq!(first.relocated, None);
        let print = print.unwrap();
        let after = r#"<main><p>intro</p><span class="note">new</span><div class="price-v2 product">$42.00</div></main>"#;
        let (again, _) = select_adaptive(after, ".product-price", None, 5, Some(&print)).unwrap();
        assert_eq!(again.matches, vec!["$42.00"]);
        assert_eq!(again.relocated.as_deref(), Some("div.price-v2.product"));
    }

    #[test]
    fn a_page_with_nothing_like_the_remembered_element_is_still_no_match() {
        let before = r#"<div class="product-price">$42.00</div>"#;
        let (_, print) = select_adaptive(before, ".product-price", None, 5, None).unwrap();
        let other = r#"<nav><a href="/">Home</a></nav><p>About us and our long history</p>"#;
        let error = select_adaptive(other, ".product-price", None, 5, print.as_ref()).unwrap_err();
        assert_eq!(error.code(), "NO_MATCH");
    }

    #[test]
    fn the_store_key_keeps_names_apart_per_site() {
        assert_eq!(
            store_key("price", Some("https://shop.test/a?x=1")),
            "shop.test|price"
        );
        assert_eq!(store_key("price", None), "|price");
    }

    #[test]
    fn the_same_content_hashes_the_same_and_a_change_does_not() {
        assert_eq!(content_hash("price 42"), content_hash("price 42"));
        assert_ne!(content_hash("price 42"), content_hash("price 43"));
        assert_eq!(content_hash("x").len(), 16);
    }

    #[test]
    fn retry_after_reads_seconds_or_a_date() {
        let now = chrono::DateTime::parse_from_rfc2822("Wed, 21 Oct 2026 07:28:00 GMT")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(retry_after_seconds(Some("120"), now), Some(120));
        assert_eq!(
            retry_after_seconds(Some("Wed, 21 Oct 2026 07:29:30 GMT"), now),
            Some(90)
        );
        assert_eq!(
            retry_after_seconds(Some("Wed, 21 Oct 2026 07:00:00 GMT"), now),
            Some(0)
        );
        assert_eq!(retry_after_seconds(Some("soon"), now), None);
        assert_eq!(retry_after_seconds(None, now), None);
    }

    #[test]
    fn a_page_with_the_hash_the_caller_already_has_comes_back_empty() {
        let page = finish_page(
            "https://a.test/",
            "https://a.test/",
            200,
            "hello world",
            Format::Text,
            false,
            None,
            None,
        );
        assert!(!page.unchanged);
        assert_eq!(page.content, "hello world");
        let again = finish_page(
            "https://a.test/",
            "https://a.test/",
            200,
            "hello world",
            Format::Text,
            false,
            None,
            Some(&page.hash),
        );
        assert!(again.unchanged);
        assert_eq!(again.content, "");
        assert_eq!(again.tokens, 0);
        assert_eq!(again.hash, page.hash);
    }

    #[test]
    fn more_than_ten_urls_or_none_are_refused() {
        assert!(checked_urls(&[]).is_err());
        let many: Vec<String> = (0..11).map(|i| format!("https://a.test/{i}")).collect();
        assert!(checked_urls(&many).is_err());
        assert_eq!(checked_urls(&many[..10]).unwrap().len(), 10);
    }

    const PAGE: &str = r#"<!doctype html><html><head><title>Shop - Widget</title><style>.x{color:red}</style>
<script type="application/ld+json">{"@type":"Product","name":"Widget"}</script>
<script id="__NEXT_DATA__" type="application/json">{"props":{"price":42}}</script>
<script>track()</script></head>
<body>
<header><a href="/">Home</a></header>
<nav><ul><li><a href="/a">Catalog</a></li></ul></nav>
<div class="cookie-banner">We use cookies</div>
<div class="box ad-slot">Buy now</div>
<main>
<h1>Widget</h1>
<p>A <strong>great</strong> widget with <a href="/docs">docs</a> and <code>code</code>.</p>
<ul><li>One</li><li>Two <em>fine</em></li></ul>
<pre><code>let x = 1;
let y = 2;</code></pre>
<blockquote><p>Quoted</p></blockquote>
<table><tr><th>Name</th><th>Price</th></tr><tr><td>Widget</td><td class="product-price">$42</td></tr></table>
<aside>Related</aside>
<p hidden>Secret</p>
<img src="/w.png" alt="Widget photo">
</main>
<footer>Copyright</footer>
</body></html>"#;

    fn base() -> url::Url {
        url::Url::parse("https://shop.test/p/widget").unwrap()
    }

    #[test]
    fn markdown_is_the_main_content_without_page_chrome() {
        let out = extract(PAGE, Some(&base()), Format::Markdown, false);
        assert_eq!(
            out.content,
            "# Widget\n\nA **great** widget with [docs](https://shop.test/docs) and `code`.\n\n- One\n- Two *fine*\n\n```\nlet x = 1;\nlet y = 2;\n```\n\n> Quoted\n\n| Name | Price |\n| --- | --- |\n| Widget | $42 |\n\n![Widget photo](https://shop.test/w.png)"
        );
        assert_eq!(out.title.as_deref(), Some("Shop - Widget"));
    }

    #[test]
    fn what_was_removed_is_counted_by_kind() {
        let out = extract(PAGE, None, Format::Markdown, false);
        let expected: BTreeMap<&str, usize> = [
            ("ads", 1),
            ("aside", 1),
            ("cookie", 1),
            ("footer", 1),
            ("header", 1),
            ("hidden", 1),
            ("nav", 1),
            ("script", 3),
            ("style", 1),
        ]
        .into_iter()
        .collect();
        assert_eq!(out.removed, expected);
    }

    #[test]
    fn text_has_no_markup() {
        let out = extract(PAGE, Some(&base()), Format::Text, false);
        assert!(out.content.starts_with("Widget\n\nA great widget with docs and code.\n\n- One\n- Two fine\n\nlet x = 1;\nlet y = 2;\n\nQuoted"));
        assert!(!out.content.contains("Copyright"));
        assert!(!out.content.contains('*'));
    }

    #[test]
    fn html_keeps_the_markup_of_the_main_content_only() {
        let out = extract(PAGE, None, Format::Html, false);
        assert!(out.content.contains("<h1>Widget</h1>"));
        assert!(out.content.contains("<td class=\"product-price\">$42</td>"));
        assert!(!out.content.contains("<nav"));
        assert!(!out.content.contains("<aside"));
        assert!(!out.content.contains("Copyright"));
    }

    #[test]
    fn raw_keeps_the_chrome_but_never_scripts_or_styles() {
        let out = extract(PAGE, None, Format::Markdown, true);
        assert!(out.content.contains("Catalog"));
        assert!(out.content.contains("Copyright"));
        assert!(out.content.contains("We use cookies"));
        assert!(!out.content.contains("track()"));
        assert!(!out.content.contains("color:red"));
        assert_eq!(out.removed.get("nav"), None);
    }

    #[test]
    fn a_page_without_main_falls_back_to_the_body_minus_chrome() {
        let html =
            "<body><nav>Menu</nav><div><p>Hello  \n world</p></div><footer>Foot</footer></body>";
        let out = extract(html, None, Format::Markdown, false);
        assert_eq!(out.content, "Hello world");
    }

    #[test]
    fn the_largest_article_wins_when_there_is_no_main() {
        let html = "<body><article><p>Short</p></article><article><h2>Long</h2><p>Much longer body text here</p></article></body>";
        let out = extract(html, None, Format::Markdown, false);
        assert_eq!(out.content, "## Long\n\nMuch longer body text here");
    }

    #[test]
    fn a_header_inside_the_article_is_content() {
        let html = "<body><header>Site</header><article><header><h1>Title</h1></header><p>Body</p></article></body>";
        let out = extract(html, None, Format::Markdown, false);
        assert_eq!(out.content, "# Title\n\nBody");
    }

    #[test]
    fn nested_and_ordered_lists_keep_their_shape() {
        let html = "<main><ol><li>First<ul><li>Inner</li></ul></li><li>Second</li></ol></main>";
        let out = extract(html, None, Format::Markdown, false);
        assert_eq!(out.content, "1. First\n  - Inner\n2. Second");
    }

    #[test]
    fn embedded_json_is_reported_as_structured_data() {
        let out = extract(PAGE, None, Format::Markdown, false);
        assert_eq!(out.structured.len(), 2);
        assert_eq!(out.structured[0].kind, "json_ld");
        assert_eq!(out.structured[0].data.as_ref().unwrap()["@type"], "Product");
        assert_eq!(out.structured[1].kind, "embedded_state");
        assert_eq!(out.structured[1].id.as_deref(), Some("__NEXT_DATA__"));
        assert_eq!(
            out.structured[1].data.as_ref().unwrap()["props"]["price"],
            42
        );
    }

    #[test]
    fn large_embedded_json_is_capped_to_a_preview() {
        let big = format!("{{\"items\":\"{}\"}}", "x".repeat(5000));
        let html = format!(
            "<head><script type=\"application/ld+json\">{big}</script></head><body><p>a</p></body>"
        );
        let out = extract(&html, None, Format::Text, false);
        let block = &out.structured[0];
        assert!(block.truncated);
        assert_eq!(block.data, None);
        assert_eq!(
            block.preview.as_ref().unwrap().chars().count(),
            STRUCTURED_CHARS
        );
        assert_eq!(block.chars, big.chars().count());
    }

    #[test]
    fn at_most_five_structured_blocks_are_returned() {
        let scripts = "<script type=\"application/ld+json\">{\"a\":1}</script>".repeat(8);
        let out = extract(
            &format!("<head>{scripts}</head><body></body>"),
            None,
            Format::Text,
            false,
        );
        assert_eq!(out.structured.len(), STRUCTURED_BLOCKS);
    }

    #[test]
    fn blocks_are_recognised_by_status_and_challenge_markers() {
        assert_eq!(block_reason(200, "<html><body>fine</body></html>"), None);
        assert_eq!(block_reason(404, "not found"), None);
        assert_eq!(block_reason(403, "Forbidden"), Some("http_403"));
        assert_eq!(block_reason(429, ""), Some("rate_limited"));
        assert_eq!(block_reason(401, ""), Some("login_required"));
        assert_eq!(
            block_reason(
                403,
                "<title>Just a moment...</title><script>window._cf_chl_opt={}</script>"
            ),
            Some("cloudflare_challenge")
        );
        assert_eq!(
            block_reason(503, "<div id=\"challenge-platform\"></div>"),
            Some("cloudflare_challenge")
        );
        assert_eq!(
            block_reason(
                200,
                "<title>Just a moment...</title><script>window._cf_chl_opt={}</script>"
            ),
            Some("cloudflare_challenge")
        );
        assert_eq!(
            block_reason(403, "<div class=\"g-recaptcha\"></div>"),
            Some("captcha")
        );
        assert_eq!(
            block_reason(200, "<form><div class=\"g-recaptcha\"></div></form>"),
            None
        );
    }

    #[test]
    fn select_returns_text_or_an_attribute() {
        let out = select(PAGE, ".product-price", None, 20).unwrap();
        assert_eq!(out.matches, vec!["$42"]);
        assert_eq!(out.total, 1);
        assert!(!out.truncated && out.untrusted);
        let out = select(PAGE, "main a", Some("href"), 20).unwrap();
        assert_eq!(out.matches, vec!["/docs"]);
    }

    #[test]
    fn select_caps_the_matches_and_reports_the_total() {
        let out = select(PAGE, "li", None, 2).unwrap();
        assert_eq!(out.matches, vec!["Catalog", "One"]);
        assert_eq!(out.total, 3);
        assert!(out.truncated);
    }

    #[test]
    fn zero_matches_is_an_error_with_similar_selectors() {
        let error = select(PAGE, "td.price", None, 20).unwrap_err();
        let DevError::NoMatch { what, candidates } = error else {
            panic!("expected NoMatch")
        };
        assert_eq!(what, "selector td.price");
        assert_eq!(candidates[0], "td");
        assert!(candidates.contains(&".product-price".to_string()));
        assert!(candidates.len() <= 5);
        let DevError::NoMatch { candidates, .. } = select(PAGE, "mian h2", None, 20).unwrap_err()
        else {
            panic!("expected NoMatch")
        };
        assert!(candidates.contains(&"main".to_string()));
    }

    #[test]
    fn an_attribute_nobody_has_is_also_no_match() {
        let error = select(PAGE, "h1", Some("href"), 20).unwrap_err();
        assert!(matches!(error, DevError::NoMatch { .. }));
    }

    #[test]
    fn a_broken_selector_is_invalid_input() {
        assert!(matches!(
            select(PAGE, "div[", None, 20),
            Err(DevError::InvalidInput(_))
        ));
    }

    #[tokio::test]
    async fn select_needs_a_url_or_html() {
        let error = select_from(SelectRequest {
            selector: "p".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
        assert!(matches!(error, DevError::InvalidInput(_)));
        let out = select_from(SelectRequest {
            html: Some("<p>hi</p>".into()),
            selector: "p".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(out.matches, vec!["hi"]);
    }

    #[tokio::test]
    #[ignore = "network: run by hand with --ignored"]
    async fn example_com_is_fetched_as_markdown() {
        let page = fetch(FetchRequest {
            url: "https://example.com".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(page.status, 200);
        assert_eq!(page.title.as_deref(), Some("Example Domain"));
        assert!(page.content.contains("documentation examples"));
        assert_eq!(page.removed.get("script"), Some(&1));
        assert!(page.untrusted && !page.truncated);
    }
}
