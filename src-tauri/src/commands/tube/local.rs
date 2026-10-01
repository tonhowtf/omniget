//! Account-free OmniTube: followed channels (RSS feed with a browse
//! fallback), watch history and resume points, watch later, and
//! import/export of subscriptions from other clients.

use std::sync::atomic::{AtomicI64, Ordering};

use futures::StreamExt;
use omniget_core::core::tube::{
    ChannelItem, ChannelRef, HistoryEntry, ImportReport, Item, LocalSub, Page, Shelf, ShelfKind,
    Thumb, VideoItem,
};

use super::db::{self, SubRow};

/// Channels fetched in parallel during one feed refresh.
const FEED_BATCH: usize = 8;
/// At most this many channels hit the network per refresh; the rest come
/// from the cache and catch up on the next one (Grayjay's per-round budget).
const FEED_BUDGET: usize = 150;
/// Without an explicit refresh, the feed is re-fetched after this long.
const FEED_STALE_MS: i64 = 15 * 60_000;
const FEED_LIMIT: u32 = 300;
/// Pause after the service refuses us (429 / "confirm you're not a bot").
const REFUSAL_PAUSE_MS: i64 = 30 * 60_000;

static LAST_FEED_REFRESH: AtomicI64 = AtomicI64::new(0);
static PAUSED_UNTIL: AtomicI64 = AtomicI64::new(0);

// ---------- refusal gate (shared with the auto-downloader) ----------

pub fn paused_until() -> Option<i64> {
    let until = PAUSED_UNTIL.load(Ordering::Relaxed);
    (until > db::now_ms()).then_some(until)
}

pub fn is_refusal(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("429") || e.contains("not a bot") || e.contains("too many requests")
}

/// Returns true when the error closed the gate.
pub fn note_error(err: &str) -> bool {
    if is_refusal(err) {
        PAUSED_UNTIL.store(db::now_ms() + REFUSAL_PAUSE_MS, Ordering::Relaxed);
        tracing::warn!("[tube] refused by the service, pausing channel checks for 30 min");
        true
    } else {
        false
    }
}

// ---------- channel checks ----------

pub fn rss_url(channel_id: &str) -> String {
    format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}")
}

/// A video longer than three minutes cannot be a Short.
pub fn short_from_duration(duration_s: Option<u32>) -> Option<bool> {
    match duration_s {
        Some(d) if d > 180 => Some(false),
        _ => None,
    }
}

/// `/shorts/<id>` redirects to `/watch` for regular videos. `None` when the
/// check could not run.
pub async fn classify_short(video_id: &str) -> Option<bool> {
    let url = format!("https://www.youtube.com/shorts/{video_id}");
    let resp = super::http::client().head(&url).send().await.ok()?;
    // A consent or sign-in interstitial says nothing about the video.
    if !resp.status().is_success() || resp.url().host_str() != Some("www.youtube.com") {
        return None;
    }
    Some(resp.url().path().starts_with("/shorts/"))
}

pub struct Checked {
    pub name: Option<String>,
    /// Videos seen for the first time.
    pub new: Vec<VideoItem>,
}

async fn fetch_channel(
    channel_id: &str,
) -> Result<(Option<String>, Vec<(VideoItem, Option<bool>)>), String> {
    let rss_err = match super::http::get_text(&rss_url(channel_id)).await {
        Ok(xml) => match omniget_core::core::tube::rss::parse_channel_feed(&xml) {
            Some((name, items)) => {
                let items = items
                    .into_iter()
                    .map(|v| {
                        let short = short_from_duration(v.duration_s);
                        (v, short)
                    })
                    .collect();
                return Ok((Some(name).filter(|n| !n.is_empty()), items));
            }
            None => "unreadable feed".to_string(),
        },
        Err(e) => e,
    };
    if is_refusal(&rss_err) {
        return Err(rss_err);
    }
    // The feed 404s for some channels (and for a while after creation); the
    // Videos tab never lists Shorts, so everything there is a regular video.
    match super::browse::channel_videos(channel_id).await {
        Ok(items) => Ok((None, items.into_iter().map(|v| (v, Some(false))).collect())),
        Err(e) => Err(format!("rss: {rss_err}; browse: {e}")),
    }
}

/// Fetches one channel, stores what it found and schedules the next check
/// (adaptive on success, exponential backoff on failure).
pub async fn check_channel(sub: &SubRow) -> Result<Checked, String> {
    let now = db::now_ms();
    match fetch_channel(&sub.channel_id).await {
        Ok((name, items)) => {
            let channel_id = sub.channel_id.clone();
            let name_for_db = name.clone();
            let new = db::with_mut(move |c| {
                let tx = c.transaction()?;
                if let Some(n) = name_for_db.as_deref() {
                    db::sub_upsert(&tx, &channel_id, n, None, now)?;
                }
                let mut new = Vec::new();
                for (v, short) in items {
                    if db::feed_upsert(&tx, &channel_id, &v, short)? {
                        new.push(v);
                    }
                }
                let published = db::feed_published(&tx, &channel_id, 20)?;
                let next = now
                    + super::autodl::jitter(
                        super::autodl::adaptive_interval_ms(&published, now),
                        &channel_id,
                        now,
                    );
                db::sub_checked_ok(&tx, &channel_id, now, next)?;
                tx.commit()?;
                Ok(new)
            })?;
            Ok(Checked { name, new })
        }
        Err(e) => {
            note_error(&e);
            let fails = sub.fail_count.saturating_add(1);
            let next = now + super::autodl::backoff_ms(fails);
            let _ = db::with(|c| db::sub_checked_fail(c, &sub.channel_id, fails, next));
            tracing::debug!("[tube] channel check failed: {e}");
            Err(e)
        }
    }
}

// ---------- read helpers used by browse/filters ----------

/// 0-100 from the local history, for `hide_watched` and the red bar on cards.
pub fn watched_pct(video_id: &str) -> Option<u8> {
    let e = db::with(|c| db::history_get(c, video_id)).ok()??;
    pct(e.position_s, e.duration_s)
}

fn pct(position_s: u32, duration_s: Option<u32>) -> Option<u8> {
    let d = duration_s.filter(|d| *d > 0)?;
    Some(((position_s as u64 * 100) / d as u64).min(100) as u8)
}

pub fn is_subscribed(channel_id: &str) -> bool {
    db::with(|c| db::sub_get(c, channel_id))
        .ok()
        .flatten()
        .is_some()
}

pub fn subs_add(channel_id: &str, name: &str, avatar: Option<&str>) -> Result<(), String> {
    if !channel_id.starts_with("UC") {
        return Err("Invalid channel id".into());
    }
    db::with(|c| db::sub_upsert(c, channel_id, name, avatar, db::now_ms())).map(|_| ())
}

pub fn subs_remove(channel_id: &str) -> Result<(), String> {
    db::with(|c| db::sub_remove(c, channel_id))
}

pub fn watch_later_add(v: &VideoItem) -> Result<(), String> {
    db::with(|c| db::watch_later_add(c, v, db::now_ms()))
}

pub fn watch_later_remove(video_id: &str) -> Result<(), String> {
    db::with(|c| db::watch_later_remove(c, video_id))
}

pub fn watch_later_has(video_id: &str) -> bool {
    db::with(|c| db::watch_later_has(c, video_id)).unwrap_or(false)
}

fn thumbs_of(url: Option<String>) -> Vec<Thumb> {
    url.map(|url| {
        vec![Thumb {
            url,
            width: 0,
            height: 0,
        }]
    })
    .unwrap_or_default()
}

fn history_video(e: HistoryEntry) -> VideoItem {
    VideoItem {
        watched_pct: pct(e.position_s, e.duration_s),
        id: e.video_id,
        title: e.title,
        channel: e.channel,
        thumbs: thumbs_of(e.thumb),
        duration_s: e.duration_s,
        ..Default::default()
    }
}

fn channel_item(s: &SubRow) -> ChannelItem {
    ChannelItem {
        id: s.channel_id.clone(),
        name: s.name.clone(),
        avatar: s.avatar.clone(),
        subscribed: Some(true),
        ..Default::default()
    }
}

// ---------- pages ----------

fn feed_item(row: db::FeedRow, subs: &[SubRow]) -> Item {
    let sub = subs.iter().find(|s| s.channel_id == row.channel_id);
    let mut badges = Vec::new();
    if row.is_live {
        badges.push(omniget_core::core::tube::Badge::Live);
    }
    if row.is_short == Some(true) {
        return Item::Short(omniget_core::core::tube::ShortItem {
            id: row.video_id,
            title: row.title,
            thumbs: thumbs_of(row.thumb),
            views_text: None,
            channel: sub.map(|s| ChannelRef {
                id: s.channel_id.clone(),
                name: s.name.clone(),
                avatar: s.avatar.clone(),
                ..Default::default()
            }),
        });
    }
    Item::Video(VideoItem {
        watched_pct: watched_pct(&row.video_id),
        id: row.video_id,
        title: row.title,
        channel: Some(ChannelRef {
            id: row.channel_id.clone(),
            name: sub.map(|s| s.name.clone()).unwrap_or_default(),
            avatar: sub.and_then(|s| s.avatar.clone()),
            ..Default::default()
        }),
        thumbs: thumbs_of(row.thumb),
        duration_s: row.duration_s,
        views: row.views,
        published_ms: row.published_ms,
        badges,
        ..Default::default()
    })
}

/// The subscriptions feed without an account: every followed channel's RSS
/// in batches, newest first. Channels in backoff keep their cached rows.
pub async fn local_feed(refresh: bool) -> Result<Page, String> {
    let now = db::now_ms();
    let stale = now - LAST_FEED_REFRESH.load(Ordering::Relaxed) > FEED_STALE_MS;
    if (refresh || stale) && paused_until().is_none() {
        LAST_FEED_REFRESH.store(now, Ordering::Relaxed);
        let mut subs = db::with(db::subs_all)?;
        subs.retain(|s| s.fail_count == 0 || s.next_check_ms <= now);
        subs.sort_by_key(|s| s.last_checked_ms.unwrap_or(0));
        subs.truncate(FEED_BUDGET);
        let mut stream = futures::stream::iter(subs)
            .map(|s| async move { check_channel(&s).await })
            .buffer_unordered(FEED_BATCH);
        while let Some(res) = stream.next().await {
            if let Err(e) = res {
                if is_refusal(&e) {
                    break;
                }
            }
        }
    }
    let (subs, rows) = db::with(|c| Ok((db::subs_all(c)?, db::feed_recent(c, FEED_LIMIT)?)))?;
    let items: Vec<Item> = rows
        .into_iter()
        .filter(|r| r.state != "deleted" || r.path.is_some())
        .map(|r| feed_item(r, &subs))
        .collect();
    Ok(Page {
        shelves: vec![Shelf {
            id: "subscriptions".into(),
            kind: ShelfKind::Grid,
            items,
            ..Default::default()
        }],
        ..Default::default()
    })
}

/// "You" without an account. Shelf titles are i18n keys; the frontend
/// translates any title that starts with `tube.`.
pub async fn local_library_page() -> Result<Page, String> {
    let (history, later, subs) = db::with(|c| {
        Ok((
            db::history_list(c, 24)?,
            db::watch_later_list(c, 24)?,
            db::subs_all(c)?,
        ))
    })?;
    let mut shelves = Vec::new();
    if !history.is_empty() {
        shelves.push(Shelf {
            id: "history".into(),
            title: Some("tube.library.history".into()),
            kind: ShelfKind::Row,
            items: history
                .into_iter()
                .map(|e| Item::Video(history_video(e)))
                .collect(),
            ..Default::default()
        });
    }
    if !later.is_empty() {
        shelves.push(Shelf {
            id: "watch_later".into(),
            title: Some("tube.library.watch_later".into()),
            kind: ShelfKind::Row,
            items: later.into_iter().map(Item::Video).collect(),
            ..Default::default()
        });
    }
    if !subs.is_empty() {
        shelves.push(Shelf {
            id: "subscriptions".into(),
            title: Some("tube.library.local_subs".into()),
            kind: ShelfKind::List,
            items: subs
                .iter()
                .map(|s| Item::Channel(channel_item(s)))
                .collect(),
            ..Default::default()
        });
    }
    Ok(Page {
        shelves,
        ..Default::default()
    })
}

/// Local watch history as a list page (used by `tube_history` without an
/// account).
pub fn local_history_page(limit: u32) -> Result<Page, String> {
    let history = db::with(|c| db::history_list(c, limit))?;
    Ok(Page {
        shelves: vec![Shelf {
            id: "history".into(),
            kind: ShelfKind::List,
            items: history
                .into_iter()
                .map(|e| Item::Video(history_video(e)))
                .collect(),
            ..Default::default()
        }],
        ..Default::default()
    })
}

// ---------- commands: subscriptions ----------

#[tauri::command]
pub async fn tube_subs_list() -> Result<Vec<LocalSub>, String> {
    let subs = db::with(db::subs_all)?;
    Ok(subs.iter().map(SubRow::to_local).collect())
}

#[tauri::command]
pub async fn tube_subs_add(
    channel_id: String,
    name: String,
    avatar: Option<String>,
) -> Result<(), String> {
    subs_add(&channel_id, &name, avatar.as_deref())
}

#[tauri::command]
pub async fn tube_subs_remove(channel_id: String) -> Result<(), String> {
    subs_remove(&channel_id)
}

#[tauri::command]
pub async fn tube_subs_update(
    channel_id: String,
    notify: Option<bool>,
    auto_download: Option<bool>,
) -> Result<(), String> {
    db::with(|c| {
        if let Some(n) = notify {
            c.execute(
                "UPDATE subs SET notify = ?2 WHERE channel_id = ?1",
                rusqlite::params![channel_id, n],
            )?;
        }
        if let Some(a) = auto_download {
            // Check soon after turning it on instead of waiting a full interval.
            c.execute(
                "UPDATE subs SET auto_download = ?2, next_check_ms = CASE WHEN ?2 THEN 0 ELSE next_check_ms END
                 WHERE channel_id = ?1",
                rusqlite::params![channel_id, a],
            )?;
        }
        Ok(())
    })
}

// ---------- commands: history and resume ----------

#[tauri::command]
pub async fn tube_local_history(limit: Option<u32>) -> Result<Vec<HistoryEntry>, String> {
    let limit = limit.unwrap_or(200).clamp(1, 5000);
    db::with(|c| db::history_list(c, limit))
}

#[tauri::command]
pub async fn tube_local_history_clear() -> Result<(), String> {
    db::with(db::history_clear)
}

#[tauri::command]
pub async fn tube_local_history_remove(video_id: String) -> Result<(), String> {
    db::with(|c| db::history_remove(c, &video_id))
}

#[tauri::command]
pub async fn tube_progress(mut entry: HistoryEntry, send_account: bool) -> Result<(), String> {
    if entry.video_id.is_empty() {
        return Err("Missing video id".into());
    }
    if entry.watched_ms <= 0 {
        entry.watched_ms = db::now_ms();
    }
    if entry.kind.is_empty() {
        entry.kind = "video".into();
    }
    db::with(|c| db::history_upsert(c, &entry))?;

    // report_watchtime checks `send_history` itself and only logs errors.
    if send_account && super::account::is_logged_in() {
        let id = entry.video_id.clone();
        let pos = entry.position_s as f64;
        tauri::async_runtime::spawn(async move {
            super::streams::report_watchtime(&id, pos).await;
        });
    }
    Ok(())
}

/// Resume point: only for videos of 5 min or more that were not finished
/// (< 90 %) and got past the first 15 s. Backs off 5 s for context.
pub fn resume_point(position_s: u32, duration_s: Option<u32>) -> Option<f64> {
    let d = duration_s?;
    if d < 300 || position_s <= 15 {
        return None;
    }
    if position_s as u64 * 100 >= d as u64 * 90 {
        return None;
    }
    Some((position_s as f64 - 5.0).max(0.0))
}

#[tauri::command]
pub async fn tube_resume_for(video_id: String) -> Result<Option<f64>, String> {
    let e = db::with(|c| db::history_get(c, &video_id))?;
    Ok(e.and_then(|e| resume_point(e.position_s, e.duration_s)))
}

// ---------- commands: import / export ----------

fn is_sqlite(bytes: &[u8]) -> bool {
    bytes.starts_with(b"SQLite format 3\0")
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

fn newpipe_channel_id(url: &str) -> Option<String> {
    let i = url.find("/channel/")?;
    let id: String = url[i + 9..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    id.starts_with("UC").then_some(id)
}

fn newpipe_video_id(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "v") {
        return Some(v.into_owned());
    }
    let seg = u.path_segments()?.next_back()?.to_string();
    (seg.len() == 11).then_some(seg)
}

/// NewPipe / PipePipe `newpipe.db` (Room schema): subscriptions, history with
/// progress, and local playlists. Only service 0 (YouTube).
fn read_newpipe_db(bytes: &[u8]) -> Result<omniget_core::core::tube::imports::Imported, String> {
    use omniget_core::core::tube::{ImportedSub, ImportedWatch};
    let tmp = std::env::temp_dir().join(format!("omnitube-import-{}.db", db::now_ms()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let result = (|| -> rusqlite::Result<omniget_core::core::tube::imports::Imported> {
        let c = rusqlite::Connection::open_with_flags(
            &tmp,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let mut subs = Vec::new();
        {
            let mut st = c.prepare("SELECT url, name FROM subscriptions WHERE service_id = 0")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            })?;
            for row in rows {
                let (url, name) = row?;
                if let Some(id) = url.as_deref().and_then(newpipe_channel_id) {
                    subs.push(ImportedSub {
                        channel_id: id,
                        name: name.unwrap_or_default(),
                    });
                }
            }
        }
        let mut history = Vec::new();
        {
            let mut st = c.prepare(
                "SELECT s.url, s.title, s.uploader, s.uploader_url, s.duration,
                        MAX(h.access_date), st.progress_time
                 FROM stream_history h
                 JOIN streams s ON s.uid = h.stream_id
                 LEFT JOIN stream_state st ON st.stream_id = s.uid
                 WHERE s.service_id = 0
                 GROUP BY s.uid",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                ))
            })?;
            for row in rows {
                let (url, title, uploader, uploader_url, duration, at, progress_ms) = row?;
                if let Some(id) = newpipe_video_id(&url) {
                    history.push(ImportedWatch {
                        video_id: id,
                        title,
                        channel_name: uploader,
                        channel_id: uploader_url.as_deref().and_then(newpipe_channel_id),
                        watched_ms: at,
                        position_s: progress_ms.map(|p| (p.max(0) / 1000) as u32),
                        duration_s: duration.map(|d| d.max(0) as u32),
                    });
                }
            }
        }
        let mut playlists = Vec::new();
        {
            let mut st = c.prepare("SELECT uid, name FROM playlists")?;
            let lists: Vec<(i64, Option<String>)> = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut items = c.prepare(
                "SELECT s.url FROM playlist_stream_join j JOIN streams s ON s.uid = j.stream_id
                 WHERE j.playlist_id = ?1 AND s.service_id = 0 ORDER BY j.join_index",
            )?;
            for (uid, name) in lists {
                let ids: Vec<String> = items
                    .query_map([uid], |r| r.get::<_, String>(0))?
                    .filter_map(|u| u.ok().and_then(|u| newpipe_video_id(&u)))
                    .collect();
                if !ids.is_empty() {
                    playlists.push((name.unwrap_or_default(), ids));
                }
            }
        }
        Ok(omniget_core::core::tube::imports::Imported {
            format: "newpipe".into(),
            subs,
            history,
            playlists,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result.map_err(|e| format!("Unreadable NewPipe database: {e}"))
}

fn newpipe_db_from_zip(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name("newpipe.db").ok()?;
    let mut out = Vec::new();
    file.read_to_end(&mut out).ok()?;
    Some(out)
}

fn merge_imported(
    report: &mut ImportReport,
    imported: omniget_core::core::tube::imports::Imported,
) -> Result<(), String> {
    if report.format == "unknown" || report.format.is_empty() {
        report.format = imported.format.clone();
    }
    let now = db::now_ms();
    let (added, skipped, hist, lists) = db::with_mut(|c| {
        let tx = c.transaction()?;
        let (mut added, mut skipped, mut hist, mut lists) = (0u32, 0u32, 0u32, 0u32);
        for s in &imported.subs {
            if !s.channel_id.starts_with("UC") {
                skipped += 1;
                continue;
            }
            if db::sub_upsert(&tx, &s.channel_id, &s.name, None, now)? {
                added += 1;
            } else {
                skipped += 1;
            }
        }
        for w in &imported.history {
            let entry = HistoryEntry {
                video_id: w.video_id.clone(),
                title: w.title.clone(),
                channel: match (&w.channel_id, &w.channel_name) {
                    (None, None) => None,
                    (id, name) => Some(ChannelRef {
                        id: id.clone().unwrap_or_default(),
                        name: name.clone().unwrap_or_default(),
                        ..Default::default()
                    }),
                },
                thumb: None,
                duration_s: w.duration_s,
                position_s: w.position_s.unwrap_or(0),
                watched_ms: w.watched_ms.unwrap_or(now),
                kind: "video".into(),
            };
            if db::history_import(&tx, &entry)? {
                hist += 1;
            }
        }
        for (title, ids) in &imported.playlists {
            db::playlist_import(&tx, title, ids, now)?;
            lists += 1;
        }
        tx.commit()?;
        Ok((added, skipped, hist, lists))
    })?;
    report.subs_added += added;
    report.subs_skipped += skipped;
    report.history_added += hist;
    report.playlists_added += lists;
    Ok(())
}

#[tauri::command]
pub async fn tube_import(path: String) -> Result<ImportReport, String> {
    let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
    let file_name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut report = ImportReport {
        format: "unknown".into(),
        ..Default::default()
    };

    let newpipe_db = if is_sqlite(&bytes) {
        Some(bytes.clone())
    } else if is_zip(&bytes) {
        newpipe_db_from_zip(&bytes)
    } else {
        None
    };
    if let Some(dbytes) = newpipe_db {
        match tokio::task::spawn_blocking(move || read_newpipe_db(&dbytes))
            .await
            .map_err(|e| e.to_string())?
        {
            // The database is a superset of the subscriptions.json next to it.
            Ok(imported) => {
                merge_imported(&mut report, imported)?;
                return Ok(report);
            }
            Err(e) if is_sqlite(&bytes) => return Err(e),
            Err(e) => report.errors.push(e),
        }
    }

    let imported = omniget_core::core::tube::imports::parse_import(&bytes, &file_name)?;
    merge_imported(&mut report, imported)?;
    Ok(report)
}

/// `format`: "opml" | "newpipe".
#[tauri::command]
pub async fn tube_export_subs(path: String, format: String) -> Result<(), String> {
    let subs: Vec<LocalSub> = db::with(db::subs_all)?
        .iter()
        .map(SubRow::to_local)
        .collect();
    let text = match format.as_str() {
        "newpipe" => omniget_core::core::tube::imports::to_newpipe_json(&subs),
        "opml" => omniget_core::core::tube::imports::to_opml(&subs),
        _ => return Err("Unknown export format".into()),
    };
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| e.to_string())
}
