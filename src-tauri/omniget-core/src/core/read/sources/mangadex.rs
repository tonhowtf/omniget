use serde::Deserialize;

use super::{get_json, Hit, Page, RemoteChapter, RemotePage};

pub const API: &str = "https://api.mangadex.org";

#[derive(Debug, Deserialize)]
pub struct Collection<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub limit: u64,
}

#[derive(Debug, Deserialize)]
pub struct Entity<T> {
    pub id: String,
    pub attributes: T,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
}

#[derive(Debug, Deserialize)]
pub struct Relationship {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub attributes: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MangaAttrs {
    pub title: std::collections::HashMap<String, String>,
    pub alt_titles: Vec<std::collections::HashMap<String, String>>,
    pub description: std::collections::HashMap<String, String>,
    pub original_language: String,
    pub status: Option<String>,
    pub year: Option<i32>,
    pub content_rating: Option<String>,
    pub publication_demographic: Option<String>,
    pub last_chapter: Option<String>,
    pub tags: Vec<Entity<TagAttrs>>,
    pub available_translated_languages: Vec<Option<String>>,
    pub links: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct TagAttrs {
    pub name: std::collections::HashMap<String, String>,
    pub group: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ChapterAttrs {
    pub volume: Option<String>,
    pub chapter: Option<String>,
    pub title: Option<String>,
    pub translated_language: String,
    pub external_url: Option<String>,
    pub publish_at: Option<String>,
    pub pages: u32,
    pub is_unavailable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtHome {
    pub base_url: String,
    pub chapter: AtHomeChapter,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtHomeChapter {
    pub hash: String,
    pub data: Vec<String>,
    #[serde(default)]
    pub data_saver: Vec<String>,
}

pub fn pick(map: &std::collections::HashMap<String, String>, lang: &str) -> String {
    [lang, "en", "ja-ro", "ko-ro", "zh-ro"]
        .iter()
        .find_map(|l| map.get(*l).filter(|s| !s.is_empty()))
        .or_else(|| map.values().find(|s| !s.is_empty()))
        .cloned()
        .unwrap_or_default()
}

pub fn search_url(
    query: &str,
    langs: &[String],
    ratings: &[String],
    order: &str,
    tags: &[String],
    status: &[String],
    offset: u64,
) -> String {
    let mut u = url::Url::parse(&format!("{API}/manga")).expect("static url");
    {
        let mut q = u.query_pairs_mut();
        if !query.trim().is_empty() {
            q.append_pair("title", query.trim());
        }
        q.append_pair("limit", "30");
        q.append_pair("offset", &offset.to_string());
        q.append_pair("includes[]", "cover_art");
        q.append_pair("includes[]", "author");
        q.append_pair("includes[]", "artist");
        for l in langs {
            q.append_pair("availableTranslatedLanguage[]", l);
        }
        let ratings: Vec<String> = if ratings.is_empty() {
            vec!["safe".into(), "suggestive".into()]
        } else {
            ratings.to_vec()
        };
        for r in &ratings {
            q.append_pair("contentRating[]", r);
        }
        for t in tags {
            q.append_pair("includedTags[]", t);
        }
        for s in status {
            q.append_pair("status[]", s);
        }
        let key = match order {
            "latest" => "order[latestUploadedChapter]",
            "follows" => "order[followedCount]",
            "rating" => "order[rating]",
            "year" => "order[year]",
            "created" => "order[createdAt]",
            "title" => "order[title]",
            _ if query.trim().is_empty() => "order[followedCount]",
            _ => "order[relevance]",
        };
        q.append_pair(key, if key == "order[title]" { "asc" } else { "desc" });
    }
    u.to_string()
}

pub fn cover_url(manga_id: &str, file: &str, thumb: bool) -> String {
    let suffix = if thumb { ".512.jpg" } else { "" };
    format!("https://uploads.mangadex.org/covers/{manga_id}/{file}{suffix}")
}

pub fn to_hit(m: Entity<MangaAttrs>, lang: &str) -> Hit {
    let a = &m.attributes;
    let mut authors: Vec<String> = Vec::new();
    let mut cover = None;
    for r in &m.relationships {
        match r.kind.as_str() {
            "author" | "artist" => {
                if let Some(n) = r.attributes.as_ref().and_then(|x| x["name"].as_str()) {
                    if !authors.iter().any(|a| a == n) {
                        authors.push(n.to_string());
                    }
                }
            }
            "cover_art" => {
                cover = r
                    .attributes
                    .as_ref()
                    .and_then(|x| x["fileName"].as_str())
                    .map(|f| cover_url(&m.id, f, true));
            }
            _ => {}
        }
    }
    let subjects = a
        .tags
        .iter()
        .map(|t| pick(&t.attributes.name, "en"))
        .collect();
    let mut title = pick(&a.title, lang);
    if let Some(alt) = a.alt_titles.iter().find_map(|t| t.get(lang)) {
        if lang != "en" {
            title = alt.clone();
        }
    }
    Hit {
        source: "mangadex".into(),
        key: m.id.clone(),
        title,
        authors,
        cover,
        summary: pick(&a.description, lang),
        language: a.original_language.clone(),
        year: a.year,
        subjects,
        kind: "series".into(),
        downloads: vec![],
        url: format!("https://mangadex.org/title/{}", m.id),
        extra: serde_json::json!({
            "status": a.status,
            "rating": a.content_rating,
            "demographic": a.publication_demographic,
            "lastChapter": a.last_chapter,
            "languages": a.available_translated_languages.iter().flatten().collect::<Vec<_>>(),
            "links": a.links,
        }),
    }
}

pub async fn search(client: &reqwest::Client, url: &str, lang: &str) -> Result<Page, String> {
    let r: Collection<Entity<MangaAttrs>> = get_json(client, url).await?;
    let next_off = r.offset + r.limit;
    let next = (next_off < r.total && next_off < 10_000).then(|| {
        static OFFSET_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = OFFSET_RE.get_or_init(|| regex::Regex::new(r"offset=\d+").expect("static regex"));
        re.replace(url, format!("offset={next_off}").as_str())
            .to_string()
    });
    Ok(Page {
        total: Some(r.total),
        next,
        hits: r.data.into_iter().map(|m| to_hit(m, lang)).collect(),
    })
}

pub async fn details(client: &reqwest::Client, id: &str, lang: &str) -> Result<Hit, String> {
    #[derive(Deserialize)]
    struct One {
        data: Entity<MangaAttrs>,
    }
    let u = format!("{API}/manga/{id}?includes[]=cover_art&includes[]=author&includes[]=artist");
    let r: One = get_json(client, &u).await?;
    let mut h = to_hit(r.data, lang);
    if let Some(c) = &h.cover {
        h.cover = Some(c.trim_end_matches(".512.jpg").to_string());
    }
    Ok(h)
}

pub async fn chapters(
    client: &reqwest::Client,
    id: &str,
    langs: &[String],
) -> Result<Vec<RemoteChapter>, String> {
    let mut out = Vec::new();
    let mut offset = 0u64;
    loop {
        let mut u = url::Url::parse(&format!("{API}/manga/{id}/feed")).expect("static url");
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("limit", "500");
            q.append_pair("offset", &offset.to_string());
            q.append_pair("order[volume]", "asc");
            q.append_pair("order[chapter]", "asc");
            q.append_pair("includes[]", "scanlation_group");
            for r in ["safe", "suggestive", "erotica", "pornographic"] {
                q.append_pair("contentRating[]", r);
            }
            for l in langs {
                q.append_pair("translatedLanguage[]", l);
            }
        }
        let r: Collection<Entity<ChapterAttrs>> = get_json(client, u.as_str()).await?;
        let n = r.data.len() as u64;
        for c in r.data {
            let a = c.attributes;
            if a.is_unavailable {
                continue;
            }
            let group = c
                .relationships
                .iter()
                .filter(|r| r.kind == "scanlation_group")
                .filter_map(|r| r.attributes.as_ref()?["name"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(RemoteChapter {
                key: c.id,
                title: a.title.unwrap_or_default(),
                number: a.chapter.as_deref().and_then(|v| v.parse().ok()),
                volume: a.volume.as_deref().and_then(|v| v.parse().ok()),
                language: a.translated_language,
                scanlator: group,
                published: a
                    .publish_at
                    .as_deref()
                    .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
                    .map(|d| d.timestamp_millis()),
                pages: Some(a.pages),
                external_url: a.external_url,
            });
        }
        offset += n;
        if n == 0 || offset >= r.total {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Ok(out)
}

pub async fn pages(
    client: &reqwest::Client,
    chapter_id: &str,
    data_saver: bool,
) -> Result<Vec<RemotePage>, String> {
    let r: AtHome = get_json(client, &format!("{API}/at-home/server/{chapter_id}")).await?;
    let (dir, files) = if data_saver && !r.chapter.data_saver.is_empty() {
        ("data-saver", r.chapter.data_saver)
    } else {
        ("data", r.chapter.data)
    };
    Ok(files
        .into_iter()
        .map(|f| RemotePage {
            url: format!("{}/{dir}/{}/{f}", r.base_url, r.chapter.hash),
            ..Default::default()
        })
        .collect())
}
