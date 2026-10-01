use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use grammers_client::grammers_tl_types as tl;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::api::{self, TelegramMediaItem};
use super::auth::TelegramSessionHandle;
use super::events;
use super::history::{self, MediaFilter};
use crate::core::telegram::host;

const PAGE: i32 = 100;

pub fn file_uid(media: &tl::enums::MessageMedia) -> Option<String> {
    match media {
        tl::enums::MessageMedia::Document(d) => match d.document.as_ref()? {
            tl::enums::Document::Document(doc) => Some(format!("d{}", doc.id)),
            tl::enums::Document::Empty(_) => None,
        },
        tl::enums::MessageMedia::Photo(p) => match p.photo.as_ref()? {
            tl::enums::Photo::Photo(photo) => Some(format!("p{}", photo.id)),
            tl::enums::Photo::Empty(_) => None,
        },
        _ => None,
    }
}

fn index_dir() -> PathBuf {
    let d = host::data_dir().join("index");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn db_path(chat_id: i64) -> PathBuf {
    index_dir().join(format!("{}.sqlite", chat_id))
}

fn open(chat_id: i64) -> Result<Connection, String> {
    let c = Connection::open(db_path(chat_id)).map_err(|e| e.to_string())?;
    c.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS media (
            msg_id INTEGER PRIMARY KEY,
            date INTEGER NOT NULL,
            media_type TEXT NOT NULL,
            size INTEGER NOT NULL,
            name TEXT NOT NULL,
            caption TEXT,
            grouped_id INTEGER,
            file_uid TEXT
         );
         CREATE INDEX IF NOT EXISTS media_date ON media(date);
         CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
    )
    .map_err(|e| e.to_string())?;
    Ok(c)
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Coverage {
    pub newest_id: i32,
    pub oldest_id: i32,
    pub complete: bool,
    pub updated_at: i64,
    pub oldest_date: i64,
}

fn meta_get(c: &Connection, key: &str) -> i64 {
    c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
        .unwrap_or(0)
}

fn meta_set(c: &Connection, key: &str, value: i64) {
    let _ = c.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    );
}

fn coverage_of(c: &Connection) -> Coverage {
    Coverage {
        newest_id: meta_get(c, "newest_id") as i32,
        oldest_id: meta_get(c, "oldest_id") as i32,
        complete: meta_get(c, "complete") != 0,
        updated_at: meta_get(c, "updated_at"),
        oldest_date: meta_get(c, "oldest_date"),
    }
}

fn save_coverage(c: &Connection, cov: &Coverage) {
    meta_set(c, "newest_id", cov.newest_id as i64);
    meta_set(c, "oldest_id", cov.oldest_id as i64);
    meta_set(c, "complete", cov.complete as i64);
    meta_set(c, "updated_at", super::now_secs());
    meta_set(c, "oldest_date", cov.oldest_date);
}

pub fn coverage(chat_id: i64) -> Coverage {
    if !db_path(chat_id).is_file() {
        return Coverage::default();
    }
    open(chat_id).map(|c| coverage_of(&c)).unwrap_or_default()
}

pub fn stats(chat_id: i64) -> serde_json::Value {
    let (indexed, cov) = if db_path(chat_id).is_file() {
        match open(chat_id) {
            Ok(c) => (
                c.query_row("SELECT COUNT(*) FROM media", [], |r| r.get::<_, i64>(0))
                    .unwrap_or(0),
                coverage_of(&c),
            ),
            Err(_) => (0, Coverage::default()),
        }
    } else {
        (0, Coverage::default())
    };
    json!({
        "chat_id": chat_id,
        "indexed": indexed,
        "newest_id": cov.newest_id,
        "oldest_id": cov.oldest_id,
        "complete": cov.complete,
        "updated_at": cov.updated_at,
    })
}

pub fn clear(chat_id: Option<i64>) {
    match chat_id {
        Some(id) => {
            let p = db_path(id);
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{}", p.display(), suffix));
            }
        }
        None => {
            let _ = std::fs::remove_dir_all(index_dir());
        }
    }
}

struct Row {
    item: TelegramMediaItem,
    uid: Option<String>,
}

fn upsert(c: &mut Connection, rows: &[Row]) -> Result<(), String> {
    let tx = c.transaction().map_err(|e| e.to_string())?;
    {
        let mut st = tx
            .prepare(
                "INSERT INTO media (msg_id, date, media_type, size, name, caption, grouped_id, file_uid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(msg_id) DO UPDATE SET date = excluded.date,
                    media_type = excluded.media_type, size = excluded.size, name = excluded.name,
                    caption = excluded.caption, grouped_id = excluded.grouped_id,
                    file_uid = excluded.file_uid",
            )
            .map_err(|e| e.to_string())?;
        for r in rows {
            let it = &r.item;
            st.execute(params![
                it.message_id,
                it.date,
                it.media_type,
                it.file_size as i64,
                it.file_name,
                it.caption,
                it.grouped_id,
                r.uid,
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn query(chat_id: i64, filter: &MediaFilter, after_id: i32) -> Vec<TelegramMediaItem> {
    query_rows(chat_id, filter, after_id)
        .into_iter()
        .map(|(it, _)| it)
        .collect()
}

pub fn query_rows(
    chat_id: i64,
    filter: &MediaFilter,
    after_id: i32,
) -> Vec<(TelegramMediaItem, Option<String>)> {
    if !db_path(chat_id).is_file() {
        return Vec::new();
    }
    let Ok(c) = open(chat_id) else {
        return Vec::new();
    };
    let mut sql = String::from(
        "SELECT msg_id, date, media_type, size, name, caption, grouped_id, file_uid FROM media WHERE msg_id > ?1",
    );
    let mut nums: Vec<i64> = vec![after_id as i64];
    let mut bound = |sql: &mut String, cond: &str, v: i64| {
        nums.push(v);
        sql.push_str(&format!(" AND {} ?{}", cond, nums.len()));
    };
    if let Some(v) = filter.from_date {
        bound(&mut sql, "date >=", v as i64);
    }
    if let Some(v) = filter.to_date {
        bound(&mut sql, "date <=", v as i64);
    }
    if let Some(v) = filter.min_size.filter(|v| *v > 0.0) {
        bound(&mut sql, "size >=", v as i64);
    }
    if let Some(v) = filter.max_size.filter(|v| *v > 0.0) {
        bound(&mut sql, "size <=", v as i64);
    }
    if let Some(v) = filter.from_message_id.filter(|v| *v > 0) {
        bound(&mut sql, "msg_id >=", v);
    }
    if let Some(v) = filter.to_message_id.filter(|v| *v > 0) {
        bound(&mut sql, "msg_id <=", v);
    }
    sql.push_str(" ORDER BY msg_id DESC");
    let Ok(mut st) = c.prepare(&sql) else {
        return Vec::new();
    };
    let limit = filter.limit.filter(|l| *l > 0).map(|l| l as usize);
    let rows = st.query_map(rusqlite::params_from_iter(nums.iter()), |r| {
        Ok((
            TelegramMediaItem {
                message_id: r.get(0)?,
                date: r.get(1)?,
                media_type: r.get(2)?,
                file_size: r.get::<_, i64>(3)?.max(0) as u64,
                file_name: r.get(4)?,
                webpage: None,
                caption: r.get(5)?,
                grouped_id: r.get(6)?,
            },
            r.get::<_, Option<String>>(7)?,
        ))
    });
    let Ok(rows) = rows else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in rows.flatten() {
        if filter.matches(&row.0) {
            out.push(row);
            if limit.is_some_and(|l| out.len() >= l) {
                break;
            }
        }
    }
    out
}

fn scans() -> &'static Mutex<HashMap<String, CancellationToken>> {
    static S: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_scan(scan_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    scans()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(scan_id.to_string(), token.clone());
    token
}

pub fn unregister_scan(scan_id: &str) {
    scans()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(scan_id);
}

pub fn cancel_scan(scan_id: &str) -> bool {
    match scans()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(scan_id)
    {
        Some(t) => {
            t.cancel();
            true
        }
        None => false,
    }
}

pub fn new_scan_id() -> String {
    format!("scan{}", events::next_id())
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateResult {
    pub scanned: usize,
    pub coverage: Coverage,
    pub cancelled: bool,
}

pub struct UpdateOpts<'a> {
    pub scan_id: &'a str,
    pub max_scan: usize,
    pub takeout: bool,
    pub filter: Option<&'a MediaFilter>,
    pub stop_at_id: i32,
    pub emit: bool,
}

fn messages_of(result: tl::enums::messages::Messages) -> Vec<tl::enums::Message> {
    match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    }
}

async fn page(
    client: &grammers_client::Client,
    peer: &tl::enums::InputPeer,
    takeout_id: Option<i64>,
    offset_id: i32,
    min_id: i32,
    fix_ext: bool,
) -> anyhow::Result<(Vec<Row>, i32, i32, i64, usize)> {
    let request = tl::functions::messages::GetHistory {
        peer: peer.clone(),
        offset_id,
        offset_date: 0,
        add_offset: 0,
        limit: PAGE,
        max_id: 0,
        min_id,
        hash: 0,
    };
    let result = match takeout_id {
        Some(tid) => api::invoke_with_takeout(client, tid, request).await,
        None => api::invoke_with_flood_wait(client, &request).await,
    }
    .map_err(|e| anyhow::anyhow!("messages.GetHistory: {}", e))?;
    let messages = messages_of(result);
    let mut rows = Vec::new();
    let mut lowest = i32::MAX;
    let mut highest = 0;
    let mut oldest_date = 0i64;
    for raw in &messages {
        let (id, date) = match raw {
            tl::enums::Message::Message(m) => (m.id, m.date as i64),
            tl::enums::Message::Service(m) => (m.id, m.date as i64),
            tl::enums::Message::Empty(m) => (m.id, 0),
        };
        if id <= min_id {
            continue;
        }
        lowest = lowest.min(id);
        highest = highest.max(id);
        if date > 0 && (oldest_date == 0 || date < oldest_date) {
            oldest_date = date;
        }
        if let tl::enums::Message::Message(m) = raw {
            if let Some(item) = history::to_item(m, fix_ext) {
                let uid = m.media.as_ref().and_then(file_uid);
                rows.push(Row { item, uid });
            }
        }
    }
    Ok((rows, lowest, highest, oldest_date, messages.len()))
}

pub async fn update(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    opts: &UpdateOpts<'_>,
    cancel: &CancellationToken,
    mut need_more: impl FnMut(&Coverage, usize) -> bool + Send,
) -> anyhow::Result<UpdateResult> {
    let (client, peer, _) = history::peer_for(handle, chat_id, chat_type).await?;
    let fix_ext = super::fix_file_extensions();
    let mut conn = open(chat_id).map_err(|e| anyhow::anyhow!(e))?;
    let mut cov = coverage_of(&conn);
    let mut matched = opts
        .filter
        .map(|f| {
            let mut f = f.clone();
            f.limit = None;
            query(chat_id, &f, 0).len()
        })
        .unwrap_or(0);
    let takeout_id = if opts.takeout {
        Some(api::init_takeout(handle).await?)
    } else {
        None
    };
    let mut scanned = 0usize;
    let cap_hit = |n: usize| opts.max_scan > 0 && n >= opts.max_scan;
    let emit = |scanned: usize, matched: usize, done: bool, cancelled: bool| {
        if opts.emit {
            events::emit(
                "telegram-chat-scan",
                json!({
                    "scan_id": opts.scan_id,
                    "chat_id": chat_id,
                    "scanned": scanned,
                    "matched": matched,
                    "done": done,
                    "cancelled": cancelled,
                }),
            );
        }
    };
    let count_matches = |rows: &[Row]| {
        opts.filter
            .map(|f| rows.iter().filter(|r| f.matches(&r.item)).count())
            .unwrap_or(rows.len())
    };

    let result: anyhow::Result<()> = async {
        if cov.newest_id > 0 {
            let mut offset_id = 0;
            let mut top = cov.newest_id;
            let mut lowest_read = i32::MAX;
            let mut reached = false;
            loop {
                if cancel.is_cancelled() {
                    return Ok(());
                }
                let (rows, lowest, highest, _, raw) = page(
                    &client,
                    &peer,
                    takeout_id,
                    offset_id,
                    cov.newest_id,
                    fix_ext,
                )
                .await?;
                if raw == 0 || lowest == i32::MAX {
                    reached = true;
                    break;
                }
                scanned += raw;
                matched += count_matches(&rows);
                upsert(&mut conn, &rows).map_err(|e| anyhow::anyhow!(e))?;
                top = top.max(highest);
                lowest_read = lowest_read.min(lowest);
                emit(scanned, matched, false, false);
                if (raw as i32) < PAGE || lowest <= cov.newest_id + 1 {
                    reached = true;
                    break;
                }
                if cap_hit(scanned) {
                    break;
                }
                offset_id = lowest;
                if takeout_id.is_none() {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }
            if reached {
                cov.newest_id = top;
            } else {
                cov.newest_id = top;
                cov.oldest_id = lowest_read;
                cov.complete = false;
                cov.oldest_date = 0;
            }
            save_coverage(&conn, &cov);
        }

        while !cov.complete && !cap_hit(scanned) && need_more(&cov, matched) {
            if cancel.is_cancelled() {
                return Ok(());
            }
            if opts.stop_at_id > 0 && cov.oldest_id > 0 && cov.oldest_id <= opts.stop_at_id {
                break;
            }
            let offset_id = if cov.oldest_id > 0 { cov.oldest_id } else { 0 };
            let (rows, lowest, highest, oldest_date, raw) =
                page(&client, &peer, takeout_id, offset_id, 0, fix_ext).await?;
            if raw == 0 || lowest == i32::MAX {
                cov.complete = true;
                save_coverage(&conn, &cov);
                break;
            }
            scanned += raw;
            matched += count_matches(&rows);
            upsert(&mut conn, &rows).map_err(|e| anyhow::anyhow!(e))?;
            cov.newest_id = cov.newest_id.max(highest);
            cov.oldest_id = if cov.oldest_id > 0 {
                cov.oldest_id.min(lowest)
            } else {
                lowest
            };
            if oldest_date > 0 {
                cov.oldest_date = oldest_date;
            }
            if lowest <= 1 || (raw as i32) < PAGE {
                cov.complete = true;
            }
            save_coverage(&conn, &cov);
            emit(scanned, matched, false, false);
            if takeout_id.is_none() {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
        Ok(())
    }
    .await;

    if takeout_id.is_some() {
        let _ = api::finish_takeout(handle, result.is_ok()).await;
    }
    let cancelled = cancel.is_cancelled();
    emit(scanned, matched, true, cancelled);
    result?;
    Ok(UpdateResult {
        scanned,
        coverage: cov,
        cancelled,
    })
}

pub fn needs_more_for(filter: &MediaFilter, cov: &Coverage, matched: usize) -> bool {
    if cov.complete {
        return false;
    }
    if let Some(limit) = filter.limit.filter(|l| *l > 0) {
        if matched as u64 >= limit {
            return false;
        }
    }
    if let Some(from) = filter.from_date {
        if cov.oldest_date > 0 && (cov.oldest_date as f64) < from {
            return false;
        }
    }
    if let Some(from) = filter.from_message_id.filter(|v| *v > 0) {
        if cov.oldest_id > 0 && (cov.oldest_id as i64) <= from {
            return false;
        }
    }
    true
}

fn search_filter_for(kind: &str) -> Option<tl::enums::MessagesFilter> {
    use tl::enums::MessagesFilter as F;
    Some(match kind {
        "video" => F::InputMessagesFilterVideo,
        "photo" => F::InputMessagesFilterPhotos,
        "document" => F::InputMessagesFilterDocument,
        "audio" => F::InputMessagesFilterMusic,
        "gif" => F::InputMessagesFilterGif,
        "round_video" => F::InputMessagesFilterRoundVideo,
        "voice" | "round_voice" => F::InputMessagesFilterRoundVoice,
        _ => return None,
    })
}

async fn search_by_type(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    kinds: &[String],
    filter: &MediaFilter,
    opts: &UpdateOpts<'_>,
    cancel: &CancellationToken,
) -> anyhow::Result<UpdateResult> {
    let (client, peer, _) = history::peer_for(handle, chat_id, chat_type).await?;
    let fix_ext = super::fix_file_extensions();
    let mut conn = open(chat_id).map_err(|e| anyhow::anyhow!(e))?;
    let mut scanned = 0usize;
    let mut matched = 0usize;
    let limit = filter.limit.filter(|l| *l > 0).map(|l| l as usize);
    let min_id = opts.stop_at_id.max(0);
    let emit = |scanned: usize, matched: usize, done: bool, cancelled: bool| {
        if opts.emit {
            events::emit(
                "telegram-chat-scan",
                json!({
                    "scan_id": opts.scan_id,
                    "chat_id": chat_id,
                    "scanned": scanned,
                    "matched": matched,
                    "done": done,
                    "cancelled": cancelled,
                }),
            );
        }
    };
    'kinds: for kind in kinds {
        let Some(tl_filter) = search_filter_for(kind) else {
            continue;
        };
        let mut offset_id = filter.to_message_id.map(|v| v as i32 + 1).unwrap_or(0);
        loop {
            if cancel.is_cancelled() {
                emit(scanned, matched, true, true);
                return Ok(UpdateResult {
                    scanned,
                    coverage: coverage_of(&conn),
                    cancelled: true,
                });
            }
            let request = tl::functions::messages::Search {
                peer: peer.clone(),
                q: String::new(),
                from_id: None,
                saved_peer_id: None,
                saved_reaction: None,
                top_msg_id: None,
                filter: tl_filter.clone(),
                min_date: filter.from_date.map(|d| d as i32).unwrap_or(0),
                max_date: filter.to_date.map(|d| d as i32).unwrap_or(0),
                offset_id,
                add_offset: 0,
                limit: PAGE,
                max_id: 0,
                min_id,
                hash: 0,
            };
            let result = api::invoke_with_flood_wait(&client, &request)
                .await
                .map_err(|e| anyhow::anyhow!("messages.search: {}", e))?;
            let messages = messages_of(result);
            if messages.is_empty() {
                break;
            }
            let mut rows = Vec::new();
            let mut lowest = i32::MAX;
            for raw in &messages {
                if let tl::enums::Message::Message(m) = raw {
                    lowest = lowest.min(m.id);
                    if let Some(item) = history::to_item(m, fix_ext) {
                        let uid = m.media.as_ref().and_then(file_uid);
                        rows.push(Row { item, uid });
                    }
                }
            }
            scanned += messages.len();
            matched += rows.iter().filter(|r| filter.matches(&r.item)).count();
            upsert(&mut conn, &rows).map_err(|e| anyhow::anyhow!(e))?;
            emit(scanned, matched, false, false);
            let enough = limit.is_some_and(|l| matched >= l);
            let capped = opts.max_scan > 0 && scanned >= opts.max_scan;
            if enough || capped {
                break 'kinds;
            }
            if lowest == i32::MAX || lowest <= min_id + 1 || (messages.len() as i32) < PAGE {
                break;
            }
            offset_id = lowest;
        }
    }
    emit(scanned, matched, true, false);
    Ok(UpdateResult {
        scanned,
        coverage: coverage_of(&conn),
        cancelled: false,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn scan_filtered(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    filter: &MediaFilter,
    scan_id: &str,
    max_scan: usize,
    takeout: bool,
    after_id: i32,
) -> anyhow::Result<(Vec<(TelegramMediaItem, Option<String>)>, UpdateResult)> {
    let cancel = register_scan(scan_id);
    let opts = UpdateOpts {
        scan_id,
        max_scan,
        takeout,
        filter: Some(filter),
        stop_at_id: after_id.max(filter.from_message_id.unwrap_or(0) as i32),
        emit: true,
    };
    let kinds: Vec<String> = filter.media_types.clone().unwrap_or_default();
    let by_type = !kinds.is_empty()
        && kinds.iter().all(|k| search_filter_for(k).is_some())
        && !coverage(chat_id).complete;
    let res = if by_type {
        search_by_type(handle, chat_id, chat_type, &kinds, filter, &opts, &cancel).await
    } else {
        update(
            handle,
            chat_id,
            chat_type,
            &opts,
            &cancel,
            |cov, matched| needs_more_for(filter, cov, matched),
        )
        .await
    };
    unregister_scan(scan_id);
    let res = res?;
    if res.cancelled {
        return Err(anyhow::anyhow!("Scan cancelled"));
    }
    Ok((query_rows(chat_id, filter, after_id), res))
}
