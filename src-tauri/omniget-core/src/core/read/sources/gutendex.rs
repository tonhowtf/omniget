use serde::Deserialize;

use super::{get_json, Download, Hit, Page};

pub const BASE: &str = "https://gutendex.com/books/";

#[derive(Debug, Deserialize)]
pub struct Response {
    pub count: u64,
    pub next: Option<String>,
    pub results: Vec<Book>,
}

#[derive(Debug, Deserialize)]
pub struct Person {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Book {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<Person>,
    #[serde(default)]
    pub summaries: Vec<String>,
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub bookshelves: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub copyright: Option<bool>,
    #[serde(default)]
    pub formats: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub download_count: u64,
}

pub fn search_url(query: &str, languages: &str, topic: &str, sort: &str) -> String {
    let mut u = url::Url::parse(BASE).expect("static url");
    {
        let mut q = u.query_pairs_mut();
        if !query.trim().is_empty() {
            q.append_pair("search", query.trim());
        }
        if !languages.is_empty() {
            q.append_pair("languages", languages);
        }
        if !topic.is_empty() {
            q.append_pair("topic", topic);
        }
        if !sort.is_empty() && sort != "popular" {
            q.append_pair("sort", sort);
        }
        q.append_pair("copyright", "false");
    }
    u.to_string()
}

pub fn display_name(n: &str) -> String {
    match n.split_once(", ") {
        Some((last, first)) if !first.contains(',') => format!("{first} {last}"),
        _ => n.to_string(),
    }
}

pub fn to_hit(b: Book) -> Hit {
    let mut downloads = Vec::new();
    let order = [
        ("application/epub+zip", "epub", "EPUB"),
        ("application/x-mobipocket-ebook", "mobi", "Kindle"),
        ("text/plain; charset=utf-8", "txt", "Text"),
        ("text/plain; charset=us-ascii", "txt", "Text"),
        ("text/plain", "txt", "Text"),
        ("text/html", "html", "HTML"),
    ];
    for (mime, fmt, label) in order {
        if let Some(u) = b.formats.get(mime) {
            if !downloads.iter().any(|d: &Download| d.format == fmt) {
                downloads.push(Download {
                    url: u.clone(),
                    format: fmt.into(),
                    label: label.into(),
                });
            }
        }
    }
    let mut subjects = b.subjects;
    subjects.extend(
        b.bookshelves
            .into_iter()
            .map(|s| s.trim_start_matches("Category: ").to_string()),
    );
    Hit {
        source: "gutenberg".into(),
        key: b.id.to_string(),
        title: b.title,
        authors: b.authors.iter().map(|a| display_name(&a.name)).collect(),
        cover: b.formats.get("image/jpeg").cloned(),
        summary: b.summaries.into_iter().next().unwrap_or_default(),
        language: b.languages.into_iter().next().unwrap_or_default(),
        year: None,
        subjects,
        kind: "book".into(),
        downloads,
        url: format!("https://www.gutenberg.org/ebooks/{}", b.id),
        extra: serde_json::json!({ "downloads": b.download_count }),
    }
}

pub async fn search(client: &reqwest::Client, url: &str) -> Result<Page, String> {
    let r: Response = get_json(client, url).await?;
    Ok(Page {
        total: Some(r.count),
        next: r.next,
        hits: r.results.into_iter().map(to_hit).collect(),
    })
}

pub async fn search_or_catalog(
    client: &reqwest::Client,
    url: &str,
    query: &str,
    languages: &str,
    cache: &std::path::Path,
) -> Result<Page, String> {
    let quick = tokio::time::timeout(std::time::Duration::from_secs(12), search(client, url)).await;
    match quick.unwrap_or_else(|_| Err("Gutendex timed out".into())) {
        Ok(p) => Ok(p),
        Err(e) => {
            let csv = catalog_text(client, cache)
                .await
                .map_err(|c| format!("{e}; catalog: {c}"))?;
            let hits = search_catalog(&csv, query, languages, 60);
            Ok(Page {
                total: Some(hits.len() as u64),
                next: None,
                hits,
            })
        }
    }
}

pub async fn catalog_text(
    client: &reqwest::Client,
    cache: &std::path::Path,
) -> Result<String, String> {
    use std::io::Read;
    let fresh = std::fs::metadata(cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 86_400);
    if !fresh {
        let bytes = client
            .get(CATALOG_URL)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(d) = cache.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = cache.with_extension(format!("gz.tmp.{}", std::process::id()));
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(&tmp, cache) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
    }
    let cache_path = cache.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let gz = std::fs::read(&cache_path).map_err(|e| e.to_string())?;
        let mut out = String::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_string(&mut out)
            .map_err(|e| e.to_string())?;
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub const CATALOG_URL: &str = "https://www.gutenberg.org/cache/epub/feeds/pg_catalog.csv.gz";

fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            ('\r', false) => {}
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn catalog_author(a: &str) -> String {
    let parts: Vec<&str> = a.split(", ").collect();
    let parts: Vec<&str> = parts
        .into_iter()
        .filter(|p| !p.chars().next().is_some_and(|c| c.is_ascii_digit()) && !p.starts_with('-'))
        .collect();
    let name = parts.join(", ");
    let name = name.split(" [").next().unwrap_or(&name).to_string();
    display_name(&name)
}

pub fn search_catalog(csv: &str, query: &str, languages: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let langs: Vec<&str> = languages
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut out = Vec::new();
    for r in csv_records(csv).into_iter().skip(1) {
        if r.len() < 9 || r[1] != "Text" {
            continue;
        }
        let (id, title, lang, authors) = (&r[0], &r[3], &r[4], &r[5]);
        if !langs.is_empty() && !langs.iter().any(|l| lang.split("; ").any(|x| x == *l)) {
            continue;
        }
        let hay = format!("{} {}", title.to_lowercase(), authors.to_lowercase());
        if !words.iter().all(|w| hay.contains(w.as_str())) {
            continue;
        }
        let dl = |suffix: &str, format: &str, label: &str| Download {
            url: format!("https://www.gutenberg.org/ebooks/{id}{suffix}"),
            format: format.into(),
            label: label.into(),
        };
        out.push(Hit {
            source: "gutenberg".into(),
            key: id.clone(),
            title: title.split_whitespace().collect::<Vec<_>>().join(" "),
            authors: authors
                .split("; ")
                .filter(|a| !a.is_empty())
                .map(catalog_author)
                .collect(),
            cover: Some(format!(
                "https://www.gutenberg.org/cache/epub/{id}/pg{id}.cover.medium.jpg"
            )),
            summary: String::new(),
            language: lang.split("; ").next().unwrap_or("").to_string(),
            year: None,
            subjects: r[6]
                .split("; ")
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            kind: "book".into(),
            downloads: vec![
                dl(".epub3.images", "epub", "EPUB"),
                dl(".kf8.images", "azw3", "Kindle"),
                dl(".txt.utf-8", "txt", "Text"),
            ],
            url: format!("https://www.gutenberg.org/ebooks/{id}"),
            extra: serde_json::Value::Null,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}
