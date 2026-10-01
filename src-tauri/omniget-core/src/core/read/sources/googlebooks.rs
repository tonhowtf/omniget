//! Google Books volumes API, used only as a metadata provider (no key
//! needed at the volumes the library looks up).

use serde::Deserialize;

use super::get_json;
use super::openlibrary::MetaCandidate;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Resp {
    items: Vec<Item>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Item {
    volume_info: Info,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Info {
    title: String,
    subtitle: Option<String>,
    authors: Vec<String>,
    publisher: String,
    published_date: String,
    description: String,
    industry_identifiers: Vec<Ident>,
    page_count: Option<u32>,
    categories: Vec<String>,
    image_links: Option<Links>,
    language: String,
    canonical_volume_link: String,
    series_info: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Ident {
    #[serde(rename = "type")]
    kind: String,
    identifier: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Links {
    thumbnail: Option<String>,
    small_thumbnail: Option<String>,
}

pub fn query(title: &str, author: &str, isbn: &str) -> String {
    let clean = |s: &str| s.replace(['"', ':'], " ").trim().to_string();
    if !isbn.trim().is_empty() {
        format!("isbn:{}", isbn.trim().replace('-', ""))
    } else if author.trim().is_empty() {
        format!("intitle:{}", clean(title))
    } else {
        format!("intitle:{} inauthor:{}", clean(title), clean(author))
    }
}

pub async fn lookup(
    client: &reqwest::Client,
    title: &str,
    author: &str,
    isbn: &str,
) -> Result<Vec<MetaCandidate>, String> {
    let mut u = url::Url::parse("https://www.googleapis.com/books/v1/volumes").expect("static url");
    u.query_pairs_mut()
        .append_pair("q", &query(title, author, isbn))
        .append_pair("maxResults", "8")
        .append_pair("printType", "books");
    let r: Resp = get_json(client, u.as_str()).await?;
    Ok(r.items
        .into_iter()
        .map(|i| {
            let v = i.volume_info;
            let isbn = v
                .industry_identifiers
                .iter()
                .find(|x| x.kind == "ISBN_13")
                .or_else(|| v.industry_identifiers.iter().find(|x| x.kind == "ISBN_10"))
                .map(|x| x.identifier.clone())
                .unwrap_or_default();
            let cover = v
                .image_links
                .as_ref()
                .and_then(|l| l.thumbnail.clone().or(l.small_thumbnail.clone()))
                // Bigger picture and https.
                .map(|t| {
                    t.replace("http://", "https://")
                        .replace("&edge=curl", "")
                        .replace("zoom=1", "zoom=0")
                });
            MetaCandidate {
                source: "googlebooks".into(),
                title: match v.subtitle {
                    Some(s) if !s.is_empty() => format!("{}: {s}", v.title),
                    _ => v.title,
                },
                authors: v.authors,
                description: v.description,
                publisher: v.publisher,
                published: v.published_date,
                language: v.language,
                subjects: v.categories,
                isbn,
                series: String::new(),
                pages: v.page_count,
                cover,
                url: v.canonical_volume_link,
            }
        })
        .collect())
}
