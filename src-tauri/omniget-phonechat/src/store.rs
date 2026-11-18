use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::model::{MessageKind, Summary};
use crate::rules::{Limits, Rule};

const RUN_LOG_ROWS: i64 = 2000;
const LIMITS_KEY: &str = "rule_limits";
const WAIT_SECS: i64 = 24 * 3600;

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
CREATE TABLE IF NOT EXISTS chats (
  jid TEXT PRIMARY KEY,
  name TEXT,
  last_ts INTEGER,
  unread INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS contacts (
  jid TEXT PRIMARY KEY,
  full_name TEXT,
  push_name TEXT
);
CREATE TABLE IF NOT EXISTS aliases (
  lid TEXT PRIMARY KEY,
  pn TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
  chat TEXT NOT NULL,
  id TEXT NOT NULL,
  from_me INTEGER NOT NULL,
  sender TEXT,
  push_name TEXT,
  ts INTEGER NOT NULL,
  kind TEXT NOT NULL,
  text TEXT,
  mime TEXT,
  size INTEGER,
  file_name TEXT,
  duration INTEGER,
  thumb BLOB,
  raw BLOB,
  saved_path TEXT,
  PRIMARY KEY (chat, id)
);
CREATE INDEX IF NOT EXISTS messages_chat_ts ON messages(chat, ts);
CREATE TABLE IF NOT EXISTS scheduled (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  chat TEXT NOT NULL,
  text TEXT NOT NULL,
  reply_to TEXT,
  at INTEGER NOT NULL,
  created INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  error TEXT,
  sent_id TEXT,
  sent_at INTEGER
);
CREATE INDEX IF NOT EXISTS scheduled_due ON scheduled(status, at);
CREATE TABLE IF NOT EXISTS reactions (
  chat TEXT NOT NULL,
  id TEXT NOT NULL,
  sender TEXT NOT NULL,
  emoji TEXT NOT NULL,
  PRIMARY KEY (chat, id, sender)
);
CREATE TABLE IF NOT EXISTS contact_tags (
  chat TEXT NOT NULL,
  tag TEXT NOT NULL,
  PRIMARY KEY (chat, tag)
);
CREATE TABLE IF NOT EXISTS contact_fields (
  chat TEXT NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY (chat, key)
);
CREATE TABLE IF NOT EXISTS rule_waits (
  chat TEXT PRIMARY KEY,
  rule_id INTEGER NOT NULL,
  field TEXT NOT NULL,
  ts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS drafts (
  chat TEXT PRIMARY KEY,
  text TEXT NOT NULL,
  ts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS rules (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS rule_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  rule_id INTEGER NOT NULL,
  chat TEXT NOT NULL,
  ts INTEGER NOT NULL,
  outcome TEXT NOT NULL,
  detail TEXT,
  sent_id TEXT
);
CREATE INDEX IF NOT EXISTS rule_runs_ts ON rule_runs(ts);
CREATE TABLE IF NOT EXISTS rule_optouts (
  chat TEXT PRIMARY KEY,
  ts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pending_ops (
  chat TEXT NOT NULL,
  id TEXT NOT NULL,
  op TEXT NOT NULL,
  text TEXT,
  PRIMARY KEY (chat, id, op)
);
";

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ChatRow {
    pub jid: String,
    pub name: String,
    pub is_group: bool,
    pub last_ts: Option<i64>,
    pub unread: u32,
    pub archived: bool,
    pub last_kind: Option<MessageKind>,
    pub last_text: Option<String>,
    pub last_from_me: bool,
    pub media_count: u32,
    pub message_count: u32,
    pub tags: Vec<String>,
    pub fields: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessageRow {
    pub id: String,
    pub chat: String,
    pub from_me: bool,
    pub sender: Option<String>,
    pub sender_name: Option<String>,
    pub ts: i64,
    pub kind: MessageKind,
    pub text: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub file_name: Option<String>,
    pub duration: Option<u32>,
    pub thumb: Option<String>,
    pub saved_path: Option<String>,
    pub edited: bool,
    pub deleted: bool,
    pub seq: i64,
    pub quote_id: Option<String>,
    pub reactions: Vec<ReactionCount>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RuleRun {
    pub id: i64,
    pub rule_id: i64,
    pub chat: String,
    pub chat_name: String,
    pub ts: i64,
    pub outcome: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct RuleStat {
    pub rule_id: i64,
    pub sent: u32,
    pub skipped: u32,
    pub failed: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct OptOut {
    pub chat: String,
    pub name: String,
    pub ts: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ReactionCount {
    pub emoji: String,
    pub count: u32,
    pub mine: bool,
}

pub struct QuoteRef {
    pub id: String,
    pub from_me: bool,
    pub sender: Option<String>,
    pub wire_sender: Option<String>,
    pub text: Option<String>,
    pub raw: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Scheduled {
    pub id: i64,
    pub chat: String,
    pub text: String,
    pub reply_to: Option<String>,
    pub at: i64,
    pub created: i64,
    pub status: String,
    pub error: Option<String>,
    pub sent_id: Option<String>,
    pub sent_at: Option<i64>,
    pub rule_id: Option<i64>,
}

pub struct MediaRow {
    pub id: String,
    pub from_me: bool,
    pub wire_chat: Option<String>,
    pub wire_sender: Option<String>,
    pub ts: i64,
    pub kind: MessageKind,
    pub mime: Option<String>,
    pub file_name: Option<String>,
    pub size: Option<u64>,
    pub raw: Vec<u8>,
    pub saved_path: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct MediaStats {
    pub image: u32,
    pub video: u32,
    pub audio: u32,
    pub document: u32,
    pub sticker: u32,
    pub image_bytes: u64,
    pub video_bytes: u64,
    pub audio_bytes: u64,
    pub document_bytes: u64,
    pub sticker_bytes: u64,
    pub saved: u32,
}

pub struct NewMessage<'a> {
    pub chat: &'a str,
    pub id: &'a str,
    pub from_me: bool,
    pub sender: Option<&'a str>,
    pub push_name: Option<&'a str>,
    pub ts: i64,
    pub summary: &'a Summary,
    pub raw: Option<&'a [u8]>,
    pub wire_chat: Option<&'a str>,
    pub wire_sender: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReadTarget {
    pub id: String,
    pub wire_chat: String,
    pub wire_sender: Option<String>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const COLUMNS: &[(&str, &str)] = &[
    ("edited", "INTEGER NOT NULL DEFAULT 0"),
    ("deleted", "INTEGER NOT NULL DEFAULT 0"),
    ("wire_chat", "TEXT"),
    ("wire_sender", "TEXT"),
    ("live", "INTEGER NOT NULL DEFAULT 0"),
    ("quote_id", "TEXT"),
];

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    let mut have = Vec::new();
    {
        let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
        for name in stmt.query_map([], |r| r.get::<_, String>(1))? {
            have.push(name?);
        }
    }
    for (col, ty) in COLUMNS {
        if !have.iter().any(|h| h == col) {
            conn.execute_batch(&format!("ALTER TABLE messages ADD COLUMN {col} {ty};"))?;
        }
    }
    let has_rule: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('scheduled') WHERE name = 'rule_id')",
        [],
        |r| r.get(0),
    )?;
    if !has_rule {
        conn.execute_batch("ALTER TABLE scheduled ADD COLUMN rule_id INTEGER;")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS messages_live ON messages(live) WHERE live = 1;",
    )?;
    Ok(())
}

fn kind_of(s: String) -> MessageKind {
    MessageKind::parse(&s).unwrap_or(MessageKind::Text)
}

pub fn is_group(jid: &str) -> bool {
    jid.ends_with("@g.us")
}

pub fn phone_label(jid: &str) -> String {
    let user = jid.split('@').next().unwrap_or(jid);
    let user = user.split(':').next().unwrap_or(user);
    if jid.ends_with("@s.whatsapp.net") && user.chars().all(|c| c.is_ascii_digit()) {
        format!("+{user}")
    } else {
        user.to_string()
    }
}

fn data_url(thumb: Option<Vec<u8>>) -> Option<String> {
    use base64::Engine;
    thumb.filter(|t| !t.is_empty()).map(|t| {
        format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(t)
        )
    })
}

pub struct Names {
    contacts: HashMap<String, (Option<String>, Option<String>)>,
    aliases: HashMap<String, String>,
}

impl Names {
    pub fn resolve(&self, jid: &str, fallback_push: Option<&str>) -> String {
        let key = self.aliases.get(jid).map(String::as_str).unwrap_or(jid);
        for k in [key, jid] {
            if let Some((full, push)) = self.contacts.get(k) {
                if let Some(n) = full.as_deref().filter(|s| !s.is_empty()) {
                    return n.to_string();
                }
                if let Some(n) = push.as_deref().filter(|s| !s.is_empty()) {
                    return n.to_string();
                }
            }
        }
        if let Some(p) = fallback_push.filter(|s| !s.trim().is_empty()) {
            return p.to_string();
        }
        phone_label(key)
    }
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn wipe(&self) -> rusqlite::Result<()> {
        let c = self.lock();
        c.execute_batch(
            "DELETE FROM messages; DELETE FROM chats; DELETE FROM contacts; DELETE FROM aliases; DELETE FROM pending_ops; DELETE FROM reactions; DELETE FROM scheduled; DELETE FROM rule_runs; DELETE FROM rule_optouts; DELETE FROM contact_tags; DELETE FROM contact_fields; DELETE FROM rule_waits; DELETE FROM drafts; VACUUM;",
        )
    }

    pub fn with_tx<T>(
        &self,
        f: impl FnOnce(&Tx<'_>) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let mut c = self.lock();
        let tx = c.transaction()?;
        let out = f(&Tx { tx: &tx })?;
        tx.commit()?;
        Ok(out)
    }

    pub fn canonical(&self, jid: &str) -> String {
        let c = self.lock();
        canonical_on(&c, jid)
    }

    pub fn chats(&self, query: Option<&str>) -> rusqlite::Result<Vec<ChatRow>> {
        let c = self.lock();
        let names = names_on(&c)?;
        let mut stmt = c.prepare(
            "SELECT c.jid, c.name, c.last_ts, c.unread, c.archived,
                    (SELECT kind FROM messages m WHERE m.chat = c.jid ORDER BY ts DESC LIMIT 1),
                    (SELECT text FROM messages m WHERE m.chat = c.jid ORDER BY ts DESC LIMIT 1),
                    (SELECT from_me FROM messages m WHERE m.chat = c.jid ORDER BY ts DESC LIMIT 1),
                    (SELECT COUNT(*) FROM messages m WHERE m.chat = c.jid AND kind IN ('image','video','audio','voice','document','sticker')),
                    (SELECT COUNT(*) FROM messages m WHERE m.chat = c.jid)
             FROM chats c",
        )?;
        let rows = stmt.query_map([], |r| {
            let jid: String = r.get(0)?;
            let name: Option<String> = r.get(1)?;
            Ok((
                jid,
                name,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })?;
        let needle = query.map(|q| fold(q.trim())).filter(|q| !q.is_empty());
        let mut out = Vec::new();
        for row in rows {
            let (jid, name, last_ts, unread, archived, kind, text, from_me, media, total) = row?;
            let display = name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| names.resolve(&jid, None));
            if let Some(q) = &needle {
                if !fold(&display).contains(q.as_str()) && !jid.contains(q.as_str()) {
                    continue;
                }
            }
            out.push(ChatRow {
                is_group: is_group(&jid),
                jid,
                name: display,
                last_ts,
                unread: unread.max(0) as u32,
                archived: archived != 0,
                last_kind: kind.map(kind_of),
                last_text: text,
                last_from_me: from_me.unwrap_or(0) != 0,
                media_count: media.max(0) as u32,
                message_count: total.max(0) as u32,
                tags: Vec::new(),
                fields: Vec::new(),
            });
        }
        let mut tagged: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = c.prepare("SELECT chat, tag FROM contact_tags ORDER BY tag")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (chat, tag) = row?;
                tagged.entry(chat).or_default().push(tag);
            }
        }
        let mut filled: HashMap<String, Vec<(String, String)>> = HashMap::new();
        {
            let mut stmt = c.prepare("SELECT chat, key, value FROM contact_fields ORDER BY key")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (chat, key, value) = row?;
                filled.entry(chat).or_default().push((key, value));
            }
        }
        for chat in &mut out {
            if let Some(tags) = tagged.remove(&chat.jid) {
                chat.tags = tags;
            }
            if let Some(fields) = filled.remove(&chat.jid) {
                chat.fields = fields;
            }
        }
        out.sort_by(|a, b| {
            b.last_ts
                .unwrap_or(0)
                .cmp(&a.last_ts.unwrap_or(0))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(out)
    }

    pub fn chat_name(&self, jid: &str) -> rusqlite::Result<String> {
        let c = self.lock();
        let name: Option<String> = c
            .query_row("SELECT name FROM chats WHERE jid = ?1", [jid], |r| r.get(0))
            .optional()?
            .flatten();
        if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
            return Ok(n);
        }
        Ok(names_on(&c)?.resolve(jid, None))
    }

    pub fn messages(
        &self,
        chat: &str,
        before_ts: Option<i64>,
        limit: u32,
    ) -> rusqlite::Result<Vec<MessageRow>> {
        let c = self.lock();
        let names = names_on(&c)?;
        let mut stmt = c.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE chat = ?1 AND ts < ?2 ORDER BY ts DESC, rowid DESC LIMIT ?3"
        ))?;
        let before = before_ts.unwrap_or(i64::MAX);
        let mut out: Vec<MessageRow> = stmt
            .query_map(params![chat, before, limit as i64], |r| {
                message_row(r, &names)
            })?
            .collect::<rusqlite::Result<_>>()?;
        out.reverse();
        attach_reactions(&c, &mut out)?;
        Ok(out)
    }

    pub fn latest_seq(&self) -> rusqlite::Result<i64> {
        let c = self.lock();
        c.query_row("SELECT COALESCE(MAX(rowid), 0) FROM messages", [], |r| {
            r.get(0)
        })
    }

    pub fn live_after(
        &self,
        chat: Option<&str>,
        after: i64,
        include_mine: bool,
        limit: u32,
    ) -> rusqlite::Result<Vec<MessageRow>> {
        let c = self.lock();
        let names = names_on(&c)?;
        let mut stmt = c.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
             WHERE live = 1 AND rowid > ?1 AND (?2 IS NULL OR chat = ?2) AND (?3 = 1 OR from_me = 0)
             ORDER BY rowid ASC LIMIT ?4"
        ))?;
        let mut rows: Vec<MessageRow> = stmt
            .query_map(
                params![after, chat, include_mine as i64, limit as i64],
                |r| message_row(r, &names),
            )?
            .collect::<rusqlite::Result<_>>()?;
        attach_reactions(&c, &mut rows)?;
        Ok(rows)
    }

    pub fn quote_ref(&self, chat: &str, id: &str) -> rusqlite::Result<Option<QuoteRef>> {
        let c = self.lock();
        c.query_row(
            "SELECT from_me, sender, wire_sender, text, raw FROM messages WHERE chat = ?1 AND id = ?2",
            params![chat, id],
            |r| {
                Ok(QuoteRef {
                    id: id.to_string(),
                    from_me: r.get::<_, i64>(0)? != 0,
                    sender: r.get(1)?,
                    wire_sender: r.get(2)?,
                    text: r.get(3)?,
                    raw: r.get(4)?,
                })
            },
        )
        .optional()
    }

    pub fn contacts_without_chat(&self, query: &str) -> rusqlite::Result<Vec<(String, String)>> {
        let needle = fold(query.trim());
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let c = self.lock();
        let mut stmt = c.prepare(
            "SELECT jid, COALESCE(NULLIF(full_name, ''), push_name) FROM contacts
             WHERE jid LIKE '%@s.whatsapp.net' AND jid NOT IN (SELECT jid FROM chats)",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (jid, name) = row?;
            let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
                continue;
            };
            if fold(&name).contains(&needle) || jid.contains(&needle) {
                out.push((jid, name));
            }
        }
        out.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(out)
    }

    pub fn schedule_add(
        &self,
        chat: &str,
        text: &str,
        reply_to: Option<&str>,
        at: i64,
        now: i64,
    ) -> rusqlite::Result<Scheduled> {
        let c = self.lock();
        c.execute(
            "INSERT INTO scheduled(chat, text, reply_to, at, created) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![chat, text, reply_to, at, now],
        )?;
        let id = c.last_insert_rowid();
        scheduled_on(&c, id).map(|s| s.expect("just inserted"))
    }

    pub fn schedule_follow_up(
        &self,
        rule_id: i64,
        chat: &str,
        text: &str,
        at: i64,
        now: i64,
    ) -> rusqlite::Result<()> {
        self.lock().execute(
            "INSERT INTO scheduled(chat, text, at, created, rule_id) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![chat, text, at, now, rule_id],
        )?;
        Ok(())
    }

    pub fn cancel_follow_ups(&self, chat: &str) -> rusqlite::Result<usize> {
        self.lock().execute(
            "UPDATE scheduled SET status = 'cancelled'
             WHERE chat = ?1 AND status = 'pending' AND rule_id IS NOT NULL",
            [chat],
        )
    }

    pub fn tag_set(&self, chat: &str, tag: &str, on: bool) -> rusqlite::Result<bool> {
        let c = self.lock();
        let n = if on {
            c.execute(
                "INSERT INTO contact_tags(chat, tag) VALUES(?1, ?2) ON CONFLICT DO NOTHING",
                params![chat, tag],
            )?
        } else {
            c.execute(
                "DELETE FROM contact_tags WHERE chat = ?1 AND tag = ?2",
                params![chat, tag],
            )?
        };
        Ok(n > 0)
    }

    pub fn tags_of(&self, chat: &str) -> rusqlite::Result<Vec<String>> {
        let c = self.lock();
        let mut stmt = c.prepare("SELECT tag FROM contact_tags WHERE chat = ?1 ORDER BY tag")?;
        let rows = stmt
            .query_map([chat], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn field_set(&self, chat: &str, key: &str, value: &str) -> rusqlite::Result<()> {
        let c = self.lock();
        if value.is_empty() {
            c.execute(
                "DELETE FROM contact_fields WHERE chat = ?1 AND key = ?2",
                params![chat, key],
            )?;
        } else {
            c.execute(
                "INSERT INTO contact_fields(chat, key, value) VALUES(?1, ?2, ?3)
                 ON CONFLICT(chat, key) DO UPDATE SET value = excluded.value",
                params![chat, key, value],
            )?;
        }
        Ok(())
    }

    pub fn fields_of(&self, chat: &str) -> rusqlite::Result<Vec<(String, String)>> {
        let c = self.lock();
        let mut stmt =
            c.prepare("SELECT key, value FROM contact_fields WHERE chat = ?1 ORDER BY key")?;
        let rows = stmt
            .query_map([chat], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn wait_set(&self, chat: &str, rule_id: i64, field: &str, ts: i64) -> rusqlite::Result<()> {
        self.lock().execute(
            "INSERT INTO rule_waits(chat, rule_id, field, ts) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(chat) DO UPDATE SET rule_id = excluded.rule_id, field = excluded.field, ts = excluded.ts",
            params![chat, rule_id, field, ts],
        )?;
        Ok(())
    }

    pub fn wait_take(&self, chat: &str, now: i64) -> rusqlite::Result<Option<(i64, String)>> {
        let c = self.lock();
        let hit: Option<(i64, String, i64)> = c
            .query_row(
                "SELECT rule_id, field, ts FROM rule_waits WHERE chat = ?1",
                [chat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((rule_id, field, ts)) = hit else {
            return Ok(None);
        };
        c.execute("DELETE FROM rule_waits WHERE chat = ?1", [chat])?;
        Ok((now - ts <= WAIT_SECS).then_some((rule_id, field)))
    }

    pub fn draft_set(&self, chat: &str, text: &str, ts: i64) -> rusqlite::Result<()> {
        self.lock().execute(
            "INSERT INTO drafts(chat, text, ts) VALUES(?1, ?2, ?3)
             ON CONFLICT(chat) DO UPDATE SET text = excluded.text, ts = excluded.ts",
            params![chat, text, ts],
        )?;
        Ok(())
    }

    pub fn draft(&self, chat: &str) -> rusqlite::Result<Option<String>> {
        self.lock()
            .query_row("SELECT text FROM drafts WHERE chat = ?1", [chat], |r| {
                r.get(0)
            })
            .optional()
    }

    pub fn draft_clear(&self, chat: &str) -> rusqlite::Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM drafts WHERE chat = ?1", [chat])?
            > 0)
    }

    pub fn tag_counts(&self) -> rusqlite::Result<Vec<(String, u32)>> {
        let c = self.lock();
        let mut stmt =
            c.prepare("SELECT tag, COUNT(*) FROM contact_tags GROUP BY tag ORDER BY tag")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)?.max(0) as u32)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn schedule_get(&self, id: i64) -> rusqlite::Result<Option<Scheduled>> {
        scheduled_on(&self.lock(), id)
    }

    pub fn schedule_list(
        &self,
        include_done: bool,
        limit: u32,
    ) -> rusqlite::Result<Vec<Scheduled>> {
        let c = self.lock();
        let mut stmt = c.prepare(&format!(
            "SELECT {SCHEDULED_COLUMNS} FROM scheduled WHERE (?1 = 1 OR status = 'pending')
             ORDER BY CASE status WHEN 'pending' THEN 0 ELSE 1 END, CASE status WHEN 'pending' THEN at ELSE -at END LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![include_done as i64, limit as i64], scheduled_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn schedule_next(&self) -> rusqlite::Result<Option<Scheduled>> {
        let c = self.lock();
        c.query_row(
            &format!(
                "SELECT {SCHEDULED_COLUMNS} FROM scheduled WHERE status = 'pending' ORDER BY at ASC, id ASC LIMIT 1"
            ),
            [],
            scheduled_row,
        )
        .optional()
    }

    pub fn schedule_cancel(&self, id: i64) -> rusqlite::Result<bool> {
        let c = self.lock();
        let n = c.execute(
            "UPDATE scheduled SET status = 'cancelled' WHERE id = ?1 AND status = 'pending'",
            [id],
        )?;
        Ok(n > 0)
    }

    pub fn schedule_finish(
        &self,
        id: i64,
        status: &str,
        error: Option<&str>,
        sent_id: Option<&str>,
        sent_at: Option<i64>,
    ) -> rusqlite::Result<()> {
        let c = self.lock();
        c.execute(
            "UPDATE scheduled SET status = ?2, error = ?3, sent_id = ?4, sent_at = ?5 WHERE id = ?1 AND status = 'pending'",
            params![id, status, error, sent_id, sent_at],
        )?;
        Ok(())
    }

    pub fn oldest(&self, chat: &str) -> rusqlite::Result<Option<(String, bool, i64)>> {
        let c = self.lock();
        c.query_row(
            "SELECT id, from_me, ts FROM messages WHERE chat = ?1 ORDER BY ts ASC LIMIT 1",
            [chat],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?)),
        )
        .optional()
    }

    pub fn media_rows(&self, chat: &str, kinds: &[MessageKind]) -> rusqlite::Result<Vec<MediaRow>> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let c = self.lock();
        let list = kinds
            .iter()
            .map(|k| format!("'{}'", k.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, ts, kind, mime, file_name, size, raw, saved_path, from_me, wire_chat, wire_sender FROM messages
             WHERE chat = ?1 AND raw IS NOT NULL AND deleted = 0 AND kind IN ({list}) ORDER BY ts ASC"
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map([chat], |r| {
                Ok(MediaRow {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    kind: kind_of(r.get(2)?),
                    mime: r.get(3)?,
                    file_name: r.get(4)?,
                    size: r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
                    raw: r.get(6)?,
                    saved_path: r.get(7)?,
                    from_me: r.get::<_, i64>(8)? != 0,
                    wire_chat: r.get(9)?,
                    wire_sender: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn media_row(&self, chat: &str, id: &str) -> rusqlite::Result<Option<MediaRow>> {
        let c = self.lock();
        c.query_row(
            "SELECT id, ts, kind, mime, file_name, size, raw, saved_path, from_me, wire_chat, wire_sender FROM messages WHERE chat = ?1 AND id = ?2 AND raw IS NOT NULL AND deleted = 0",
            params![chat, id],
            |r| {
                Ok(MediaRow {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    kind: kind_of(r.get(2)?),
                    mime: r.get(3)?,
                    file_name: r.get(4)?,
                    size: r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
                    raw: r.get(6)?,
                    saved_path: r.get(7)?,
                    from_me: r.get::<_, i64>(8)? != 0,
                    wire_chat: r.get(9)?,
                    wire_sender: r.get(10)?,
                })
            },
        )
        .optional()
    }

    pub fn set_saved(&self, chat: &str, id: &str, path: &str) -> rusqlite::Result<()> {
        let c = self.lock();
        c.execute(
            "UPDATE messages SET saved_path = ?3 WHERE chat = ?1 AND id = ?2",
            params![chat, id, path],
        )?;
        Ok(())
    }

    pub fn media_stats(&self, chat: &str) -> rusqlite::Result<MediaStats> {
        let c = self.lock();
        let mut stmt = c.prepare(
            "SELECT kind, COUNT(*), COALESCE(SUM(size), 0), SUM(CASE WHEN saved_path IS NOT NULL THEN 1 ELSE 0 END)
             FROM messages WHERE chat = ?1 AND raw IS NOT NULL AND deleted = 0 GROUP BY kind",
        )?;
        let mut s = MediaStats::default();
        let rows = stmt.query_map([chat], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (kind, n, bytes, saved) = row?;
            let (n, bytes) = (n.max(0) as u32, bytes.max(0) as u64);
            match kind.as_str() {
                "image" => (s.image, s.image_bytes) = (s.image + n, s.image_bytes + bytes),
                "video" => (s.video, s.video_bytes) = (s.video + n, s.video_bytes + bytes),
                "audio" | "voice" => {
                    (s.audio, s.audio_bytes) = (s.audio + n, s.audio_bytes + bytes)
                }
                "document" => {
                    (s.document, s.document_bytes) = (s.document + n, s.document_bytes + bytes)
                }
                "sticker" => {
                    (s.sticker, s.sticker_bytes) = (s.sticker + n, s.sticker_bytes + bytes)
                }
                _ => continue,
            }
            s.saved += saved.max(0) as u32;
        }
        Ok(s)
    }

    pub fn limits(&self) -> rusqlite::Result<Limits> {
        let c = self.lock();
        let raw: Option<String> = c
            .query_row("SELECT value FROM kv WHERE key = ?1", [LIMITS_KEY], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(raw
            .and_then(|r| serde_json::from_str::<Limits>(&r).ok())
            .unwrap_or_default()
            .clamped())
    }

    pub fn set_limits(&self, limits: &Limits) -> rusqlite::Result<()> {
        let body = serde_json::to_string(limits).unwrap_or_default();
        self.lock().execute(
            "INSERT INTO kv(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![LIMITS_KEY, body],
        )?;
        Ok(())
    }

    pub fn rules(&self) -> rusqlite::Result<Vec<Rule>> {
        let c = self.lock();
        let mut stmt = c.prepare("SELECT id, body FROM rules ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, body)| {
                let mut rule: Rule = serde_json::from_str(&body).ok()?;
                rule.id = id;
                Some(rule)
            })
            .collect())
    }

    pub fn rule_save(&self, rule: &Rule) -> rusqlite::Result<Option<i64>> {
        let body = serde_json::to_string(rule).unwrap_or_default();
        let c = self.lock();
        if rule.id == 0 {
            c.execute("INSERT INTO rules(body) VALUES(?1)", [&body])?;
            return Ok(Some(c.last_insert_rowid()));
        }
        let n = c.execute(
            "UPDATE rules SET body = ?2 WHERE id = ?1",
            params![rule.id, body],
        )?;
        Ok((n > 0).then_some(rule.id))
    }

    pub fn rule_delete(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM rules WHERE id = ?1", [id])?
            > 0)
    }

    pub fn run_start(&self, rule_id: i64, chat: &str, ts: i64) -> rusqlite::Result<i64> {
        let c = self.lock();
        c.execute(
            "INSERT INTO rule_runs(rule_id, chat, ts, outcome) VALUES(?1, ?2, ?3, 'queued')",
            params![rule_id, chat, ts],
        )?;
        let id = c.last_insert_rowid();
        c.execute("DELETE FROM rule_runs WHERE id <= ?1", [id - RUN_LOG_ROWS])?;
        Ok(id)
    }

    pub fn run_finish(
        &self,
        id: i64,
        outcome: &str,
        detail: Option<&str>,
        sent_id: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.lock().execute(
            "UPDATE rule_runs SET outcome = ?2, detail = ?3, sent_id = ?4 WHERE id = ?1",
            params![id, outcome, detail, sent_id],
        )?;
        Ok(())
    }

    pub fn run_counts(&self, now: i64) -> rusqlite::Result<(u32, u32)> {
        let c = self.lock();
        let count = |since: i64| -> rusqlite::Result<u32> {
            c.query_row(
                "SELECT COUNT(*) FROM rule_runs WHERE ts > ?1 AND outcome IN ('queued', 'sent')",
                [since],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as u32)
        };
        Ok((count(now - 3600)?, count(now - 86_400)?))
    }

    pub fn last_runs(&self, chat: &str) -> rusqlite::Result<HashMap<i64, i64>> {
        let c = self.lock();
        let mut stmt = c.prepare(
            "SELECT rule_id, MAX(ts) FROM rule_runs
             WHERE chat = ?1 AND outcome IN ('queued', 'sent') GROUP BY rule_id",
        )?;
        let rows = stmt
            .query_map([chat], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn runs(&self, limit: u32) -> rusqlite::Result<Vec<RuleRun>> {
        let c = self.lock();
        let names = names_on(&c)?;
        let mut stmt = c.prepare(
            "SELECT r.id, r.rule_id, r.chat, r.ts, r.outcome, r.detail, c.name
             FROM rule_runs r LEFT JOIN chats c ON c.jid = r.chat ORDER BY r.id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                let chat: String = r.get(2)?;
                let stored: Option<String> = r.get(6)?;
                Ok(RuleRun {
                    id: r.get(0)?,
                    rule_id: r.get(1)?,
                    chat_name: stored
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| names.resolve(&chat, None)),
                    chat,
                    ts: r.get(3)?,
                    outcome: r.get(4)?,
                    detail: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn rule_stats(&self) -> rusqlite::Result<Vec<RuleStat>> {
        let c = self.lock();
        let mut stmt = c.prepare(
            "SELECT rule_id, SUM(outcome = 'sent'), SUM(outcome = 'skipped'), SUM(outcome = 'failed')
             FROM rule_runs GROUP BY rule_id ORDER BY rule_id",
        )?;
        let count = |r: &rusqlite::Row<'_>, i: usize| -> rusqlite::Result<u32> {
            Ok(r.get::<_, i64>(i)?.max(0) as u32)
        };
        let rows = stmt
            .query_map([], |r| {
                Ok(RuleStat {
                    rule_id: r.get(0)?,
                    sent: count(r, 1)?,
                    skipped: count(r, 2)?,
                    failed: count(r, 3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn opt_out(&self, chat: &str, ts: i64) -> rusqlite::Result<()> {
        self.lock().execute(
            "INSERT INTO rule_optouts(chat, ts) VALUES(?1, ?2) ON CONFLICT(chat) DO NOTHING",
            params![chat, ts],
        )?;
        Ok(())
    }

    pub fn opt_in(&self, chat: &str) -> rusqlite::Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM rule_optouts WHERE chat = ?1", [chat])?
            > 0)
    }

    pub fn opted_out(&self, chat: &str) -> rusqlite::Result<bool> {
        self.lock().query_row(
            "SELECT EXISTS(SELECT 1 FROM rule_optouts WHERE chat = ?1)",
            [chat],
            |r| r.get(0),
        )
    }

    pub fn opt_outs(&self) -> rusqlite::Result<Vec<OptOut>> {
        let c = self.lock();
        let names = names_on(&c)?;
        let mut stmt = c.prepare("SELECT chat, ts FROM rule_optouts ORDER BY ts DESC")?;
        let rows = stmt
            .query_map([], |r| {
                let chat: String = r.get(0)?;
                Ok(OptOut {
                    name: names.resolve(&chat, None),
                    chat,
                    ts: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn last_manual_ts(&self, chat: &str) -> rusqlite::Result<Option<i64>> {
        self.lock().query_row(
            "SELECT MAX(ts) FROM messages WHERE chat = ?1 AND from_me = 1
             AND id NOT IN (SELECT sent_id FROM rule_runs WHERE chat = ?1 AND sent_id IS NOT NULL)",
            [chat],
            |r| r.get(0),
        )
    }

    pub fn rule_sent(&self, chat: &str, id: &str) -> rusqlite::Result<bool> {
        self.lock().query_row(
            "SELECT EXISTS(SELECT 1 FROM rule_runs WHERE chat = ?1 AND sent_id = ?2)",
            params![chat, id],
            |r| r.get(0),
        )
    }

    pub fn inbound_count(&self, chat: &str) -> rusqlite::Result<u32> {
        self.lock()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE chat = ?1 AND from_me = 0",
                [chat],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as u32)
    }

    pub fn all_messages(&self, chat: &str) -> rusqlite::Result<Vec<MessageRow>> {
        self.messages(chat, None, u32::MAX >> 1)
    }

    pub fn set_raw(&self, chat: &str, id: &str, raw: &[u8]) -> rusqlite::Result<()> {
        let c = self.lock();
        c.execute(
            "UPDATE messages SET raw = ?3 WHERE chat = ?1 AND id = ?2",
            params![chat, id, raw],
        )?;
        Ok(())
    }

    pub fn read_targets(&self, chat: &str, limit: u32) -> rusqlite::Result<Vec<ReadTarget>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let c = self.lock();
        let mut stmt = c.prepare(
            "SELECT id, COALESCE(wire_chat, chat), wire_sender FROM messages
             WHERE chat = ?1 AND from_me = 0 AND deleted = 0 ORDER BY ts DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![chat, limit as i64], |r| {
                Ok(ReadTarget {
                    id: r.get(0)?,
                    wire_chat: r.get(1)?,
                    wire_sender: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn unread(&self, chat: &str) -> rusqlite::Result<u32> {
        let c = self.lock();
        let n: Option<i64> = c
            .query_row("SELECT unread FROM chats WHERE jid = ?1", [chat], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(n.unwrap_or(0).max(0) as u32)
    }

    pub fn mark_read(&self, chat: &str) -> rusqlite::Result<()> {
        let c = self.lock();
        c.execute("UPDATE chats SET unread = 0 WHERE jid = ?1", [chat])?;
        Ok(())
    }

    pub fn set_unread(&self, chat: &str, unread: u32) -> rusqlite::Result<()> {
        let c = self.lock();
        c.execute(
            "UPDATE chats SET unread = ?2 WHERE jid = ?1",
            params![chat, unread as i64],
        )?;
        Ok(())
    }

    pub fn set_archived(&self, chat: &str, archived: bool) -> rusqlite::Result<()> {
        let c = self.lock();
        c.execute(
            "UPDATE chats SET archived = ?2 WHERE jid = ?1",
            params![chat, archived as i64],
        )?;
        Ok(())
    }
}

const MESSAGE_COLUMNS: &str = "id, chat, from_me, sender, push_name, ts, kind, text, mime, size, file_name, duration, thumb, saved_path, edited, deleted, rowid, quote_id";

fn message_row(r: &rusqlite::Row<'_>, names: &Names) -> rusqlite::Result<MessageRow> {
    let chat: String = r.get(1)?;
    let sender: Option<String> = r.get(3)?;
    let push: Option<String> = r.get(4)?;
    let from_me = r.get::<_, i64>(2)? != 0;
    let sender_name = if from_me {
        None
    } else {
        let who = sender.clone().unwrap_or_else(|| chat.clone());
        Some(names.resolve(&who, push.as_deref()))
    };
    Ok(MessageRow {
        id: r.get(0)?,
        chat,
        from_me,
        sender,
        sender_name,
        ts: r.get(5)?,
        kind: kind_of(r.get(6)?),
        text: r.get(7)?,
        mime: r.get(8)?,
        size: r.get::<_, Option<i64>>(9)?.map(|v| v.max(0) as u64),
        file_name: r.get(10)?,
        duration: r.get::<_, Option<i64>>(11)?.map(|v| v.max(0) as u32),
        thumb: data_url(r.get(12)?),
        saved_path: r.get(13)?,
        edited: r.get::<_, i64>(14)? != 0,
        deleted: r.get::<_, i64>(15)? != 0,
        seq: r.get(16)?,
        quote_id: r.get(17)?,
        reactions: Vec::new(),
    })
}

fn attach_reactions(c: &Connection, rows: &mut [MessageRow]) -> rusqlite::Result<()> {
    let mut stmt = c.prepare_cached(
        "SELECT emoji, COUNT(*), MAX(sender = '') FROM reactions WHERE chat = ?1 AND id = ?2
         GROUP BY emoji ORDER BY MIN(rowid)",
    )?;
    for row in rows {
        row.reactions = stmt
            .query_map(params![row.chat, row.id], |r| {
                Ok(ReactionCount {
                    emoji: r.get(0)?,
                    count: r.get::<_, i64>(1)?.max(0) as u32,
                    mine: r.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(())
}

const SCHEDULED_COLUMNS: &str =
    "id, chat, text, reply_to, at, created, status, error, sent_id, sent_at, rule_id";

fn scheduled_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Scheduled> {
    Ok(Scheduled {
        id: r.get(0)?,
        chat: r.get(1)?,
        text: r.get(2)?,
        reply_to: r.get(3)?,
        at: r.get(4)?,
        created: r.get(5)?,
        status: r.get(6)?,
        error: r.get(7)?,
        sent_id: r.get(8)?,
        sent_at: r.get(9)?,
        rule_id: r.get(10)?,
    })
}

fn scheduled_on(c: &Connection, id: i64) -> rusqlite::Result<Option<Scheduled>> {
    c.query_row(
        &format!("SELECT {SCHEDULED_COLUMNS} FROM scheduled WHERE id = ?1"),
        [id],
        scheduled_row,
    )
    .optional()
}

pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

fn canonical_on(c: &Connection, jid: &str) -> String {
    if !jid.ends_with("@lid") {
        return jid.to_string();
    }
    c.query_row("SELECT pn FROM aliases WHERE lid = ?1", [jid], |r| {
        r.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
    .unwrap_or_else(|| jid.to_string())
}

fn names_on(c: &Connection) -> rusqlite::Result<Names> {
    let mut contacts = HashMap::new();
    let mut stmt = c.prepare("SELECT jid, full_name, push_name FROM contacts")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)))? {
        let (j, f, p) = row?;
        contacts.insert(j, (f, p));
    }
    let mut aliases = HashMap::new();
    let mut stmt = c.prepare("SELECT lid, pn FROM aliases")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (l, p) = row?;
        aliases.insert(l, p);
    }
    Ok(Names { contacts, aliases })
}

pub struct Tx<'a> {
    tx: &'a rusqlite::Transaction<'a>,
}

impl Tx<'_> {
    pub fn canonical(&self, jid: &str) -> String {
        canonical_on(self.tx, jid)
    }

    pub fn alias(&self, lid: &str, pn: &str) -> rusqlite::Result<()> {
        if lid.is_empty() || pn.is_empty() || !lid.ends_with("@lid") {
            return Ok(());
        }
        self.tx.execute(
            "INSERT INTO aliases(lid, pn) VALUES(?1, ?2) ON CONFLICT(lid) DO UPDATE SET pn = excluded.pn",
            params![lid, pn],
        )?;
        Ok(())
    }

    pub fn upsert_chat(
        &self,
        jid: &str,
        name: Option<&str>,
        last_ts: Option<i64>,
        unread: Option<u32>,
        archived: Option<bool>,
    ) -> rusqlite::Result<()> {
        self.tx.execute(
            "INSERT INTO chats(jid, name, last_ts, unread, archived) VALUES(?1, ?2, ?3, COALESCE(?4, 0), COALESCE(?5, 0))
             ON CONFLICT(jid) DO UPDATE SET
               name = COALESCE(excluded.name, chats.name),
               last_ts = MAX(COALESCE(chats.last_ts, 0), COALESCE(excluded.last_ts, 0)),
               unread = COALESCE(?4, chats.unread),
               archived = COALESCE(?5, chats.archived)",
            params![
                jid,
                name.filter(|n| !n.trim().is_empty()),
                last_ts,
                unread.map(|u| u as i64),
                archived.map(|a| a as i64)
            ],
        )?;
        Ok(())
    }

    pub fn bump_unread(&self, jid: &str) -> rusqlite::Result<()> {
        self.tx
            .execute("UPDATE chats SET unread = unread + 1 WHERE jid = ?1", [jid])?;
        Ok(())
    }

    pub fn contact(
        &self,
        jid: &str,
        full_name: Option<&str>,
        push_name: Option<&str>,
    ) -> rusqlite::Result<()> {
        let full = full_name.filter(|n| !n.trim().is_empty());
        let push = push_name.filter(|n| !n.trim().is_empty());
        if full.is_none() && push.is_none() {
            return Ok(());
        }
        self.tx.execute(
            "INSERT INTO contacts(jid, full_name, push_name) VALUES(?1, ?2, ?3)
             ON CONFLICT(jid) DO UPDATE SET
               full_name = COALESCE(excluded.full_name, contacts.full_name),
               push_name = COALESCE(excluded.push_name, contacts.push_name)",
            params![jid, full, push],
        )?;
        Ok(())
    }

    pub fn insert_message(&self, m: &NewMessage<'_>) -> rusqlite::Result<bool> {
        let Some(kind) = m.summary.kind else {
            return Ok(false);
        };
        let n = self.tx.execute(
            "INSERT INTO messages(chat, id, from_me, sender, push_name, ts, kind, text, mime, size, file_name, duration, thumb, raw, wire_chat, wire_sender, quote_id)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(chat, id) DO NOTHING",
            params![
                m.chat,
                m.id,
                m.from_me as i64,
                m.sender,
                m.push_name.filter(|p| !p.trim().is_empty()),
                m.ts,
                kind.as_str(),
                m.summary.text,
                m.summary.mime,
                m.summary.size.map(|s| s.min(i64::MAX as u64) as i64),
                m.summary.file_name,
                m.summary.duration.map(|d| d as i64),
                m.summary.thumb,
                if kind.media().is_some() { m.raw } else { None },
                m.wire_chat,
                m.wire_sender,
                m.summary.quote_id,
            ],
        )?;
        if n > 0 {
            self.replay_pending(m.chat, m.id)?;
        }
        Ok(n > 0)
    }

    pub fn set_reaction(
        &self,
        chat: &str,
        id: &str,
        sender: Option<&str>,
        emoji: &str,
    ) -> rusqlite::Result<bool> {
        let sender = sender.unwrap_or_default();
        let current: Option<String> = self
            .tx
            .query_row(
                "SELECT emoji FROM reactions WHERE chat = ?1 AND id = ?2 AND sender = ?3",
                params![chat, id, sender],
                |r| r.get(0),
            )
            .optional()?;
        if current.as_deref().unwrap_or_default() == emoji {
            return Ok(false);
        }
        self.tx.execute(
            "DELETE FROM reactions WHERE chat = ?1 AND id = ?2 AND sender = ?3",
            params![chat, id, sender],
        )?;
        if !emoji.is_empty() {
            self.tx.execute(
                "INSERT INTO reactions(chat, id, sender, emoji) VALUES(?1, ?2, ?3, ?4)",
                params![chat, id, sender, emoji],
            )?;
        }
        Ok(true)
    }

    pub fn mark_live(&self, chat: &str, id: &str) -> rusqlite::Result<()> {
        self.tx.execute(
            "UPDATE messages SET live = 1 WHERE chat = ?1 AND id = ?2",
            params![chat, id],
        )?;
        Ok(())
    }

    fn replay_pending(&self, chat: &str, id: &str) -> rusqlite::Result<()> {
        let ops: Vec<(String, Option<String>)> = {
            let mut stmt = self
                .tx
                .prepare("SELECT op, text FROM pending_ops WHERE chat = ?1 AND id = ?2")?;
            let rows = stmt
                .query_map(params![chat, id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        if ops.is_empty() {
            return Ok(());
        }
        self.tx.execute(
            "DELETE FROM pending_ops WHERE chat = ?1 AND id = ?2",
            params![chat, id],
        )?;
        let revoked = ops.iter().any(|(op, _)| op == "revoke");
        if let Some((_, text)) = ops.iter().find(|(op, _)| op == "edit") {
            self.apply_edit(chat, id, text.as_deref())?;
        }
        if revoked {
            self.apply_revoke(chat, id)?;
        }
        Ok(())
    }

    fn remember(&self, chat: &str, id: &str, op: &str, text: Option<&str>) -> rusqlite::Result<()> {
        self.tx.execute(
            "INSERT INTO pending_ops(chat, id, op, text) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(chat, id, op) DO UPDATE SET text = excluded.text",
            params![chat, id, op, text],
        )?;
        Ok(())
    }

    pub fn apply_edit(&self, chat: &str, id: &str, text: Option<&str>) -> rusqlite::Result<bool> {
        let n = self.tx.execute(
            "UPDATE messages SET text = COALESCE(?3, text), edited = 1 WHERE chat = ?1 AND id = ?2 AND deleted = 0",
            params![chat, id, text.filter(|t| !t.trim().is_empty())],
        )?;
        if n == 0 {
            self.remember(chat, id, "edit", text)?;
        }
        Ok(n > 0)
    }

    pub fn apply_revoke(&self, chat: &str, id: &str) -> rusqlite::Result<bool> {
        let n = self.tx.execute(
            "UPDATE messages SET text = NULL, thumb = NULL, raw = NULL, deleted = 1 WHERE chat = ?1 AND id = ?2",
            params![chat, id],
        )?;
        if n == 0 {
            self.remember(chat, id, "revoke", None)?;
        }
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Summary {
        Summary {
            kind: Some(MessageKind::Text),
            text: Some(t.into()),
            ..Default::default()
        }
    }

    #[test]
    fn chats_sort_resolve_names_and_search() {
        let s = Store::open_memory().unwrap();
        let photo = Summary {
            kind: Some(MessageKind::Image),
            size: Some(500),
            ..Default::default()
        };
        s.with_tx(|tx| {
            tx.upsert_chat("5511999@s.whatsapp.net", None, Some(100), Some(2), None)?;
            tx.upsert_chat("123@g.us", Some("Família"), Some(200), None, None)?;
            tx.upsert_chat("777@lid", None, Some(50), None, None)?;
            tx.alias("777@lid", "5521888@s.whatsapp.net")?;
            tx.contact("5521888@s.whatsapp.net", Some("Bia"), None)?;
            tx.insert_message(&NewMessage {
                chat: "123@g.us",
                id: "a",
                from_me: false,
                sender: Some("5511999@s.whatsapp.net"),
                push_name: Some("Ana"),
                ts: 200,
                summary: &photo,
                raw: Some(&[1, 2, 3]),
                wire_chat: None,
                wire_sender: None,
            })?;
            tx.insert_message(&NewMessage {
                chat: "5511999@s.whatsapp.net",
                id: "b",
                from_me: true,
                sender: None,
                push_name: None,
                ts: 100,
                summary: &text("oi"),
                raw: Some(&[9]),
                wire_chat: None,
                wire_sender: None,
            })?;
            Ok(())
        })
        .unwrap();
        let chats = s.chats(None).unwrap();
        assert_eq!(chats[0].name, "Família");
        assert!(chats[0].is_group);
        assert_eq!(chats[0].media_count, 1);
        assert_eq!(chats[1].name, "+5511999");
        assert_eq!(chats[1].unread, 2);
        assert!(chats[1].last_from_me);
        assert_eq!(chats[2].name, "Bia");
        assert_eq!(s.chats(Some("fam")).unwrap().len(), 1);
        assert_eq!(s.chats(Some("5511")).unwrap().len(), 1);
        let msgs = s.messages("123@g.us", None, 50).unwrap();
        assert_eq!(msgs[0].sender_name.as_deref(), Some("Ana"));
        let stats = s.media_stats("123@g.us").unwrap();
        assert_eq!((stats.image, stats.image_bytes), (1, 500));
        assert_eq!(
            s.media_stats("5511999@s.whatsapp.net").unwrap(),
            MediaStats::default()
        );
        assert_eq!(s.canonical("777@lid"), "5521888@s.whatsapp.net");
    }

    #[test]
    fn duplicate_messages_are_ignored_and_paging_is_chronological() {
        let s = Store::open_memory().unwrap();
        s.with_tx(|tx| {
            tx.upsert_chat("c@s.whatsapp.net", None, Some(3), None, None)?;
            for (i, ts) in [(1, 10), (2, 20), (3, 30)] {
                let id = i.to_string();
                let sum = text("x");
                assert!(tx.insert_message(&NewMessage {
                    chat: "c@s.whatsapp.net",
                    id: &id,
                    from_me: false,
                    sender: None,
                    push_name: None,
                    ts,
                    summary: &sum,
                    raw: None,
                    wire_chat: None,
                    wire_sender: None,
                })?);
            }
            let sum = text("dup");
            assert!(!tx.insert_message(&NewMessage {
                chat: "c@s.whatsapp.net",
                id: "1",
                from_me: false,
                sender: None,
                push_name: None,
                ts: 10,
                summary: &sum,
                raw: None,
                wire_chat: None,
                wire_sender: None,
            })?);
            Ok(())
        })
        .unwrap();
        let page = s.messages("c@s.whatsapp.net", None, 2).unwrap();
        assert_eq!(page.iter().map(|m| m.ts).collect::<Vec<_>>(), vec![20, 30]);
        let older = s.messages("c@s.whatsapp.net", Some(20), 2).unwrap();
        assert_eq!(older.iter().map(|m| m.ts).collect::<Vec<_>>(), vec![10]);
        assert_eq!(s.oldest("c@s.whatsapp.net").unwrap().unwrap().2, 10);
    }

    fn put(
        tx: &Tx<'_>,
        id: &str,
        ts: i64,
        summary: &Summary,
        from_me: bool,
    ) -> rusqlite::Result<bool> {
        tx.insert_message(&NewMessage {
            chat: "g@g.us",
            id,
            from_me,
            sender: Some("1@s.whatsapp.net"),
            push_name: None,
            ts,
            summary,
            raw: Some(&[7]),
            wire_chat: Some("g@g.us"),
            wire_sender: Some("1@lid"),
        })
    }

    #[test]
    fn edits_and_revokes_update_the_cache_even_out_of_order() {
        let s = Store::open_memory().unwrap();
        let photo = Summary {
            kind: Some(MessageKind::Image),
            text: Some("legenda".into()),
            thumb: Some(vec![1]),
            ..Default::default()
        };
        s.with_tx(|tx| {
            tx.upsert_chat("g@g.us", None, Some(1), None, None)?;
            put(tx, "a", 10, &text("oi"), false)?;
            put(tx, "b", 20, &photo, false)?;
            assert!(tx.apply_edit("g@g.us", "a", Some("oi, tudo bem?"))?);
            assert!(tx.apply_revoke("g@g.us", "b")?);
            assert!(!tx.apply_revoke("g@g.us", "c")?);
            assert!(!tx.apply_edit("g@g.us", "d", Some("novo"))?);
            put(tx, "c", 30, &text("vai sumir"), false)?;
            put(tx, "d", 40, &text("velho"), true)?;
            Ok(())
        })
        .unwrap();
        let rows = s.messages("g@g.us", None, 10).unwrap();
        let by = |id: &str| rows.iter().find(|m| m.id == id).unwrap().clone();
        assert_eq!(by("a").text.as_deref(), Some("oi, tudo bem?"));
        assert!(by("a").edited && !by("a").deleted);
        assert!(by("b").deleted);
        assert_eq!(by("b").text, None);
        assert_eq!(by("b").thumb, None);
        assert!(by("c").deleted);
        assert_eq!(by("d").text.as_deref(), Some("novo"));
        assert!(by("d").edited);
        assert!(s.media_row("g@g.us", "b").unwrap().is_none());
        assert_eq!(s.media_stats("g@g.us").unwrap().image, 0);
        s.with_tx(|tx| {
            assert!(!tx.apply_edit("g@g.us", "b", Some("x"))?);
            Ok(())
        })
        .unwrap();
        let targets = s.read_targets("g@g.us", 2).unwrap();
        assert_eq!(
            targets,
            vec![ReadTarget {
                id: "a".into(),
                wire_chat: "g@g.us".into(),
                wire_sender: Some("1@lid".into()),
            }]
        );
    }

    #[test]
    fn migration_adds_columns_to_an_old_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE messages (chat TEXT NOT NULL, id TEXT NOT NULL, from_me INTEGER NOT NULL, sender TEXT, push_name TEXT, ts INTEGER NOT NULL, kind TEXT NOT NULL, text TEXT, mime TEXT, size INTEGER, file_name TEXT, duration INTEGER, thumb BLOB, raw BLOB, saved_path TEXT, PRIMARY KEY (chat, id));
                 INSERT INTO messages(chat, id, from_me, ts, kind, text) VALUES('c', 'x', 0, 5, 'text', 'oi');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let rows = s.messages("c", None, 5).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].edited && !rows[0].deleted);
    }

    #[test]
    fn live_cursor_skips_history_and_own_messages() {
        let s = Store::open_memory().unwrap();
        let start = s.latest_seq().unwrap();
        let msg = |id: &'static str, from_me: bool, ts: i64| NewMessage {
            chat: "123@g.us",
            id,
            from_me,
            sender: (!from_me).then_some("5511999@s.whatsapp.net"),
            push_name: None,
            ts,
            summary: Box::leak(Box::new(text(id))),
            raw: None,
            wire_chat: None,
            wire_sender: None,
        };
        s.with_tx(|tx| {
            tx.insert_message(&msg("old", false, 10))?;
            tx.insert_message(&msg("new", false, 20))?;
            tx.mark_live("123@g.us", "new")?;
            tx.insert_message(&msg("mine", true, 21))?;
            tx.mark_live("123@g.us", "mine")
        })
        .unwrap();
        let theirs = s.live_after(None, start, false, 50).unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].id, "new");
        let all = s.live_after(Some("123@g.us"), start, true, 50).unwrap();
        assert_eq!(all.len(), 2);
        let after = s.live_after(None, all[1].seq, true, 50).unwrap();
        assert!(after.is_empty());
        assert!(s
            .live_after(Some("x@g.us"), start, true, 50)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn scheduled_sends_run_in_order_and_cancel() {
        let s = Store::open_memory().unwrap();
        let late = s
            .schedule_add("1@s.whatsapp.net", "b", None, 200, 1)
            .unwrap();
        let early = s
            .schedule_add("1@s.whatsapp.net", "a", Some("q"), 100, 1)
            .unwrap();
        assert_eq!(s.schedule_next().unwrap().unwrap().id, early.id);
        s.schedule_finish(early.id, "sent", None, Some("X"), Some(100))
            .unwrap();
        assert_eq!(s.schedule_next().unwrap().unwrap().id, late.id);
        assert!(s.schedule_cancel(late.id).unwrap());
        assert!(!s.schedule_cancel(late.id).unwrap());
        assert!(s.schedule_next().unwrap().is_none());
        assert!(s.schedule_list(false, 10).unwrap().is_empty());
        let all = s.schedule_list(true, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            s.schedule_get(early.id)
                .unwrap()
                .unwrap()
                .sent_id
                .as_deref(),
            Some("X")
        );
    }

    #[test]
    fn search_ignores_accents_and_finds_contacts_without_chat() {
        let s = Store::open_memory().unwrap();
        s.with_tx(|tx| {
            tx.upsert_chat("9@g.us", Some("Gajos de Lúla"), Some(5), None, None)?;
            tx.contact("553798079444@s.whatsapp.net", Some("ramon"), None)
        })
        .unwrap();
        assert_eq!(s.chats(Some("gajos de lula")).unwrap().len(), 1);
        let found = s.contacts_without_chat("RAMÓN").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(fold("Ação Ñ"), "acao n");
    }
    #[test]
    fn reactions_are_counted_replaced_and_removed() {
        let s = Store::open_memory().unwrap();
        s.with_tx(|tx| {
            put(tx, "m1", 10, &text("oi"), false)?;
            assert!(tx.set_reaction("g@g.us", "m1", Some("1@s.whatsapp.net"), "👍")?);
            assert!(tx.set_reaction("g@g.us", "m1", Some("2@s.whatsapp.net"), "👍")?);
            assert!(tx.set_reaction("g@g.us", "m1", None, "❤️")?);
            assert!(!tx.set_reaction("g@g.us", "m1", None, "❤️")?);
            Ok(())
        })
        .unwrap();
        let rows = s.messages("g@g.us", None, 10).unwrap();
        assert_eq!(
            rows[0].reactions,
            vec![
                ReactionCount {
                    emoji: "👍".into(),
                    count: 2,
                    mine: false
                },
                ReactionCount {
                    emoji: "❤️".into(),
                    count: 1,
                    mine: true
                },
            ]
        );
        s.with_tx(|tx| {
            assert!(tx.set_reaction("g@g.us", "m1", Some("1@s.whatsapp.net"), "😂")?);
            assert!(tx.set_reaction("g@g.us", "m1", None, "")?);
            assert!(!tx.set_reaction("g@g.us", "m1", None, "")?);
            Ok(())
        })
        .unwrap();
        let rows = s.messages("g@g.us", None, 10).unwrap();
        assert_eq!(
            rows[0].reactions,
            vec![
                ReactionCount {
                    emoji: "👍".into(),
                    count: 1,
                    mine: false
                },
                ReactionCount {
                    emoji: "😂".into(),
                    count: 1,
                    mine: false
                },
            ]
        );
        s.wipe().unwrap();
        s.with_tx(|tx| put(tx, "m1", 10, &text("oi"), false))
            .unwrap();
        assert!(s.messages("g@g.us", None, 10).unwrap()[0]
            .reactions
            .is_empty());
    }

    #[test]
    fn quoted_message_id_is_kept() {
        let s = Store::open_memory().unwrap();
        let reply = Summary {
            quote_id: Some("m1".into()),
            ..text("resposta")
        };
        s.with_tx(|tx| {
            put(tx, "m1", 10, &text("oi"), false)?;
            put(tx, "m2", 11, &reply, true)
        })
        .unwrap();
        let rows = s.messages("g@g.us", None, 10).unwrap();
        assert_eq!(rows[0].quote_id, None);
        assert_eq!(rows[1].quote_id.as_deref(), Some("m1"));
    }
    #[test]
    fn rules_limits_runs_and_opt_outs_round_trip() {
        use crate::rules::{Action, Match, Trigger};
        let s = Store::open_memory().unwrap();
        assert_eq!(s.limits().unwrap(), Limits::default());
        let on = Limits {
            enabled: true,
            per_hour: 5,
            ..Default::default()
        };
        s.set_limits(&on).unwrap();
        assert_eq!(s.limits().unwrap(), on.clone().clamped());

        let mut rule = Rule {
            id: 0,
            name: "preço".into(),
            enabled: true,
            trigger: Trigger::Keyword {
                mode: Match::Contains,
                words: vec!["preço".into()],
            },
            chats: Vec::new(),
            action: Action::Reply {
                text: "R$ 10".into(),
            },
            cooldown_minutes: 60,
            tag: None,
            follow_ups: Vec::new(),
            needs_tag: None,
            ask: None,
            own_messages: false,
        };
        let id = s.rule_save(&rule).unwrap().unwrap();
        rule.id = id;
        rule.enabled = false;
        assert_eq!(s.rule_save(&rule).unwrap(), Some(id));
        assert_eq!(s.rules().unwrap(), vec![rule.clone()]);
        rule.id = id + 9;
        assert_eq!(s.rule_save(&rule).unwrap(), None);

        let chat = "g@g.us";
        let run = s.run_start(id, chat, 1000).unwrap();
        assert_eq!(s.run_counts(1001).unwrap(), (1, 1));
        s.with_tx(|tx| put(tx, "auto", 1001, &text("R$ 10"), true))
            .unwrap();
        s.run_finish(run, "sent", None, Some("auto")).unwrap();
        assert_eq!(s.last_manual_ts(chat).unwrap(), None);
        s.with_tx(|tx| put(tx, "hand", 1002, &text("oi"), true))
            .unwrap();
        assert_eq!(s.last_manual_ts(chat).unwrap(), Some(1002));
        assert_eq!(s.last_runs(chat).unwrap(), HashMap::from([(id, 1000)]));
        let failed = s.run_start(id, chat, 1003).unwrap();
        s.run_finish(failed, "failed", Some("offline"), None)
            .unwrap();
        assert_eq!(s.run_counts(1004).unwrap(), (1, 1));
        assert_eq!(s.run_counts(1000 + 3601).unwrap(), (0, 1));
        assert_eq!(
            s.rule_stats().unwrap(),
            vec![RuleStat {
                rule_id: id,
                sent: 1,
                skipped: 0,
                failed: 1
            }]
        );
        let log = s.runs(10).unwrap();
        assert_eq!(log[0].outcome, "failed");
        assert_eq!(log[0].detail.as_deref(), Some("offline"));
        assert_eq!(log[1].outcome, "sent");
        s.with_tx(|tx| put(tx, "in", 1005, &text("oi"), false))
            .unwrap();
        assert_eq!(s.inbound_count(chat).unwrap(), 1);

        assert!(!s.opted_out(chat).unwrap());
        s.opt_out(chat, 5).unwrap();
        s.opt_out(chat, 6).unwrap();
        assert!(s.opted_out(chat).unwrap());
        assert_eq!(s.opt_outs().unwrap().len(), 1);
        assert!(s.opt_in(chat).unwrap());
        assert!(!s.opt_in(chat).unwrap());

        s.with_tx(|tx| tx.upsert_chat(chat, Some("Grupo"), Some(1), None, None))
            .unwrap();
        assert!(s.tag_set(chat, "cliente", true).unwrap());
        assert!(!s.tag_set(chat, "cliente", true).unwrap());
        assert!(s.tag_set(chat, "vip", true).unwrap());
        assert_eq!(s.chats(None).unwrap()[0].tags, vec!["cliente", "vip"]);
        assert_eq!(
            s.tag_counts().unwrap(),
            vec![("cliente".to_string(), 1), ("vip".to_string(), 1)]
        );
        assert!(s.tag_set(chat, "vip", false).unwrap());
        assert_eq!(s.tags_of(chat).unwrap(), vec!["cliente"]);
        s.field_set(chat, "cidade", "BH").unwrap();
        s.field_set(chat, "cidade", "Contagem").unwrap();
        s.field_set(chat, "bairro", "Centro").unwrap();
        s.field_set(chat, "bairro", "").unwrap();
        let fields = vec![("cidade".to_string(), "Contagem".to_string())];
        assert_eq!(s.fields_of(chat).unwrap(), fields);
        assert_eq!(s.chats(None).unwrap()[0].fields, fields);
        s.wait_set(chat, id, "cidade", 100).unwrap();
        assert_eq!(s.wait_take(chat, 200).unwrap(), Some((id, "cidade".into())));
        assert_eq!(s.wait_take(chat, 200).unwrap(), None);
        s.wait_set(chat, id, "cidade", 100).unwrap();
        assert_eq!(s.wait_take(chat, 100 + WAIT_SECS + 1).unwrap(), None);
        s.draft_set(chat, "um", 1).unwrap();
        s.draft_set(chat, "dois", 2).unwrap();
        assert_eq!(s.draft(chat).unwrap().as_deref(), Some("dois"));
        assert!(s.draft_clear(chat).unwrap());
        assert_eq!(s.draft(chat).unwrap(), None);
        s.schedule_follow_up(id, chat, "e aí?", 5000, 1000).unwrap();
        s.schedule_add(chat, "manual", None, 6000, 1000).unwrap();
        assert_eq!(s.schedule_next().unwrap().unwrap().rule_id, Some(id));
        assert_eq!(s.cancel_follow_ups(chat).unwrap(), 1);
        let next = s.schedule_next().unwrap().unwrap();
        assert_eq!((next.text.as_str(), next.rule_id), ("manual", None));

        assert!(s.rule_delete(id).unwrap());
        s.wipe().unwrap();
        assert!(s.tag_counts().unwrap().is_empty());
        assert!(s.runs(10).unwrap().is_empty());
        assert_eq!(s.limits().unwrap(), on.clamped());
    }
}
