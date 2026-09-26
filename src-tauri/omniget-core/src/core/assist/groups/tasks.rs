//! Who answers, delegated tasks and the limits that bound them.
//!
//! - A user message is routed once: explicit `@mentions` → only those
//!   members; none → the coordinator, else the room's default bot. A bot's
//!   message is never routed, so bots cannot trigger each other in a loop.
//! - A member hands work to another only through `delegate_task`. The task
//!   is a row with a single, transactional claim; the child runs in its own
//!   session (`"<room>~<bot>"`) and its result stays in the room with its
//!   author, even when the coordinator later writes a summary.
//! - Depth, delegations and turns per round, concurrency, time and tokens are
//!   checked here, in the backend, whatever the prompt says (B03).
//! - Cancelling the room (or a parent run/task) cancels what is still active
//!   and keeps what already finished (B05).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::super::db::AssistDb;
use super::super::{new_id, now_ms};
use super::store::{self, AuthorKind, MentionPolicy, NewMessage, Room, RoomMessage};
use super::{ERR_GROUP, ERR_GROUP_BUSY, ERR_GROUP_CLAIMED, ERR_GROUP_LIMIT, ERR_GROUP_NOT_FOUND};

// ── limits ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RoomLimits {
    /// A task created by a task created by… stops here (1 = only direct
    /// delegations from a turn the user started).
    pub max_depth: u32,
    /// Delegated tasks per round (one round = one user message).
    pub max_delegations_per_round: u32,
    /// Turns (user-routed and delegated) per round.
    pub max_turns_per_round: u32,
    /// Runs of this room at the same time.
    pub max_concurrency: u32,
    /// Wall time of one delegated task.
    pub max_task_ms: u64,
    /// Tokens (input + output, as reported) per round; `None` = no cap.
    /// Unknown usage is not counted as zero: a run that reported nothing
    /// counts `unknown_run_tokens`.
    pub max_tokens_per_round: Option<u64>,
    pub unknown_run_tokens: u64,
}

impl Default for RoomLimits {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_delegations_per_round: 4,
            max_turns_per_round: 8,
            max_concurrency: 3,
            max_task_ms: 180_000,
            max_tokens_per_round: None,
            unknown_run_tokens: 4_000,
        }
    }
}

impl RoomLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_concurrency == 0 || self.max_turns_per_round == 0 {
            return Err(format!("{ERR_GROUP}: limits must allow at least one turn"));
        }
        if self.max_depth > 5 || self.max_delegations_per_round > 32 || self.max_concurrency > 8 {
            return Err(format!("{ERR_GROUP}: limits above the app's ceiling"));
        }
        if self.max_task_ms < 5_000 || self.max_task_ms > 3_600_000 {
            return Err(format!(
                "{ERR_GROUP}: task time must be between 5 s and 1 h"
            ));
        }
        Ok(())
    }
}

// ── runs of a room ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoomRun {
    pub run_id: String,
    pub room: String,
    pub bot: String,
    pub task_id: Option<String>,
    pub reply_to: Option<String>,
    pub round: i64,
    /// `running`, `completed`, `failed`, `cancelled`, `interrupted`.
    pub state: String,
    pub tokens: Option<i64>,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
}

fn run_from_row(r: &Row) -> rusqlite::Result<RoomRun> {
    Ok(RoomRun {
        run_id: r.get(0)?,
        room: r.get(1)?,
        bot: r.get(2)?,
        task_id: r.get(3)?,
        reply_to: r.get(4)?,
        round: r.get(5)?,
        state: r.get(6)?,
        tokens: r.get(7)?,
        started_ms: r.get(8)?,
        finished_ms: r.get(9)?,
    })
}

const RUN_COLS: &str =
    "run_id, room, bot, task_id, reply_to, round, state, tokens, started_ms, finished_ms";

pub fn runs(db: &AssistDb, room: &str) -> Result<Vec<RoomRun>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {RUN_COLS} FROM groups_runs WHERE room = ?1 ORDER BY started_ms"
        ))?;
        let rows = st.query_map([room], run_from_row)?;
        rows.collect()
    })
}

pub fn run(db: &AssistDb, run_id: &str) -> Result<Option<RoomRun>, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {RUN_COLS} FROM groups_runs WHERE run_id = ?1"),
            [run_id],
            run_from_row,
        )
        .optional()
    })
}

struct RoundUse {
    turns: i64,
    active: i64,
    delegations: i64,
    tokens: i64,
}

fn round_use(
    tx: &rusqlite::Transaction,
    room: &str,
    round: i64,
    unknown: u64,
) -> Result<RoundUse, String> {
    let q = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Result<i64, String> {
        tx.query_row(sql, p, |r| r.get::<_, i64>(0))
            .map_err(|e| format!("{ERR_GROUP}: {e}"))
    };
    Ok(RoundUse {
        turns: q(
            "SELECT count(*) FROM groups_runs WHERE room = ?1 AND round = ?2",
            &[&room, &round],
        )?,
        active: q(
            "SELECT count(*) FROM groups_runs WHERE room = ?1 AND state = 'running'",
            &[&room],
        )?,
        delegations: q(
            "SELECT count(*) FROM groups_tasks WHERE room = ?1 AND round = ?2",
            &[&room, &round],
        )?,
        // A run still going, or one that never reported usage, is charged
        // the configured estimate: unknown is not free.
        tokens: q(
            "SELECT COALESCE(SUM(COALESCE(tokens, ?3)), 0) FROM groups_runs WHERE room = ?1 AND round = ?2",
            &[&room, &round, &(unknown as i64)],
        )?,
    })
}

fn check_admission(
    tx: &rusqlite::Transaction,
    room: &Room,
    round: i64,
    delegating: bool,
) -> Result<(), String> {
    let l = &room.limits;
    let u = round_use(tx, &room.id, round, l.unknown_run_tokens)?;
    if u.active >= l.max_concurrency as i64 {
        return Err(format!(
            "{ERR_GROUP_LIMIT}: concurrency — {} runs already active in this room (max {})",
            u.active, l.max_concurrency
        ));
    }
    if u.turns >= l.max_turns_per_round as i64 {
        return Err(format!(
            "{ERR_GROUP_LIMIT}: turns — this round already used {} turns (max {})",
            u.turns, l.max_turns_per_round
        ));
    }
    if delegating && u.delegations >= l.max_delegations_per_round as i64 {
        return Err(format!(
            "{ERR_GROUP_LIMIT}: rounds — {} delegations already in this round (max {})",
            u.delegations, l.max_delegations_per_round
        ));
    }
    if let Some(max) = l.max_tokens_per_round {
        // Reserve one unknown run for the one being admitted.
        if u.tokens + l.unknown_run_tokens as i64 > max as i64 {
            return Err(format!(
                "{ERR_GROUP_LIMIT}: tokens — this round has used about {} of {} tokens",
                u.tokens, max
            ));
        }
    }
    Ok(())
}

fn insert_run(
    tx: &rusqlite::Transaction,
    run_id: &str,
    room: &str,
    bot: &str,
    task: Option<&str>,
    reply_to: Option<&str>,
    round: i64,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO groups_runs(run_id, room, bot, task_id, reply_to, round, state, started_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7)",
        params![run_id, room, bot, task, reply_to, round, now_ms()],
    )
    .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    Ok(())
}

fn bot_busy(tx: &rusqlite::Transaction, room: &str, bot: &str) -> Result<bool, String> {
    tx.query_row(
        "SELECT count(*) FROM groups_runs WHERE room = ?1 AND bot = ?2 AND state = 'running'",
        params![room, bot],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| format!("{ERR_GROUP}: {e}"))
}

// ── dispatcher (the app starts real turns) ───────────────────────────────

/// What the app implements: start one member turn in its own session
/// (`"<room>~<bot>"`), cancel one. The run id returned is the turn's request
/// id; the app reports the end through [`finish_run`].
#[async_trait::async_trait]
pub trait RoomDispatcher: Send + Sync {
    async fn start(&self, room: &str, bot: &str, input: String) -> Result<String, String>;
    fn cancel(&self, run_id: &str);
}

static DISPATCHER: RwLock<Option<Arc<dyn RoomDispatcher>>> = RwLock::new(None);

pub fn set_dispatcher(d: Arc<dyn RoomDispatcher>) {
    *DISPATCHER.write().unwrap_or_else(|e| e.into_inner()) = Some(d);
}

pub fn dispatcher() -> Option<Arc<dyn RoomDispatcher>> {
    DISPATCHER.read().unwrap_or_else(|e| e.into_inner()).clone()
}

// ── mentions and routing (B01) ───────────────────────────────────────────

/// The `@handle` of a bot: its name lowercased, spaces as `-`, only
/// `[a-z0-9_-]`. The composer's autocomplete inserts exactly this.
pub fn handle_of(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if c.is_whitespace() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// One character folded for matching a mention: lowercase, accents off.
fn fold_char(c: char) -> char {
    use unicode_normalization::UnicodeNormalization;
    let base = std::iter::once(c).nfd().next().unwrap_or(c);
    base.to_lowercase().next().unwrap_or(base)
}

/// Length in chars of `candidate` at `chars[start..]`, compared folded,
/// ending at a word boundary; `None` when it does not match.
fn match_at(chars: &[char], start: usize, candidate: &str) -> Option<usize> {
    let cand: Vec<char> = candidate.chars().map(fold_char).collect();
    if cand.is_empty() || start + cand.len() > chars.len() {
        return None;
    }
    for (k, c) in cand.iter().enumerate() {
        if fold_char(chars[start + k]) != *c {
            return None;
        }
    }
    let end = start + cand.len();
    let boundary = end == chars.len()
        || !(chars[end].is_alphanumeric() || chars[end] == '_' || chars[end] == '-');
    boundary.then_some(cand.len())
}

/// Members mentioned in `text`, in order, without repeats. A mention is `@`
/// followed by a member's id, its handle (what the autocomplete inserts) or
/// its display name as shown in the room header ("@Companheiro de leitura"),
/// compared without case or accents; the longest match wins, so "@Ana Maria"
/// is not read as "@Ana". Anything else (an e-mail, an unknown name) is
/// ignored.
pub fn mentions(text: &str, members: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let start = i + 1;
            let mut best: Option<(usize, &String)> = None;
            for (id, name) in members {
                for cand in [id.as_str(), handle_of(name).as_str(), name.trim()] {
                    if let Some(len) = match_at(&chars, start, cand) {
                        if best.map(|(l, _)| len > l).unwrap_or(true) {
                            best = Some((len, id));
                        }
                    }
                }
            }
            match best {
                Some((len, id)) => {
                    if !out.contains(id) {
                        out.push(id.clone());
                    }
                    i = start + len;
                }
                None => i = start,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Who answers a user message.
pub fn route(room: &Room, text: &str, names: &HashMap<String, String>) -> Vec<String> {
    let members: Vec<(String, String)> = room
        .members
        .iter()
        .map(|m| {
            (
                m.bot.clone(),
                names.get(&m.bot).cloned().unwrap_or_default(),
            )
        })
        .collect();
    let mentioned = mentions(text, &members);
    if !mentioned.is_empty() {
        return mentioned;
    }
    if room.mention_policy == MentionPolicy::MentionsOnly {
        return Vec::new();
    }
    room.coordinator
        .clone()
        .or_else(|| room.default_bot.clone())
        .or_else(|| room.members.first().map(|m| m.bot.clone()))
        .into_iter()
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Started {
    pub bot: String,
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skipped {
    pub bot: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendOutcome {
    pub message: RoomMessage,
    pub round: i64,
    pub started: Vec<Started>,
    pub skipped: Vec<Skipped>,
}

/// The input a member turn receives for a user message. The room context
/// (recent visible messages with authors) comes from the turn hook, so the
/// task text stays the user's words.
fn user_turn_input(text: &str) -> String {
    text.to_string()
}

/// Persists a user message, opens a new round and starts a run for each
/// routed member (and only them). Returns who started and who was skipped.
pub async fn send_user_message(
    db: &AssistDb,
    dispatcher: &dyn RoomDispatcher,
    room_id: &str,
    text: &str,
    names: &HashMap<String, String>,
) -> Result<SendOutcome, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("{ERR_GROUP}: empty message"));
    }
    let room = store::get_room(db, room_id)?;
    // LOCAL_USER entry: an external room is never dispatched from here (H1).
    db.with(|c| {
        Ok(super::super::external_config::deny_local_dispatch(
            c,
            Some(room_id),
            "",
        ))
    })??;
    let (message, round) = db.tx(|tx| {
        tx.execute(
            "UPDATE groups_rooms SET round = round + 1 WHERE id = ?1",
            [room_id],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        let round: i64 = tx
            .query_row(
                "SELECT round FROM groups_rooms WHERE id = ?1",
                [room_id],
                |r| r.get(0),
            )
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        let m = store::insert_message_tx(
            tx,
            &NewMessage {
                room: room_id,
                author: AuthorKind::User,
                bot_id: None,
                reply_to: None,
                run_id: None,
                task_id: None,
                round,
                text,
                status: "done",
            },
        )?;
        Ok((m, round))
    })?;
    let mut started = Vec::new();
    let mut skipped = Vec::new();
    for bot in route(&room, text, names) {
        let admitted = db.tx(|tx| {
            // A derived bot added to a local room never runs as LOCAL_USER.
            super::super::external_config::deny_local_dispatch(tx, None, &bot)?;
            if bot_busy(tx, room_id, &bot)? {
                return Err(format!(
                    "{ERR_GROUP_BUSY}: still answering the previous message"
                ));
            }
            check_admission(tx, &room, round, false)
        });
        if let Err(reason) = admitted {
            skipped.push(Skipped { bot, reason });
            continue;
        }
        match dispatcher.start(room_id, &bot, user_turn_input(text)).await {
            Ok(run_id) => {
                db.tx(|tx| insert_run(tx, &run_id, room_id, &bot, None, Some(&message.id), round))?;
                apply_early(db, &run_id);
                started.push(Started { bot, run_id });
            }
            Err(reason) => skipped.push(Skipped { bot, reason }),
        }
    }
    Ok(SendOutcome {
        message,
        round,
        started,
        skipped,
    })
}

// ── delegated tasks (B02, B03) ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub room: String,
    pub parent_task: Option<String>,
    pub creator_bot: String,
    pub creator_run: Option<String>,
    pub recipient: String,
    pub question: String,
    pub context: String,
    pub scopes: Vec<String>,
    pub deliverable: String,
    pub limit_ms: i64,
    pub depth: i64,
    pub round: i64,
    /// `pending`, `claimed`, `completed`, `failed`, `cancelled`, `interrupted`.
    pub state: String,
    pub claim_run: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub message_id: Option<String>,
    pub created_ms: i64,
    pub finished_ms: Option<i64>,
}

fn task_from_row(r: &Row) -> rusqlite::Result<Task> {
    let scopes: String = r.get(8)?;
    Ok(Task {
        id: r.get(0)?,
        room: r.get(1)?,
        parent_task: r.get(2)?,
        creator_bot: r.get(3)?,
        creator_run: r.get(4)?,
        recipient: r.get(5)?,
        question: r.get(6)?,
        context: r.get(7)?,
        scopes: serde_json::from_str(&scopes).unwrap_or_default(),
        deliverable: r.get(9)?,
        limit_ms: r.get(10)?,
        depth: r.get(11)?,
        round: r.get(12)?,
        state: r.get(13)?,
        claim_run: r.get(14)?,
        result: r.get(15)?,
        error: r.get(16)?,
        message_id: r.get(17)?,
        created_ms: r.get(18)?,
        finished_ms: r.get(19)?,
    })
}

const TASK_COLS: &str = "id, room, parent_task, creator_bot, creator_run, recipient, question, context, scopes, deliverable, limit_ms, depth, round, state, claim_run, result, error, message_id, created_ms, finished_ms";

pub fn task(db: &AssistDb, id: &str) -> Result<Task, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {TASK_COLS} FROM groups_tasks WHERE id = ?1"),
            [id],
            task_from_row,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_GROUP_NOT_FOUND}: no task {id}"))
}

pub fn tasks(db: &AssistDb, room: &str) -> Result<Vec<Task>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {TASK_COLS} FROM groups_tasks WHERE room = ?1 ORDER BY created_ms"
        ))?;
        let rows = st.query_map([room], task_from_row)?;
        rows.collect()
    })
}

/// What a member asks for when it delegates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DelegateRequest {
    pub to: String,
    pub question: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub deliverable: String,
    /// Seconds; clamped to the room's `max_task_ms`.
    #[serde(default)]
    pub limit_s: Option<u64>,
}

/// Validates the limits and records a pending task, in one transaction.
/// `origin` (`"<run>:<tool call>"`) makes a replayed tool call return the
/// task it already created instead of a second one.
pub fn admit_delegation(
    db: &AssistDb,
    room_id: &str,
    creator_bot: &str,
    creator_run: Option<&str>,
    origin: Option<&str>,
    req: &DelegateRequest,
) -> Result<Task, String> {
    admit_delegation_guarded(db, room_id, creator_bot, creator_run, origin, req, None)
}
/// Adds a scoped ownership check/receipt inside the existing task transaction.
pub(crate) fn admit_delegation_guarded(
    db: &AssistDb,
    room_id: &str,
    creator_bot: &str,
    creator_run: Option<&str>,
    origin: Option<&str>,
    req: &DelegateRequest,
    guard: Option<&dyn Fn(&rusqlite::Transaction, &Task) -> Result<(), String>>,
) -> Result<Task, String> {
    let room = store::get_room(db, room_id)?;
    // Only the guarded external admission (`external_config::create_task`)
    // may delegate in an external room or involve a derived bot (H1).
    if guard.is_none() {
        db.with(|c| {
            Ok(super::super::external_config::deny_local_dispatch(
                c,
                Some(room_id),
                creator_bot,
            ))
        })??;
    }
    if !room.members.iter().any(|m| m.bot == creator_bot) {
        return Err(format!(
            "{ERR_GROUP}: {creator_bot} is not a member of this room"
        ));
    }
    let wanted = req.to.trim().trim_start_matches('@').to_ascii_lowercase();
    let to = room
        .members
        .iter()
        .find(|m| m.bot.to_ascii_lowercase() == wanted || handle_of(&m.bot) == wanted)
        .map(|m| m.bot.clone())
        .ok_or_else(|| format!("{ERR_GROUP}: `{}` is not a member of this room", req.to))?;
    if guard.is_none() {
        db.with(|c| {
            Ok(super::super::external_config::deny_local_dispatch(
                c, None, &to,
            ))
        })??;
    }
    if to == creator_bot {
        return Err(format!("{ERR_GROUP}: a member cannot delegate to itself"));
    }
    if req.question.trim().is_empty() {
        return Err(format!("{ERR_GROUP}: the task needs a question"));
    }
    let limit_ms = req
        .limit_s
        .map(|s| s.saturating_mul(1000))
        .unwrap_or(room.limits.max_task_ms)
        .clamp(5_000, room.limits.max_task_ms) as i64;
    db.tx(|tx| {
        if let Some(o) = origin {
            let existing = tx
                .query_row(
                    &format!("SELECT {TASK_COLS} FROM groups_tasks WHERE origin = ?1"),
                    [o],
                    task_from_row,
                )
                .optional()
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            if let Some(t) = existing {
                if let Some(guard)=guard {guard(tx,&t)?;}
                return Ok(t);
            }
        }
        // Depth and round come from the run that is delegating: a user-routed
        // run is depth 0; a run working on task T is at T's depth.
        let (parent_task, parent_depth, round) = match creator_run {
            Some(run) => {
                let r = tx
                    .query_row(
                        &format!("SELECT {RUN_COLS} FROM groups_runs WHERE run_id = ?1"),
                        [run],
                        run_from_row,
                    )
                    .optional()
                    .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
                match r {
                    Some(r) if r.room == room_id => {
                        let depth = match &r.task_id {
                            Some(t) => tx
                                .query_row("SELECT depth FROM groups_tasks WHERE id = ?1", [t], |x| {
                                    x.get::<_, i64>(0)
                                })
                                .map_err(|e| format!("{ERR_GROUP}: {e}"))?,
                            None => 0,
                        };
                        (r.task_id.clone(), depth, r.round)
                    }
                    _ => (None, 0, room.round),
                }
            }
            None => (None, 0, room.round),
        };
        let depth = parent_depth + 1;
        if depth > room.limits.max_depth as i64 {
            return Err(format!(
                "{ERR_GROUP_LIMIT}: depth — a delegated task cannot delegate further (max depth {})",
                room.limits.max_depth
            ));
        }
        check_admission(tx, &room, round, true)?;
        if bot_busy(tx, room_id, &to)? {
            return Err(format!("{ERR_GROUP_BUSY}: {to} is already working; try again later"));
        }
        let t = Task {
            id: new_id(),
            room: room_id.to_string(),
            parent_task,
            creator_bot: creator_bot.to_string(),
            creator_run: creator_run.map(str::to_string),
            recipient: to.clone(),
            question: req.question.trim().to_string(),
            context: req.context.trim().to_string(),
            scopes: req.scopes.clone(),
            deliverable: req.deliverable.trim().to_string(),
            limit_ms,
            depth,
            round,
            state: "pending".into(),
            claim_run: None,
            result: None,
            error: None,
            message_id: None,
            created_ms: now_ms(),
            finished_ms: None,
        };
        tx.execute(
            "INSERT INTO groups_tasks(id, room, parent_task, creator_bot, creator_run, recipient, question, context,
                scopes, deliverable, limit_ms, depth, round, state, origin, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'pending', ?14, ?15)",
            params![
                t.id,
                t.room,
                t.parent_task,
                t.creator_bot,
                t.creator_run,
                t.recipient,
                t.question,
                t.context,
                serde_json::to_string(&t.scopes).unwrap_or_default(),
                t.deliverable,
                t.limit_ms,
                t.depth,
                t.round,
                origin,
                t.created_ms
            ],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        if let Some(guard)=guard {guard(tx,&t)?;}
        Ok(t)
    })
}

/// The single acceptance of a task: `pending → claimed` by one run, in one
/// statement. Claiming again with the same run is a no-op that returns the
/// task (idempotent after a restart); any other run gets `ERR_GROUP_CLAIMED`.
pub fn claim(db: &AssistDb, task_id: &str, run_id: &str) -> Result<Task, String> {
    db.tx(|tx| {
        let n = tx
            .execute(
                "UPDATE groups_tasks SET state = 'claimed', claim_run = ?2, claimed_ms = ?3
                 WHERE id = ?1 AND state = 'pending'",
                params![task_id, run_id, now_ms()],
            )
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        let t = tx
            .query_row(
                &format!("SELECT {TASK_COLS} FROM groups_tasks WHERE id = ?1"),
                [task_id],
                task_from_row,
            )
            .optional()
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?
            .ok_or_else(|| format!("{ERR_GROUP_NOT_FOUND}: no task {task_id}"))?;
        if n == 1 {
            insert_run(
                tx,
                run_id,
                &t.room,
                &t.recipient,
                Some(&t.id),
                None,
                t.round,
            )?;
            return Ok(t);
        }
        if t.claim_run.as_deref() == Some(run_id) {
            return Ok(t);
        }
        Err(format!(
            "{ERR_GROUP_CLAIMED}: task {task_id} is {} (claimed by another run)",
            t.state
        ))
    })
}

/// The text a delegated member receives.
pub fn task_input(t: &Task) -> String {
    let mut s = format!(
        "Task delegated to you by @{} in this room.\nQuestion: {}",
        t.creator_bot, t.question
    );
    if !t.context.is_empty() {
        s.push_str(&format!("\nContext you need: {}", t.context));
    }
    if !t.deliverable.is_empty() {
        s.push_str(&format!("\nDeliver: {}", t.deliverable));
    }
    if !t.scopes.is_empty() {
        s.push_str(&format!("\nYou may use: {}", t.scopes.join(", ")));
    }
    s.push_str(&format!(
        "\nTime limit: {} s. Answer with the deliverable only; say plainly what you could not verify.",
        t.limit_ms / 1000
    ));
    s
}

// ── the end of a run ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEnd {
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunEnd {
    fn as_str(self) -> &'static str {
        match self {
            RunEnd::Completed => "completed",
            RunEnd::Failed => "failed",
            RunEnd::Cancelled => "cancelled",
            RunEnd::Interrupted => "interrupted",
        }
    }
}

/// Waiters of delegated tasks (the `delegate_task` call that is awaiting
/// its child). In memory only: after a restart nobody waits any more.
static WAITERS: Mutex<Option<HashMap<String, Arc<tokio::sync::Notify>>>> = Mutex::new(None);

fn waiter(task_id: &str) -> Arc<tokio::sync::Notify> {
    WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Default::default)
        .entry(task_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Notify::new()))
        .clone()
}

fn wake(task_id: &str) {
    let w = WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .and_then(|m| m.remove(task_id));
    if let Some(w) = w {
        w.notify_waiters();
    }
}

/// A turn can end before its run row exists (it failed at once, before
/// `send_user_message` / `claim` recorded it). Its end waits here and is
/// applied as soon as the row is written.
type EarlyEnd = (RunEnd, String, Option<String>, Option<i64>);
static EARLY: Mutex<Option<HashMap<String, EarlyEnd>>> = Mutex::new(None);

fn apply_early(db: &AssistDb, run_id: &str) {
    let e = EARLY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .and_then(|m| m.remove(run_id));
    if let Some((end, text, error, tokens)) = e {
        let _ = finish_run(db, run_id, end, &text, error.as_deref(), tokens);
    }
}

/// Records the end of a member run: its message in the room (with author,
/// run id, task and reply), the task result, the tokens used. Idempotent: a
/// run that already ended is left alone (a late event cannot rewrite it).
/// Returns the message written, if any, plus runs to cancel (children of a
/// parent run that did not complete).
pub fn finish_run(
    db: &AssistDb,
    run_id: &str,
    end: RunEnd,
    text: &str,
    error: Option<&str>,
    tokens: Option<i64>,
) -> Result<(Option<RoomMessage>, Vec<String>), String> {
    let (msg, task_id, cascade) = db.tx(|tx| {
        let Some(r) = tx
            .query_row(
                &format!("SELECT {RUN_COLS} FROM groups_runs WHERE run_id = ?1"),
                [run_id],
                run_from_row,
            )
            .optional()
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?
        else {
            EARLY
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert_with(Default::default)
                .insert(
                    run_id.to_string(),
                    (end, text.to_string(), error.map(str::to_string), tokens),
                );
            return Ok((None, None, Vec::new()));
        };
        if r.state != "running" {
            return Ok((None, None, Vec::new()));
        }
        tx.execute(
            "UPDATE groups_runs SET state = ?2, tokens = ?3, finished_ms = ?4 WHERE run_id = ?1",
            params![run_id, end.as_str(), tokens, now_ms()],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        let status = match end {
            RunEnd::Completed => "done",
            RunEnd::Failed => "failed",
            RunEnd::Cancelled => "cancelled",
            RunEnd::Interrupted => "interrupted",
        };
        let body = if text.trim().is_empty() {
            error.unwrap_or("").to_string()
        } else {
            text.to_string()
        };
        let reply_to = r.reply_to.clone();
        let msg = if body.trim().is_empty() && end == RunEnd::Cancelled {
            None
        } else {
            Some(store::insert_message_tx(
                tx,
                &NewMessage {
                    room: &r.room,
                    author: AuthorKind::Bot,
                    bot_id: Some(&r.bot),
                    reply_to: reply_to.as_deref(),
                    run_id: Some(run_id),
                    task_id: r.task_id.as_deref(),
                    round: r.round,
                    text: &body,
                    status,
                },
            )?)
        };
        if let Some(t) = &r.task_id {
            let state = match end {
                RunEnd::Completed => "completed",
                other => other.as_str(),
            };
            tx.execute(
                "UPDATE groups_tasks SET state = ?2, result = ?3, error = ?4, message_id = ?5, finished_ms = ?6
                 WHERE id = ?1 AND state IN ('pending','claimed')",
                params![
                    t,
                    state,
                    (end == RunEnd::Completed).then_some(text),
                    error,
                    msg.as_ref().map(|m| m.id.clone()),
                    now_ms()
                ],
            )
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        }
        // A parent that did not complete takes its still-active children down.
        let cascade = if end != RunEnd::Completed {
            cancel_children_of_run_tx(tx, run_id)?
        } else {
            Vec::new()
        };
        Ok((msg, r.task_id.clone(), cascade))
    })?;
    if let Some(t) = task_id {
        wake(&t);
    }
    Ok((msg, cascade))
}

/// Tasks created by `run` (and their descendants) that are still active →
/// cancelled; returns the runs working on them.
fn cancel_children_of_run_tx(tx: &rusqlite::Transaction, run: &str) -> Result<Vec<String>, String> {
    let ids: Vec<String> = {
        let mut st = tx
            .prepare("SELECT id FROM groups_tasks WHERE creator_run = ?1 AND state IN ('pending','claimed')")
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        let rows = st
            .query_map([run], |r| r.get(0))
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?
    };
    let mut runs = Vec::new();
    for id in ids {
        runs.extend(cancel_task_tree_tx(tx, &id)?);
    }
    Ok(runs)
}

fn cancel_task_tree_tx(tx: &rusqlite::Transaction, task_id: &str) -> Result<Vec<String>, String> {
    let mut runs = Vec::new();
    let claim: Option<String> = tx
        .query_row(
            "SELECT claim_run FROM groups_tasks WHERE id = ?1 AND state IN ('pending','claimed')",
            [task_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?
        .flatten();
    tx.execute(
        "UPDATE groups_tasks SET state = 'cancelled', finished_ms = ?2 WHERE id = ?1 AND state IN ('pending','claimed')",
        params![task_id, now_ms()],
    )
    .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    if let Some(run) = claim {
        let n = tx
            .execute(
                "UPDATE groups_runs SET state = 'cancelled', finished_ms = ?2 WHERE run_id = ?1 AND state = 'running'",
                params![run, now_ms()],
            )
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        if n > 0 {
            runs.push(run.clone());
        }
        runs.extend(cancel_children_of_run_tx(tx, &run)?);
    }
    let children: Vec<String> = {
        let mut st = tx
            .prepare("SELECT id FROM groups_tasks WHERE parent_task = ?1 AND state IN ('pending','claimed')")
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        let rows = st
            .query_map([task_id], |r| r.get(0))
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?
    };
    for c in children {
        runs.extend(cancel_task_tree_tx(tx, &c)?);
    }
    Ok(runs)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CancelOutcome {
    /// Runs stopped (the app cancels their turns).
    pub cancelled_runs: Vec<String>,
    pub cancelled_tasks: Vec<String>,
    /// Tasks that had already finished and keep their result.
    pub kept_tasks: Vec<String>,
}

/// Cancels everything still active in the room (B05). Finished runs and
/// tasks keep their state and their messages. Calls the dispatcher for each
/// run stopped, and writes one system note saying what was partial.
pub fn cancel_room(
    db: &AssistDb,
    room_id: &str,
    dispatcher: Option<&dyn RoomDispatcher>,
) -> Result<CancelOutcome, String> {
    store::get_room(db, room_id)?;
    let out = db.tx(|tx| {
        let now = now_ms();
        let active_tasks: Vec<String> = {
            let mut st = tx
                .prepare("SELECT id FROM groups_tasks WHERE room = ?1 AND state IN ('pending','claimed')")
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            let rows = st
                .query_map([room_id], |r| r.get(0))
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            rows.collect::<rusqlite::Result<_>>()
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?
        };
        let kept: Vec<String> = {
            let mut st = tx
                .prepare("SELECT id FROM groups_tasks WHERE room = ?1 AND state = 'completed'")
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            let rows = st
                .query_map([room_id], |r| r.get(0))
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            rows.collect::<rusqlite::Result<_>>()
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?
        };
        let runs: Vec<(String, String)> = {
            let mut st = tx
                .prepare("SELECT run_id, bot FROM groups_runs WHERE room = ?1 AND state = 'running'")
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            let rows = st
                .query_map([room_id], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            rows.collect::<rusqlite::Result<_>>()
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?
        };
        tx.execute(
            "UPDATE groups_tasks SET state = 'cancelled', finished_ms = ?2 WHERE room = ?1 AND state IN ('pending','claimed')",
            params![room_id, now],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        tx.execute(
            "UPDATE groups_runs SET state = 'cancelled', finished_ms = ?2 WHERE room = ?1 AND state = 'running'",
            params![room_id, now],
        )
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        if !runs.is_empty() || !active_tasks.is_empty() {
            let round: i64 = tx
                .query_row("SELECT round FROM groups_rooms WHERE id = ?1", [room_id], |r| r.get(0))
                .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
            let who: Vec<String> = runs.iter().map(|(_, b)| format!("@{b}")).collect();
            store::insert_message_tx(
                tx,
                &NewMessage {
                    room: room_id,
                    author: AuthorKind::System,
                    bot_id: None,
                    reply_to: None,
                    run_id: None,
                    task_id: None,
                    round,
                    text: &format!(
                        "cancelled: {} · finished results kept: {}",
                        if who.is_empty() { "-".to_string() } else { who.join(", ") },
                        kept.len()
                    ),
                    status: "done",
                },
            )?;
        }
        Ok(CancelOutcome {
            cancelled_runs: runs.into_iter().map(|(r, _)| r).collect(),
            cancelled_tasks: active_tasks,
            kept_tasks: kept,
        })
    })?;
    for t in &out.cancelled_tasks {
        wake(t);
    }
    if let Some(d) = dispatcher {
        for r in &out.cancelled_runs {
            d.cancel(r);
        }
    }
    Ok(out)
}

/// After a restart nothing is running: runs and claimed tasks become
/// `interrupted` (never re-run on their own: a turn may have written files),
/// pending tasks too (the delegating turn that awaited them is gone).
pub fn recover(db: &AssistDb) -> Result<usize, String> {
    db.tx(|tx| {
        let now = now_ms();
        let a = tx
            .execute(
                "UPDATE groups_runs SET state = 'interrupted', finished_ms = ?1 WHERE state = 'running'",
                [now],
            )
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        let b = tx
            .execute(
                "UPDATE groups_tasks SET state = 'interrupted', finished_ms = ?1 WHERE state IN ('pending','claimed')",
                [now],
            )
            .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
        Ok(a + b)
    })
}

/// The whole delegation: admit, start the child in its own session, claim,
/// wait for its end or the time limit. What the `delegate_task` tool runs.
pub async fn delegate(
    db: &AssistDb,
    dispatcher: &dyn RoomDispatcher,
    room_id: &str,
    creator_bot: &str,
    creator_run: Option<&str>,
    origin: Option<&str>,
    req: &DelegateRequest,
) -> Result<Task, String> {
    let t = admit_delegation(db, room_id, creator_bot, creator_run, origin, req)?;
    if t.state != "pending" {
        // A replay of a call that already delegated: report, do not re-run.
        return Ok(t);
    }
    let notify = waiter(&t.id);
    let run_id = match dispatcher
        .start(room_id, &t.recipient, task_input(&t))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            db.with(|c| {
                c.execute(
                    "UPDATE groups_tasks SET state = 'failed', error = ?2, finished_ms = ?3 WHERE id = ?1 AND state = 'pending'",
                    params![t.id, e, now_ms()],
                )
            })?;
            return task(db, &t.id);
        }
    };
    if let Err(e) = claim(db, &t.id, &run_id) {
        dispatcher.cancel(&run_id);
        return Err(e);
    }
    apply_early(db, &run_id);
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(t.limit_ms as u64);
    loop {
        let current = task(db, &t.id)?;
        if !matches!(current.state.as_str(), "pending" | "claimed") {
            return Ok(current);
        }
        let n = notify.notified();
        tokio::pin!(n);
        n.as_mut().enable();
        // Re-check after registering, so a wake between the two is not lost.
        let current = task(db, &t.id)?;
        if !matches!(current.state.as_str(), "pending" | "claimed") {
            return Ok(current);
        }
        if tokio::time::timeout_at(deadline, n).await.is_err() {
            // Time limit: stop the child, keep whatever it said as partial.
            let (_, cascade) = finish_run(
                db,
                &run_id,
                RunEnd::Cancelled,
                "",
                Some(&format!(
                    "{ERR_GROUP_LIMIT}: time — {} s",
                    t.limit_ms / 1000
                )),
                None,
            )?;
            dispatcher.cancel(&run_id);
            for r in cascade {
                dispatcher.cancel(&r);
            }
            db.with(|c| {
                c.execute(
                    "UPDATE groups_tasks SET state = 'failed', error = ?2 WHERE id = ?1",
                    params![
                        t.id,
                        format!("{ERR_GROUP_LIMIT}: time — {} s", t.limit_ms / 1000)
                    ],
                )
            })?;
            return task(db, &t.id);
        }
    }
}
