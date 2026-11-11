use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use super::rules::Slot;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schedule_items(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  platform TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  fp TEXT NOT NULL,
  due_ms INTEGER,
  status TEXT NOT NULL,
  forced INTEGER NOT NULL DEFAULT 0,
  result TEXT,
  post_id TEXT,
  post_url TEXT,
  error TEXT,
  created_by TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  sent_ms INTEGER
);
CREATE INDEX IF NOT EXISTS schedule_items_due ON schedule_items(status, due_ms);
CREATE TABLE IF NOT EXISTS schedule_slots(
  platform TEXT NOT NULL,
  weekday INTEGER NOT NULL,
  minute INTEGER NOT NULL,
  PRIMARY KEY(platform, weekday, minute)
);
CREATE TABLE IF NOT EXISTS schedule_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS schedule_feeds(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  platform TEXT NOT NULL,
  url TEXT NOT NULL,
  template TEXT NOT NULL,
  interval_min INTEGER NOT NULL,
  monthly_cap INTEGER NOT NULL,
  primed INTEGER NOT NULL DEFAULT 0,
  seen TEXT NOT NULL DEFAULT '[]',
  month TEXT NOT NULL DEFAULT '',
  month_count INTEGER NOT NULL DEFAULT 0,
  skipped INTEGER NOT NULL DEFAULT 0,
  last_checked_ms INTEGER,
  last_error TEXT,
  created_ms INTEGER NOT NULL,
  UNIQUE(platform, url)
);
";

pub const STATUSES: [&str; 7] = [
    "draft",
    "scheduled",
    "sending",
    "sent",
    "failed",
    "missed",
    "cancelled",
];
pub const DEFAULT_GRACE_MIN: i64 = 10;

#[derive(Debug, Clone)]
pub struct Item {
    pub id: i64,
    pub platform: String,
    pub kind: String,
    pub payload: Value,
    pub due_ms: Option<i64>,
    pub status: String,
    pub forced: bool,
    pub result: Value,
    pub post_id: Option<String>,
    pub post_url: Option<String>,
    pub error: Option<String>,
    pub created_by: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub sent_ms: Option<i64>,
}

const COLS: &str = "id, platform, kind, payload, due_ms, status, forced, result, post_id, post_url, error, created_by, created_ms, updated_ms, sent_ms";

fn item(r: &Row) -> rusqlite::Result<Item> {
    let payload: String = r.get(3)?;
    let result: Option<String> = r.get(7)?;
    Ok(Item {
        id: r.get(0)?,
        platform: r.get(1)?,
        kind: r.get(2)?,
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        due_ms: r.get(4)?,
        status: r.get(5)?,
        forced: r.get::<_, i64>(6)? != 0,
        result: result
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null),
        post_id: r.get(8)?,
        post_url: r.get(9)?,
        error: r.get(10)?,
        created_by: r.get(11)?,
        created_ms: r.get(12)?,
        updated_ms: r.get(13)?,
        sent_ms: r.get(14)?,
    })
}

pub struct NewItem<'a> {
    pub platform: &'a str,
    pub kind: &'a str,
    pub payload: &'a Value,
    pub fp: &'a str,
    pub due_ms: Option<i64>,
    pub status: &'a str,
    pub by: &'a str,
}

pub fn insert(c: &Connection, n: &NewItem, now: i64) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO schedule_items(platform, kind, payload, fp, due_ms, status, created_by, created_ms, updated_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",
        params![n.platform, n.kind, n.payload.to_string(), n.fp, n.due_ms, n.status, n.by, now],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn get(c: &Connection, platform: &str, id: i64) -> rusqlite::Result<Option<Item>> {
    c.query_row(
        &format!("SELECT {COLS} FROM schedule_items WHERE id=?1 AND platform=?2"),
        params![id, platform],
        item,
    )
    .optional()
}

pub fn replace(
    c: &Connection,
    id: i64,
    kind: &str,
    payload: &Value,
    fp: &str,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE schedule_items SET kind=?2, payload=?3, fp=?4, updated_ms=?5 WHERE id=?1",
        params![id, kind, payload.to_string(), fp, now],
    )?;
    Ok(())
}

pub fn reschedule(
    c: &Connection,
    id: i64,
    due_ms: i64,
    forced: bool,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE schedule_items SET status='scheduled', due_ms=?2, forced=?3, error=NULL, updated_ms=?4 WHERE id=?1",
        params![id, due_ms, forced as i64, now],
    )?;
    Ok(())
}

pub fn cancel(c: &Connection, id: i64, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE schedule_items SET status='cancelled', updated_ms=?2 WHERE id=?1",
        params![id, now],
    )?;
    Ok(())
}

pub fn claim(c: &Connection, id: i64, now: i64) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "UPDATE schedule_items SET status='sending', updated_ms=?2 WHERE id=?1 AND status='scheduled'",
        params![id, now],
    )? == 1)
}

pub fn progress(c: &Connection, id: i64, result: &Value, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE schedule_items SET result=?2, updated_ms=?3 WHERE id=?1",
        params![id, result.to_string(), now],
    )?;
    Ok(())
}

pub struct Finish<'a> {
    pub status: &'a str,
    pub result: &'a Value,
    pub post_id: Option<&'a str>,
    pub post_url: Option<&'a str>,
    pub error: Option<&'a str>,
}

pub fn finish(c: &Connection, id: i64, f: &Finish, now: i64) -> rusqlite::Result<()> {
    let sent = (f.status == "sent").then_some(now);
    c.execute(
        "UPDATE schedule_items SET status=?2, result=?3, post_id=?4, post_url=?5, error=?6, sent_ms=?7, forced=0, updated_ms=?8 WHERE id=?1",
        params![id, f.status, f.result.to_string(), f.post_id, f.post_url, f.error, sent, now],
    )?;
    Ok(())
}

pub fn mark(c: &Connection, id: i64, status: &str, error: &str, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE schedule_items SET status=?2, error=?3, forced=0, updated_ms=?4 WHERE id=?1",
        params![id, status, error, now],
    )?;
    Ok(())
}

pub fn recover_interrupted(c: &Connection, now: i64) -> rusqlite::Result<usize> {
    c.execute(
        "UPDATE schedule_items SET status='failed', forced=0, updated_ms=?1, error='INTERRUPTED: {\"message\":\"OmniGet closed while this was being sent\",\"retryable\":false,\"next_step\":\"check the account to see what was published, then use send_now to continue or cancel\"}' WHERE status='sending'",
        params![now],
    )
}

pub fn next_due(c: &Connection) -> rusqlite::Result<Option<Item>> {
    c.query_row(
        &format!("SELECT {COLS} FROM schedule_items WHERE status='scheduled' AND due_ms IS NOT NULL ORDER BY due_ms, id LIMIT 1"),
        [],
        item,
    )
    .optional()
}

pub struct Filter<'a> {
    pub platform: &'a str,
    pub statuses: &'a [String],
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub id: Option<i64>,
    pub newest_first: bool,
    pub limit: usize,
}

const WHEN: &str = "COALESCE(sent_ms, due_ms, updated_ms)";

pub fn list(c: &Connection, f: &Filter) -> rusqlite::Result<(Vec<Item>, usize)> {
    let marks = if f.statuses.is_empty() {
        "'draft','scheduled','sending','sent','failed','missed'".to_string()
    } else {
        f.statuses
            .iter()
            .filter(|s| STATUSES.contains(&s.as_str()))
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let filter = format!(
        "platform=?1 AND status IN ({marks}) AND (?2 IS NULL OR {WHEN} >= ?2) AND (?3 IS NULL OR {WHEN} <= ?3) AND (?4 IS NULL OR id=?4)"
    );
    let total: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM schedule_items WHERE {filter}"),
        params![f.platform, f.from_ms, f.to_ms, f.id],
        |r| r.get(0),
    )?;
    let order = if f.newest_first { "DESC" } else { "ASC" };
    let mut st = c.prepare(&format!(
        "SELECT {COLS} FROM schedule_items WHERE {filter} ORDER BY {WHEN} {order}, id {order} LIMIT {}",
        f.limit
    ))?;
    let items = st
        .query_map(params![f.platform, f.from_ms, f.to_ms, f.id], item)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((items, total as usize))
}

pub fn counts(c: &Connection, platform: &str) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut st =
        c.prepare("SELECT status, COUNT(*) FROM schedule_items WHERE platform=?1 GROUP BY status")?;
    let rows = st.query_map(params![platform], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

pub fn taken(c: &Connection, platform: &str) -> rusqlite::Result<HashSet<i64>> {
    let mut st = c.prepare(
        "SELECT due_ms FROM schedule_items WHERE platform=?1 AND status IN ('scheduled','sending') AND due_ms IS NOT NULL",
    )?;
    let rows = st.query_map(params![platform], |r| r.get(0))?;
    rows.collect()
}

pub fn duplicate(
    c: &Connection,
    platform: &str,
    fp: &str,
    since_ms: i64,
    except: Option<i64>,
) -> rusqlite::Result<Option<i64>> {
    c.query_row(
        "SELECT id FROM schedule_items WHERE platform=?1 AND fp=?2 AND (?4 IS NULL OR id<>?4) AND (status IN ('scheduled','sending') OR (status='sent' AND sent_ms>?3)) LIMIT 1",
        params![platform, fp, since_ms, except],
        |r| r.get(0),
    )
    .optional()
}

pub fn slots(c: &Connection, platform: &str) -> rusqlite::Result<Vec<Slot>> {
    let mut st = c.prepare(
        "SELECT weekday, minute FROM schedule_slots WHERE platform=?1 ORDER BY weekday, minute",
    )?;
    let rows = st.query_map(params![platform], |r| {
        Ok(Slot {
            weekday: r.get::<_, i64>(0)? as u8,
            minute: r.get::<_, i64>(1)? as u16,
        })
    })?;
    rows.collect()
}

pub fn set_slots(c: &mut Connection, platform: &str, slots: &[Slot]) -> rusqlite::Result<()> {
    let tx = c.transaction()?;
    tx.execute(
        "DELETE FROM schedule_slots WHERE platform=?1",
        params![platform],
    )?;
    for s in slots {
        tx.execute(
            "INSERT OR IGNORE INTO schedule_slots(platform, weekday, minute) VALUES(?1,?2,?3)",
            params![platform, s.weekday, s.minute],
        )?;
    }
    tx.commit()
}

pub fn setting(c: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT value FROM schedule_settings WHERE key=?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

pub fn set_setting(c: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO schedule_settings(key, value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn grace_min(c: &Connection, platform: &str) -> i64 {
    setting(c, &format!("grace_minutes:{platform}"))
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_GRACE_MIN)
}

pub fn long_posts(c: &Connection, platform: &str) -> String {
    setting(c, &format!("long_posts:{platform}"))
        .ok()
        .flatten()
        .filter(|v| v == "on" || v == "off")
        .unwrap_or_else(|| "auto".into())
}

#[derive(Debug, Clone)]
pub struct Feed {
    pub id: i64,
    pub platform: String,
    pub url: String,
    pub template: String,
    pub interval_min: i64,
    pub monthly_cap: i64,
    pub primed: bool,
    pub seen: Vec<String>,
    pub month: String,
    pub month_count: i64,
    pub skipped: i64,
    pub last_checked_ms: Option<i64>,
    pub last_error: Option<String>,
}

const FEED_COLS: &str = "id, platform, url, template, interval_min, monthly_cap, primed, seen, month, month_count, skipped, last_checked_ms, last_error";

fn feed(r: &Row) -> rusqlite::Result<Feed> {
    let seen: String = r.get(7)?;
    Ok(Feed {
        id: r.get(0)?,
        platform: r.get(1)?,
        url: r.get(2)?,
        template: r.get(3)?,
        interval_min: r.get(4)?,
        monthly_cap: r.get(5)?,
        primed: r.get::<_, i64>(6)? != 0,
        seen: serde_json::from_str(&seen).unwrap_or_default(),
        month: r.get(8)?,
        month_count: r.get(9)?,
        skipped: r.get(10)?,
        last_checked_ms: r.get(11)?,
        last_error: r.get(12)?,
    })
}

pub fn feeds(c: &Connection, platform: Option<&str>) -> rusqlite::Result<Vec<Feed>> {
    let mut st = c.prepare(&format!(
        "SELECT {FEED_COLS} FROM schedule_feeds WHERE (?1 IS NULL OR platform=?1) ORDER BY id"
    ))?;
    let rows = st.query_map(params![platform], feed)?;
    rows.collect()
}

pub fn feed_get(c: &Connection, id: i64) -> rusqlite::Result<Option<Feed>> {
    c.query_row(
        &format!("SELECT {FEED_COLS} FROM schedule_feeds WHERE id=?1"),
        params![id],
        feed,
    )
    .optional()
}

pub fn feed_add(
    c: &Connection,
    platform: &str,
    url: &str,
    template: &str,
    interval_min: i64,
    monthly_cap: i64,
    now: i64,
) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO schedule_feeds(platform, url, template, interval_min, monthly_cap, created_ms) VALUES(?1,?2,?3,?4,?5,?6)",
        params![platform, url, template, interval_min, monthly_cap, now],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn feed_remove(c: &Connection, platform: &str, id: i64) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "DELETE FROM schedule_feeds WHERE id=?1 AND platform=?2",
        params![id, platform],
    )? == 1)
}

pub fn feed_checked(
    c: &Connection,
    f: &Feed,
    error: Option<&str>,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE schedule_feeds SET primed=?2, seen=?3, month=?4, month_count=?5, skipped=?6, last_checked_ms=?7, last_error=?8 WHERE id=?1",
        params![
            f.id,
            f.primed as i64,
            serde_json::to_string(&f.seen).unwrap_or_else(|_| "[]".into()),
            f.month,
            f.month_count,
            f.skipped,
            now,
            error
        ],
    )?;
    Ok(())
}
