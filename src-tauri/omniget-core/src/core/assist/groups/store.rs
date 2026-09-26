//! Tables of the groups module: conversation context, rooms, members,
//! messages with typed authorship, shares. Every function takes the database
//! explicitly so tests use their own file; the app passes `db::global()`.

use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::super::ctx::{ContextKind, Scope};
use super::super::db::AssistDb;
use super::super::{new_id, now_ms};
use super::{RoomLimits, ERR_GROUP, ERR_GROUP_NOT_FOUND, ROOM_PREFIX};

// ── conversation context (A04, A05) ──────────────────────────────────────

fn kind_str(k: ContextKind) -> &'static str {
    match k {
        ContextKind::Projectless => "projectless",
        ContextKind::Project => "project",
    }
}

/// Stored context of one conversation id (as the coordinator sees it).
/// Missing row = personal.
pub fn context_in(
    db: &AssistDb,
    conversation: &str,
) -> Result<(ContextKind, Option<PathBuf>), String> {
    let row: Option<(String, Option<String>)> = db.with(|c| {
        c.query_row(
            "SELECT kind, workspace FROM groups_conversations WHERE id = ?1",
            [conversation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
    })?;
    Ok(match row {
        Some((k, Some(ws))) if k == "project" => (ContextKind::Project, Some(PathBuf::from(ws))),
        _ => (ContextKind::Projectless, None),
    })
}

/// Sets the context of one conversation; `None` = personal. The path must
/// already be canonical (code_tools canonicalises before calling).
pub fn set_context_in(
    db: &AssistDb,
    conversation: &str,
    path: Option<&Path>,
) -> Result<(), String> {
    let kind = if path.is_some() {
        ContextKind::Project
    } else {
        ContextKind::Projectless
    };
    let ws = path.map(|p| p.to_string_lossy().to_string());
    db.with(|c| {
        c.execute(
            "INSERT INTO groups_conversations(id, kind, workspace, imported, updated_ms)
             VALUES (?1, ?2, ?3, 0, ?4)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, workspace = excluded.workspace,
                updated_ms = excluded.updated_ms, imported = 0",
            params![conversation, kind_str(kind), ws, now_ms()],
        )
    })?;
    Ok(())
}

/// Imports the legacy `workspaces.json` bindings: only conversations with no
/// row yet, so a choice made after the import is never overwritten.
pub fn import_legacy_in(
    db: &AssistDb,
    map: &std::collections::HashMap<String, PathBuf>,
) -> Result<usize, String> {
    db.tx(|tx| {
        let mut n = 0;
        for (conv, path) in map {
            n += tx
                .execute(
                    "INSERT OR IGNORE INTO groups_conversations(id, kind, workspace, imported, updated_ms)
                     VALUES (?1, 'project', ?2, 1, ?3)",
                    params![conv, path.to_string_lossy(), now_ms()],
                )
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        }
        Ok(n)
    })
}

// ── rooms ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Member {
    pub bot: String,
    /// Free text shown in the room ("Curador", "Pesquisador").
    #[serde(default)]
    pub role: String,
}

/// How a user message picks who answers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MentionPolicy {
    /// `@bot` answers alone; without a mention, the coordinator, else the
    /// room's default bot. Never everybody.
    #[default]
    MentionsOrCoordinator,
    /// Only explicit mentions start a run; a message without one is kept
    /// in the room and nobody answers.
    MentionsOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Room {
    pub id: String,
    pub title: String,
    pub members: Vec<Member>,
    pub coordinator: Option<String>,
    pub default_bot: Option<String>,
    pub mention_policy: MentionPolicy,
    pub limits: RoomLimits,
    pub round: i64,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoomDraft {
    pub title: String,
    pub members: Vec<Member>,
    #[serde(default)]
    pub coordinator: Option<String>,
    #[serde(default)]
    pub default_bot: Option<String>,
    #[serde(default)]
    pub mention_policy: MentionPolicy,
    #[serde(default)]
    pub limits: Option<RoomLimits>,
}

pub fn is_room_id(id: &str) -> bool {
    id.len() == ROOM_PREFIX.len() + 12
        && id.starts_with(ROOM_PREFIX)
        && id[ROOM_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit())
}

/// `"<room>~<bot>"` → `(room, bot)`. Also accepts `_` as the separator,
/// which is what a file-name sanitiser turns `~` into.
pub fn participant(conversation: &str) -> Option<(&str, &str)> {
    let cut = ROOM_PREFIX.len() + 12;
    if conversation.len() <= cut + 1 || !conversation.is_char_boundary(cut) {
        return None;
    }
    let (room, rest) = conversation.split_at(cut);
    let sep = rest.chars().next()?;
    if !is_room_id(room) || !(sep == '~' || sep == '_') {
        return None;
    }
    let bot = &rest[1..];
    (!bot.is_empty()).then_some((room, bot))
}

/// The session id of one member inside a room.
pub fn participant_id(room: &str, bot: &str) -> String {
    format!("{room}~{bot}")
}

/// The room a conversation id belongs to (itself or as a participant).
pub fn room_of(conversation: &str) -> Option<&str> {
    if is_room_id(conversation) {
        return Some(conversation);
    }
    participant(conversation).map(|(r, _)| r)
}

fn validate(draft: &RoomDraft) -> Result<(), String> {
    if draft.title.trim().is_empty() {
        return Err(format!("{ERR_GROUP}: the room needs a name"));
    }
    if draft.members.is_empty() {
        return Err(format!("{ERR_GROUP}: add at least one bot"));
    }
    let mut seen = std::collections::HashSet::new();
    for m in &draft.members {
        if m.bot.trim().is_empty() || m.bot.contains('~') {
            return Err(format!("{ERR_GROUP}: invalid bot id"));
        }
        if !seen.insert(m.bot.as_str()) {
            return Err(format!("{ERR_GROUP}: {} is in the room twice", m.bot));
        }
    }
    for who in [&draft.coordinator, &draft.default_bot]
        .into_iter()
        .flatten()
    {
        if !seen.contains(who.as_str()) {
            return Err(format!("{ERR_GROUP}: {who} is not a member of the room"));
        }
    }
    if let Some(l) = &draft.limits {
        l.validate()?;
    }
    Ok(())
}

pub fn create_room(db: &AssistDb, draft: &RoomDraft) -> Result<Room, String> {
    validate(draft)?;
    let id = format!(
        "{ROOM_PREFIX}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    );
    db.tx(|tx| create_room_tx(tx, &id, draft))?;
    get_room(db, &id)
}

/// Scoped callers attach ownership/idempotency in the same transaction.
pub(crate) fn create_room_tx(
    tx: &rusqlite::Transaction,
    id: &str,
    draft: &RoomDraft,
) -> Result<(), String> {
    validate(draft)?;
    if !is_room_id(id) {
        return Err(format!("{ERR_GROUP}: invalid room id"));
    }
    let limits = draft.limits.clone().unwrap_or_default();
    let now = now_ms();
    tx.execute(
            "INSERT INTO groups_rooms(id, title, coordinator, default_bot, mention_policy, limits, round, created_ms, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
            params![
                id,
                draft.title.trim(),
                draft.coordinator,
                draft.default_bot,
                serde_json::to_string(&draft.mention_policy).unwrap_or_default(),
                serde_json::to_string(&limits).unwrap_or_default(),
                now
            ],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    write_members(tx, &id, &draft.members)?;
    // A room is a conversation too: personal until a folder is picked.
    tx.execute(
        "INSERT OR IGNORE INTO groups_conversations(id, kind, workspace, imported, updated_ms)
             VALUES (?1, 'projectless', NULL, 0, ?2)",
        params![id, now],
    )
    .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    Ok(())
}

fn write_members(tx: &rusqlite::Transaction, room: &str, members: &[Member]) -> Result<(), String> {
    tx.execute("DELETE FROM groups_members WHERE room = ?1", [room])
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    for (i, m) in members.iter().enumerate() {
        tx.execute(
            "INSERT INTO groups_members(room, bot, role, position, added_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![room, m.bot, m.role, i as i64, now_ms()],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    }
    Ok(())
}

/// Replaces the room's settings. Removing a member cuts its future access
/// (it is no longer resolved into the room); what it already saw stays with
/// the provider it went to, and the UI says so.
pub fn update_room(db: &AssistDb, id: &str, draft: &RoomDraft) -> Result<Room, String> {
    validate(draft)?;
    let current = get_room(db, id)?;
    let limits = draft.limits.clone().unwrap_or(current.limits);
    db.tx(|tx| {
        tx.execute(
            "UPDATE groups_rooms SET title = ?2, coordinator = ?3, default_bot = ?4, mention_policy = ?5,
                limits = ?6, updated_ms = ?7 WHERE id = ?1",
            params![
                id,
                draft.title.trim(),
                draft.coordinator,
                draft.default_bot,
                serde_json::to_string(&draft.mention_policy).unwrap_or_default(),
                serde_json::to_string(&limits).unwrap_or_default(),
                now_ms()
            ],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        write_members(tx, id, &draft.members)?;
        Ok(())
    })?;
    get_room(db, id)
}

pub fn delete_room(db: &AssistDb, id: &str) -> Result<(), String> {
    db.tx(|tx| {
        for sql in [
            "DELETE FROM groups_shares WHERE room = ?1",
            "DELETE FROM groups_runs WHERE room = ?1",
            "DELETE FROM groups_tasks WHERE room = ?1",
            "DELETE FROM groups_messages WHERE room = ?1",
            "DELETE FROM groups_members WHERE room = ?1",
            "DELETE FROM groups_rooms WHERE id = ?1",
            "DELETE FROM groups_conversations WHERE id = ?1",
        ] {
            tx.execute(sql, [id])
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        }
        Ok(())
    })
}

fn room_from_row(r: &Row) -> rusqlite::Result<Room> {
    let policy: String = r.get(4)?;
    let limits: String = r.get(5)?;
    Ok(Room {
        id: r.get(0)?,
        title: r.get(1)?,
        members: Vec::new(),
        coordinator: r.get(2)?,
        default_bot: r.get(3)?,
        mention_policy: serde_json::from_str(&policy).unwrap_or_default(),
        limits: serde_json::from_str(&limits).unwrap_or_default(),
        round: r.get(6)?,
        created_ms: r.get(7)?,
        updated_ms: r.get(8)?,
    })
}

const ROOM_COLS: &str =
    "id, title, coordinator, default_bot, mention_policy, limits, round, created_ms, updated_ms";

fn members_of(db: &AssistDb, room: &str) -> Result<Vec<Member>, String> {
    db.with(|c| {
        let mut st =
            c.prepare("SELECT bot, role FROM groups_members WHERE room = ?1 ORDER BY position")?;
        let rows = st.query_map([room], |r| {
            Ok(Member {
                bot: r.get(0)?,
                role: r.get(1)?,
            })
        })?;
        rows.collect()
    })
}

pub fn get_room(db: &AssistDb, id: &str) -> Result<Room, String> {
    let room = db
        .with(|c| {
            c.query_row(
                &format!("SELECT {ROOM_COLS} FROM groups_rooms WHERE id = ?1"),
                [id],
                room_from_row,
            )
            .optional()
        })?
        .ok_or_else(|| format!("{ERR_GROUP_NOT_FOUND}: no room {id}"))?;
    Ok(Room {
        members: members_of(db, id)?,
        ..room
    })
}

pub fn list_rooms(db: &AssistDb) -> Result<Vec<Room>, String> {
    let rooms: Vec<Room> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {ROOM_COLS} FROM groups_rooms ORDER BY updated_ms DESC"
        ))?;
        let rows = st.query_map([], room_from_row)?;
        rows.collect()
    })?;
    rooms
        .into_iter()
        .map(|r| {
            let members = members_of(db, &r.id)?;
            Ok(Room { members, ..r })
        })
        .collect()
}

// ── messages ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    User,
    Bot,
    /// Notes the app writes into the room (a limit reached, a run cancelled).
    System,
}

impl AuthorKind {
    fn as_str(self) -> &'static str {
        match self {
            AuthorKind::User => "user",
            AuthorKind::Bot => "bot",
            AuthorKind::System => "system",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "bot" => AuthorKind::Bot,
            "system" => AuthorKind::System,
            _ => AuthorKind::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoomMessage {
    pub id: String,
    pub room: String,
    pub seq: i64,
    pub author: AuthorKind,
    pub bot_id: Option<String>,
    pub reply_to: Option<String>,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub round: i64,
    pub text: String,
    /// `done`, `failed`, `cancelled`, `interrupted`.
    pub status: String,
    pub created_ms: i64,
}

pub struct NewMessage<'a> {
    pub room: &'a str,
    pub author: AuthorKind,
    pub bot_id: Option<&'a str>,
    pub reply_to: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub round: i64,
    pub text: &'a str,
    pub status: &'a str,
}

pub fn insert_message_tx(
    tx: &rusqlite::Transaction,
    m: &NewMessage,
) -> Result<RoomMessage, String> {
    let id = new_id();
    let now = now_ms();
    let seq: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM groups_messages WHERE room = ?1",
            [m.room],
            |r| r.get(0),
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    tx.execute(
        "INSERT INTO groups_messages(id, room, seq, author_kind, bot_id, reply_to, run_id, task_id, round, text, status, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            m.room,
            seq,
            m.author.as_str(),
            m.bot_id,
            m.reply_to,
            m.run_id,
            m.task_id,
            m.round,
            m.text,
            m.status,
            now
        ],
    )
    .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    tx.execute(
        "UPDATE groups_rooms SET updated_ms = ?2 WHERE id = ?1",
        params![m.room, now],
    )
    .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    Ok(RoomMessage {
        id,
        room: m.room.to_string(),
        seq,
        author: m.author,
        bot_id: m.bot_id.map(str::to_string),
        reply_to: m.reply_to.map(str::to_string),
        run_id: m.run_id.map(str::to_string),
        task_id: m.task_id.map(str::to_string),
        round: m.round,
        text: m.text.to_string(),
        status: m.status.to_string(),
        created_ms: now,
    })
}

pub fn insert_message(db: &AssistDb, m: &NewMessage) -> Result<RoomMessage, String> {
    db.tx(|tx| insert_message_tx(tx, m))
}

fn message_from_row(r: &Row) -> rusqlite::Result<RoomMessage> {
    let author: String = r.get(3)?;
    Ok(RoomMessage {
        id: r.get(0)?,
        room: r.get(1)?,
        seq: r.get(2)?,
        author: AuthorKind::parse(&author),
        bot_id: r.get(4)?,
        reply_to: r.get(5)?,
        run_id: r.get(6)?,
        task_id: r.get(7)?,
        round: r.get(8)?,
        text: r.get(9)?,
        status: r.get(10)?,
        created_ms: r.get(11)?,
    })
}

const MSG_COLS: &str =
    "id, room, seq, author_kind, bot_id, reply_to, run_id, task_id, round, text, status, created_ms";

/// Every message of a room, oldest first (the UI's transcript).
pub fn messages(db: &AssistDb, room: &str) -> Result<Vec<RoomMessage>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {MSG_COLS} FROM groups_messages WHERE room = ?1 ORDER BY seq"
        ))?;
        let rows = st.query_map([room], message_from_row)?;
        rows.collect()
    })
}

/// The last `n` messages everybody in the room can see, oldest first: what
/// a member's turn receives as room context (never another member's private
/// session, never delegated work that is still running).
pub fn recent_visible(db: &AssistDb, room: &str, n: usize) -> Result<Vec<RoomMessage>, String> {
    let mut out: Vec<RoomMessage> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {MSG_COLS} FROM groups_messages WHERE room = ?1 AND status IN ('done','failed')
             ORDER BY seq DESC LIMIT ?2"
        ))?;
        let rows = st.query_map(params![room, n as i64], message_from_row)?;
        rows.collect()
    })?;
    out.reverse();
    Ok(out)
}

// ── shares (memory reaches a room only by an explicit user choice) ──────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Share {
    pub id: String,
    pub room: String,
    /// Scope key being shared (`user`, `bot:<id>`).
    pub scope: String,
    /// One memory record, or `None` for the whole scope.
    pub record_id: Option<String>,
    /// Who chose to share (always the local user; kept for provenance).
    pub shared_by: String,
    pub note: String,
    pub created_ms: i64,
}

/// Records an explicit share. Only the UI principal may call this (the
/// command layer); a bot tool never can, so a model cannot widen its scope.
pub fn add_share(
    db: &AssistDb,
    room: &str,
    scope: &Scope,
    record_id: Option<&str>,
    note: &str,
) -> Result<Share, String> {
    if matches!(scope, Scope::Room { .. }) {
        return Err(format!(
            "{ERR_GROUP}: a room scope cannot be shared into a room"
        ));
    }
    get_room(db, room)?;
    let share = Share {
        id: new_id(),
        room: room.to_string(),
        scope: scope.key(),
        record_id: record_id.map(str::to_string),
        shared_by: super::super::ctx::LOCAL_USER.to_string(),
        note: note.to_string(),
        created_ms: now_ms(),
    };
    db.with(|c| {
        c.execute(
            "INSERT INTO groups_shares(id, room, scope, record_id, shared_by, note, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                share.id,
                share.room,
                share.scope,
                share.record_id,
                share.shared_by,
                share.note,
                share.created_ms
            ],
        )
    })?;
    Ok(share)
}

pub fn revoke_share(db: &AssistDb, id: &str) -> Result<(), String> {
    db.with(|c| {
        c.execute(
            "UPDATE groups_shares SET revoked_ms = ?2 WHERE id = ?1 AND revoked_ms IS NULL",
            params![id, now_ms()],
        )
    })?;
    Ok(())
}

/// Active shares of a room (whole scopes and single records).
pub fn shares(db: &AssistDb, room: &str) -> Result<Vec<Share>, String> {
    db.with(|c| {
        let mut st = c.prepare(
            "SELECT id, room, scope, record_id, shared_by, note, created_ms FROM groups_shares
             WHERE room = ?1 AND revoked_ms IS NULL ORDER BY created_ms",
        )?;
        let rows = st.query_map([room], |r| {
            Ok(Share {
                id: r.get(0)?,
                room: r.get(1)?,
                scope: r.get(2)?,
                record_id: r.get(3)?,
                shared_by: r.get(4)?,
                note: r.get(5)?,
                created_ms: r.get(6)?,
            })
        })?;
        rows.collect()
    })
}
