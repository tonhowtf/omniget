use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::scrape::{self, Format};
use super::{estimate_tokens, output_file, write_new, DevError};

pub const DEFAULT_PAGES: usize = 20;
pub const MAX_PAGES: usize = 500;
pub const DEFAULT_DEPTH: usize = 2;
pub const MAX_DEPTH: usize = 10;
pub const DEFAULT_DELAY_MS: u64 = 500;
pub const MAX_DELAY_MS: u64 = 30_000;
pub const DEFAULT_WAIT_MS: u64 = 20_000;
pub const MAX_WAIT_MS: u64 = 50_000;
pub const DEFAULT_ROWS: usize = 50;
const MAX_QUEUE: usize = 5_000;
const MAX_SEEN: usize = 20_000;
const MAX_FAILED: usize = 200;
const STATE_FILE: &str = "crawl.json";
const AGENT: &str = "omniget";
const SKIPPED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp", "avif", "css", "js", "mjs", "json",
    "xml", "rss", "atom", "pdf", "zip", "gz", "tar", "rar", "7z", "dmg", "exe", "msi", "apk",
    "mp3", "mp4", "m4a", "mkv", "webm", "mov", "avi", "wav", "flac", "woff", "woff2", "ttf", "eot",
    "doc", "docx", "xls", "xlsx", "ppt", "pptx", "iso", "bin",
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrawlRequest {
    pub url: String,
    pub output: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub max_pages: Option<usize>,
    pub max_depth: Option<usize>,
    pub format: Format,
    pub obey_robots: Option<bool>,
    pub delay_ms: Option<u64>,
    pub wait_ms: Option<u64>,
    pub limit: Option<usize>,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageRow {
    pub url: String,
    pub title: Option<String>,
    pub file: String,
    pub tokens: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailedRow {
    pub url: String,
    pub code: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub start: String,
    pub queue: VecDeque<(String, usize)>,
    pub seen: BTreeSet<String>,
    pub hashes: BTreeSet<String>,
    pub pages: Vec<PageRow>,
    pub failed: Vec<FailedRow>,
    pub failed_total: usize,
    pub duplicates: usize,
    pub skipped_by_robots: usize,
    pub dropped: usize,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crawled {
    pub output: String,
    pub start: String,
    pub pages: Vec<PageRow>,
    pub total: usize,
    pub truncated: bool,
    pub saved_now: usize,
    pub failed: Vec<FailedRow>,
    pub failed_total: usize,
    pub duplicates: usize,
    pub skipped_by_robots: usize,
    pub dropped: usize,
    pub queued: usize,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_s: Option<u64>,
    pub next_step: &'static str,
    pub untrusted: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Robots {
    rules: Vec<(bool, String)>,
    pub delay: Option<Duration>,
}

fn rule_matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(rest) => (rest, true),
        None => (pattern, false),
    };
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(mut rest) = path.strip_prefix(first) else {
        return false;
    };
    let mut wildcard = false;
    for part in parts {
        wildcard = true;
        if part.is_empty() {
            continue;
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    if anchored {
        return rest.is_empty() || (wildcard && pattern.ends_with('*'));
    }
    true
}

impl Robots {
    pub fn parse(text: &str) -> Self {
        let mut named = Robots::default();
        let mut wildcard = Robots::default();
        let (mut for_named, mut for_wildcard, mut in_agents) = (false, false, false);
        let mut found_named = false;
        for line in text.lines().take(5_000) {
            let line = line.split('#').next().unwrap_or_default().trim();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            if key == "user-agent" {
                if !in_agents {
                    (for_named, for_wildcard) = (false, false);
                }
                in_agents = true;
                let agent = value.to_ascii_lowercase();
                if agent == "*" {
                    for_wildcard = true;
                } else if !agent.is_empty() && AGENT.contains(&agent) {
                    for_named = true;
                    found_named = true;
                }
                continue;
            }
            in_agents = false;
            let targets = [(for_named, &mut named), (for_wildcard, &mut wildcard)];
            for (_, robots) in targets.into_iter().filter(|(on, _)| *on) {
                match key.as_str() {
                    "allow" | "disallow" if !value.is_empty() => {
                        robots.rules.push((key == "allow", value.to_string()));
                    }
                    "crawl-delay" => {
                        robots.delay = value
                            .parse::<f64>()
                            .ok()
                            .filter(|s| s.is_finite() && *s > 0.0)
                            .map(|s| Duration::from_millis((s * 1000.0) as u64));
                    }
                    _ => {}
                }
            }
        }
        if found_named {
            named
        } else {
            wildcard
        }
    }

    pub fn allows(&self, path: &str) -> bool {
        self.rules
            .iter()
            .filter(|(_, pattern)| rule_matches(pattern, path))
            .max_by_key(|(allow, pattern)| (pattern.len(), *allow))
            .is_none_or(|(allow, _)| *allow)
    }
}

fn pattern_matches(pattern: &str, url: &str) -> bool {
    let mut rest = url;
    let mut parts = pattern.split('*').peekable();
    let mut first = true;
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        if part.is_empty() {
            first = false;
            continue;
        }
        let found = if first && !pattern.starts_with('*') && pattern.contains("://") {
            rest.starts_with(part).then_some(0)
        } else {
            rest.find(part)
        };
        match found {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
        if last && pattern.ends_with('$') {
            return rest.is_empty();
        }
        first = false;
    }
    true
}

pub fn wanted(url: &url::Url, host: &str, include: &[String], exclude: &[String]) -> bool {
    if url.host_str() != Some(host) {
        return false;
    }
    let extension = url
        .path()
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    if extension.is_some_and(|ext| SKIPPED_EXTENSIONS.contains(&ext.as_str())) {
        return false;
    }
    let text = url.as_str();
    (include.is_empty() || include.iter().any(|p| pattern_matches(p, text)))
        && !exclude.iter().any(|p| pattern_matches(p, text))
}

pub fn file_name(index: usize, url: &url::Url, format: Format) -> String {
    let mut slug = String::new();
    let query = url.query().unwrap_or_default();
    for c in url.path().chars().chain(['?']).chain(query.chars()) {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 60 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let extension = match format {
        Format::Markdown => "md",
        Format::Text => "txt",
        Format::Html => "html",
    };
    format!(
        "{index:04}-{}.{extension}",
        if slug.is_empty() { "index" } else { slug }
    )
}

fn enqueue(state: &mut State, url: String, depth: usize) {
    if state.seen.contains(&url) {
        return;
    }
    if state.seen.len() >= MAX_SEEN || state.queue.len() >= MAX_QUEUE {
        state.dropped += 1;
        return;
    }
    state.seen.insert(url.clone());
    state.queue.push_back((url, depth));
}

fn fail(state: &mut State, url: &str, code: &str) {
    state.failed_total += 1;
    if state.failed.len() < MAX_FAILED {
        state.failed.push(FailedRow {
            url: url.to_string(),
            code: code.to_string(),
        });
    }
}

fn open_state(folder: &Path, start: &url::Url) -> Result<State, DevError> {
    let path = folder.join(STATE_FILE);
    let unreadable = |message: String| DevError::Read {
        path: path.clone(),
        message,
    };
    match std::fs::read(&path) {
        Ok(bytes) => {
            let state: State =
                serde_json::from_slice(&bytes).map_err(|e| unreadable(e.to_string()))?;
            if state.start != start.as_str() {
                return Err(DevError::InvalidInput(format!(
                    "{} holds a crawl of {}; pass that url to continue it, or another output folder",
                    folder.display(),
                    state.start
                )));
            }
            Ok(state)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let occupied = std::fs::read_dir(folder)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false);
            if occupied {
                return Err(DevError::OutputExists(folder.to_path_buf()));
            }
            let mut state = State {
                start: start.to_string(),
                ..State::default()
            };
            enqueue(&mut state, start.to_string(), 0);
            Ok(state)
        }
        Err(e) => Err(unreadable(e.to_string())),
    }
}

fn save_state(folder: &Path, state: &State) -> Result<(), DevError> {
    let path = folder.join(STATE_FILE);
    let failed = |message: String| DevError::Write {
        path: path.clone(),
        message,
    };
    std::fs::create_dir_all(folder).map_err(|e| failed(e.to_string()))?;
    let bytes = serde_json::to_vec(state).map_err(|e| failed(e.to_string()))?;
    let fresh = folder.join(format!("{STATE_FILE}.new"));
    std::fs::write(&fresh, bytes).map_err(|e| failed(e.to_string()))?;
    std::fs::rename(&fresh, &path).map_err(|e| failed(e.to_string()))
}

async fn robots_for(start: &url::Url, proxy: Option<&str>) -> Robots {
    let Ok(address) = start.join("/robots.txt") else {
        return Robots::default();
    };
    match scrape::get(address.as_str(), proxy).await {
        Ok(fetched) => Robots::parse(&fetched.body),
        Err(_) => Robots::default(),
    }
}

struct Plan<'a> {
    request: &'a CrawlRequest,
    folder: &'a Path,
    host: &'a str,
    max_depth: usize,
}

fn store_page(
    state: &mut State,
    plan: &Plan,
    url: &str,
    depth: usize,
    fetched: &scrape::Fetched,
) -> Result<(), DevError> {
    let Plan {
        request,
        folder,
        host,
        max_depth,
    } = *plan;
    let page = scrape::finish_page(
        url,
        &fetched.final_url,
        fetched.status,
        &fetched.body,
        request.format,
        false,
        Some(usize::MAX / 8),
        None,
    );
    let base = url::Url::parse(&fetched.final_url).ok();
    if depth < max_depth {
        if let Some(base) = &base {
            for link in scrape::links(&fetched.body, base) {
                if wanted(&link, host, &request.include, &request.exclude) {
                    enqueue(state, link.to_string(), depth + 1);
                }
            }
        }
    }
    if page.content.is_empty() {
        fail(state, url, "EMPTY");
        return Ok(());
    }
    if !state.hashes.insert(page.hash.clone()) {
        state.duplicates += 1;
        return Ok(());
    }
    let named = base.as_ref().map_or_else(
        || "0000-page.md".to_string(),
        |u| file_name(state.pages.len() + 1, u, request.format),
    );
    write_new(&folder.join(&named), page.content.as_bytes())?;
    state.pages.push(PageRow {
        url: fetched.final_url.clone(),
        title: page.title,
        file: named,
        tokens: estimate_tokens(&page.content),
        depth,
    });
    Ok(())
}

pub async fn crawl(request: CrawlRequest) -> Result<Crawled, DevError> {
    let start = super::http::checked_url(&request.url)?;
    let host = start.host_str().unwrap_or_default().to_string();
    let max_pages = request
        .max_pages
        .unwrap_or(DEFAULT_PAGES)
        .clamp(1, MAX_PAGES);
    let max_depth = request.max_depth.unwrap_or(DEFAULT_DEPTH).min(MAX_DEPTH);
    let wait = Duration::from_millis(request.wait_ms.unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS));
    let folder = match &request.output {
        Some(path) => PathBuf::from(path),
        None => output_file("crawl", "d"),
    };
    let mut state = open_state(&folder, &start)?;
    let proxy = request.proxy.as_deref();
    let robots = if request.obey_robots == Some(false) {
        Robots::default()
    } else {
        robots_for(&start, proxy).await
    };
    let asked = request.delay_ms.unwrap_or(DEFAULT_DELAY_MS);
    let polite = robots.delay.map_or(0, |d| d.as_millis() as u64);
    state.delay_ms = state.delay_ms.max(asked).max(polite).min(MAX_DELAY_MS);

    let plan = Plan {
        request: &request,
        folder: &folder,
        host: &host,
        max_depth,
    };
    let deadline = Instant::now() + wait;
    let before = state.pages.len();
    let mut stopped = None;
    let mut retry_after_s = None;
    let mut requests = 0u32;
    while state.pages.len() < max_pages {
        let Some((url, depth)) = state.queue.pop_front() else {
            break;
        };
        let pause = if requests == 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(state.delay_ms)
        };
        if Instant::now() + pause >= deadline {
            state.queue.push_front((url, depth));
            stopped = Some("time");
            break;
        }
        let allowed = url::Url::parse(&url).is_ok_and(|parsed| {
            let path = match parsed.query() {
                Some(query) => format!("{}?{query}", parsed.path()),
                None => parsed.path().to_string(),
            };
            robots.allows(&path)
        });
        if !allowed {
            state.skipped_by_robots += 1;
            continue;
        }
        tokio::time::sleep(pause).await;
        requests += 1;
        match scrape::get(&url, proxy).await {
            Ok(fetched) if !fetched.html => fail(&mut state, &url, "NOT_HTML"),
            Ok(fetched) => store_page(&mut state, &plan, &url, depth, &fetched)?,
            Err(DevError::Blocked {
                reason,
                retry_after,
                ..
            }) => {
                state.queue.push_front((url, depth));
                if reason == "rate_limited" {
                    state.delay_ms = (state.delay_ms.max(250) * 2).min(MAX_DELAY_MS);
                    retry_after_s = retry_after;
                    stopped = Some("rate_limited");
                } else {
                    stopped = Some("blocked");
                }
                break;
            }
            Err(error) => fail(&mut state, &url, error.code()),
        }
        save_state(&folder, &state)?;
    }
    save_state(&folder, &state)?;

    Ok(summary(
        &folder,
        &state,
        before,
        stopped,
        retry_after_s,
        request.limit.unwrap_or(DEFAULT_ROWS),
        max_pages,
    ))
}

fn summary(
    folder: &Path,
    state: &State,
    before: usize,
    stopped: Option<&'static str>,
    retry_after_s: Option<u64>,
    limit: usize,
    max_pages: usize,
) -> Crawled {
    let total = state.pages.len();
    let done = stopped.is_none() && (state.queue.is_empty() || total >= max_pages);
    let next_step = match stopped {
        Some("time") => "the time budget of this call ended; call op=run again with the same url and output to continue where it stopped",
        Some("rate_limited") => "the site asked for a slower pace, and the delay was doubled; wait retry_after_s (or a minute) and call op=run again with the same url and output",
        Some("paused") => "this folder holds an unfinished crawl; call op=run with its url and this output to continue",
        Some(_) => "the site blocked plain HTTP; add or check proxies with dev_proxy and call op=run again with the same url and output, or read the pages with the gd_* browser tools",
        None if total == 0 => "no page was saved; read failed and skippedByRobots, then widen include or raise maxDepth",
        None if !state.queue.is_empty() => "maxPages was reached with links still queued; call op=run again with the same url and output and a larger maxPages to continue",
        None => "read the files listed in pages from the output folder; crawl.json there holds the full manifest",
    };
    Crawled {
        output: folder.to_string_lossy().into_owned(),
        start: state.start.clone(),
        pages: state
            .pages
            .iter()
            .skip(total.saturating_sub(limit))
            .cloned()
            .collect(),
        total,
        truncated: total > limit,
        saved_now: total - before,
        failed: state.failed.iter().take(limit).cloned().collect(),
        failed_total: state.failed_total,
        duplicates: state.duplicates,
        skipped_by_robots: state.skipped_by_robots,
        dropped: state.dropped,
        queued: state.queue.len(),
        done,
        stopped,
        retry_after_s,
        next_step,
        untrusted: true,
    }
}

pub fn status(output: &str, limit: Option<usize>) -> Result<Crawled, DevError> {
    let folder = PathBuf::from(output);
    let path = folder.join(STATE_FILE);
    let unreadable = |message: String| DevError::Read {
        path: path.clone(),
        message,
    };
    let file = std::fs::File::open(&path).map_err(|e| unreadable(e.to_string()))?;
    let state: State = serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|e| unreadable(e.to_string()))?;
    let stopped = (!state.queue.is_empty()).then_some("paused");
    let total = state.pages.len();
    Ok(summary(
        &folder,
        &state,
        total,
        stopped,
        None,
        limit.unwrap_or(DEFAULT_ROWS),
        usize::MAX,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn robots_rules_follow_the_longest_match_and_the_named_group_wins() {
        let robots = Robots::parse(
            "User-agent: *\nDisallow: /private/\nAllow: /private/open\nDisallow: /*.php$\nCrawl-delay: 2\n\nUser-agent: otherbot\nDisallow: /\n",
        );
        assert!(robots.allows("/"));
        assert!(robots.allows("/docs/page"));
        assert!(!robots.allows("/private/secret"));
        assert!(robots.allows("/private/open/door"));
        assert!(!robots.allows("/a/b.php"));
        assert!(robots.allows("/a/b.php?x=1"));
        assert_eq!(robots.delay, Some(Duration::from_secs(2)));

        let named =
            Robots::parse("User-agent: *\nDisallow: /\n\nUser-agent: OmniGet\nDisallow: /admin\n");
        assert!(named.allows("/docs"));
        assert!(!named.allows("/admin/users"));
        assert!(!Robots::parse("User-agent: *\nDisallow: /\n").allows("/anything"));
        assert!(Robots::parse("<html>not a robots file</html>").allows("/anything"));
        assert!(Robots::parse("User-agent: *\nDisallow:\n").allows("/anything"));
    }

    #[test]
    fn only_pages_of_the_same_site_that_pass_the_filters_are_followed() {
        let url = |text: &str| url::Url::parse(text).unwrap();
        let none: [String; 0] = [];
        assert!(wanted(
            &url("https://site.example/docs/a"),
            "site.example",
            &none,
            &none
        ));
        assert!(!wanted(
            &url("https://other.example/docs/a"),
            "site.example",
            &none,
            &none
        ));
        assert!(!wanted(
            &url("https://site.example/logo.PNG"),
            "site.example",
            &none,
            &none
        ));
        assert!(!wanted(
            &url("https://site.example/file.zip?x=1"),
            "site.example",
            &none,
            &none
        ));
        let include = ["/docs/*".to_string()];
        let exclude = ["*/changelog*".to_string(), "?page=".to_string()];
        assert!(wanted(
            &url("https://site.example/docs/a"),
            "site.example",
            &include,
            &exclude
        ));
        assert!(!wanted(
            &url("https://site.example/blog/a"),
            "site.example",
            &include,
            &exclude
        ));
        assert!(!wanted(
            &url("https://site.example/docs/changelog/1"),
            "site.example",
            &include,
            &exclude
        ));
        assert!(!wanted(
            &url("https://site.example/docs/a?page=2"),
            "site.example",
            &include,
            &exclude
        ));
    }

    #[test]
    fn a_file_name_comes_from_the_address_and_never_leaves_the_folder() {
        let url = |text: &str| url::Url::parse(text).unwrap();
        assert_eq!(
            file_name(1, &url("https://s.example/"), Format::Markdown),
            "0001-index.md"
        );
        assert_eq!(
            file_name(
                12,
                &url("https://s.example/Docs/Getting%20Started?v=2"),
                Format::Text
            ),
            "0012-docs-getting-20started-v-2.txt"
        );
        let hostile = file_name(
            3,
            &url("https://s.example/..%2f..%2f..%2fetc/passwd"),
            Format::Html,
        );
        assert_eq!(hostile, "0003-2f-2f-2fetc-passwd.html");
        assert!(!hostile.contains('/') && !hostile.contains(".."));
        let long = file_name(
            4,
            &url(&format!("https://s.example/{}", "a".repeat(300))),
            Format::Markdown,
        );
        assert!(long.len() <= 70, "{long}");
    }

    async fn site(pages: Vec<(&'static str, String)>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = vec![0u8; 4096];
                let n = stream.read(&mut buffer).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..n]).into_owned();
                let path = head.split(' ').nth(1).unwrap_or("/").to_string();
                let answer = match pages.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) if path.ends_with(".txt") => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    Some((_, body)) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = stream.write_all(answer.as_bytes()).await;
            }
        });
        (base, task)
    }

    fn page(title: &str, body: &str) -> String {
        format!("<html><head><title>{title}</title></head><body><main>{body}</main></body></html>")
    }

    fn request(base: &str, folder: &Path) -> CrawlRequest {
        CrawlRequest {
            url: format!("{base}/"),
            output: Some(folder.to_string_lossy().into_owned()),
            delay_ms: Some(0),
            proxy: Some("direct".into()),
            ..CrawlRequest::default()
        }
    }

    #[tokio::test]
    async fn a_crawl_saves_each_page_once_obeys_robots_and_reports_the_rest() {
        let (base, server) = site(vec![
            ("/robots.txt", "User-agent: *\nDisallow: /secret\n".to_string()),
            ("/", page("Home", r#"<p>welcome</p><a href="/a">a</a><a href="/b">b</a><a href="/secret">s</a><a href="/gone">g</a><a href="https://elsewhere.example/">out</a>"#)),
            ("/a", page("A", r#"<p>page a</p><a href="/">home</a><a href="/copy">copy</a>"#)),
            ("/b", page("B", "<p>page b</p>")),
            ("/copy", page("A", r#"<p>page a</p><a href="/">home</a><a href="/copy">copy</a>"#)),
            ("/secret", page("S", "<p>hidden</p>")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("out");
        let out = crawl(request(&base, &folder)).await.unwrap();
        server.abort();
        assert!(out.done, "{out:?}");
        let titles: Vec<_> = out.pages.iter().map(|p| p.title.clone().unwrap()).collect();
        assert_eq!(titles, ["Home", "A", "B"]);
        assert_eq!(out.skipped_by_robots, 1);
        assert_eq!(out.duplicates, 1);
        assert_eq!(out.failed_total, 1);
        assert_eq!(out.failed[0].code, "HTTP_STATUS");
        assert!(out.untrusted);
        let first = std::fs::read_to_string(folder.join(&out.pages[0].file)).unwrap();
        assert!(first.contains("welcome"));
        assert!(folder.join("crawl.json").is_file());
    }

    #[tokio::test]
    async fn a_crawl_stops_at_the_page_limit_and_continues_from_the_same_folder() {
        let (base, server) = site(vec![
            (
                "/",
                page("Home", r#"<p>home</p><a href="/a">a</a><a href="/b">b</a>"#),
            ),
            ("/a", page("A", "<p>page a</p>")),
            ("/b", page("B", "<p>page b</p>")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("out");
        let mut first = request(&base, &folder);
        first.max_pages = Some(2);
        let out = crawl(first).await.unwrap();
        assert_eq!((out.total, out.queued, out.saved_now), (2, 1, 2));
        assert!(out.next_step.contains("larger maxPages"));
        let paused = status(&folder.to_string_lossy(), None).unwrap();
        assert_eq!((paused.done, paused.stopped), (false, Some("paused")));
        assert!(matches!(
            status(&dir.path().join("nothing").to_string_lossy(), None).unwrap_err(),
            DevError::Read { .. }
        ));
        let again = crawl(request(&base, &folder)).await.unwrap();
        server.abort();
        assert_eq!((again.total, again.queued, again.saved_now), (3, 0, 1));
        assert!(again.done);
        let read = status(&folder.to_string_lossy(), Some(2)).unwrap();
        assert_eq!((read.total, read.pages.len(), read.truncated), (3, 2, true));
        assert_eq!((read.saved_now, read.done, read.stopped), (0, true, None));
        assert_eq!(read.start, format!("{base}/"));

        let mut other = request(&base, &folder);
        other.url = format!("{base}/a");
        assert!(matches!(
            crawl(other).await.unwrap_err(),
            DevError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn the_time_budget_ends_a_call_without_losing_the_queue() {
        let (base, server) = site(vec![
            (
                "/",
                page("Home", r#"<p>home</p><a href="/a">a</a><a href="/b">b</a>"#),
            ),
            ("/a", page("A", "<p>page a</p>")),
            ("/b", page("B", "<p>page b</p>")),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let mut slow = request(&base, &dir.path().join("out"));
        slow.delay_ms = Some(5_000);
        slow.wait_ms = Some(1_000);
        let out = crawl(slow).await.unwrap();
        server.abort();
        assert_eq!(out.stopped, Some("time"));
        assert!(!out.done);
        assert_eq!((out.total, out.queued), (1, 2));
    }

    #[tokio::test]
    async fn a_folder_with_other_files_is_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "mine").unwrap();
        let error = crawl(request("http://127.0.0.1:9", dir.path()))
            .await
            .unwrap_err();
        assert!(matches!(error, DevError::OutputExists(_)), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes.txt")).unwrap(),
            "mine"
        );
    }
}
