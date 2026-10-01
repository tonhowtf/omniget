pub mod ai;
pub mod convert;
pub mod db;
pub mod devices;
pub mod library;
pub mod manga;
pub mod mcp;
pub mod scan;
pub mod server;
pub mod sources;
pub mod sync;

use std::path::PathBuf;

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use db::BookRow;

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

pub fn secret(key: &str) -> String {
    crate::secrets::load_secret(crate::secrets::READ, key)
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        crate::secrets::delete_secret(crate::secrets::READ, key)
    } else {
        crate::secrets::save_secret(crate::secrets::READ, key, value)
    }
}

#[derive(Serialize)]
pub struct ReadStatus {
    pub port: u16,
    pub token: String,
    pub roots: Vec<String>,
    pub book_count: i64,
    pub series_count: i64,
    pub scanning: bool,
    pub download_dir: String,
    pub share: server::ShareConfig,
    pub share_running: bool,
    pub share_port: Option<u16>,
    pub lan_ip: Option<String>,
}

pub fn default_download_dir() -> String {
    if let Some(d) = db::kv_get::<String>("download_dir").filter(|d| !d.is_empty()) {
        return d;
    }
    if let Ok(Some(first)) = db::roots().map(|r| r.into_iter().next()) {
        return PathBuf::from(first)
            .join("OmniGet")
            .to_string_lossy()
            .to_string();
    }
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .map(|d| d.join("OmniGet Books"))
        .unwrap_or_else(|| PathBuf::from("OmniGet Books"))
        .to_string_lossy()
        .to_string()
}

fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_loopback()).then(|| ip.to_string())
}

#[tauri::command]
pub async fn read_status(app: AppHandle) -> Result<ReadStatus, String> {
    let info = server::ensure(&app).await?;
    manga::start_update_loop(app.clone());
    static SCANNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !db::roots()?.is_empty() && !SCANNED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = scan::scan(app2, false).await;
        });
    }
    let share = server::share_config();
    if share.enabled && !server::lan_running() {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = server::start_lan(&app2, &share).await {
                tracing::warn!("[read] network share: {e}");
            }
        });
    }
    let (book_count, series_count) = db::with(|c| {
        Ok((
            c.query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))?,
            c.query_row(
                "SELECT COUNT(*) FROM series WHERE in_library = 1",
                [],
                |r| r.get(0),
            )?,
        ))
    })?;
    Ok(ReadStatus {
        port: info.port,
        token: info.token,
        roots: db::roots()?,
        book_count,
        series_count,
        scanning: scan::is_scanning(),
        download_dir: default_download_dir(),
        share: server::ShareConfig {
            password: if server::share_config().password.is_empty() {
                String::new()
            } else {
                "••••••".into()
            },
            ..server::share_config()
        },
        share_running: server::lan_running(),
        share_port: server::lan_port(),
        lan_ip: lan_ip(),
    })
}

#[tauri::command]
pub async fn read_set_download_dir(dir: String) -> Result<String, String> {
    blocking(move || {
        db::kv_set("download_dir", &dir.trim())?;
        Ok(default_download_dir())
    })
    .await
}

#[tauri::command]
pub async fn read_roots_add(app: AppHandle, path: String) -> Result<Vec<String>, String> {
    let p = PathBuf::from(path.trim());
    if !p.is_dir() {
        return Err("not a folder".into());
    }
    let norm = p
        .to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string();
    db::with(|c| {
        c.execute(
            "INSERT OR IGNORE INTO roots (path, added_at) VALUES (?1, ?2)",
            params![norm, db::now_ms()],
        )
    })?;
    tauri::async_runtime::spawn(async move {
        let _ = scan::scan(app, false).await;
    });
    db::roots()
}

#[tauri::command]
pub async fn read_roots_remove(path: String, forget_books: bool) -> Result<Vec<String>, String> {
    db::with(|c| c.execute("DELETE FROM roots WHERE path = ?1", [&path]))?;
    if forget_books {
        let like = format!(
            "{}%",
            path.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        db::with(|c| {
            c.execute(
                "DELETE FROM books WHERE path LIKE ?1 ESCAPE '\\' AND series_id IS NULL",
                [like],
            )
        })?;
    }
    db::roots()
}

#[tauri::command]
pub async fn read_scan(app: AppHandle, full: Option<bool>) -> Result<scan::ScanSummary, String> {
    scan::scan(app, full.unwrap_or(false)).await
}

#[tauri::command]
pub fn read_scan_cancel() {
    scan::cancel();
}

#[tauri::command]
pub async fn read_import_paths(
    app: AppHandle,
    paths: Vec<String>,
    source: Option<String>,
    source_url: Option<String>,
) -> Result<Vec<BookRow>, String> {
    let rows = blocking(move || {
        let opts = scan::IndexOpts {
            source: source.unwrap_or_else(|| "local".into()),
            source_url: source_url.unwrap_or_default(),
            ..Default::default()
        };
        let mut ids = Vec::new();
        for p in paths {
            let p = PathBuf::from(p);
            let targets: Vec<PathBuf> = if p.is_dir() {
                let images = omniget_core::core::read::comic::folder_pages(&p);
                if !images.is_empty() && images.len() >= 2 {
                    vec![p.clone()]
                } else {
                    walkdir::WalkDir::new(&p)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file() && scan::is_book_path(e.path()))
                        .map(|e| e.into_path())
                        .collect()
                }
            } else {
                vec![p]
            };
            for t in targets {
                match scan::index_file(&t, &opts) {
                    Ok(id) => ids.push(id),
                    Err(e) => tracing::debug!("[read] import {}: {e}", t.display()),
                }
            }
        }
        db::books_by_ids(&ids)
    })
    .await?;
    let _ = app.emit("read-library-changed", ());
    Ok(rows)
}

#[tauri::command]
pub async fn read_set_cover(
    app: AppHandle,
    id: i64,
    data_base64: Option<String>,
    url: Option<String>,
    path: Option<String>,
) -> Result<BookRow, String> {
    use base64::Engine;
    let bytes = match (data_base64, url, path) {
        (_, _, Some(p)) => tokio::fs::read(&p).await.map_err(|e| e.to_string())?,
        (Some(b), _, _) => base64::engine::general_purpose::STANDARD
            .decode(b.split(',').next_back().unwrap_or(""))
            .map_err(|e| e.to_string())?,
        (None, Some(u), None) => sources::http()
            .get(&u)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map_err(|e| e.to_string())?
            .to_vec(),
        _ => return Err("no image".into()),
    };
    let row = blocking(move || {
        let hash = scan::store_cover(&bytes).ok_or("not an image")?;
        db::with(|c| {
            c.execute(
                "UPDATE books SET cover = ?1 WHERE id = ?2",
                params![hash, id],
            )
        })?;
        db::book(id)
    })
    .await?;
    let _ = app.emit("read-library-changed", ());
    Ok(row)
}

#[tauri::command]
pub async fn read_prepare(app: AppHandle, id: i64) -> Result<String, String> {
    let info = server::ensure(&app).await?;
    blocking(move || {
        let b = db::book(id)?;
        let out = prepared_path(&b).ok_or("no cache folder")?;
        if !out.exists() {
            use omniget_core::core::read::epub_writer as w;
            let bytes = std::fs::read(&b.path).map_err(|e| e.to_string())?;
            let text = w::decode_text(&bytes);
            let meta = w::EpubMeta {
                title: b.title.clone(),
                authors: b.authors.clone(),
                language: b.language.clone(),
                ..Default::default()
            };
            let chapters = match b.format.as_str() {
                "md" => w::markdown_chapters(&text, &b.title),
                "html" => vec![w::html_chapter(&text, &b.title)],
                _ => w::text_chapters(&text, &b.title),
            };
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            w::write_reflowable(&out, &meta, &chapters, None)?;
        }
        Ok(format!(
            "http://127.0.0.1:{}/bc/{id}?k={}",
            info.port, info.token
        ))
    })
    .await
}

pub fn prepared_path(b: &BookRow) -> Option<PathBuf> {
    db::read_dir().map(|d| {
        d.join("cache")
            .join(format!("{}-{}.epub", b.id, b.mtime_ms))
    })
}

#[tauri::command]
pub async fn read_save_url(app: AppHandle, url: String) -> Result<BookRow, String> {
    let r = sources::http()
        .get(url.trim())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("{} from the page", r.status()));
    }
    let final_url = r.url().to_string();
    let html = r.text().await.map_err(|e| e.to_string())?;
    let row = blocking(move || {
        use omniget_core::core::read::epub_writer as w;
        let host = url::Url::parse(&final_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let ch = w::article_from_html(&html, &host);
        let dir = PathBuf::from(default_download_dir()).join("Articles");
        let out = dir.join(sanitize_filename::sanitize(format!(
            "{}.epub",
            ch.title.chars().take(90).collect::<String>()
        )));
        let meta = w::EpubMeta {
            title: ch.title.clone(),
            authors: vec![host.clone()],
            publisher: host,
            source_url: final_url.clone(),
            subjects: vec!["Article".into()],
            ..Default::default()
        };
        w::write_reflowable(&out, &meta, &[ch], None)?;
        let id = scan::index_file(
            &out,
            &scan::IndexOpts {
                source: "web".into(),
                source_url: final_url,
                ..Default::default()
            },
        )?;
        db::book(id)
    })
    .await?;
    let _ = app.emit("read-library-changed", ());
    Ok(row)
}

#[tauri::command]
pub async fn read_comic_pages(id: i64) -> Result<usize, String> {
    blocking(move || {
        let b = db::book(id)?;
        let n = server::page_count(&b.path);
        if n > 0 && b.pages != Some(n as i64) {
            db::with(|c| {
                c.execute(
                    "UPDATE books SET pages = ?1 WHERE id = ?2",
                    params![n as i64, id],
                )
            })?;
        }
        Ok(n)
    })
    .await
}

#[tauri::command]
pub async fn read_share_set(
    app: AppHandle,
    mut cfg: server::ShareConfig,
) -> Result<Option<u16>, String> {
    if cfg.password != "••••••" {
        set_secret("share", &cfg.password)?;
    }
    cfg.password = secret("share");
    db::kv_set(
        "share",
        &server::ShareConfig {
            password: String::new(),
            ..cfg.clone()
        },
    )?;
    if cfg.enabled {
        server::start_lan(&app, &cfg).await.map(Some)
    } else {
        server::stop_lan();
        Ok(None)
    }
}
