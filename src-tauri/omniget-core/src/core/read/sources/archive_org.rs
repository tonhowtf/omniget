use serde::Deserialize;

use super::{get_json, Download, Hit, Page};

const SEARCH_API: &str = "https://archive.org/advancedsearch.php";
const META_API: &str = "https://archive.org/metadata";
const PAGE_SIZE: u32 = 30;

#[derive(Debug, Deserialize)]
struct SearchResp {
    response: SearchBody,
}

#[derive(Debug, Deserialize)]
struct SearchBody {
    #[serde(default)]
    #[allow(dead_code)]
    #[serde(rename = "numFound")]
    num_found: u64,
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

#[derive(Debug, Deserialize)]
struct SearchDoc {
    identifier: String,
    #[serde(default)]
    title: OneOrMany,
    #[serde(default)]
    creator: OneOrMany,
    #[serde(default)]
    description: OneOrMany,
    #[serde(default)]
    language: OneOrMany,
    #[serde(default)]
    subject: OneOrMany,
    #[serde(default)]
    year: Option<serde_json::Value>,
    #[serde(default)]
    publisher: OneOrMany,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn first(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::One(s) => s.clone(),
            Self::Many(v) => v.first().cloned().unwrap_or_default(),
        }
    }

    fn all(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(s) if !s.trim().is_empty() => vec![s.clone()],
            Self::One(_) => Vec::new(),
            Self::Many(v) => v.iter().filter(|s| !s.trim().is_empty()).cloned().collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct MetaResp {
    #[serde(default)]
    metadata: MetaInfo,
    #[serde(default)]
    files: Vec<MetaFile>,
}

#[derive(Debug, Default, Deserialize)]
struct MetaInfo {
    #[serde(default)]
    title: OneOrMany,
    #[serde(default)]
    creator: OneOrMany,
    #[serde(default)]
    description: OneOrMany,
    #[serde(default)]
    language: OneOrMany,
    #[serde(default)]
    subject: OneOrMany,
    #[serde(default)]
    year: Option<serde_json::Value>,
    #[serde(default)]
    publisher: OneOrMany,
}

#[derive(Debug, Deserialize)]
struct MetaFile {
    name: String,
    #[serde(default)]
    format: String,
    #[serde(default)]
    private: Option<String>,
}

fn parse_year(v: &Option<serde_json::Value>) -> Option<i32> {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_i64().map(|y| y as i32),
        Some(serde_json::Value::String(s)) => s.get(..4).and_then(|p| p.parse::<i32>().ok()),
        _ => None,
    }
}

pub fn search_url(query: &str, mode: &str, lang: &str, page: u32) -> String {
    let mut parts = vec!["mediatype:texts".to_string()];
    let q = query.trim();
    if !q.is_empty() {
        parts.push(format!("({q})"));
    }
    if !lang.trim().is_empty() {
        parts.push(format!("language:({})", lang.trim()));
    }
    let expr = parts.join(" AND ");
    let sort = if mode == "latest" {
        "publicdate desc"
    } else {
        "downloads desc"
    };
    let mut u = url::Url::parse(SEARCH_API).expect("static url");
    {
        let mut qp = u.query_pairs_mut();
        qp.append_pair("q", &expr);
        for f in [
            "identifier",
            "title",
            "creator",
            "description",
            "language",
            "subject",
            "year",
            "publisher",
        ] {
            qp.append_pair("fl[]", f);
        }
        qp.append_pair("sort[]", sort);
        qp.append_pair("rows", &PAGE_SIZE.to_string());
        qp.append_pair("page", &page.max(1).to_string());
        qp.append_pair("output", "json");
    }
    u.to_string()
}

pub async fn browse(
    client: &reqwest::Client,
    query: &str,
    mode: &str,
    lang: &str,
    page: u32,
) -> Result<Page, String> {
    let page = page.max(1);
    let url = search_url(query, mode, lang, page);
    let r: SearchResp = get_json(client, &url).await?;
    let total = r.response.num_found;
    let hits: Vec<Hit> = r
        .response
        .docs
        .into_iter()
        .map(|d| {
            let id = d.identifier;
            let title = {
                let t = d.title.first();
                if t.is_empty() {
                    id.clone()
                } else {
                    t
                }
            };
            Hit {
                source: "archiveorg".into(),
                key: id.clone(),
                title,
                authors: d.creator.all(),
                cover: Some(format!("https://archive.org/services/img/{id}")),
                summary: d.description.first(),
                language: d.language.first(),
                year: parse_year(&d.year),
                subjects: d.subject.all().into_iter().take(10).collect(),
                kind: "book".into(),
                downloads: Vec::new(),
                url: format!("https://archive.org/details/{id}"),
                extra: serde_json::json!({
                    "identifier": id,
                    "publisher": d.publisher.first(),
                }),
            }
        })
        .collect();
    let fetched = (page as u64) * (PAGE_SIZE as u64);
    Ok(Page {
        total: Some(total),
        next: (fetched < total && !hits.is_empty()).then(|| (page + 1).to_string()),
        hits,
    })
}

fn file_format_and_label(name: &str, fmt: &str) -> Option<(&'static str, &'static str)> {
    let lower = name.to_ascii_lowercase();
    if lower.contains("_encrypted.") || lower.ends_with("_lcp.epub") || lower.ends_with(".acsm") {
        return None;
    }
    if lower.ends_with(".epub") {
        Some(("epub", "EPUB"))
    } else if lower.ends_with(".pdf") {
        let label = if lower.ends_with("_text.pdf") {
            "PDF (Text)"
        } else {
            "PDF"
        };
        Some(("pdf", label))
    } else if lower.ends_with(".mobi") {
        Some(("mobi", "MOBI"))
    } else if lower.ends_with(".cbz") {
        Some(("cbz", "CBZ"))
    } else if lower.ends_with(".cbr") {
        Some(("cbr", "CBR"))
    } else if lower.ends_with(".djvu") {
        Some(("djvu", "DjVu"))
    } else if lower.ends_with("_djvu.txt") || (lower.ends_with(".txt") && fmt.contains("Text")) {
        Some(("txt", "Text"))
    } else {
        None
    }
}

fn collect_downloads(identifier: &str, files: &[MetaFile]) -> Vec<Download> {
    let mut out: Vec<Download> = Vec::new();
    let priority = |fmt: &str| match fmt {
        "epub" => 0,
        "pdf" => 1,
        "cbz" => 2,
        "cbr" => 3,
        "mobi" => 4,
        "djvu" => 5,
        "txt" => 6,
        _ => 7,
    };
    let mut seen_fmt = std::collections::HashSet::new();
    let mut candidates: Vec<(&MetaFile, &'static str, &'static str)> = files
        .iter()
        .filter(|f| f.private.as_deref() != Some("true"))
        .filter_map(|f| {
            let (fmt, label) = file_format_and_label(&f.name, &f.format)?;
            Some((f, fmt, label))
        })
        .collect();
    candidates.sort_by_key(|(f, fmt, _)| {
        let is_text_pdf = f.name.to_ascii_lowercase().ends_with("_text.pdf");
        (priority(fmt), usize::from(is_text_pdf), f.name.len())
    });
    for (f, fmt, label) in candidates {
        if !seen_fmt.insert(fmt) {
            continue;
        }
        let encoded = url::form_urlencoded::byte_serialize(f.name.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
            .replace("%2F", "/");
        out.push(Download {
            url: format!("https://archive.org/download/{identifier}/{encoded}"),
            format: fmt.into(),
            label: label.into(),
        });
    }
    out
}

pub async fn fetch_item_downloads(
    client: &reqwest::Client,
    identifier: &str,
) -> Result<Vec<Download>, String> {
    let url = format!("{META_API}/{identifier}");
    let meta: MetaResp = get_json(client, &url).await?;
    Ok(collect_downloads(identifier, &meta.files))
}

pub async fn details(client: &reqwest::Client, identifier: &str) -> Result<Hit, String> {
    let url = format!("{META_API}/{identifier}");
    let meta: MetaResp = get_json(client, &url).await?;
    let downloads = collect_downloads(identifier, &meta.files);
    let m = meta.metadata;
    let title = {
        let t = m.title.first();
        if t.is_empty() {
            identifier.to_string()
        } else {
            t
        }
    };
    Ok(Hit {
        source: "archiveorg".into(),
        key: identifier.into(),
        title,
        authors: m.creator.all(),
        cover: Some(format!("https://archive.org/services/img/{identifier}")),
        summary: m.description.first(),
        language: m.language.first(),
        year: parse_year(&m.year),
        subjects: m.subject.all().into_iter().take(12).collect(),
        kind: "book".into(),
        downloads,
        url: format!("https://archive.org/details/{identifier}"),
        extra: serde_json::json!({
            "identifier": identifier,
            "publisher": m.publisher.first(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_encrypted_and_private_files_and_sorts_formats() {
        let files = vec![
            MetaFile {
                name: "book_encrypted.pdf".into(),
                format: "PDF".into(),
                private: None,
            },
            MetaFile {
                name: "secret.epub".into(),
                format: "EPUB".into(),
                private: Some("true".into()),
            },
            MetaFile {
                name: "book.pdf".into(),
                format: "Text PDF".into(),
                private: None,
            },
            MetaFile {
                name: "book.epub".into(),
                format: "EPUB".into(),
                private: None,
            },
            MetaFile {
                name: "book_djvu.txt".into(),
                format: "DjVuTXT".into(),
                private: None,
            },
        ];
        let dl = collect_downloads("my-book-123", &files);
        assert_eq!(dl.len(), 3);
        assert_eq!(dl[0].format, "epub");
        assert_eq!(
            dl[0].url,
            "https://archive.org/download/my-book-123/book.epub"
        );
        assert_eq!(dl[1].format, "pdf");
        assert_eq!(dl[2].format, "txt");
    }

    #[test]
    fn builds_search_url_with_mediatype_and_sort() {
        let u = search_url("rust programming", "popular", "eng", 2);
        assert!(u.contains("mediatype%3Atexts"));
        assert!(u.contains("page=2"));
        assert!(u.contains("downloads+desc"));
    }
}
