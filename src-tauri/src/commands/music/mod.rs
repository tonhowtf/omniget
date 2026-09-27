//! `/music` superpower: a local music library and player.
//!
//! Rust indexes, analyzes and serves; the webview plays (see `server.rs` for
//! why). Pure building blocks (tags, LRC, loudness, DSP, playlists) live in
//! `omniget_core::core::music` so the CLI can reuse them.

pub mod analysis;
pub mod convert;
pub mod db;
pub mod online;
pub mod scan;
pub mod server;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use omniget_core::core::music::{lrc, playlist_file, smart, tags, template};

use db::TrackRow;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

// ---------- status & library ----------

#[derive(Serialize)]
pub struct MusicStatus {
    pub port: u16,
    pub token: String,
    pub roots: Vec<String>,
    pub track_count: i64,
    pub scanning: bool,
    pub ffmpeg: bool,
    pub download_dir: String,
    pub analysis: (usize, usize, bool),
}

fn default_download_dir() -> String {
    if let Ok(Some(first)) = db::roots().map(|r| r.into_iter().next()) {
        return PathBuf::from(first)
            .join("OmniGet")
            .to_string_lossy()
            .to_string();
    }
    dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .map(|d| d.join("OmniGet"))
        .unwrap_or_else(|| PathBuf::from("OmniGet Music"))
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
pub async fn music_status(app: AppHandle) -> Result<MusicStatus, String> {
    let info = server::ensure(&app).await?;
    let track_count = db::with(|c| c.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0)))?;
    Ok(MusicStatus {
        port: info.port,
        token: info.token,
        roots: db::roots()?,
        track_count,
        scanning: scan::is_scanning(),
        ffmpeg: omniget_core::core::ffmpeg::is_ffmpeg_available().await,
        download_dir: default_download_dir(),
        analysis: analysis::progress(),
    })
}

#[tauri::command]
pub async fn music_roots_add(app: AppHandle, path: String) -> Result<Vec<String>, String> {
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
pub async fn music_roots_remove(path: String, forget_tracks: bool) -> Result<Vec<String>, String> {
    db::with(|c| c.execute("DELETE FROM roots WHERE path = ?1", [&path]))?;
    if forget_tracks {
        let like = format!(
            "{}%",
            path.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        db::with(|c| c.execute("DELETE FROM tracks WHERE path LIKE ?1 ESCAPE '\\'", [like]))?;
    }
    db::roots()
}

#[tauri::command]
pub async fn music_scan(app: AppHandle, full: Option<bool>) -> Result<scan::ScanSummary, String> {
    scan::scan(app, full.unwrap_or(false)).await
}

#[tauri::command]
pub fn music_scan_cancel() {
    scan::cancel();
}

#[tauri::command]
pub async fn music_tracks() -> Result<Vec<TrackRow>, String> {
    blocking(db::all_tracks).await
}

#[tauri::command]
pub async fn music_tracks_by_ids(ids: Vec<i64>) -> Result<Vec<TrackRow>, String> {
    blocking(move || db::tracks_by_ids(&ids)).await
}

/// Adds files or folders outside the watched roots (drag and drop, a fresh
/// download). `source_url` records where a download came from.
#[tauri::command]
pub async fn music_import_paths(
    paths: Vec<String>,
    source_url: Option<String>,
) -> Result<Vec<TrackRow>, String> {
    blocking(move || {
        let mut ids = Vec::new();
        for p in paths {
            let p = PathBuf::from(p);
            let files: Vec<PathBuf> = if p.is_dir() {
                walkdir::WalkDir::new(&p)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file() && tags::is_audio_path(e.path()))
                    .map(|e| e.into_path())
                    .collect()
            } else if tags::is_audio_path(&p) {
                vec![p]
            } else {
                Vec::new()
            };
            for f in files {
                if let Ok(id) = scan::index_file(&f) {
                    ids.push(id);
                }
            }
        }
        if let Some(url) = source_url.filter(|u| !u.is_empty()) {
            db::with(|c| {
                let mut st = c.prepare("UPDATE tracks SET source_url = ?2 WHERE id = ?1")?;
                for id in &ids {
                    st.execute(params![id, url])?;
                }
                Ok(())
            })?;
        }
        db::tracks_by_ids(&ids)
    })
    .await
}

#[tauri::command]
pub async fn music_remove_tracks(ids: Vec<i64>, delete_files: bool) -> Result<usize, String> {
    blocking(move || {
        let tracks = db::tracks_by_ids(&ids)?;
        if delete_files {
            for t in &tracks {
                let _ = std::fs::remove_file(&t.path);
                if let Some(l) = scan::sidecar_lrc(Path::new(&t.path)) {
                    let _ = std::fs::remove_file(l);
                }
            }
        }
        db::with_mut(|c| {
            let tx = c.transaction()?;
            {
                let mut st = tx.prepare("DELETE FROM tracks WHERE id = ?1")?;
                for t in &tracks {
                    st.execute([t.id])?;
                }
            }
            tx.commit()
        })?;
        Ok(tracks.len())
    })
    .await
}

#[tauri::command]
pub async fn music_record_play(
    id: i64,
    ms_played: i64,
    completed: bool,
    skipped: bool,
) -> Result<(), String> {
    blocking(move || {
        let dur: i64 = db::with(|c| {
            c.query_row("SELECT duration_ms FROM tracks WHERE id = ?1", [id], |r| r.get(0))
                .optional()
        })?
        .unwrap_or(0);
        // a "play" is what scrobblers count: finished, or 4 min, or half the track
        let counts = completed || ms_played >= 240_000 || (dur > 0 && ms_played * 2 >= dur);
        let now = db::now_ms();
        db::with(|c| {
            if ms_played >= 5_000 {
                c.execute(
                    "INSERT INTO history (track_id, played_at, ms_played, completed) VALUES (?1, ?2, ?3, ?4)",
                    params![id, now, ms_played, completed as i64],
                )?;
            }
            if counts {
                c.execute(
                    "UPDATE tracks SET play_count = play_count + 1, last_played = ?2 WHERE id = ?1",
                    params![id, now],
                )?;
            } else if skipped {
                c.execute("UPDATE tracks SET skip_count = skip_count + 1 WHERE id = ?1", [id])?;
            }
            Ok(())
        })
    })
    .await
}

#[tauri::command]
pub async fn music_set_rating(ids: Vec<i64>, rating: i64) -> Result<(), String> {
    let r = rating.clamp(0, 5);
    blocking(move || {
        db::with(|c| {
            let mut st = c.prepare("UPDATE tracks SET rating = ?2 WHERE id = ?1")?;
            for id in &ids {
                st.execute(params![id, r])?;
            }
            Ok(())
        })
    })
    .await
}

#[tauri::command]
pub async fn music_set_liked(ids: Vec<i64>, liked: bool) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            let mut st = c.prepare("UPDATE tracks SET liked = ?2 WHERE id = ?1")?;
            for id in &ids {
                st.execute(params![id, liked as i64])?;
            }
            Ok(())
        })
    })
    .await
}

#[tauri::command]
pub fn music_download_dir() -> String {
    default_download_dir()
}

// ---------- tags ----------

#[derive(Serialize)]
pub struct TagDetail {
    pub meta: tags::TrackMeta,
    pub path: String,
    pub size: u64,
    pub cover: Option<String>,
}

#[tauri::command]
pub async fn music_tags_read(id: i64) -> Result<TagDetail, String> {
    blocking(move || {
        let t = db::track(id)?;
        let (meta, _) = tags::read(Path::new(&t.path)).map_err(|e| e.to_string())?;
        let size = std::fs::metadata(&t.path).map(|m| m.len()).unwrap_or(0);
        Ok(TagDetail {
            meta,
            path: t.path,
            size,
            cover: t.cover,
        })
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CoverInput {
    Keep,
    Remove,
    File { path: String },
    Url { url: String },
}

#[derive(Serialize)]
pub struct WriteReport {
    pub updated: Vec<TrackRow>,
    pub failed: Vec<(i64, String)>,
}

#[tauri::command]
pub async fn music_tags_write(
    ids: Vec<i64>,
    patch: tags::TagPatch,
    cover: Option<CoverInput>,
) -> Result<WriteReport, String> {
    let cover_bytes: Option<Option<Vec<u8>>> = match cover {
        None | Some(CoverInput::Keep) => None,
        Some(CoverInput::Remove) => Some(None),
        Some(CoverInput::File { path }) => {
            Some(Some(std::fs::read(&path).map_err(|e| e.to_string())?))
        }
        Some(CoverInput::Url { url }) => Some(Some(online::fetch_image(&url).await?)),
    };
    blocking(move || {
        let tracks = db::tracks_by_ids(&ids)?;
        let mut failed = Vec::new();
        let mut ok = Vec::new();
        for t in tracks {
            let change = match &cover_bytes {
                None => tags::CoverChange::Keep,
                Some(None) => tags::CoverChange::Remove,
                Some(Some(b)) => tags::CoverChange::Set(b.clone()),
            };
            match tags::write(Path::new(&t.path), &patch, change) {
                Ok(()) => {
                    let _ = scan::index_file(Path::new(&t.path));
                    ok.push(t.id);
                }
                Err(e) => failed.push((t.id, e.to_string())),
            }
        }
        Ok(WriteReport {
            updated: db::tracks_by_ids(&ok)?,
            failed,
        })
    })
    .await
}

/// Writes the analyzed ReplayGain values into the files (loudgain style).
#[tauri::command]
pub async fn music_write_replaygain(ids: Vec<i64>) -> Result<WriteReport, String> {
    blocking(move || {
        let tracks = db::tracks_by_ids(&ids)?;
        let mut failed = Vec::new();
        let mut ok = Vec::new();
        use omniget_core::core::music::loudness::{format_gain_tag, format_peak_tag};
        for t in tracks {
            let Some(g) = t.rg_track_gain else {
                failed.push((t.id, "not analyzed".into()));
                continue;
            };
            if t.codec == "Opus" {
                match tags::write_opus_r128(Path::new(&t.path), Some(g), t.rg_album_gain) {
                    Ok(()) => ok.push(t.id),
                    Err(e) => failed.push((t.id, e.to_string())),
                }
                continue;
            }
            let patch = tags::TagPatch {
                rg_track_gain: Some(format_gain_tag(g)),
                rg_track_peak: t.rg_track_peak.map(format_peak_tag),
                rg_album_gain: t.rg_album_gain.map(format_gain_tag),
                rg_album_peak: t.rg_album_peak.map(format_peak_tag),
                ..Default::default()
            };
            match tags::write(Path::new(&t.path), &patch, tags::CoverChange::Keep) {
                Ok(()) => {
                    let _ = scan::index_file(Path::new(&t.path));
                    ok.push(t.id);
                }
                Err(e) => failed.push((t.id, e.to_string())),
            }
        }
        Ok(WriteReport {
            updated: db::tracks_by_ids(&ok)?,
            failed,
        })
    })
    .await
}

// ---------- analysis ----------

/// Waveform for the seek bar; computed on the spot (fast, mono 11 kHz) when
/// the track was never analyzed.
#[tauri::command]
pub async fn music_peaks(id: i64) -> Result<Vec<u8>, String> {
    if let Some(p) = analysis::load_peaks(id) {
        return Ok(p);
    }
    let t = db::track(id)?;
    let r = analysis::analyze_file(&t.path, t.duration_ms, true).await?;
    analysis::store_peaks(id, &r.peaks)?;
    Ok(r.peaks)
}

#[tauri::command]
pub fn music_analyze(app: AppHandle, ids: Vec<i64>) {
    analysis::enqueue(app, ids);
}

#[tauri::command]
pub fn music_analysis_cancel() {
    analysis::cancel();
}

#[tauri::command]
pub async fn music_spectrogram(id: i64) -> Result<analysis::Spectrogram, String> {
    let t = db::track(id)?;
    analysis::spectrogram(&t.path, 900, 256).await
}

// ---------- lyrics ----------

#[derive(Serialize)]
pub struct LyricsPayload {
    /// "sidecar", "tag", "subtitles", "cache" or "none"
    pub source: String,
    pub raw: String,
    pub lyrics: lrc::Lyrics,
}

fn lyrics_cache(id: i64) -> Option<PathBuf> {
    db::music_dir().map(|d| d.join("lyrics").join(format!("{id}.lrc")))
}

fn subtitle_sidecar(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    let stem = path.file_stem()?.to_str()?;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with(stem)
                && matches!(
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_ascii_lowercase())
                        .as_deref(),
                    Some("vtt") | Some("srt")
                )
        })
}

#[tauri::command]
pub async fn music_lyrics_get(id: i64) -> Result<LyricsPayload, String> {
    blocking(move || {
        let t = db::track(id)?;
        let path = Path::new(&t.path);
        let pack = |source: &str, raw: String, lyrics: lrc::Lyrics| LyricsPayload {
            source: source.into(),
            raw,
            lyrics,
        };
        if let Some(p) = scan::sidecar_lrc(path) {
            if let Ok(raw) = std::fs::read_to_string(&p) {
                let l = lrc::parse(&raw);
                return Ok(pack("sidecar", raw, l));
            }
        }
        if let Some(p) = lyrics_cache(id).filter(|p| p.exists()) {
            if let Ok(raw) = std::fs::read_to_string(&p) {
                let l = lrc::parse(&raw);
                return Ok(pack("cache", raw, l));
            }
        }
        if let Ok((m, _)) = tags::read(path) {
            if !m.lyrics.trim().is_empty() {
                let l = lrc::parse(&m.lyrics);
                return Ok(pack("tag", m.lyrics, l));
            }
        }
        if let Some(p) = subtitle_sidecar(path) {
            if let Ok(raw) = std::fs::read_to_string(&p) {
                let l = lrc::from_subtitles(&raw);
                if l.is_synced() {
                    let text = lrc::serialize(&l);
                    return Ok(pack("subtitles", text, l));
                }
            }
        }
        Ok(pack("none", String::new(), lrc::Lyrics::default()))
    })
    .await
}

#[tauri::command]
pub async fn music_lyrics_search(
    id: i64,
    artist: Option<String>,
    title: Option<String>,
) -> Result<Vec<online::LrclibRecord>, String> {
    let t = db::track(id)?;
    let artist = artist.unwrap_or(t.artist.clone());
    let title = title.unwrap_or(t.title.clone());
    online::lrclib_find(&artist, &title, &t.album, t.duration_ms as f64 / 1000.0).await
}

/// `target`: "sidecar" (a .lrc next to the file), "tag" (inside the file)
/// or "cache" (app folder, for read-only music folders).
#[tauri::command]
pub async fn music_lyrics_save(
    id: i64,
    text: String,
    target: String,
) -> Result<LyricsPayload, String> {
    blocking(move || {
        let t = db::track(id)?;
        let path = PathBuf::from(&t.path);
        let parsed = lrc::parse(&text);
        let mut used = target.clone();
        let res: Result<(), String> = match target.as_str() {
            "tag" => tags::write(
                &path,
                &tags::TagPatch {
                    lyrics: Some(text.clone()),
                    ..Default::default()
                },
                tags::CoverChange::Keep,
            )
            .map_err(|e| e.to_string()),
            "cache" => Err(String::new()),
            _ => std::fs::write(path.with_extension("lrc"), &text).map_err(|e| e.to_string()),
        };
        if res.is_err() {
            // read-only folder or format without lyrics support: keep it in the app
            let p = lyrics_cache(id).ok_or("no app data folder")?;
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::write(&p, &text).map_err(|e| e.to_string())?;
            used = "cache".into();
        }
        let has = !text.trim().is_empty();
        db::with(|c| {
            c.execute(
                "UPDATE tracks SET has_lyrics = ?2 WHERE id = ?1",
                params![id, has as i64],
            )
        })?;
        Ok(LyricsPayload {
            source: used,
            raw: text,
            lyrics: parsed,
        })
    })
    .await
}

// ---------- playlists ----------

#[derive(Serialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub smart: Option<smart::SmartQuery>,
    pub count: i64,
    pub duration_ms: i64,
    pub covers: Vec<String>,
    pub updated_at: i64,
}

#[tauri::command]
pub async fn music_playlists() -> Result<Vec<Playlist>, String> {
    blocking(|| {
        let base: Vec<(i64, String, Option<String>, i64)> = db::with(|c| {
            let mut st = c.prepare(
                "SELECT id, name, smart, updated_at FROM playlists ORDER BY name COLLATE NOCASE",
            )?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect();
            rows
        })?;
        let mut out = Vec::new();
        for (id, name, smart_json, updated_at) in base {
            let smart_q: Option<smart::SmartQuery> =
                smart_json.and_then(|s| serde_json::from_str(&s).ok());
            let tracks = playlist_track_rows(id, smart_q.as_ref())?;
            let mut covers: Vec<String> = Vec::new();
            for t in &tracks {
                if let Some(c) = &t.cover {
                    if !covers.contains(c) {
                        covers.push(c.clone());
                    }
                    if covers.len() == 4 {
                        break;
                    }
                }
            }
            out.push(Playlist {
                id,
                name,
                count: tracks.len() as i64,
                duration_ms: tracks.iter().map(|t| t.duration_ms).sum(),
                smart: smart_q,
                covers,
                updated_at,
            });
        }
        Ok(out)
    })
    .await
}

fn playlist_track_rows(
    id: i64,
    smart_q: Option<&smart::SmartQuery>,
) -> Result<Vec<TrackRow>, String> {
    match smart_q {
        Some(q) => {
            let compiled = smart::compile(q, db::now_ms());
            db::with(|c| {
                let sql = format!(
                    "SELECT {} FROM tracks WHERE {}{}",
                    db::TRACK_COLUMNS,
                    compiled.where_sql,
                    compiled.tail
                );
                let mut st = c.prepare(&sql)?;
                let params: Vec<Box<dyn rusqlite::ToSql>> = compiled
                    .params
                    .iter()
                    .map(|p| -> Box<dyn rusqlite::ToSql> {
                        match p {
                            smart::Param::Text(s) => Box::new(s.clone()),
                            smart::Param::Number(n) => Box::new(*n),
                        }
                    })
                    .collect();
                let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
                let rows = st.query_map(refs.as_slice(), db::row_to_track)?.collect();
                rows
            })
        }
        None => db::with(|c| {
            let cols = db::TRACK_COLUMNS
                .split(", ")
                .map(|c| format!("t.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            let mut st = c.prepare(&format!(
                "SELECT {cols} FROM playlist_items i JOIN tracks t ON t.id = i.track_id
                 WHERE i.playlist_id = ?1 ORDER BY i.position"
            ))?;
            let rows = st.query_map([id], db::row_to_track)?.collect();
            rows
        }),
    }
}

#[tauri::command]
pub async fn music_playlist_tracks(id: i64) -> Result<Vec<TrackRow>, String> {
    blocking(move || {
        let smart_json: Option<String> = db::with(|c| {
            c.query_row("SELECT smart FROM playlists WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
        })?
        .flatten();
        let q: Option<smart::SmartQuery> = smart_json.and_then(|s| serde_json::from_str(&s).ok());
        playlist_track_rows(id, q.as_ref())
    })
    .await
}

/// Preview of a smart query before it is saved.
#[tauri::command]
pub async fn music_smart_preview(query: smart::SmartQuery) -> Result<Vec<TrackRow>, String> {
    blocking(move || playlist_track_rows(0, Some(&query))).await
}

#[tauri::command]
pub async fn music_playlist_create(
    name: String,
    smart: Option<smart::SmartQuery>,
    track_ids: Option<Vec<i64>>,
) -> Result<i64, String> {
    blocking(move || {
        let now = db::now_ms();
        let smart_json = smart.map(|s| serde_json::to_string(&s).unwrap_or_default());
        let id = db::with(|c| {
            c.execute(
                "INSERT INTO playlists (name, smart, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                params![name.trim(), smart_json, now],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        if let Some(ids) = track_ids {
            set_items(id, &ids)?;
        }
        Ok(id)
    })
    .await
}

#[tauri::command]
pub async fn music_playlist_update(
    id: i64,
    name: Option<String>,
    smart: Option<smart::SmartQuery>,
) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            if let Some(n) = name {
                c.execute(
                    "UPDATE playlists SET name = ?2 WHERE id = ?1",
                    params![id, n.trim()],
                )?;
            }
            if let Some(s) = smart {
                c.execute(
                    "UPDATE playlists SET smart = ?2 WHERE id = ?1",
                    params![id, serde_json::to_string(&s).unwrap_or_default()],
                )?;
            }
            c.execute(
                "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
                params![id, db::now_ms()],
            )?;
            Ok(())
        })
    })
    .await
}

#[tauri::command]
pub async fn music_playlist_delete(id: i64) -> Result<(), String> {
    blocking(move || {
        db::with(|c| c.execute("DELETE FROM playlists WHERE id = ?1", [id])).map(|_| ())
    })
    .await
}

fn set_items(id: i64, ids: &[i64]) -> Result<(), String> {
    db::with_mut(|c| {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id])?;
        {
            let mut st = tx.prepare(
                "INSERT INTO playlist_items (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
            )?;
            for (i, t) in ids.iter().enumerate() {
                st.execute(params![id, i as i64, t])?;
            }
        }
        tx.execute(
            "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
            params![id, db::now_ms()],
        )?;
        tx.commit()
    })
}

#[tauri::command]
pub async fn music_playlist_set_items(id: i64, track_ids: Vec<i64>) -> Result<(), String> {
    blocking(move || set_items(id, &track_ids)).await
}

#[tauri::command]
pub async fn music_playlist_add(id: i64, track_ids: Vec<i64>) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            let start: i64 = c.query_row(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_items WHERE playlist_id = ?1",
                [id],
                |r| r.get(0),
            )?;
            let mut st = c.prepare(
                "INSERT INTO playlist_items (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
            )?;
            for (i, t) in track_ids.iter().enumerate() {
                st.execute(params![id, start + i as i64, t])?;
            }
            c.execute(
                "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
                params![id, db::now_ms()],
            )?;
            Ok(())
        })
    })
    .await
}

#[derive(Serialize)]
pub struct ImportReport {
    pub playlist_id: i64,
    pub matched: usize,
    pub missing: Vec<String>,
    pub streams: Vec<playlist_file::PlaylistEntry>,
}

/// Imports an .m3u/.m3u8/.pls: local files become a playlist (indexed if
/// they weren't), URLs come back as radio stations to save.
#[tauri::command]
pub async fn music_playlist_import(path: String) -> Result<ImportReport, String> {
    blocking(move || {
        let p = PathBuf::from(&path);
        let text = std::fs::read(&p).map_err(|e| e.to_string())?;
        let text = String::from_utf8(text.clone()).unwrap_or_else(|_| text.iter().map(|&b| b as char).collect());
        let entries = playlist_file::parse_any(&text, &path, p.parent());
        let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Playlist".into());
        let mut ids = Vec::new();
        let mut missing = Vec::new();
        let mut streams = Vec::new();
        for e in entries {
            if e.is_url() {
                streams.push(e);
                continue;
            }
            let existing: Option<i64> = db::with(|c| {
                c.query_row("SELECT id FROM tracks WHERE path = ?1", [&e.location], |r| r.get(0))
                    .optional()
            })?;
            match existing {
                Some(id) => ids.push(id),
                None if Path::new(&e.location).exists() => match scan::index_file(Path::new(&e.location)) {
                    Ok(id) => ids.push(id),
                    Err(_) => missing.push(e.location),
                },
                None => missing.push(e.location),
            }
        }
        let now = db::now_ms();
        let playlist_id = if ids.is_empty() {
            0
        } else {
            let id = db::with(|c| {
                c.execute(
                    "INSERT INTO playlists (name, smart, created_at, updated_at) VALUES (?1, NULL, ?2, ?2)",
                    params![name, now],
                )?;
                Ok(c.last_insert_rowid())
            })?;
            set_items(id, &ids)?;
            id
        };
        Ok(ImportReport {
            playlist_id,
            matched: ids.len(),
            missing,
            streams,
        })
    })
    .await
}

#[tauri::command]
pub async fn music_playlist_export(id: i64, path: String) -> Result<usize, String> {
    blocking(move || {
        let smart_json: Option<String> = db::with(|c| {
            c.query_row("SELECT smart FROM playlists WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
        })?
        .flatten();
        let q: Option<smart::SmartQuery> = smart_json.and_then(|s| serde_json::from_str(&s).ok());
        let tracks = playlist_track_rows(id, q.as_ref())?;
        write_m3u(&tracks, &path)
    })
    .await
}

/// Exports any list of tracks (the queue, an album, a selection).
#[tauri::command]
pub async fn music_export_m3u(track_ids: Vec<i64>, path: String) -> Result<usize, String> {
    blocking(move || {
        let tracks = db::tracks_by_ids(&track_ids)?;
        write_m3u(&tracks, &path)
    })
    .await
}

fn write_m3u(tracks: &[TrackRow], path: &str) -> Result<usize, String> {
    let entries: Vec<playlist_file::PlaylistEntry> = tracks
        .iter()
        .map(|t| playlist_file::PlaylistEntry {
            location: t.path.clone(),
            title: Some(if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} - {}", t.artist, t.title)
            }),
            duration_s: Some(t.duration_ms as f64 / 1000.0),
            logo: None,
            group: None,
        })
        .collect();
    let base = Path::new(path).parent();
    std::fs::write(path, playlist_file::write_m3u(&entries, base)).map_err(|e| e.to_string())?;
    Ok(entries.len())
}

// ---------- history, stats, duplicates ----------

#[derive(Serialize)]
pub struct HistoryEntry {
    pub track_id: i64,
    pub played_at: i64,
    pub ms_played: i64,
    pub completed: bool,
}

#[tauri::command]
pub async fn music_history(limit: Option<i64>) -> Result<Vec<HistoryEntry>, String> {
    let limit = limit.unwrap_or(200).clamp(1, 5000);
    blocking(move || {
        db::with(|c| {
            let mut st = c.prepare(
                "SELECT track_id, played_at, ms_played, completed FROM history ORDER BY played_at DESC LIMIT ?1",
            )?;
            let rows = st
                .query_map([limit], |r| {
                    Ok(HistoryEntry {
                        track_id: r.get(0)?,
                        played_at: r.get(1)?,
                        ms_played: r.get(2)?,
                        completed: r.get::<_, i64>(3)? != 0,
                    })
                })?
                .collect();
            rows
        })
    })
    .await
}

#[tauri::command]
pub async fn music_history_clear() -> Result<(), String> {
    blocking(|| db::with(|c| c.execute("DELETE FROM history", [])).map(|_| ())).await
}

#[derive(Serialize, Default)]
pub struct Stats {
    pub tracks: i64,
    pub albums: i64,
    pub artists: i64,
    pub total_ms: i64,
    pub total_bytes: i64,
    pub listened_ms_30d: i64,
    pub plays_30d: i64,
    /// (day start ms, ms listened) for the last 30 days
    pub per_day: Vec<(i64, i64)>,
    /// ms listened per hour of the day (local time offset applied by UI)
    pub per_hour: Vec<i64>,
    pub top_tracks: Vec<(i64, i64)>,
    pub top_artists: Vec<(String, i64)>,
    pub top_genres: Vec<(String, i64)>,
    pub formats: Vec<(String, i64)>,
}

#[tauri::command]
pub async fn music_stats(tz_offset_min: Option<i64>) -> Result<Stats, String> {
    let off = tz_offset_min.unwrap_or(0) * 60_000;
    blocking(move || {
        db::with(|c| {
            let mut s = Stats::default();
            (s.tracks, s.total_ms, s.total_bytes) = c.query_row(
                "SELECT COUNT(*), COALESCE(SUM(duration_ms), 0), COALESCE(SUM(size), 0) FROM tracks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            s.albums = c.query_row(
                "SELECT COUNT(*) FROM (SELECT DISTINCT CASE WHEN album_artist = '' THEN artist ELSE album_artist END, album FROM tracks WHERE album <> '')",
                [],
                |r| r.get(0),
            )?;
            s.artists = c.query_row("SELECT COUNT(DISTINCT artist) FROM tracks WHERE artist <> ''", [], |r| r.get(0))?;
            let since = db::now_ms() - 30 * 86_400_000;
            (s.plays_30d, s.listened_ms_30d) = c.query_row(
                "SELECT COUNT(*), COALESCE(SUM(ms_played), 0) FROM history WHERE played_at >= ?1",
                [since],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut st = c.prepare(
                "SELECT ((played_at - ?2) / 86400000) * 86400000 + ?2 AS day, SUM(ms_played) FROM history
                 WHERE played_at >= ?1 GROUP BY day ORDER BY day",
            )?;
            s.per_day = st
                .query_map(params![since, -off], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            s.per_hour = vec![0; 24];
            let mut st = c.prepare(
                "SELECT ((played_at + ?1) / 3600000) % 24 AS h, SUM(ms_played) FROM history GROUP BY h",
            )?;
            for row in st.query_map([off], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
                let (h, ms) = row?;
                if (0..24).contains(&h) {
                    s.per_hour[h as usize] = ms;
                }
            }
            let mut st = c.prepare("SELECT id, play_count FROM tracks WHERE play_count > 0 ORDER BY play_count DESC LIMIT 25")?;
            s.top_tracks = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let mut st = c.prepare(
                "SELECT artist, SUM(play_count) AS p FROM tracks WHERE artist <> '' GROUP BY artist HAVING p > 0 ORDER BY p DESC LIMIT 15",
            )?;
            s.top_artists = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let mut st = c.prepare(
                "SELECT genre, COUNT(*) AS n FROM tracks WHERE genre <> '' GROUP BY genre ORDER BY n DESC LIMIT 15",
            )?;
            s.top_genres = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let mut st = c.prepare("SELECT codec, COUNT(*) AS n FROM tracks GROUP BY codec ORDER BY n DESC")?;
            s.formats = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            Ok(s)
        })
    })
    .await
}

fn norm_key(s: &str) -> String {
    let lower = s.to_lowercase();
    // "Song (Remastered 2011)" and "Song - 2011 Remaster" are the same song
    let cut = lower
        .split(" (")
        .next()
        .unwrap_or(&lower)
        .split(" - ")
        .next()
        .unwrap_or(&lower)
        .split(" [")
        .next()
        .unwrap_or(&lower)
        .to_string();
    cut.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn quality_rank(t: &TrackRow) -> i64 {
    let lossless = matches!(
        t.codec.as_str(),
        "FLAC" | "ALAC" | "WAV" | "AIFF" | "WavPack" | "APE"
    );
    let mut score = if lossless { 10_000_000 } else { 0 };
    score += t.bitrate.unwrap_or(0) * 100;
    score += t.sample_rate.unwrap_or(0) / 100;
    score
}

/// Same artist + title (ignoring remaster/version suffixes) within 4 s of
/// each other. Each group is sorted best quality first.
#[tauri::command]
pub async fn music_duplicates() -> Result<Vec<Vec<TrackRow>>, String> {
    blocking(|| {
        let tracks = db::all_tracks()?;
        let mut groups: HashMap<(String, String), Vec<TrackRow>> = HashMap::new();
        for t in tracks {
            let k = (norm_key(&t.artist), norm_key(&t.title));
            if k.1.is_empty() {
                continue;
            }
            groups.entry(k).or_default().push(t);
        }
        let mut out = Vec::new();
        for (_, mut g) in groups {
            if g.len() < 2 {
                continue;
            }
            g.sort_by_key(|t| t.duration_ms);
            // split into clusters whose durations are close
            let mut cluster: Vec<TrackRow> = Vec::new();
            for t in g {
                if let Some(last) = cluster.last() {
                    if (t.duration_ms - last.duration_ms).abs() > 4000 {
                        if cluster.len() > 1 {
                            let mut c = std::mem::take(&mut cluster);
                            c.sort_by_key(|t| -quality_rank(t));
                            out.push(c);
                        } else {
                            cluster.clear();
                        }
                    }
                }
                cluster.push(t);
            }
            if cluster.len() > 1 {
                cluster.sort_by_key(|t| -quality_rank(t));
                out.push(cluster);
            }
        }
        out.sort_by(|a, b| a[0].artist.to_lowercase().cmp(&b[0].artist.to_lowercase()));
        Ok(out)
    })
    .await
}

// ---------- rename ----------

#[derive(Serialize)]
pub struct RenamePlan {
    pub id: i64,
    pub from: String,
    pub to: String,
    pub conflict: bool,
}

fn rename_plan(ids: &[i64], tpl: &str) -> Result<Vec<RenamePlan>, String> {
    let roots = db::roots()?;
    let tracks = db::tracks_by_ids(ids)?;
    let mut out = Vec::new();
    let mut taken = std::collections::HashSet::new();
    for t in tracks {
        let src = PathBuf::from(&t.path);
        let meta = tags::read(&src).map(|(m, _)| m).unwrap_or_default();
        let base = roots
            .iter()
            .filter(|r| t.path.starts_with(r.as_str()))
            .max_by_key(|r| r.len())
            .map(PathBuf::from)
            .or_else(|| src.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        let mut dest = base.clone();
        for part in template::render(tpl, &meta).split('/') {
            dest.push(part);
        }
        if let Some(ext) = src.extension() {
            dest.set_extension(ext);
        }
        let to = dest.to_string_lossy().to_string();
        let conflict = to != t.path && (dest.exists() || !taken.insert(to.clone()));
        out.push(RenamePlan {
            id: t.id,
            from: t.path,
            to,
            conflict,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn music_rename_preview(
    ids: Vec<i64>,
    template: String,
) -> Result<Vec<RenamePlan>, String> {
    blocking(move || rename_plan(&ids, &template)).await
}

#[tauri::command]
pub async fn music_rename_apply(
    ids: Vec<i64>,
    template: String,
) -> Result<Vec<RenamePlan>, String> {
    blocking(move || {
        let plan = rename_plan(&ids, &template)?;
        let mut done = Vec::new();
        for p in plan {
            if p.conflict || p.from == p.to {
                continue;
            }
            let to = PathBuf::from(&p.to);
            if let Some(d) = to.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            if std::fs::rename(&p.from, &to).is_err() {
                // across drives: copy then delete
                std::fs::copy(&p.from, &to).map_err(|e| e.to_string())?;
                std::fs::remove_file(&p.from).map_err(|e| e.to_string())?;
            }
            if let Some(lrc_path) = scan::sidecar_lrc(Path::new(&p.from)) {
                let _ = std::fs::rename(lrc_path, to.with_extension("lrc"));
            }
            let folder = to
                .parent()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_default();
            db::with(|c| {
                c.execute(
                    "UPDATE tracks SET path = ?2, folder = ?3 WHERE id = ?1",
                    params![p.id, p.to, folder],
                )
            })?;
            done.push(p);
        }
        Ok(done)
    })
    .await
}

// ---------- MusicBrainz ----------

#[tauri::command]
pub async fn music_mb_search(
    id: i64,
    artist: Option<String>,
    title: Option<String>,
    album: Option<String>,
) -> Result<Vec<online::MbCandidate>, String> {
    let t = db::track(id)?;
    online::musicbrainz_search(
        &artist.unwrap_or(t.artist),
        &title.unwrap_or(t.title),
        &album.unwrap_or_default(),
    )
    .await
}

// ---------- conversion ----------

#[tauri::command]
pub async fn music_convert(
    app: AppHandle,
    request: convert::ConvertRequest,
) -> Result<Vec<convert::Job>, String> {
    convert::start(app, request).await
}

#[tauri::command]
pub async fn music_convert_cancel(id: Option<u64>) {
    convert::cancel(id).await;
}

#[tauri::command]
pub async fn music_convert_jobs() -> Vec<convert::Job> {
    convert::jobs().await
}

#[tauri::command]
pub async fn music_convert_clear() {
    convert::clear_finished().await;
}

// ---------- radio ----------

#[tauri::command]
pub async fn music_radio_search(
    query: online::RadioQuery,
) -> Result<Vec<online::RbStation>, String> {
    online::radio_search(&query).await
}

#[tauri::command]
pub async fn music_radio_tags() -> Result<Vec<online::RbTag>, String> {
    online::radio_tags().await
}

#[tauri::command]
pub async fn music_radio_stations() -> Result<Vec<db::Station>, String> {
    blocking(|| {
        db::with(|c| {
            let mut st = c.prepare(
                "SELECT id, name, url, homepage, favicon, tags, country, codec, bitrate, hls, last_played, play_count
                 FROM stations ORDER BY COALESCE(last_played, added_at) DESC",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok(db::Station {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        url: r.get(2)?,
                        homepage: r.get(3)?,
                        favicon: r.get(4)?,
                        tags: r.get(5)?,
                        country: r.get(6)?,
                        codec: r.get(7)?,
                        bitrate: r.get(8)?,
                        hls: r.get::<_, i64>(9)? != 0,
                        last_played: r.get(10)?,
                        play_count: r.get(11)?,
                    })
                })?
                .collect();
            rows
        })
    })
    .await
}

#[tauri::command]
pub async fn music_radio_save(station: db::Station) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            c.execute(
                "INSERT INTO stations (id, name, url, homepage, favicon, tags, country, codec, bitrate, hls, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, url = excluded.url,
                    homepage = excluded.homepage, favicon = excluded.favicon, tags = excluded.tags,
                    country = excluded.country, codec = excluded.codec, bitrate = excluded.bitrate, hls = excluded.hls",
                params![
                    station.id,
                    station.name,
                    station.url,
                    station.homepage,
                    station.favicon,
                    station.tags,
                    station.country,
                    station.codec,
                    station.bitrate,
                    station.hls as i64,
                    db::now_ms()
                ],
            )
        })
        .map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn music_radio_remove(id: String) -> Result<(), String> {
    blocking(move || {
        db::with(|c| c.execute("DELETE FROM stations WHERE id = ?1", [id])).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn music_radio_played(id: String) -> Result<(), String> {
    let uuid = id.clone();
    tauri::async_runtime::spawn(async move { online::radio_click(&uuid).await });
    blocking(move || {
        db::with(|c| {
            c.execute(
                "UPDATE stations SET play_count = play_count + 1, last_played = ?2 WHERE id = ?1",
                params![id, db::now_ms()],
            )
        })
        .map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn music_radio_resolve(url: String) -> Result<String, String> {
    online::resolve_stream(&url).await
}
