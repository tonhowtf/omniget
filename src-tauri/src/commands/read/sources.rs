//! Where books and series come from. One dispatch for every source kind:
//!
//! * built-in legal catalogs (Project Gutenberg, Standard Ebooks, Open
//!   Library, MangaDex, MANGA Plus);
//! * OPDS catalogs the user adds (Calibre-Web, Komga, Kavita, a friend's
//!   OmniGet...), with Basic auth;
//! * declarative sources from repositories the user adds;
//! * the Suwayomi bridge, which exposes every Mihon extension the user
//!   installed there.
//!
//! OmniGet ships no source for unlicensed content; the last two only run
//! what the user brings.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use omniget_core::core::read::opds::{self, OpdsFeed};
use omniget_core::core::read::sources::declarative::{self, DeclarativeSource};
use omniget_core::core::read::sources::suwayomi;
use omniget_core::core::read::sources::{
    self as src, gutendex, mangadex, mangaplus, openlibrary, standardebooks, Hit, Page,
    RemoteChapter, RemotePage,
};

use super::{blocking, db};

pub fn http() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(src::client)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    /// `gutenberg`, `standardebooks`, `openlibrary`, `mangadex`,
    /// `mangaplus`, `opds:<id>`, `decl:<id>` or `suwa:<source id>`.
    pub id: String,
    pub name: String,
    /// `book` or `manga`.
    pub kind: String,
    pub lang: String,
    pub icon: String,
    pub builtin: bool,
    pub search: bool,
    pub latest: bool,
    pub nsfw: bool,
    pub url: String,
    pub filters: serde_json::Value,
}

fn builtin() -> Vec<SourceInfo> {
    let mk = |id: &str, name: &str, kind: &str, url: &str, latest: bool| SourceInfo {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        lang: "all".into(),
        icon: String::new(),
        builtin: true,
        search: true,
        latest,
        nsfw: false,
        url: url.into(),
        filters: serde_json::json!([]),
    };
    vec![
        mk(
            "gutenberg",
            "Project Gutenberg",
            "book",
            "https://www.gutenberg.org",
            false,
        ),
        mk(
            "standardebooks",
            "Standard Ebooks",
            "book",
            "https://standardebooks.org",
            true,
        ),
        mk(
            "openlibrary",
            "Open Library",
            "book",
            "https://openlibrary.org",
            false,
        ),
        mk(
            "mangadex",
            "MangaDex",
            "manga",
            "https://mangadex.org",
            true,
        ),
        mk(
            "mangaplus",
            "MANGA Plus",
            "manga",
            "https://mangaplus.shueisha.co.jp",
            true,
        ),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Catalog {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub url: String,
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub config: serde_json::Value,
    pub enabled: bool,
}

pub fn catalogs() -> Result<Vec<Catalog>, String> {
    db::with(|c| {
        let mut st = c.prepare(
            "SELECT id, kind, title, url, username, password, config, enabled FROM catalogs ORDER BY title",
        )?;
        let rows = st
            .query_map([], |r| {
                let cfg: String = r.get(6)?;
                let id: i64 = r.get(0)?;
                let stored: String = r.get(5)?;
                Ok(Catalog {
                    id,
                    kind: r.get(1)?,
                    title: r.get(2)?,
                    url: r.get(3)?,
                    username: r.get(4)?,
                    password: if stored.is_empty() {
                        super::secret(&format!("catalog:{id}"))
                    } else {
                        stored
                    },
                    config: serde_json::from_str(&cfg).unwrap_or(serde_json::json!({})),
                    enabled: r.get::<_, i64>(7)? != 0,
                })
            })?
            .collect();
        rows
    })
}

fn catalog(id: i64) -> Result<Catalog, String> {
    catalogs()?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "catalog not found".into())
}

/// Declarative sources installed from repositories, parsed.
fn declarative_sources() -> Vec<(i64, DeclarativeSource)> {
    catalogs()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.kind == "declarative" && c.enabled)
        .filter_map(|c| {
            serde_json::from_value::<DeclarativeSource>(c.config["source"].clone())
                .ok()
                .map(|s| (c.id, s))
        })
        .collect()
}

fn suwayomi_conn() -> Option<suwayomi::Conn> {
    catalogs()
        .ok()?
        .into_iter()
        .find(|c| c.kind == "suwayomi" && c.enabled)
        .map(|c| suwayomi::Conn::new(&c.url, &c.username, &c.password))
}

#[tauri::command]
pub async fn read_sources() -> Result<Vec<SourceInfo>, String> {
    let mut out = builtin();
    for c in blocking(catalogs).await? {
        if !c.enabled {
            continue;
        }
        match c.kind.as_str() {
            "opds" => out.push(SourceInfo {
                id: format!("opds:{}", c.id),
                name: c.title.clone(),
                kind: "book".into(),
                lang: "all".into(),
                icon: String::new(),
                builtin: false,
                search: true,
                latest: true,
                nsfw: false,
                url: c.url.clone(),
                filters: serde_json::json!([]),
            }),
            "declarative" => {
                if let Ok(s) =
                    serde_json::from_value::<DeclarativeSource>(c.config["source"].clone())
                {
                    out.push(SourceInfo {
                        id: format!("decl:{}", c.id),
                        name: s.name.clone(),
                        kind: s.kind.clone().unwrap_or_else(|| "manga".into()),
                        lang: s.lang.clone(),
                        icon: s.icon.clone().unwrap_or_default(),
                        builtin: false,
                        search: s.search.is_some(),
                        latest: s.latest.is_some(),
                        nsfw: s.nsfw,
                        url: s.base_url.clone(),
                        filters: serde_json::to_value(&s.filters).unwrap_or_default(),
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(conn) = suwayomi_conn() {
        if let Ok(list) = conn.sources().await {
            for s in list {
                out.push(SourceInfo {
                    id: format!("suwa:{}", s.id),
                    name: s.display_name,
                    kind: "manga".into(),
                    lang: s.lang,
                    icon: s.icon_url,
                    builtin: false,
                    search: true,
                    latest: s.supports_latest,
                    nsfw: s.is_nsfw,
                    url: String::new(),
                    filters: serde_json::json!([]),
                });
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BrowseReq {
    pub source: String,
    pub query: String,
    /// `popular`, `latest` or `search` (the default when a query is set).
    pub mode: String,
    /// Opaque "next" cursor from a previous page (a URL for most sources).
    pub cursor: Option<String>,
    pub page: u32,
    pub lang: String,
    pub langs: Vec<String>,
    pub filters: serde_json::Value,
    pub nsfw: bool,
}

#[tauri::command]
pub async fn read_browse(req: BrowseReq) -> Result<Page, String> {
    let c = http();
    let page = req.page.max(1);
    let mode = if req.mode.is_empty() {
        if req.query.trim().is_empty() {
            "popular"
        } else {
            "search"
        }
    } else {
        req.mode.as_str()
    };
    match req.source.as_str() {
        "gutenberg" => {
            let url = req.cursor.clone().unwrap_or_else(|| {
                let topic = req.filters["topic"].as_str().unwrap_or("");
                let sort = if mode == "latest" {
                    "descending"
                } else {
                    "popular"
                };
                gutendex::search_url(&req.query, &req.langs.join(","), topic, sort)
            });
            let cache = db::read_dir()
                .unwrap_or_default()
                .join("cache")
                .join("pg_catalog.csv.gz");
            let q = if req.query.trim().is_empty() {
                "the"
            } else {
                req.query.as_str()
            };
            gutendex::search_or_catalog(c, &url, q, &req.langs.join(","), &cache).await
        }
        "openlibrary" => {
            let only_free = req.filters["ebooks"].as_bool().unwrap_or(true);
            let url = openlibrary::search_url(
                if req.query.trim().is_empty() {
                    "subject:fiction"
                } else {
                    &req.query
                },
                req.langs.first().map(|s| s.as_str()).unwrap_or(""),
                if only_free { "ebooks" } else { "everything" },
                page,
            );
            openlibrary::search(c, &url, page).await
        }
        "standardebooks" => standardebooks::browse(c, &req.query, req.cursor.as_deref()).await,
        "mangadex" => {
            let url = req.cursor.clone().unwrap_or_else(|| {
                let tags: Vec<String> = req.filters["tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect();
                let status: Vec<String> = req.filters["status"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect();
                let mut ratings = vec!["safe".to_string(), "suggestive".to_string()];
                if req.nsfw {
                    ratings.push("erotica".into());
                }
                let order = if mode == "latest" {
                    "latest"
                } else {
                    req.filters["order"].as_str().unwrap_or("")
                };
                mangadex::search_url(&req.query, &req.langs, &ratings, order, &tags, &status, 0)
            });
            mangadex::search(c, &url, if req.lang.is_empty() { "en" } else { &req.lang }).await
        }
        "mangaplus" => {
            let lang = if req.lang.is_empty() {
                "en"
            } else {
                req.lang.as_str()
            };
            mangaplus::browse(c, &req.query, mode, lang).await
        }
        s if s.starts_with("opds:") => {
            let id: i64 = s[5..].parse().map_err(|_| "bad catalog id")?;
            let cat = blocking(move || catalog(id)).await?;
            let url = match (&req.cursor, req.query.trim().is_empty()) {
                (Some(u), _) => u.clone(),
                (None, true) => cat.url.clone(),
                (None, false) => opds_search_url(&cat, &req.query).await?,
            };
            let feed = opds_fetch(&cat, &url).await?;
            Ok(opds_to_page(&feed, &req.source))
        }
        s if s.starts_with("decl:") => {
            let id: i64 = s[5..].parse().map_err(|_| "bad source id")?;
            let (_, source) = declarative_sources()
                .into_iter()
                .find(|(cid, _)| *cid == id)
                .ok_or("source not installed")?;
            declarative::browse(c, &source, mode, &req.query, page, &req.filters)
                .await
                .map(|mut p| {
                    for h in &mut p.hits {
                        h.source = req.source.clone();
                    }
                    p
                })
        }
        s if s.starts_with("suwa:") => {
            let conn = suwayomi_conn().ok_or("Suwayomi bridge is off")?;
            conn.browse(&s[5..], mode, &req.query, page).await
        }
        other => Err(format!("unknown source {other}")),
    }
}

/// Searches every enabled source of `kind` at once, a page each.
#[tauri::command]
pub async fn read_global_search(
    query: String,
    kind: String,
    langs: Vec<String>,
) -> Result<Vec<(String, Page)>, String> {
    let list: Vec<SourceInfo> = read_sources()
        .await?
        .into_iter()
        .filter(|s| s.kind == kind && s.search)
        .collect();
    let futs = list.into_iter().map(|s| {
        let q = query.clone();
        let langs = langs.clone();
        async move {
            let r = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                read_browse(BrowseReq {
                    source: s.id.clone(),
                    query: q,
                    mode: "search".into(),
                    langs,
                    page: 1,
                    ..Default::default()
                }),
            )
            .await;
            match r {
                Ok(Ok(p)) => Some((s.id, p)),
                _ => None,
            }
        }
    });
    Ok(futures::future::join_all(futs)
        .await
        .into_iter()
        .flatten()
        .collect())
}

// ---------- OPDS ----------

async fn opds_fetch(cat: &Catalog, url: &str) -> Result<OpdsFeed, String> {
    let mut req = http().get(url).header(
        "accept",
        "application/atom+xml, application/opds+json, application/xml;q=0.9, */*;q=0.5",
    );
    if !cat.username.is_empty() {
        req = req.basic_auth(&cat.username, Some(&cat.password));
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    if r.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("login required".into());
    }
    if !r.status().is_success() {
        return Err(format!("{} from catalog", r.status()));
    }
    let base = r.url().to_string();
    let body = r.text().await.map_err(|e| e.to_string())?;
    opds::parse(&body, &base)
}

async fn opds_search_url(cat: &Catalog, query: &str) -> Result<String, String> {
    let root = opds_fetch(cat, &cat.url).await?;
    let link = root.search.ok_or("this catalog has no search")?;
    let template = if link.contains('{') {
        link
    } else {
        let mut req = http().get(&link);
        if !cat.username.is_empty() {
            req = req.basic_auth(&cat.username, Some(&cat.password));
        }
        let desc = req
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        opds::opensearch_template(&desc, &link).ok_or("unreadable search description")?
    };
    Ok(opds::fill_search(&template, query))
}

fn opds_to_page(f: &OpdsFeed, source: &str) -> Page {
    let hits = f
        .entries
        .iter()
        .map(|e| {
            let downloads = e
                .acquisitions
                .iter()
                .map(|l| src::Download {
                    url: l.href.clone(),
                    format: mime_to_format(&l.kind),
                    label: if l.title.is_empty() {
                        mime_to_format(&l.kind).to_uppercase()
                    } else {
                        l.title.clone()
                    },
                })
                .collect();
            Hit {
                source: source.to_string(),
                key: e.navigation.clone().unwrap_or_else(|| e.id.clone()),
                title: e.title.clone(),
                authors: e.authors.clone(),
                cover: e.thumbnail.clone().or(e.cover.clone()),
                summary: e.summary.clone(),
                language: e.language.clone(),
                year: e.updated.get(..4).and_then(|y| y.parse().ok()),
                subjects: e.categories.clone(),
                kind: if e.navigation.is_some() && e.acquisitions.is_empty() {
                    "folder".into()
                } else {
                    "book".into()
                },
                downloads,
                url: String::new(),
                extra: serde_json::json!({
                    "series": e.series,
                    "seriesIndex": e.series_index,
                    "stream": e.stream,
                    "streamCount": e.stream_count,
                }),
            }
        })
        .collect();
    Page {
        hits,
        total: f.total,
        next: f.next.clone(),
    }
}

pub fn mime_to_format(m: &str) -> String {
    let m = m.split(';').next().unwrap_or("").trim();
    match m {
        "application/epub+zip" => "epub",
        "application/kepub+zip" => "kepub",
        "application/pdf" => "pdf",
        "application/x-mobipocket-ebook" => "mobi",
        "application/vnd.amazon.ebook" | "application/x-mobi8-ebook" => "azw3",
        "application/x-fictionbook+xml" | "text/fb2+xml" => "fb2",
        "application/vnd.comicbook+zip" | "application/x-cbz" => "cbz",
        "application/vnd.comicbook-rar" | "application/x-cbr" => "cbr",
        "text/plain" => "txt",
        "text/html" | "application/xhtml+xml" => "html",
        "application/zip" => "zip",
        _ => m.rsplit('/').next().unwrap_or("file"),
    }
    .to_string()
}

/// Opens a navigation link inside an OPDS catalog (folders, facets).
#[tauri::command]
pub async fn read_opds_open(source: String, url: String) -> Result<Page, String> {
    let id: i64 = source
        .strip_prefix("opds:")
        .and_then(|s| s.parse().ok())
        .ok_or("not an OPDS catalog")?;
    let cat = blocking(move || catalog(id)).await?;
    let feed = opds_fetch(&cat, &url).await?;
    Ok(opds_to_page(&feed, &source))
}

/// Download URL for a catalog file, with the catalog's credentials inlined
/// for the download engine (Basic auth in the URL).
#[tauri::command]
pub async fn read_opds_download_url(source: String, url: String) -> Result<String, String> {
    let Some(id) = source
        .strip_prefix("opds:")
        .and_then(|s| s.parse::<i64>().ok())
    else {
        return Ok(url);
    };
    let cat = blocking(move || catalog(id)).await?;
    if cat.username.is_empty() {
        return Ok(url);
    }
    let mut u = url::Url::parse(&url).map_err(|e| e.to_string())?;
    let _ = u.set_username(&cat.username);
    let _ = u.set_password(Some(&cat.password));
    Ok(u.to_string())
}

#[tauri::command]
pub async fn read_catalogs() -> Result<Vec<Catalog>, String> {
    // Passwords stay in Rust.
    Ok(blocking(catalogs)
        .await?
        .into_iter()
        .map(|mut c| {
            c.password = if c.password.is_empty() {
                String::new()
            } else {
                "••••••".into()
            };
            c
        })
        .collect())
}

/// Adds an OPDS catalog or connects the Suwayomi bridge. The catalog is
/// fetched once first so a typo fails here, not later.
#[tauri::command]
pub async fn read_catalog_add(
    kind: String,
    title: String,
    url: String,
    username: String,
    password: String,
) -> Result<i64, String> {
    let url = url.trim().to_string();
    let mut title = title.trim().to_string();
    match kind.as_str() {
        "opds" => {
            let probe = Catalog {
                url: url.clone(),
                username: username.clone(),
                password: password.clone(),
                ..Default::default()
            };
            let feed = opds_fetch(&probe, &url).await?;
            if title.is_empty() {
                title = if feed.title.is_empty() {
                    url.clone()
                } else {
                    feed.title
                };
            }
        }
        "suwayomi" => {
            let conn = suwayomi::Conn::new(&url, &username, &password);
            let about = conn.about().await?;
            if title.is_empty() {
                title = format!("Suwayomi {about}");
            }
        }
        _ => return Err("unknown catalog kind".into()),
    }
    blocking(move || {
        let id = db::with(|c| {
            c.execute(
                "INSERT INTO catalogs (kind, title, url, username, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![kind, title, url, username, db::now_ms()],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        super::set_secret(&format!("catalog:{id}"), &password)?;
        Ok(id)
    })
    .await
}

#[tauri::command]
pub async fn read_catalog_remove(id: i64) -> Result<(), String> {
    blocking(move || {
        let _ = super::set_secret(&format!("catalog:{id}"), "");
        db::with(|c| c.execute("DELETE FROM catalogs WHERE id = ?1", [id])).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn read_catalog_toggle(id: i64, enabled: bool) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            c.execute(
                "UPDATE catalogs SET enabled = ?1 WHERE id = ?2",
                params![enabled as i64, id],
            )
        })
        .map(|_| ())
    })
    .await
}

// ---------- declarative source repositories ----------

/// Fetches a repository index (or a single source file) and installs every
/// source in it, verifying each file's sha256.
#[tauri::command]
pub async fn read_repo_install(url: String) -> Result<Vec<String>, String> {
    let installed = declarative::fetch_repo(http(), url.trim()).await?;
    let names: Vec<String> = installed.iter().map(|s| s.name.clone()).collect();
    let repo = url.trim().to_string();
    blocking(move || {
        db::with_mut(|c| {
            let tx = c.transaction()?;
            for s in &installed {
                // Replace an older version of the same source.
                let like = format!("%\"id\":\"{}\"%", s.id.replace('"', ""));
                tx.execute(
                    "DELETE FROM catalogs WHERE kind = 'declarative' AND config LIKE ?1",
                    [like],
                )?;
                let cfg = serde_json::json!({ "repo": repo, "source": s });
                tx.execute(
                    "INSERT INTO catalogs (kind, title, url, config, added_at) VALUES ('declarative', ?1, ?2, ?3, ?4)",
                    params![s.name, s.base_url, cfg.to_string(), db::now_ms()],
                )?;
            }
            tx.commit()
        })
    })
    .await?;
    Ok(names)
}

/// Checks a declarative source end to end (search, details, chapters,
/// pages) and reports which step broke; the health light in the UI.
#[tauri::command]
pub async fn read_source_test(source: String) -> Result<declarative::HealthReport, String> {
    let id: i64 = source
        .strip_prefix("decl:")
        .and_then(|s| s.parse().ok())
        .ok_or("only installed sources can be tested")?;
    let (_, s) = declarative_sources()
        .into_iter()
        .find(|(cid, _)| *cid == id)
        .ok_or("source not installed")?;
    Ok(declarative::health(http(), &s).await)
}

// ---------- series details, chapters, pages ----------

#[derive(Debug, Serialize, Clone)]
pub struct SeriesDetails {
    pub hit: Hit,
    pub chapters: Vec<RemoteChapter>,
}

pub async fn details(
    source: &str,
    key: &str,
    lang: &str,
    langs: &[String],
) -> Result<SeriesDetails, String> {
    let c = http();
    match source {
        "mangadex" => {
            let (hit, chapters) = tokio::join!(
                mangadex::details(c, key, lang),
                mangadex::chapters(c, key, langs)
            );
            Ok(SeriesDetails {
                hit: hit?,
                chapters: chapters?,
            })
        }
        "mangaplus" => {
            let (hit, chapters) = mangaplus::details(c, key).await?;
            Ok(SeriesDetails { hit, chapters })
        }
        s if s.starts_with("decl:") => {
            let id: i64 = s[5..].parse().map_err(|_| "bad source id")?;
            let (_, src) = declarative_sources()
                .into_iter()
                .find(|(cid, _)| *cid == id)
                .ok_or("source not installed")?;
            let (mut hit, chapters) = declarative::details(c, &src, key).await?;
            hit.source = source.to_string();
            Ok(SeriesDetails { hit, chapters })
        }
        s if s.starts_with("suwa:") => {
            let conn = suwayomi_conn().ok_or("Suwayomi bridge is off")?;
            let (hit, chapters) = conn.details(key).await?;
            Ok(SeriesDetails {
                hit: Hit {
                    source: source.to_string(),
                    ..hit
                },
                chapters,
            })
        }
        other => Err(format!("{other} has no series")),
    }
}

pub async fn pages(
    source: &str,
    chapter_key: &str,
    data_saver: bool,
) -> Result<Vec<RemotePage>, String> {
    let c = http();
    match source {
        "mangadex" => mangadex::pages(c, chapter_key, data_saver).await,
        "mangaplus" => mangaplus::pages(c, chapter_key, data_saver).await,
        s if s.starts_with("decl:") => {
            let id: i64 = s[5..].parse().map_err(|_| "bad source id")?;
            let (_, src) = declarative_sources()
                .into_iter()
                .find(|(cid, _)| *cid == id)
                .ok_or("source not installed")?;
            declarative::pages(c, &src, chapter_key).await
        }
        s if s.starts_with("suwa:") => {
            let conn = suwayomi_conn().ok_or("Suwayomi bridge is off")?;
            conn.pages(chapter_key).await
        }
        other => Err(format!("{other} has no pages")),
    }
}

pub fn proxied(info: &super::server::ServerInfo, p: &RemotePage) -> String {
    let headers = serde_json::to_string(&p.headers).unwrap_or_else(|_| "[]".into());
    format!(
        "http://127.0.0.1:{}/img?k={}&u={}&h={}&x={}",
        info.port,
        info.token,
        urlencoding::encode(&p.url),
        urlencoding::encode(&headers),
        p.xor_key.as_deref().unwrap_or("")
    )
}

// ---------- metadata lookup ----------

#[tauri::command]
pub async fn read_meta_lookup(
    title: String,
    author: String,
    isbn: String,
) -> Result<Vec<openlibrary::MetaCandidate>, String> {
    let c = http();
    let (ol, gb) = tokio::join!(
        openlibrary::lookup(c, &title, &author, &isbn),
        src::googlebooks::lookup(c, &title, &author, &isbn)
    );
    let mut out = gb.unwrap_or_default();
    out.extend(ol.unwrap_or_default());
    if out.is_empty() {
        return Err("no matches".into());
    }
    Ok(out)
}

// ---------- Suwayomi extensions ----------

#[tauri::command]
pub async fn read_suwayomi_extensions() -> Result<Vec<serde_json::Value>, String> {
    suwayomi_conn()
        .ok_or("Suwayomi bridge is off")?
        .extensions()
        .await
}

/// `action`: install, update or uninstall.
#[tauri::command]
pub async fn read_suwayomi_extension(pkg: String, action: String) -> Result<(), String> {
    suwayomi_conn()
        .ok_or("Suwayomi bridge is off")?
        .extension(&pkg, &action)
        .await
}

/// Adds a Mihon extension repository (index URL) to the Suwayomi server.
#[tauri::command]
pub async fn read_suwayomi_add_repo(url: String) -> Result<(), String> {
    suwayomi_conn()
        .ok_or("Suwayomi bridge is off")?
        .add_repo(url.trim())
        .await
}
