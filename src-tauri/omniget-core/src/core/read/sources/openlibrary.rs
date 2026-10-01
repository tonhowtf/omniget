//! Open Library: the book search and the metadata lookup (ISBN, title +
//! author) the library uses to fill in covers, descriptions and series.
//! Only `ebook_access == "public"` items get download links (public-domain
//! scans hosted by the Internet Archive); borrowable books are not offered.

use serde::{Deserialize, Serialize};

use super::{get_json, Download, Hit, Page};

pub const FIELDS: &str = "key,title,subtitle,author_name,first_publish_year,cover_i,ia,ebook_access,language,subject,isbn,publisher,number_of_pages_median,edition_count";

#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    #[serde(rename = "numFound", default)]
    pub num_found: u64,
    #[serde(default)]
    pub docs: Vec<Doc>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct Doc {
    pub key: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub author_name: Vec<String>,
    pub first_publish_year: Option<i32>,
    pub cover_i: Option<i64>,
    pub ia: Vec<String>,
    pub ebook_access: Option<String>,
    pub language: Vec<String>,
    pub subject: Vec<String>,
    pub isbn: Vec<String>,
    pub publisher: Vec<String>,
    pub number_of_pages_median: Option<u32>,
}

pub fn cover_url(id: i64, size: char) -> String {
    format!("https://covers.openlibrary.org/b/id/{id}-{size}.jpg")
}

pub fn isbn_cover_url(isbn: &str) -> String {
    format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false")
}

/// `mode`: `everything` or `ebooks` (only public-domain readable books).
pub fn search_url(query: &str, language: &str, mode: &str, page: u32) -> String {
    let mut u = url::Url::parse("https://openlibrary.org/search.json").expect("static url");
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("q", query.trim());
        q.append_pair("fields", FIELDS);
        q.append_pair("limit", "30");
        q.append_pair("page", &page.max(1).to_string());
        if !language.is_empty() {
            q.append_pair("language", language);
        }
        if mode == "ebooks" {
            q.append_pair("ebook_access", "public");
        }
    }
    u.to_string()
}

/// Open Library lists every edition's contributor spelling; keep a few
/// distinct names.
fn tidy_authors(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in list {
        let key: String = a
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        if !out.iter().any(|x| {
            x.to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                == key
        }) {
            out.push(a.clone());
        }
        if out.len() == 3 {
            break;
        }
    }
    out
}

pub fn to_hit(d: Doc) -> Hit {
    let public = d.ebook_access.as_deref() == Some("public");
    let downloads = if public {
        d.ia.iter()
            .take(3)
            .flat_map(|id| {
                [
                    Download {
                        url: format!("https://archive.org/download/{id}/{id}.epub"),
                        format: "epub".into(),
                        label: format!("EPUB ({id})"),
                    },
                    Download {
                        url: format!("https://archive.org/download/{id}/{id}.pdf"),
                        format: "pdf".into(),
                        label: format!("PDF ({id})"),
                    },
                ]
            })
            .collect()
    } else {
        Vec::new()
    };
    let title = match &d.subtitle {
        Some(s) if !s.is_empty() => format!("{}: {s}", d.title),
        _ => d.title.clone(),
    };
    Hit {
        source: "openlibrary".into(),
        key: d.key.clone(),
        title,
        authors: tidy_authors(&d.author_name),
        cover: d.cover_i.map(|c| cover_url(c, 'M')),
        summary: String::new(),
        language: d.language.first().cloned().unwrap_or_default(),
        year: d.first_publish_year,
        subjects: d.subject.iter().take(12).cloned().collect(),
        kind: "book".into(),
        downloads,
        url: format!("https://openlibrary.org{}", d.key),
        extra: serde_json::json!({
            "isbn": d.isbn.first(),
            "publisher": d.publisher.first(),
            "pages": d.number_of_pages_median,
            "access": d.ebook_access,
        }),
    }
}

pub async fn search(client: &reqwest::Client, url: &str, page: u32) -> Result<Page, String> {
    let r: SearchResponse = get_json(client, url).await?;
    let shown = page.max(1) as u64 * 30;
    let next = (shown < r.num_found)
        .then(|| url.replace(&format!("page={page}"), &format!("page={}", page + 1)));
    Ok(Page {
        total: Some(r.num_found),
        next,
        hits: r.docs.into_iter().map(to_hit).collect(),
    })
}

/// Metadata candidate for "fetch metadata" in the library.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetaCandidate {
    pub source: String,
    pub title: String,
    pub authors: Vec<String>,
    pub description: String,
    pub publisher: String,
    pub published: String,
    pub language: String,
    pub subjects: Vec<String>,
    pub isbn: String,
    pub series: String,
    pub pages: Option<u32>,
    pub cover: Option<String>,
    pub url: String,
}

/// Title/author or ISBN lookup, best matches first.
pub async fn lookup(
    client: &reqwest::Client,
    title: &str,
    author: &str,
    isbn: &str,
) -> Result<Vec<MetaCandidate>, String> {
    let mut u = url::Url::parse("https://openlibrary.org/search.json").expect("static url");
    {
        let mut q = u.query_pairs_mut();
        if !isbn.is_empty() {
            q.append_pair("isbn", isbn);
        } else {
            q.append_pair("title", title);
            if !author.is_empty() {
                q.append_pair("author", author);
            }
        }
        q.append_pair("fields", FIELDS);
        q.append_pair("limit", "8");
    }
    let r: SearchResponse = get_json(client, u.as_str()).await?;
    let mut out = Vec::new();
    for d in r.docs {
        let description = work_description(client, &d.key).await.unwrap_or_default();
        out.push(MetaCandidate {
            source: "openlibrary".into(),
            title: d.title.clone(),
            authors: tidy_authors(&d.author_name),
            description,
            publisher: d.publisher.first().cloned().unwrap_or_default(),
            published: d
                .first_publish_year
                .map(|y| y.to_string())
                .unwrap_or_default(),
            language: d.language.first().cloned().unwrap_or_default(),
            subjects: d.subject.iter().take(10).cloned().collect(),
            isbn: d
                .isbn
                .iter()
                .find(|i| i.len() == 13)
                .or(d.isbn.first())
                .cloned()
                .unwrap_or_default(),
            series: String::new(),
            pages: d.number_of_pages_median,
            cover: d.cover_i.map(|c| cover_url(c, 'L')),
            url: format!("https://openlibrary.org{}", d.key),
        });
    }
    Ok(out)
}

async fn work_description(client: &reqwest::Client, key: &str) -> Option<String> {
    if !key.starts_with("/works/") {
        return None;
    }
    let v: serde_json::Value = get_json(client, &format!("https://openlibrary.org{key}.json"))
        .await
        .ok()?;
    match &v["description"] {
        serde_json::Value::String(s) => Some(s.clone()),
        o => o["value"].as_str().map(str::to_string),
    }
}
