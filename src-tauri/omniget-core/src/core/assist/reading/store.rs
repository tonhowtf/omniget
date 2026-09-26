//! Reading records in SQLite: journeys, progress, temporary round context,
//! access preferences, movie identities, availability and subtitle evidence,
//! rounds and viewing events. Rules live in [`super::rules`].

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::ctx::{AssistCtx, Scope};
use super::super::db::{AssistDb, Migration};
use super::super::web::extract::fold;
use super::super::{new_id, now_ms};

pub const ERR_READING_INPUT: &str = "ERR_READING_INPUT";
pub const ERR_READING_NOT_FOUND: &str = "ERR_READING_NOT_FOUND";
pub const ERR_READING_SCOPE: &str = "ERR_READING_SCOPE";

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "reading",
    version: 1,
    sql: "CREATE TABLE reading_journeys (
            id TEXT PRIMARY KEY,
            principal TEXT NOT NULL,
            bot_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            title TEXT NOT NULL,
            author TEXT,
            edition TEXT,
            language TEXT,
            external_id TEXT,
            status TEXT NOT NULL DEFAULT 'active'
              CHECK (status IN ('active','paused','finished','abandoned')),
            spoiler_terms_json TEXT NOT NULL DEFAULT '[]',
            notes TEXT,
            started_ms INTEGER NOT NULL,
            finished_ms INTEGER,
            updated_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_journeys_bot ON reading_journeys(bot_id, status);
          CREATE TABLE reading_progress (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            position_kind TEXT NOT NULL CHECK (position_kind IN ('chapter','page','percent','locator')),
            value TEXT NOT NULL,
            value_num REAL,
            edition TEXT,
            origin TEXT NOT NULL CHECK (origin IN ('manual','reader','bot')),
            note TEXT,
            recorded_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_progress_journey ON reading_progress(journey_id, recorded_ms);
          CREATE TABLE reading_contexts (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            text TEXT NOT NULL,
            created_ms INTEGER NOT NULL,
            expires_ms INTEGER NOT NULL
          );
          CREATE TABLE reading_access_prefs (
            principal TEXT NOT NULL,
            bot_id TEXT NOT NULL,
            region TEXT NOT NULL DEFAULT 'BR',
            subscriptions_json TEXT NOT NULL DEFAULT '[]',
            access_kinds_json TEXT NOT NULL DEFAULT '[\"subscription\",\"free\",\"rent\",\"buy\"]',
            only_confirmed INTEGER NOT NULL DEFAULT 0,
            freshness_days INTEGER NOT NULL DEFAULT 7,
            updated_ms INTEGER NOT NULL,
            PRIMARY KEY (principal, bot_id)
          );
          CREATE TABLE reading_movies (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            original_title TEXT,
            year INTEGER NOT NULL,
            director TEXT,
            external_ids_json TEXT NOT NULL DEFAULT '{}',
            ident_key TEXT NOT NULL UNIQUE,
            created_ms INTEGER NOT NULL
          );
          CREATE TABLE reading_availability (
            id TEXT PRIMARY KEY,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id) ON DELETE CASCADE,
            region TEXT NOT NULL,
            platform TEXT NOT NULL,
            platform_key TEXT NOT NULL,
            access_kind TEXT NOT NULL CHECK (access_kind IN ('subscription','rent','buy','free')),
            url TEXT,
            status TEXT NOT NULL CHECK (status IN ('confirmed','inferred','absent','unknown')),
            source_kind TEXT NOT NULL CHECK (source_kind IN ('platform','aggregator','search','user')),
            quote TEXT,
            fetch_id TEXT,
            page_date_ms INTEGER,
            note TEXT,
            bot_id TEXT,
            checked_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_availability_movie ON reading_availability(movie_id, platform_key, access_kind, checked_ms);
          CREATE TABLE reading_subtitles (
            id TEXT PRIMARY KEY,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id) ON DELETE CASCADE,
            availability_id TEXT REFERENCES reading_availability(id) ON DELETE SET NULL,
            platform_key TEXT NOT NULL,
            language TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('subtitle','audio')),
            status TEXT NOT NULL CHECK (status IN ('confirmed','inferred','absent','unknown')),
            url TEXT,
            source_kind TEXT NOT NULL CHECK (source_kind IN ('platform','aggregator','search','user')),
            quote TEXT,
            fetch_id TEXT,
            login_required INTEGER NOT NULL DEFAULT 0,
            note TEXT,
            bot_id TEXT,
            checked_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_subtitles_movie ON reading_subtitles(movie_id, platform_key, checked_ms);
          CREATE TABLE reading_rounds (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            progress_id TEXT REFERENCES reading_progress(id) ON DELETE SET NULL,
            reason TEXT,
            context_text TEXT,
            requested_count INTEGER,
            shortfall_reason TEXT,
            only_confirmed INTEGER NOT NULL DEFAULT 0,
            skill_name TEXT,
            skill_hash TEXT,
            look_for_json TEXT NOT NULL DEFAULT '[]',
            bot_id TEXT,
            created_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_rounds_journey ON reading_rounds(journey_id, created_ms);
          CREATE TABLE reading_round_items (
            id TEXT PRIMARY KEY,
            round_id TEXT NOT NULL REFERENCES reading_rounds(id) ON DELETE CASCADE,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id),
            position INTEGER NOT NULL,
            role TEXT NOT NULL CHECK (role IN ('entry','shift','surprise')),
            connection TEXT NOT NULL,
            why TEXT NOT NULL,
            moods_json TEXT NOT NULL DEFAULT '[]',
            pace TEXT NOT NULL CHECK (pace IN ('easy','attentive')),
            heavy INTEGER NOT NULL DEFAULT 0,
            depends_on_ending INTEGER NOT NULL DEFAULT 0,
            resumes_item_id TEXT,
            questions_json TEXT NOT NULL DEFAULT '[]',
            evidence_event_ids_json TEXT NOT NULL DEFAULT '[]',
            status TEXT NOT NULL CHECK (status IN ('confirmed','probable')),
            missing_json TEXT NOT NULL DEFAULT '[]',
            availability_id TEXT,
            subtitle_id TEXT
          );
          CREATE TABLE reading_viewing_events (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id),
            round_item_id TEXT,
            kind TEXT NOT NULL CHECK (kind IN ('recommended','planned','watched','abandoned','declined')),
            reaction TEXT,
            reasons_json TEXT NOT NULL DEFAULT '[]',
            origin TEXT NOT NULL CHECK (origin IN ('user','bot','app')),
            created_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_events_journey ON reading_viewing_events(journey_id, movie_id, created_ms);",
}];

fn bad(msg: impl std::fmt::Display) -> String {
    format!("{ERR_READING_INPUT}: {msg}")
}

fn json_list(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

fn to_json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

// ── Journeys ─────────────────────────────────────────────────────────────

pub const JOURNEY_STATUSES: &[&str] = &["active", "paused", "finished", "abandoned"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Journey {
    pub id: String,
    pub bot_id: String,
    pub scope: String,
    pub title: String,
    pub author: Option<String>,
    pub edition: Option<String>,
    pub language: Option<String>,
    pub external_id: Option<String>,
    pub status: String,
    pub spoiler_terms: Vec<String>,
    pub notes: Option<String>,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub updated_ms: i64,
}

const JOURNEY_COLS: &str = "id, bot_id, scope, title, author, edition, language, external_id, status, spoiler_terms_json, notes, started_ms, finished_ms, updated_ms";

fn journey_row(r: &Row) -> rusqlite::Result<Journey> {
    Ok(Journey {
        id: r.get(0)?,
        bot_id: r.get(1)?,
        scope: r.get(2)?,
        title: r.get(3)?,
        author: r.get(4)?,
        edition: r.get(5)?,
        language: r.get(6)?,
        external_id: r.get(7)?,
        status: r.get(8)?,
        spoiler_terms: json_list(&r.get::<_, String>(9)?),
        notes: r.get(10)?,
        started_ms: r.get(11)?,
        finished_ms: r.get(12)?,
        updated_ms: r.get(13)?,
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewJourney {
    pub title: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub edition: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub spoiler_terms: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

fn clean_opt(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn clean_terms(terms: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in terms {
        let t = t.trim().to_string();
        if t.chars().count() >= 3 && !out.iter().any(|o| fold(o) == fold(&t)) {
            out.push(t);
        }
    }
    out
}

/// Scope a journey created by `ctx` for `bot` lives in: the bot's private
/// scope in a direct conversation, the room when that is all it may write.
fn journey_scope(ctx: &AssistCtx, bot: &str) -> Result<String, String> {
    let own = Scope::Bot {
        bot: bot.to_string(),
    };
    if ctx.can_write(&own) {
        return Ok(own.key());
    }
    ctx.writable
        .iter()
        .find(|s| matches!(s, Scope::Room { .. }))
        .map(Scope::key)
        .ok_or_else(|| {
            format!("{ERR_READING_SCOPE}: this conversation cannot keep a reading journey")
        })
}

pub fn can_see(ctx: &AssistCtx, j: &Journey) -> bool {
    if ctx.is_user_ui() {
        return true;
    }
    let scope_ok = Scope::parse(&j.scope)
        .map(|s| ctx.can_read(&s))
        .unwrap_or(false);
    let bot_ok = match &ctx.bot_id {
        Some(b) => b == &j.bot_id || j.scope.starts_with("room:"),
        None => false,
    };
    scope_ok && bot_ok
}

pub fn can_change(ctx: &AssistCtx, j: &Journey) -> bool {
    ctx.is_user_ui()
        || (can_see(ctx, j)
            && Scope::parse(&j.scope)
                .map(|s| ctx.can_write(&s))
                .unwrap_or(false))
}

pub fn create_journey(
    db: &AssistDb,
    ctx: &AssistCtx,
    bot: &str,
    input: NewJourney,
) -> Result<Journey, String> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(bad("the book title is required"));
    }
    if bot.trim().is_empty() {
        return Err(bad("a bot is required"));
    }
    let scope = journey_scope(ctx, bot)?;
    let now = now_ms();
    let j = Journey {
        id: new_id(),
        bot_id: bot.to_string(),
        scope,
        title,
        author: clean_opt(input.author),
        edition: clean_opt(input.edition),
        language: clean_opt(input.language),
        external_id: clean_opt(input.external_id),
        status: "active".into(),
        spoiler_terms: clean_terms(input.spoiler_terms),
        notes: clean_opt(input.notes),
        started_ms: now,
        finished_ms: None,
        updated_ms: now,
    };
    db.with(|c| {
        c.execute(
            &format!("INSERT INTO reading_journeys({JOURNEY_COLS}, principal) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"),
            params![
                j.id, j.bot_id, j.scope, j.title, j.author, j.edition, j.language, j.external_id,
                j.status, to_json(&j.spoiler_terms), j.notes, j.started_ms, j.finished_ms, j.updated_ms,
                ctx.principal
            ],
        )
    })?;
    Ok(j)
}

pub fn get_journey_raw(c: &Connection, id: &str) -> rusqlite::Result<Option<Journey>> {
    c.query_row(
        &format!("SELECT {JOURNEY_COLS} FROM reading_journeys WHERE id = ?1"),
        [id],
        journey_row,
    )
    .optional()
}

pub fn get_journey(db: &AssistDb, ctx: &AssistCtx, id: &str) -> Result<Journey, String> {
    let j = db.with(|c| get_journey_raw(c, id))?;
    match j {
        Some(j) if can_see(ctx, &j) => Ok(j),
        _ => Err(format!(
            "{ERR_READING_NOT_FOUND}: no reading journey `{id}` here"
        )),
    }
}

/// Journeys of `bot` that `ctx` may see, newest first. `bot = None` with the
/// UI principal lists every journey.
pub fn list_journeys(
    db: &AssistDb,
    ctx: &AssistCtx,
    bot: Option<&str>,
) -> Result<Vec<Journey>, String> {
    let all: Vec<Journey> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {JOURNEY_COLS} FROM reading_journeys ORDER BY (status = 'active') DESC, updated_ms DESC"
        ))?;
        let rows = st.query_map([], journey_row)?;
        rows.collect()
    })?;
    Ok(all
        .into_iter()
        .filter(|j| bot.map(|b| j.bot_id == b).unwrap_or(true))
        .filter(|j| can_see(ctx, j))
        .collect())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JourneyPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub edition: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub spoiler_terms: Option<Vec<String>>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub fn update_journey(
    db: &AssistDb,
    ctx: &AssistCtx,
    id: &str,
    p: JourneyPatch,
) -> Result<Journey, String> {
    let mut j = get_journey(db, ctx, id)?;
    if !can_change(ctx, &j) {
        return Err(format!(
            "{ERR_READING_SCOPE}: this conversation cannot change that journey"
        ));
    }
    if let Some(t) = clean_opt(p.title) {
        j.title = t;
    }
    if p.author.is_some() {
        j.author = clean_opt(p.author);
    }
    if p.edition.is_some() {
        j.edition = clean_opt(p.edition);
    }
    if p.language.is_some() {
        j.language = clean_opt(p.language);
    }
    if p.external_id.is_some() {
        j.external_id = clean_opt(p.external_id);
    }
    if p.notes.is_some() {
        j.notes = clean_opt(p.notes);
    }
    if let Some(terms) = p.spoiler_terms {
        j.spoiler_terms = clean_terms(terms);
    }
    if let Some(s) = p.status {
        let s = s.trim().to_string();
        if !JOURNEY_STATUSES.contains(&s.as_str()) {
            return Err(bad(format!("status must be one of {JOURNEY_STATUSES:?}")));
        }
        if s == "finished" && j.status != "finished" {
            j.finished_ms = Some(now_ms());
        }
        if s != "finished" {
            j.finished_ms = None;
        }
        j.status = s;
    }
    j.updated_ms = now_ms();
    db.with(|c| {
        c.execute(
            "UPDATE reading_journeys SET title=?2, author=?3, edition=?4, language=?5, external_id=?6, status=?7,
                spoiler_terms_json=?8, notes=?9, finished_ms=?10, updated_ms=?11 WHERE id=?1",
            params![
                j.id, j.title, j.author, j.edition, j.language, j.external_id, j.status,
                to_json(&j.spoiler_terms), j.notes, j.finished_ms, j.updated_ms
            ],
        )
    })?;
    Ok(j)
}

pub fn delete_journey(db: &AssistDb, ctx: &AssistCtx, id: &str) -> Result<(), String> {
    let j = get_journey(db, ctx, id)?;
    if !can_change(ctx, &j) {
        return Err(format!(
            "{ERR_READING_SCOPE}: this conversation cannot delete that journey"
        ));
    }
    db.with(|c| c.execute("DELETE FROM reading_journeys WHERE id = ?1", [id]))?;
    Ok(())
}

fn touch(c: &Connection, journey: &str) -> rusqlite::Result<usize> {
    c.execute(
        "UPDATE reading_journeys SET updated_ms = ?2 WHERE id = ?1",
        params![journey, now_ms()],
    )
}

// ── Progress ─────────────────────────────────────────────────────────────

pub const POSITION_KINDS: &[&str] = &["chapter", "page", "percent", "locator"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Progress {
    pub id: String,
    pub journey_id: String,
    pub position_kind: String,
    pub value: String,
    pub value_num: Option<f64>,
    pub edition: Option<String>,
    pub origin: String,
    pub note: Option<String>,
    pub recorded_ms: i64,
}

fn progress_row(r: &Row) -> rusqlite::Result<Progress> {
    Ok(Progress {
        id: r.get(0)?,
        journey_id: r.get(1)?,
        position_kind: r.get(2)?,
        value: r.get(3)?,
        value_num: r.get(4)?,
        edition: r.get(5)?,
        origin: r.get(6)?,
        note: r.get(7)?,
        recorded_ms: r.get(8)?,
    })
}

const PROGRESS_COLS: &str =
    "id, journey_id, position_kind, value, value_num, edition, origin, note, recorded_ms";

/// Whether two positions can be ordered. Pages and locators only compare
/// within the same edition; chapters and percentages compare across.
pub fn compare_progress(a: &Progress, b: &Progress) -> Option<std::cmp::Ordering> {
    if a.position_kind != b.position_kind {
        return None;
    }
    if matches!(a.position_kind.as_str(), "page" | "locator") {
        let ea = a.edition.as_deref().map(fold);
        let eb = b.edition.as_deref().map(fold);
        if ea.is_none() || ea != eb {
            return None;
        }
    }
    a.value_num?.partial_cmp(&b.value_num?)
}

pub struct ProgressOutcome {
    pub progress: Progress,
    /// Previous position, when it can be compared with this one.
    pub compared_with: Option<(Progress, std::cmp::Ordering)>,
    /// Why the previous position was not compared, when it was not.
    pub not_comparable: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn record_progress(
    db: &AssistDb,
    ctx: &AssistCtx,
    journey_id: &str,
    kind: &str,
    value: &str,
    edition: Option<String>,
    origin: &str,
    note: Option<String>,
) -> Result<ProgressOutcome, String> {
    let j = get_journey(db, ctx, journey_id)?;
    if !can_change(ctx, &j) {
        return Err(format!(
            "{ERR_READING_SCOPE}: this conversation cannot change that journey"
        ));
    }
    if !POSITION_KINDS.contains(&kind) {
        return Err(bad(format!(
            "position kind must be one of {POSITION_KINDS:?}"
        )));
    }
    if !["manual", "reader", "bot"].contains(&origin) {
        return Err(bad("origin must be manual, reader or bot"));
    }
    let value = value.trim();
    if value.is_empty() {
        return Err(bad("the position value is required"));
    }
    let value_num = value
        .trim_end_matches('%')
        .trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok();
    if kind == "percent" {
        match value_num {
            Some(v) if (0.0..=100.0).contains(&v) => {}
            _ => return Err(bad("a percentage must be a number from 0 to 100")),
        }
    }
    if kind == "page" && value_num.is_none() {
        return Err(bad("a page must be a number"));
    }
    let edition = clean_opt(edition).or_else(|| j.edition.clone());
    let p = Progress {
        id: new_id(),
        journey_id: j.id.clone(),
        position_kind: kind.into(),
        value: value.into(),
        value_num,
        edition,
        origin: origin.into(),
        note: clean_opt(note),
        recorded_ms: now_ms(),
    };
    let prev = latest_progress(db, &j.id)?;
    db.tx(|tx| {
        tx.execute(
            &format!(
                "INSERT INTO reading_progress({PROGRESS_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"
            ),
            params![
                p.id,
                p.journey_id,
                p.position_kind,
                p.value,
                p.value_num,
                p.edition,
                p.origin,
                p.note,
                p.recorded_ms
            ],
        )
        .map_err(|e| e.to_string())?;
        touch(tx, &j.id).map_err(|e| e.to_string())?;
        Ok(())
    })?;
    let (compared_with, not_comparable) = match prev {
        None => (None, None),
        Some(prev) => match compare_progress(&p, &prev) {
            Some(o) => (Some((prev, o)), None),
            None => {
                let why = if prev.position_kind != p.position_kind {
                    format!(
                        "the previous position was a {}, this one is a {}",
                        prev.position_kind, p.position_kind
                    )
                } else {
                    "pages and locations of different editions are not the same position"
                        .to_string()
                };
                (None, Some(why))
            }
        },
    };
    Ok(ProgressOutcome {
        progress: p,
        compared_with,
        not_comparable,
    })
}

pub fn latest_progress(db: &AssistDb, journey: &str) -> Result<Option<Progress>, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {PROGRESS_COLS} FROM reading_progress WHERE journey_id = ?1 ORDER BY recorded_ms DESC, rowid DESC LIMIT 1"),
            [journey],
            progress_row,
        )
        .optional()
    })
}

pub fn progress_history(db: &AssistDb, journey: &str) -> Result<Vec<Progress>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {PROGRESS_COLS} FROM reading_progress WHERE journey_id = ?1 ORDER BY recorded_ms DESC, rowid DESC"
        ))?;
        let rows = st.query_map([journey], progress_row)?;
        rows.collect()
    })
}

// ── Temporary context ("hoje quero algo leve") ───────────────────────────

pub const DEFAULT_CONTEXT_HOURS: i64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundContext {
    pub id: String,
    pub journey_id: String,
    pub text: String,
    pub created_ms: i64,
    pub expires_ms: i64,
}

pub fn note_context(
    db: &AssistDb,
    ctx: &AssistCtx,
    journey_id: &str,
    text: &str,
    hours: Option<i64>,
) -> Result<RoundContext, String> {
    let j = get_journey(db, ctx, journey_id)?;
    if !can_change(ctx, &j) {
        return Err(format!(
            "{ERR_READING_SCOPE}: this conversation cannot change that journey"
        ));
    }
    let text = text.trim();
    if text.is_empty() {
        return Err(bad("the context text is required"));
    }
    let now = now_ms();
    let hours = hours.unwrap_or(DEFAULT_CONTEXT_HOURS).clamp(1, 48);
    let rc = RoundContext {
        id: new_id(),
        journey_id: j.id,
        text: text.into(),
        created_ms: now,
        expires_ms: now + hours * 3_600_000,
    };
    db.with(|c| {
        c.execute(
            "INSERT INTO reading_contexts(id, journey_id, text, created_ms, expires_ms) VALUES (?1,?2,?3,?4,?5)",
            params![rc.id, rc.journey_id, rc.text, rc.created_ms, rc.expires_ms],
        )
    })?;
    Ok(rc)
}

/// Unexpired contexts of a journey at `now`.
pub fn live_contexts(db: &AssistDb, journey: &str, now: i64) -> Result<Vec<RoundContext>, String> {
    db.with(|c| {
        let mut st = c.prepare(
            "SELECT id, journey_id, text, created_ms, expires_ms FROM reading_contexts
              WHERE journey_id = ?1 AND expires_ms > ?2 ORDER BY created_ms DESC",
        )?;
        let rows = st.query_map(params![journey, now], |r| {
            Ok(RoundContext {
                id: r.get(0)?,
                journey_id: r.get(1)?,
                text: r.get(2)?,
                created_ms: r.get(3)?,
                expires_ms: r.get(4)?,
            })
        })?;
        rows.collect()
    })
}

// ── Access preferences ───────────────────────────────────────────────────

pub const ACCESS_KINDS: &[&str] = &["subscription", "rent", "buy", "free"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessPrefs {
    pub bot_id: String,
    pub region: String,
    pub subscriptions: Vec<String>,
    pub access_kinds: Vec<String>,
    pub only_confirmed: bool,
    pub freshness_days: i64,
    pub updated_ms: i64,
}

impl AccessPrefs {
    pub fn defaults(bot: &str) -> Self {
        Self {
            bot_id: bot.into(),
            region: "BR".into(),
            subscriptions: Vec::new(),
            access_kinds: ACCESS_KINDS.iter().map(|s| s.to_string()).collect(),
            only_confirmed: false,
            freshness_days: 7,
            updated_ms: 0,
        }
    }

    pub fn window_ms(&self) -> i64 {
        self.freshness_days.max(1) * 86_400_000
    }
}

pub fn get_prefs(db: &AssistDb, ctx: &AssistCtx, bot: &str) -> Result<AccessPrefs, String> {
    let row = db.with(|c| {
        c.query_row(
            "SELECT region, subscriptions_json, access_kinds_json, only_confirmed, freshness_days, updated_ms
               FROM reading_access_prefs WHERE principal = ?1 AND bot_id = ?2",
            params![ctx.principal, bot],
            |r| {
                Ok(AccessPrefs {
                    bot_id: bot.into(),
                    region: r.get(0)?,
                    subscriptions: json_list(&r.get::<_, String>(1)?),
                    access_kinds: json_list(&r.get::<_, String>(2)?),
                    only_confirmed: r.get::<_, i64>(3)? != 0,
                    freshness_days: r.get(4)?,
                    updated_ms: r.get(5)?,
                })
            },
        )
        .optional()
    })?;
    Ok(row.unwrap_or_else(|| AccessPrefs::defaults(bot)))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrefsPatch {
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub subscriptions: Option<Vec<String>>,
    #[serde(default)]
    pub access_kinds: Option<Vec<String>>,
    #[serde(default)]
    pub only_confirmed: Option<bool>,
    #[serde(default)]
    pub freshness_days: Option<i64>,
}

pub fn set_prefs(
    db: &AssistDb,
    ctx: &AssistCtx,
    bot: &str,
    p: PrefsPatch,
) -> Result<AccessPrefs, String> {
    if !ctx.is_user_ui() && ctx.bot_id.as_deref() != Some(bot) {
        return Err(format!(
            "{ERR_READING_SCOPE}: a bot can only change its own reading preferences"
        ));
    }
    let mut cur = get_prefs(db, ctx, bot)?;
    if let Some(r) = p.region {
        let r = r.trim().to_ascii_uppercase();
        if r.len() != 2 || !r.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(bad("region must be a two-letter country code, like BR"));
        }
        cur.region = r;
    }
    if let Some(s) = p.subscriptions {
        let mut out: Vec<String> = Vec::new();
        for x in s
            .into_iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
        {
            if !out.iter().any(|o| platform_key(o) == platform_key(&x)) {
                out.push(x);
            }
        }
        cur.subscriptions = out;
    }
    if let Some(k) = p.access_kinds {
        for x in &k {
            if !ACCESS_KINDS.contains(&x.as_str()) {
                return Err(bad(format!("access kind must be one of {ACCESS_KINDS:?}")));
            }
        }
        if k.is_empty() {
            return Err(bad(
                "keep at least one way to watch (subscription, rent, buy or free)",
            ));
        }
        cur.access_kinds = k;
    }
    if let Some(o) = p.only_confirmed {
        cur.only_confirmed = o;
    }
    if let Some(d) = p.freshness_days {
        cur.freshness_days = d.clamp(1, 90);
    }
    cur.updated_ms = now_ms();
    db.with(|c| {
        c.execute(
            "INSERT INTO reading_access_prefs(principal, bot_id, region, subscriptions_json, access_kinds_json, only_confirmed, freshness_days, updated_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(principal, bot_id) DO UPDATE SET region=excluded.region, subscriptions_json=excluded.subscriptions_json,
               access_kinds_json=excluded.access_kinds_json, only_confirmed=excluded.only_confirmed,
               freshness_days=excluded.freshness_days, updated_ms=excluded.updated_ms",
            params![
                ctx.principal, bot, cur.region, to_json(&cur.subscriptions), to_json(&cur.access_kinds),
                cur.only_confirmed as i64, cur.freshness_days, cur.updated_ms
            ],
        )
    })?;
    Ok(cur)
}

/// Canonical platform key: "Amazon Prime Video" and "Prime Video" are one.
pub fn platform_key(name: &str) -> String {
    let k: String = fold(name)
        .replace('+', "plus")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    match k.as_str() {
        "amazonprimevideo" | "amazonprime" | "primevideo" | "amazonvideo" | "prime" => {
            "primevideo".into()
        }
        "hbomax" | "max" | "hbo" => "max".into(),
        "disney" | "disneyplus" => "disneyplus".into(),
        "appletv" | "appletvplus" | "itunes" | "appletvstore" => "appletv".into(),
        "paramount" | "paramountplus" => "paramountplus".into(),
        "googleplay" | "googleplayfilmes" | "googletv" | "googleplaymovies" => "googletv".into(),
        _ => k,
    }
}

// ── Movies ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Movie {
    pub id: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: i64,
    pub director: Option<String>,
    pub external_ids: Value,
}

fn movie_row(r: &Row) -> rusqlite::Result<Movie> {
    Ok(Movie {
        id: r.get(0)?,
        title: r.get(1)?,
        original_title: r.get(2)?,
        year: r.get(3)?,
        director: r.get(4)?,
        external_ids: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
    })
}

fn ident_key(title: &str, year: i64) -> String {
    let t: String = fold(title)
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect();
    format!(
        "{}|{year}",
        t.split_whitespace().collect::<Vec<_>>().join(" ")
    )
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewMovie {
    pub title: String,
    #[serde(default)]
    pub original_title: Option<String>,
    pub year: i64,
    #[serde(default)]
    pub director: Option<String>,
    #[serde(default)]
    pub external_ids: Option<Value>,
}

/// Finds the movie by (original title or title, year) or creates it. A
/// namesake from another year is another movie.
pub fn upsert_movie(db: &AssistDb, m: NewMovie) -> Result<(Movie, bool), String> {
    let title = m.title.trim().to_string();
    if title.is_empty() {
        return Err(bad("the film title is required"));
    }
    if !(1880..=2100).contains(&m.year) {
        return Err(bad("the film year is required (it tells namesakes apart)"));
    }
    let original = clean_opt(m.original_title);
    let key = ident_key(original.as_deref().unwrap_or(&title), m.year);
    let alt = ident_key(&title, m.year);
    let found = db.with(|c| {
        c.query_row(
            "SELECT id, title, original_title, year, director, external_ids_json FROM reading_movies WHERE ident_key IN (?1, ?2)
             OR (year = ?3 AND (lower(title) = lower(?4) OR lower(original_title) = lower(?4)))",
            params![key, alt, m.year, original.as_deref().unwrap_or(&title)],
            movie_row,
        )
        .optional()
    })?;
    if let Some(f) = found {
        return Ok((f, false));
    }
    let movie = Movie {
        id: new_id(),
        title,
        original_title: original,
        year: m.year,
        director: clean_opt(m.director),
        external_ids: m
            .external_ids
            .unwrap_or_else(|| Value::Object(Default::default())),
    };
    db.with(|c| {
        c.execute(
            "INSERT INTO reading_movies(id, title, original_title, year, director, external_ids_json, ident_key, created_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![movie.id, movie.title, movie.original_title, movie.year, movie.director, movie.external_ids.to_string(), key, now_ms()],
        )
    })?;
    Ok((movie, true))
}

pub fn get_movie(db: &AssistDb, id: &str) -> Result<Movie, String> {
    db.with(|c| {
        c.query_row(
            "SELECT id, title, original_title, year, director, external_ids_json FROM reading_movies WHERE id = ?1",
            [id],
            movie_row,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_READING_NOT_FOUND}: no film `{id}`; record it with reading_record_movie first"))
}

// ── Evidence ─────────────────────────────────────────────────────────────

pub const EVIDENCE_STATUSES: &[&str] = &["confirmed", "inferred", "absent", "unknown"];
pub const SOURCE_KINDS: &[&str] = &["platform", "aggregator", "search", "user"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Availability {
    pub id: String,
    pub movie_id: String,
    pub region: String,
    pub platform: String,
    pub platform_key: String,
    pub access_kind: String,
    pub url: Option<String>,
    pub status: String,
    pub source_kind: String,
    pub quote: Option<String>,
    pub fetch_id: Option<String>,
    pub page_date_ms: Option<i64>,
    pub note: Option<String>,
    pub bot_id: Option<String>,
    pub checked_ms: i64,
}

const AVAIL_COLS: &str = "id, movie_id, region, platform, platform_key, access_kind, url, status, source_kind, quote, fetch_id, page_date_ms, note, bot_id, checked_ms";

fn avail_row(r: &Row) -> rusqlite::Result<Availability> {
    Ok(Availability {
        id: r.get(0)?,
        movie_id: r.get(1)?,
        region: r.get(2)?,
        platform: r.get(3)?,
        platform_key: r.get(4)?,
        access_kind: r.get(5)?,
        url: r.get(6)?,
        status: r.get(7)?,
        source_kind: r.get(8)?,
        quote: r.get(9)?,
        fetch_id: r.get(10)?,
        page_date_ms: r.get(11)?,
        note: r.get(12)?,
        bot_id: r.get(13)?,
        checked_ms: r.get(14)?,
    })
}

pub fn insert_availability(db: &AssistDb, a: &Availability) -> Result<(), String> {
    db.with(|c| {
        c.execute(
            &format!("INSERT INTO reading_availability({AVAIL_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"),
            params![
                a.id, a.movie_id, a.region, a.platform, a.platform_key, a.access_kind, a.url, a.status,
                a.source_kind, a.quote, a.fetch_id, a.page_date_ms, a.note, a.bot_id, a.checked_ms
            ],
        )
    })?;
    Ok(())
}

pub fn availability_of(db: &AssistDb, movie: &str) -> Result<Vec<Availability>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {AVAIL_COLS} FROM reading_availability WHERE movie_id = ?1 ORDER BY checked_ms DESC, rowid DESC"
        ))?;
        let rows = st.query_map([movie], avail_row)?;
        rows.collect()
    })
}

pub fn get_availability(db: &AssistDb, id: &str) -> Result<Availability, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {AVAIL_COLS} FROM reading_availability WHERE id = ?1"),
            [id],
            avail_row,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_READING_NOT_FOUND}: no availability record `{id}`"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subtitle {
    pub id: String,
    pub movie_id: String,
    pub availability_id: Option<String>,
    pub platform_key: String,
    pub language: String,
    pub kind: String,
    pub status: String,
    pub url: Option<String>,
    pub source_kind: String,
    pub quote: Option<String>,
    pub fetch_id: Option<String>,
    pub login_required: bool,
    pub note: Option<String>,
    pub bot_id: Option<String>,
    pub checked_ms: i64,
}

const SUB_COLS: &str = "id, movie_id, availability_id, platform_key, language, kind, status, url, source_kind, quote, fetch_id, login_required, note, bot_id, checked_ms";

fn sub_row(r: &Row) -> rusqlite::Result<Subtitle> {
    Ok(Subtitle {
        id: r.get(0)?,
        movie_id: r.get(1)?,
        availability_id: r.get(2)?,
        platform_key: r.get(3)?,
        language: r.get(4)?,
        kind: r.get(5)?,
        status: r.get(6)?,
        url: r.get(7)?,
        source_kind: r.get(8)?,
        quote: r.get(9)?,
        fetch_id: r.get(10)?,
        login_required: r.get::<_, i64>(11)? != 0,
        note: r.get(12)?,
        bot_id: r.get(13)?,
        checked_ms: r.get(14)?,
    })
}

pub fn insert_subtitle(db: &AssistDb, s: &Subtitle) -> Result<(), String> {
    db.with(|c| {
        c.execute(
            &format!("INSERT INTO reading_subtitles({SUB_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"),
            params![
                s.id, s.movie_id, s.availability_id, s.platform_key, s.language, s.kind, s.status, s.url,
                s.source_kind, s.quote, s.fetch_id, s.login_required as i64, s.note, s.bot_id, s.checked_ms
            ],
        )
    })?;
    Ok(())
}

pub fn subtitles_of(db: &AssistDb, movie: &str) -> Result<Vec<Subtitle>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {SUB_COLS} FROM reading_subtitles WHERE movie_id = ?1 ORDER BY checked_ms DESC, rowid DESC"
        ))?;
        let rows = st.query_map([movie], sub_row)?;
        rows.collect()
    })
}

pub fn get_subtitle(db: &AssistDb, id: &str) -> Result<Subtitle, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {SUB_COLS} FROM reading_subtitles WHERE id = ?1"),
            [id],
            sub_row,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_READING_NOT_FOUND}: no subtitle record `{id}`"))
}

// ── Viewing events ───────────────────────────────────────────────────────

pub const EVENT_KINDS: &[&str] = &["recommended", "planned", "watched", "abandoned", "declined"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViewingEvent {
    pub id: String,
    pub journey_id: String,
    pub movie_id: String,
    pub round_item_id: Option<String>,
    pub kind: String,
    pub reaction: Option<String>,
    pub reasons: Vec<String>,
    pub origin: String,
    pub created_ms: i64,
}

const EVENT_COLS: &str =
    "id, journey_id, movie_id, round_item_id, kind, reaction, reasons_json, origin, created_ms";

fn event_row(r: &Row) -> rusqlite::Result<ViewingEvent> {
    Ok(ViewingEvent {
        id: r.get(0)?,
        journey_id: r.get(1)?,
        movie_id: r.get(2)?,
        round_item_id: r.get(3)?,
        kind: r.get(4)?,
        reaction: r.get(5)?,
        reasons: json_list(&r.get::<_, String>(6)?),
        origin: r.get(7)?,
        created_ms: r.get(8)?,
    })
}

pub fn insert_event(c: &Connection, e: &ViewingEvent) -> rusqlite::Result<usize> {
    c.execute(
        &format!(
            "INSERT INTO reading_viewing_events({EVENT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"
        ),
        params![
            e.id,
            e.journey_id,
            e.movie_id,
            e.round_item_id,
            e.kind,
            e.reaction,
            to_json(&e.reasons),
            e.origin,
            e.created_ms
        ],
    )
}

/// Records what the user did with a film. `recommended` is written only by
/// a round; nothing here turns a recommendation into "watched".
#[allow(clippy::too_many_arguments)]
pub fn record_viewing(
    db: &AssistDb,
    ctx: &AssistCtx,
    journey_id: &str,
    movie_id: &str,
    kind: &str,
    reaction: Option<String>,
    reasons: Vec<String>,
    origin: &str,
) -> Result<ViewingEvent, String> {
    let j = get_journey(db, ctx, journey_id)?;
    if !can_change(ctx, &j) {
        return Err(format!(
            "{ERR_READING_SCOPE}: this conversation cannot change that journey"
        ));
    }
    if kind == "recommended" {
        return Err(bad(
            "`recommended` is recorded by reading_record_round, not by hand",
        ));
    }
    if !EVENT_KINDS.contains(&kind) {
        return Err(bad(
            "kind must be one of planned, watched, abandoned, declined",
        ));
    }
    get_movie(db, movie_id)?;
    let item = db.with(|c| {
        c.query_row(
            "SELECT i.id FROM reading_round_items i JOIN reading_rounds r ON r.id = i.round_id
              WHERE r.journey_id = ?1 AND i.movie_id = ?2 ORDER BY r.created_ms DESC LIMIT 1",
            params![journey_id, movie_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
    })?;
    let e = ViewingEvent {
        id: new_id(),
        journey_id: j.id.clone(),
        movie_id: movie_id.into(),
        round_item_id: item,
        kind: kind.into(),
        reaction: clean_opt(reaction),
        reasons: reasons
            .into_iter()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .collect(),
        origin: origin.into(),
        created_ms: now_ms(),
    };
    db.tx(|tx| {
        insert_event(tx, &e).map_err(|x| x.to_string())?;
        touch(tx, &j.id).map_err(|x| x.to_string())?;
        Ok(())
    })?;
    Ok(e)
}

pub fn events_of(db: &AssistDb, journey: &str) -> Result<Vec<ViewingEvent>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {EVENT_COLS} FROM reading_viewing_events WHERE journey_id = ?1 ORDER BY created_ms DESC, rowid DESC"
        ))?;
        let rows = st.query_map([journey], event_row)?;
        rows.collect()
    })
}

/// Forgets the free-text reaction and reasons of one event; the fact that the
/// film was watched stays. Round explanations that cited it stop citing it.
pub fn forget_reaction(db: &AssistDb, ctx: &AssistCtx, event_id: &str) -> Result<(), String> {
    let journey: Option<String> = db.with(|c| {
        c.query_row(
            "SELECT journey_id FROM reading_viewing_events WHERE id = ?1",
            [event_id],
            |r| r.get(0),
        )
        .optional()
    })?;
    let Some(journey) = journey else {
        return Err(format!("{ERR_READING_NOT_FOUND}: no event `{event_id}`"));
    };
    let j = get_journey(db, ctx, &journey)?;
    if !can_change(ctx, &j) {
        return Err(format!(
            "{ERR_READING_SCOPE}: this conversation cannot change that journey"
        ));
    }
    db.with(|c| {
        c.execute(
            "UPDATE reading_viewing_events SET reaction = NULL, reasons_json = '[]' WHERE id = ?1",
            [event_id],
        )
    })?;
    Ok(())
}

/// Folds case and accents so "Eu, Tu, Eles" and "eu tu eles" meet.
fn fold_title(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A memory the user forgot may have been said about a film this journey
/// also keeps a reaction for ("gostei do humor de X"). That copy would put
/// the forgotten words back in the prompt through the reading context, so it
/// goes too: the reaction and reasons of every watched/abandoned event of a
/// film whose title (or original title) the forgotten text names. The fact
/// that the film was watched stays. `journey_scopes` limits it to journeys
/// the caller may change (`None` = the person on the memory screen: all of
/// theirs). Runs inside the caller's transaction. Returns events cleared.
pub fn forget_reactions_mentioned(
    tx: &rusqlite::Transaction,
    principal: &str,
    journey_scopes: Option<&[String]>,
    forgotten_texts: &[String],
) -> rusqlite::Result<usize> {
    let texts: Vec<String> = forgotten_texts
        .iter()
        .map(|t| format!(" {} ", fold_title(t)))
        .collect();
    if texts.iter().all(|t| t.trim().is_empty()) {
        return Ok(0);
    }
    let rows: Vec<(String, String, String, Option<String>)> = {
        let mut s = tx.prepare(
            "SELECT e.id, j.scope, m.title, m.original_title
               FROM reading_viewing_events e
               JOIN reading_journeys j ON j.id = e.journey_id
               JOIN reading_movies m ON m.id = e.movie_id
              WHERE j.principal = ?1 AND e.kind IN ('watched','abandoned')
                AND (e.reaction IS NOT NULL OR e.reasons_json <> '[]')",
        )?;
        let rows = s.query_map([principal], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut cleared = 0;
    let mut named_titles: Vec<(String, String)> = Vec::new(); // (journey scope, folded title)
    for (event, scope, title, original) in rows {
        if let Some(allowed) = journey_scopes {
            if !allowed.iter().any(|a| a == &scope) {
                continue;
            }
        }
        let names: Vec<String> = [Some(title), original]
            .into_iter()
            .flatten()
            .map(|n| fold_title(&n))
            .filter(|n| n.chars().count() >= 3)
            .collect();
        let named = names
            .iter()
            .any(|n| texts.iter().any(|t| t.contains(&format!(" {n} "))));
        if named {
            cleared += tx.execute(
                "UPDATE reading_viewing_events SET reaction = NULL, reasons_json = '[]' WHERE id = ?1",
                [&event],
            )?;
            for n in names {
                if !named_titles.iter().any(|(sc, t)| sc == &scope && t == &n) {
                    named_titles.push((scope.clone(), n));
                }
            }
        }
    }
    redact_round_texts(tx, principal, &named_titles)?;
    Ok(cleared)
}

/// Shown where a sentence citing a forgotten reaction used to be.
pub const REDACTED: &str = "[motivo removido: você pediu para esquecer]";

/// Round explanations written from a reaction ("na linha do que você curtiu
/// em X") keep quoting it after the reaction is forgotten. Every sentence of
/// a round's reason or an item's `why` that names one of these films, in a
/// journey of that scope, goes; an explanation left empty says so instead.
fn redact_round_texts(
    tx: &rusqlite::Transaction,
    principal: &str,
    named: &[(String, String)],
) -> rusqlite::Result<()> {
    if named.is_empty() {
        return Ok(());
    }
    let redact = |text: &str, titles: &[&str]| -> Option<String> {
        let mut kept = Vec::new();
        let mut dropped = false;
        for sentence in split_sentences(text) {
            let folded = format!(" {} ", fold_title(&sentence));
            if titles.iter().any(|t| folded.contains(&format!(" {t} "))) {
                dropped = true;
            } else {
                kept.push(sentence);
            }
        }
        dropped.then(|| {
            if kept.is_empty() {
                REDACTED.to_string()
            } else {
                format!("{} {REDACTED}", kept.join(" "))
            }
        })
    };
    let rounds: Vec<(String, String, Option<String>)> = {
        let mut s = tx.prepare(
            "SELECT r.id, j.scope, r.reason FROM reading_rounds r
               JOIN reading_journeys j ON j.id = r.journey_id WHERE j.principal = ?1",
        )?;
        let rows = s.query_map([principal], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (round, scope, reason) in rounds {
        let titles: Vec<&str> = named
            .iter()
            .filter(|(sc, _)| sc == &scope)
            .map(|(_, t)| t.as_str())
            .collect();
        if titles.is_empty() {
            continue;
        }
        if let Some(new) = reason.as_deref().and_then(|r| redact(r, &titles)) {
            tx.execute(
                "UPDATE reading_rounds SET reason = ?1 WHERE id = ?2",
                params![new, round],
            )?;
        }
        let items: Vec<(String, Option<String>)> = {
            let mut s =
                tx.prepare("SELECT id, why FROM reading_round_items WHERE round_id = ?1")?;
            let rows = s.query_map([&round], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (item, why) in items {
            if let Some(new) = why.as_deref().and_then(|w| redact(w, &titles)) {
                tx.execute(
                    "UPDATE reading_round_items SET why = ?1 WHERE id = ?2",
                    params![new, item],
                )?;
            }
        }
    }
    Ok(())
}

/// Sentences of a short explanation, each with its end mark.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        cur.push(c);
        if matches!(c, '.' | '!' | '?' | ';') {
            let t = cur.trim().to_string();
            if !t.is_empty() {
                out.push(t);
            }
            cur.clear();
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

/// Latest state per film in a journey (events are newest first).
pub fn latest_by_movie(events: &[ViewingEvent]) -> Vec<&ViewingEvent> {
    let mut seen = std::collections::HashSet::new();
    events
        .iter()
        .filter(|e| seen.insert(e.movie_id.clone()))
        .collect()
}

// ── Rounds (rows; validation is in `rules`) ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Round {
    pub id: String,
    pub journey_id: String,
    pub progress_id: Option<String>,
    pub reason: Option<String>,
    pub context_text: Option<String>,
    pub requested_count: Option<i64>,
    pub shortfall_reason: Option<String>,
    pub only_confirmed: bool,
    pub skill_name: Option<String>,
    pub skill_hash: Option<String>,
    pub look_for: Value,
    pub bot_id: Option<String>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundItem {
    pub id: String,
    pub round_id: String,
    pub movie_id: String,
    pub position: i64,
    pub role: String,
    pub connection: String,
    pub why: String,
    pub moods: Vec<String>,
    pub pace: String,
    pub heavy: bool,
    pub depends_on_ending: bool,
    pub resumes_item_id: Option<String>,
    pub questions: Vec<String>,
    pub evidence_event_ids: Vec<String>,
    pub status: String,
    pub missing: Vec<String>,
    pub availability_id: Option<String>,
    pub subtitle_id: Option<String>,
}

const ROUND_COLS: &str = "id, journey_id, progress_id, reason, context_text, requested_count, shortfall_reason, only_confirmed, skill_name, skill_hash, look_for_json, bot_id, created_ms";
const ITEM_COLS: &str = "id, round_id, movie_id, position, role, connection, why, moods_json, pace, heavy, depends_on_ending, resumes_item_id, questions_json, evidence_event_ids_json, status, missing_json, availability_id, subtitle_id";

pub fn insert_round(c: &Connection, r: &Round) -> rusqlite::Result<usize> {
    c.execute(
        &format!("INSERT INTO reading_rounds({ROUND_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"),
        params![
            r.id, r.journey_id, r.progress_id, r.reason, r.context_text, r.requested_count, r.shortfall_reason,
            r.only_confirmed as i64, r.skill_name, r.skill_hash, r.look_for.to_string(), r.bot_id, r.created_ms
        ],
    )
}

pub fn insert_item(c: &Connection, i: &RoundItem) -> rusqlite::Result<usize> {
    c.execute(
        &format!("INSERT INTO reading_round_items({ITEM_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)"),
        params![
            i.id, i.round_id, i.movie_id, i.position, i.role, i.connection, i.why, to_json(&i.moods), i.pace,
            i.heavy as i64, i.depends_on_ending as i64, i.resumes_item_id, to_json(&i.questions),
            to_json(&i.evidence_event_ids), i.status, to_json(&i.missing), i.availability_id, i.subtitle_id
        ],
    )
}

pub fn rounds_of(db: &AssistDb, journey: &str) -> Result<Vec<(Round, Vec<RoundItem>)>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {ROUND_COLS} FROM reading_rounds WHERE journey_id = ?1 ORDER BY created_ms DESC, rowid DESC"
        ))?;
        let rounds: Vec<Round> = st
            .query_map([journey], |r| {
                Ok(Round {
                    id: r.get(0)?,
                    journey_id: r.get(1)?,
                    progress_id: r.get(2)?,
                    reason: r.get(3)?,
                    context_text: r.get(4)?,
                    requested_count: r.get(5)?,
                    shortfall_reason: r.get(6)?,
                    only_confirmed: r.get::<_, i64>(7)? != 0,
                    skill_name: r.get(8)?,
                    skill_hash: r.get(9)?,
                    look_for: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or(Value::Array(vec![])),
                    bot_id: r.get(11)?,
                    created_ms: r.get(12)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        let mut ist = c.prepare(&format!(
            "SELECT {ITEM_COLS} FROM reading_round_items WHERE round_id = ?1 ORDER BY position"
        ))?;
        for r in rounds {
            let items = ist
                .query_map([&r.id], |x| {
                    Ok(RoundItem {
                        id: x.get(0)?,
                        round_id: x.get(1)?,
                        movie_id: x.get(2)?,
                        position: x.get(3)?,
                        role: x.get(4)?,
                        connection: x.get(5)?,
                        why: x.get(6)?,
                        moods: json_list(&x.get::<_, String>(7)?),
                        pace: x.get(8)?,
                        heavy: x.get::<_, i64>(9)? != 0,
                        depends_on_ending: x.get::<_, i64>(10)? != 0,
                        resumes_item_id: x.get(11)?,
                        questions: json_list(&x.get::<_, String>(12)?),
                        evidence_event_ids: json_list(&x.get::<_, String>(13)?),
                        status: x.get(14)?,
                        missing: json_list(&x.get::<_, String>(15)?),
                        availability_id: x.get(16)?,
                        subtitle_id: x.get(17)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.push((r, items));
        }
        Ok(out)
    })
}
