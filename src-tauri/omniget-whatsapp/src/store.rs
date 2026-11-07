use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::model::{MessageKind, Summary};

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
            "DELETE FROM messages; DELETE FROM chats; DELETE FROM contacts; DELETE FROM aliases; DELETE FROM pending_ops; VACUUM;",
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

    pub fn names(&self) -> rusqlite::Result<Names> {
        let c = self.lock();
        names_on(&c)
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
        let needle = query
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());
        let mut out = Vec::new();
        for row in rows {
            let (jid, name, last_ts, unread, archived, kind, text, from_me, media, total) = row?;
            let display = name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| names.resolve(&jid, None));
            if let Some(q) = &needle {
                if !display.to_lowercase().contains(q.as_str()) && !jid.contains(q.as_str()) {
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
            });
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
        let mut stmt = c.prepare(
            "SELECT id, chat, from_me, sender, push_name, ts, kind, text, mime, size, file_name, duration, thumb, saved_path, edited, deleted
             FROM messages WHERE chat = ?1 AND ts < ?2 ORDER BY ts DESC, rowid DESC LIMIT ?3",
        )?;
        let before = before_ts.unwrap_or(i64::MAX);
        let mut out: Vec<MessageRow> = stmt
            .query_map(params![chat, before, limit as i64], |r| {
                let sender: Option<String> = r.get(3)?;
                let push: Option<String> = r.get(4)?;
                let from_me = r.get::<_, i64>(2)? != 0;
                let sender_name = if from_me {
                    None
                } else {
                    let who = sender.clone().unwrap_or_else(|| chat.to_string());
                    Some(names.resolve(&who, push.as_deref()))
                };
                Ok(MessageRow {
                    id: r.get(0)?,
                    chat: r.get(1)?,
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
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        out.reverse();
        Ok(out)
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
            "INSERT INTO messages(chat, id, from_me, sender, push_name, ts, kind, text, mime, size, file_name, duration, thumb, raw, wire_chat, wire_sender)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
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
            ],
        )?;
        if n > 0 {
            self.replay_pending(m.chat, m.id)?;
        }
        Ok(n > 0)
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
}
