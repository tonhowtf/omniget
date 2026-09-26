//! Procedural learning, kept apart from personal memory.
//!
//! Memory (`assist::memory`) holds what the person said about themselves:
//! preferences, facts, corrections. An explicit preference goes straight
//! there and is used from the next turn on, without asking again. This
//! module holds something else: how a bot does its job. It records
//! observations with provenance (which bot, skill version and overlay were
//! loaded, which task, what feedback, what verified outcome), turns them into
//! versioned candidates for a *local overlay* (extra procedure text appended
//! to a skill for one bot), evaluates a candidate against the baseline on
//! development and holdout cases, and promotes or rolls back by policy.
//!
//! What it refuses, whatever the prompt says:
//! - a model's own words are a hypothesis, never support for a candidate;
//!   duplicates and repeated feedback on the same task count once (A14);
//! - a candidate is promoted only by an evaluation of that exact version on
//!   the current case set that shows no holdout regression (A15); never by a
//!   counter of confidence;
//! - automatic promotion of a shared procedure is off; a bot may opt in to
//!   automatic promotion of its own local, text-only overlays (no new tools,
//!   no scripts);
//! - executable candidates are `unsupported`: there is no verified isolation
//!   to run them in (A17);
//! - revoking an observation invalidates candidates that no longer have
//!   support, and rolls back an active one (A13);
//! - rollback restores the previous overlay without deleting history; a run
//!   that already started keeps the version it pinned (A16);
//! - turning learning off stops new observations and proposals; explicit
//!   memory keeps working.

pub mod eval;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};

use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::db::{AssistDb, Migration};
use super::{new_id, now_ms};

pub const ERR_LEARN: &str = "ERR_LEARN";
pub const ERR_LEARN_INPUT: &str = "ERR_LEARN_INPUT";
pub const ERR_LEARN_SUPPORT: &str = "ERR_LEARN_SUPPORT";
pub const ERR_LEARN_POLICY: &str = "ERR_LEARN_POLICY";
pub const ERR_LEARN_NOT_FOUND: &str = "ERR_LEARN_NOT_FOUND";
pub const ERR_LEARN_UNSUPPORTED: &str = "ERR_LEARN_UNSUPPORTED";
pub const ERR_LEARN_NO_FIXTURE: &str = "ERR_LEARN_NO_FIXTURE";
pub const ERR_LEARN_DISABLED: &str = "ERR_LEARN_DISABLED";

/// Distinct explicit observations (different tasks) a candidate needs.
pub const MIN_SUPPORT: usize = 2;
/// Longest overlay text.
pub const OVERLAY_MAX: usize = 4_000;

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "learning",
    version: 1,
    sql: "
CREATE TABLE learn_settings (
    bot_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1,
    auto_promote_local INTEGER NOT NULL DEFAULT 0,
    policy TEXT NOT NULL DEFAULT '{}',
    updated_ms INTEGER NOT NULL
);
CREATE TABLE learn_observations (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL DEFAULT '',
    skill_hash TEXT,
    overlay_version TEXT,
    task_ref TEXT NOT NULL DEFAULT '',
    scope TEXT NOT NULL,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    polarity INTEGER NOT NULL DEFAULT 0,
    outcome TEXT NOT NULL DEFAULT 'unverified',
    evidence_ref TEXT,
    content_hash TEXT NOT NULL,
    dup_of TEXT,
    revoked_ms INTEGER,
    revoked_reason TEXT,
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_observations_bot ON learn_observations(bot_id, skill, created_ms DESC);
CREATE INDEX learn_observations_hash ON learn_observations(content_hash);
CREATE TABLE learn_candidates (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    scope TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'overlay',
    title TEXT NOT NULL,
    state TEXT NOT NULL,
    current_version INTEGER NOT NULL DEFAULT 1,
    reason TEXT,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX learn_candidates_bot ON learn_candidates(bot_id, skill, state);
CREATE TABLE learn_candidate_versions (
    candidate_id TEXT NOT NULL REFERENCES learn_candidates(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    overlay TEXT NOT NULL,
    diff TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL DEFAULT '',
    observations TEXT NOT NULL DEFAULT '[]',
    created_ms INTEGER NOT NULL,
    PRIMARY KEY (candidate_id, version)
);
CREATE TABLE learn_eval_cases (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    split TEXT NOT NULL CHECK (split IN ('dev','holdout')),
    name TEXT NOT NULL,
    input TEXT NOT NULL,
    checks TEXT NOT NULL DEFAULT '[]',
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_eval_cases_bot ON learn_eval_cases(bot_id, skill, split);
CREATE TABLE learn_fixtures (
    case_id TEXT NOT NULL REFERENCES learn_eval_cases(id) ON DELETE CASCADE,
    variant TEXT NOT NULL,
    output TEXT NOT NULL,
    recorded_ms INTEGER NOT NULL,
    PRIMARY KEY (case_id, variant)
);
CREATE TABLE learn_eval_runs (
    id TEXT PRIMARY KEY,
    candidate_id TEXT NOT NULL,
    candidate_version INTEGER NOT NULL,
    baseline TEXT NOT NULL,
    case_set TEXT NOT NULL,
    mode TEXT NOT NULL,
    policy TEXT NOT NULL,
    results TEXT NOT NULL,
    summary TEXT NOT NULL,
    decision TEXT NOT NULL,
    reasons TEXT NOT NULL DEFAULT '[]',
    meta TEXT NOT NULL DEFAULT '{}',
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_eval_runs_candidate ON learn_eval_runs(candidate_id, created_ms DESC);
CREATE TABLE learn_active (
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    scope TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    since_ms INTEGER NOT NULL,
    PRIMARY KEY (bot_id, skill, scope)
);
CREATE TABLE learn_promotions (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    scope TEXT NOT NULL,
    action TEXT NOT NULL,
    candidate_id TEXT,
    version INTEGER,
    previous TEXT,
    eval_run_id TEXT,
    by_whom TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_promotions_bot ON learn_promotions(bot_id, skill, scope, created_ms DESC);
",
}];

// ── Types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The person said it (a reaction, a correction, a rating).
    ExplicitFeedback,
    /// Derived by the app from something the person did.
    Inferred,
    /// Came with an imported pack.
    Import,
    /// A model's own statement: a hypothesis, never support.
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Proposed,
    Evaluating,
    Eligible,
    Active,
    Rejected,
    Reverted,
}

impl CandidateState {
    pub fn as_str(self) -> &'static str {
        match self {
            CandidateState::Proposed => "proposed",
            CandidateState::Evaluating => "evaluating",
            CandidateState::Eligible => "eligible",
            CandidateState::Active => "active",
            CandidateState::Rejected => "rejected",
            CandidateState::Reverted => "reverted",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "evaluating" => CandidateState::Evaluating,
            "eligible" => CandidateState::Eligible,
            "active" => CandidateState::Active,
            "rejected" => CandidateState::Rejected,
            "reverted" => CandidateState::Reverted,
            _ => CandidateState::Proposed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub bot_id: String,
    pub enabled: bool,
    /// Opt-in: promote a bot-local, text-only overlay when its evaluation is
    /// eligible, without asking.
    pub auto_promote_local: bool,
    pub policy: eval::PromotionPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub bot_id: String,
    pub skill: String,
    pub skill_hash: Option<String>,
    pub overlay_version: Option<String>,
    pub task_ref: String,
    pub scope: String,
    pub source: Source,
    /// `preference|procedure|fact|hypothesis`
    pub kind: String,
    pub text: String,
    pub polarity: i64,
    /// `verified_pass|verified_fail|unverified`
    pub outcome: String,
    pub evidence_ref: Option<String>,
    pub content_hash: String,
    pub dup_of: Option<String>,
    pub revoked_ms: Option<i64>,
    pub revoked_reason: Option<String>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewObservation {
    pub bot_id: String,
    pub skill: String,
    pub skill_hash: Option<String>,
    pub overlay_version: Option<String>,
    pub task_ref: String,
    pub scope: Option<String>,
    pub source: Option<Source>,
    pub kind: Option<String>,
    pub text: String,
    pub polarity: i64,
    pub outcome: Option<String>,
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub bot_id: String,
    pub skill: String,
    pub scope: String,
    /// `overlay` (text) or `executable` (script: unsupported).
    pub kind: String,
    pub title: String,
    pub state: CandidateState,
    pub current_version: i64,
    pub reason: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateVersion {
    pub candidate_id: String,
    pub version: i64,
    pub overlay: String,
    pub diff: String,
    pub reason: String,
    pub observations: Vec<String>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveOverlay {
    pub bot_id: String,
    pub skill: String,
    pub scope: String,
    pub candidate_id: String,
    pub version: i64,
    pub overlay: String,
    pub since_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Promotion {
    pub id: String,
    pub bot_id: String,
    pub skill: String,
    pub scope: String,
    /// `promote|rollback|auto_rollback`
    pub action: String,
    pub candidate_id: Option<String>,
    pub version: Option<i64>,
    pub previous: Option<Value>,
    pub eval_run_id: Option<String>,
    pub by_whom: String,
    pub reason: String,
    pub created_ms: i64,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{ERR_LEARN}: {e}")
}

fn input_err(m: impl std::fmt::Display) -> String {
    format!("{ERR_LEARN_INPUT}: {m}")
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn norm(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn hash(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]);
    }
    hex::encode(&h.finalize()[..16])
}

pub fn bot_scope(bot: &str) -> String {
    format!("bot:{bot}")
}

// ── Settings ──────────────────────────────────────────────────────────────

pub fn settings(db: &AssistDb, bot: &str) -> Result<Settings, String> {
    let row: Option<(i64, i64, String)> = db.with(|c| {
        c.query_row(
            "SELECT enabled, auto_promote_local, policy FROM learn_settings WHERE bot_id = ?1",
            params![bot],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
    })?;
    Ok(match row {
        Some((en, auto, pol)) => Settings {
            bot_id: bot.into(),
            enabled: en != 0,
            auto_promote_local: auto != 0,
            policy: serde_json::from_str(&pol).unwrap_or_default(),
        },
        None => Settings {
            bot_id: bot.into(),
            enabled: true,
            auto_promote_local: false,
            policy: eval::PromotionPolicy::default(),
        },
    })
}

pub fn set_settings(db: &AssistDb, s: &Settings) -> Result<Settings, String> {
    s.policy.validate()?;
    db.with(|c| {
        c.execute(
            "INSERT INTO learn_settings(bot_id, enabled, auto_promote_local, policy, updated_ms) VALUES (?1,?2,?3,?4,?5) \
             ON CONFLICT(bot_id) DO UPDATE SET enabled=excluded.enabled, auto_promote_local=excluded.auto_promote_local, policy=excluded.policy, updated_ms=excluded.updated_ms",
            params![s.bot_id, s.enabled, s.auto_promote_local, serde_json::to_string(&s.policy).unwrap_or_default(), now_ms()],
        )
    })?;
    settings(db, &s.bot_id)
}

// ── Observations ──────────────────────────────────────────────────────────

const OBS_COLS: &str = "id, bot_id, skill, skill_hash, overlay_version, task_ref, scope, source, kind, text, polarity, outcome, evidence_ref, content_hash, dup_of, revoked_ms, revoked_reason, created_ms";

fn row_obs(r: &Row<'_>) -> rusqlite::Result<Observation> {
    Ok(Observation {
        id: r.get(0)?,
        bot_id: r.get(1)?,
        skill: r.get(2)?,
        skill_hash: r.get(3)?,
        overlay_version: r.get(4)?,
        task_ref: r.get(5)?,
        scope: r.get(6)?,
        source: serde_json::from_value(Value::String(r.get(7)?)).unwrap_or(Source::Model),
        kind: r.get(8)?,
        text: r.get(9)?,
        polarity: r.get(10)?,
        outcome: r.get(11)?,
        evidence_ref: r.get(12)?,
        content_hash: r.get(13)?,
        dup_of: r.get(14)?,
        revoked_ms: r.get(15)?,
        revoked_reason: r.get(16)?,
        created_ms: r.get(17)?,
    })
}

/// Records one observation. Returns `None` when learning is off for the bot
/// (explicit memory is not affected by that switch). A model statement is
/// stored as a hypothesis; a repeat of the same text on the same task is
/// stored as a duplicate that never counts twice.
pub fn observe(db: &AssistDb, o: NewObservation) -> Result<Option<Observation>, String> {
    if o.bot_id.trim().is_empty() || o.text.trim().is_empty() {
        return Err(input_err("an observation needs a bot and a text"));
    }
    if !settings(db, &o.bot_id)?.enabled {
        return Ok(None);
    }
    let source = o.source.unwrap_or(Source::Inferred);
    let kind = if source == Source::Model {
        "hypothesis".to_string()
    } else {
        o.kind
            .clone()
            .filter(|k| {
                matches!(
                    k.as_str(),
                    "preference" | "procedure" | "fact" | "hypothesis"
                )
            })
            .unwrap_or_else(|| "procedure".into())
    };
    let outcome = o
        .outcome
        .clone()
        .filter(|x| matches!(x.as_str(), "verified_pass" | "verified_fail" | "unverified"))
        .unwrap_or_else(|| "unverified".into());
    let text = super::missions::clip(o.text.trim(), 2000);
    let content_hash = hash(&[&o.bot_id, &o.skill, &norm(&text), &o.task_ref]);
    let scope = o.scope.clone().unwrap_or_else(|| bot_scope(&o.bot_id));
    let id = new_id();
    let now = now_ms();
    db.tx(|tx| {
        let dup: Option<String> = tx
            .query_row(
                "SELECT id FROM learn_observations WHERE content_hash = ?1 AND dup_of IS NULL AND revoked_ms IS NULL ORDER BY created_ms LIMIT 1",
                params![content_hash],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        tx.execute(
            &format!("INSERT INTO learn_observations ({OBS_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,NULL,NULL,?16)"),
            params![
                id, o.bot_id, o.skill, o.skill_hash, o.overlay_version, o.task_ref, scope,
                enum_str(&source), kind, text, o.polarity.clamp(-1, 1), outcome, o.evidence_ref,
                content_hash, dup, now
            ],
        )
        .map_err(err)?;
        Ok(())
    })?;
    observation(db, &id).map(Some)
}

pub fn observation(db: &AssistDb, id: &str) -> Result<Observation, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {OBS_COLS} FROM learn_observations WHERE id = ?1"),
            params![id],
            row_obs,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_LEARN_NOT_FOUND}: observation {id}"))
}

pub fn observations(
    db: &AssistDb,
    bot: &str,
    skill: Option<&str>,
    limit: u32,
) -> Result<Vec<Observation>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {OBS_COLS} FROM learn_observations WHERE bot_id = ?1 AND (?2 IS NULL OR skill = ?2) ORDER BY created_ms DESC LIMIT ?3"
        ))?;
        let rows = st.query_map(params![bot, skill, limit.clamp(1, 1000)], row_obs)?;
        rows.collect()
    })
}

/// What an observation set is worth as support for a candidate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Support {
    /// Explicit, not revoked, not duplicate, one per task.
    pub counted: Vec<String>,
    pub duplicates: Vec<String>,
    pub model_statements: Vec<String>,
    pub revoked: Vec<String>,
    pub contrary: Vec<String>,
    pub verified: usize,
    pub distinct_tasks: usize,
}

impl Support {
    pub fn enough(&self) -> bool {
        self.counted.len() >= MIN_SUPPORT && self.counted.len() > self.contrary.len()
    }
}

pub fn support(db: &AssistDb, ids: &[String]) -> Result<Support, String> {
    let mut s = Support::default();
    let mut tasks: HashSet<String> = HashSet::new();
    let mut polarity: Option<i64> = None;
    for id in ids {
        let o = observation(db, id)?;
        if o.revoked_ms.is_some() {
            s.revoked.push(o.id);
            continue;
        }
        if o.source == Source::Model {
            s.model_statements.push(o.id);
            continue;
        }
        if o.dup_of.is_some() {
            s.duplicates.push(o.id);
            continue;
        }
        if o.source != Source::ExplicitFeedback && o.source != Source::Inferred {
            continue;
        }
        let p = polarity.get_or_insert(o.polarity);
        if o.polarity != 0 && *p != 0 && o.polarity != *p {
            s.contrary.push(o.id);
            continue;
        }
        let task = if o.task_ref.is_empty() {
            o.id.clone()
        } else {
            o.task_ref.clone()
        };
        if !tasks.insert(task) {
            s.duplicates.push(o.id);
            continue;
        }
        if o.outcome.starts_with("verified") {
            s.verified += 1;
        }
        if o.source == Source::ExplicitFeedback {
            s.counted.push(o.id);
        }
    }
    s.distinct_tasks = tasks.len();
    Ok(s)
}

/// Stored instead of a revoked observation's reason: the free text the
/// person typed may repeat what they asked to remove (audit F-L2).
pub const REVOKED_LABEL: &str = "revoked by the person";
/// Replaces the overlay text of every candidate version derived from a
/// revoked observation.
pub const WITHDRAWN_OVERLAY: &str =
    "[withdrawn: derived from a revoked observation; revise the candidate to propose new text]";

/// Revokes an observation (the person corrected or deleted what it came
/// from). Returns the candidates touched.
///
/// Support is not the only question (audit F-L2): text derived from the
/// observation may quote it. So every candidate version that cites it has
/// its overlay, diff and reason withdrawn, its recorded fixtures (outputs
/// produced with that text) deleted, it leaves the active overlays (rolled
/// back, repeatedly if the previous one cites it too) and every pinned run,
/// and a live candidate is rejected: with support left it needs review and
/// comes back only through [`revise`] with new text.
pub fn revoke_observation(db: &AssistDb, id: &str, reason: &str) -> Result<Vec<String>, String> {
    revoke_with_label(db, id, reason, REVOKED_LABEL)
}

fn revoke_with_label(
    db: &AssistDb,
    id: &str,
    _reason: &str,
    label: &str,
) -> Result<Vec<String>, String> {
    db.with(|c| {
        c.execute(
            "UPDATE learn_observations SET revoked_ms = ?2, revoked_reason = ?3, text = '[revoked]', content_hash = 'revoked', evidence_ref = NULL WHERE id = ?1 OR dup_of = ?1",
            params![id, now_ms(), label],
        )
    })?;
    // Every version (not only the current one) that cites the observation.
    let affected: Vec<(String, i64)> = db.with(|c| {
        let mut st = c.prepare("SELECT candidate_id, version FROM learn_candidate_versions WHERE observations LIKE ?1 ORDER BY candidate_id, version")?;
        let rows = st.query_map(params![format!("%\"{id}\"%")], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })?;
    if affected.is_empty() {
        return Ok(Vec::new());
    }
    let hit: HashSet<(String, i64)> = affected.iter().cloned().collect();
    db.tx(|tx| {
        for (cid, v) in &affected {
            tx.execute(
                "UPDATE learn_candidate_versions SET overlay = ?3, diff = '', reason = '' WHERE candidate_id = ?1 AND version = ?2",
                params![cid, v, WITHDRAWN_OVERLAY],
            )
            .map_err(err)?;
            for prefix in ["candidate", "baseline"] {
                tx.execute("DELETE FROM learn_fixtures WHERE variant = ?1", params![eval::variant_key(prefix, Some((cid, *v)))]).map_err(err)?;
            }
        }
        Ok(())
    })?;
    // Drop it from the active overlays; a rollback may restore another
    // affected version, so repeat (bounded) until none is active.
    let why = "a source observation was revoked";
    let mut rolled: HashSet<String> = HashSet::new();
    for _ in 0..64 {
        let rows: Vec<(String, String, String, String, i64)> = db.with(|c| {
            let mut st =
                c.prepare("SELECT bot_id, skill, scope, candidate_id, version FROM learn_active")?;
            let rows = st.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect()
        })?;
        let Some((bot, skill, scope, cid, _)) = rows
            .into_iter()
            .find(|(_, _, _, c, v)| hit.contains(&(c.clone(), *v)))
        else {
            break;
        };
        rollback(db, &bot, &skill, &scope, "system", why)?;
        rolled.insert(cid);
    }
    unpin_versions(&hit);
    let mut touched = Vec::new();
    let cids: Vec<String> = {
        let mut v: Vec<String> = affected.iter().map(|(c, _)| c.clone()).collect();
        v.dedup();
        v
    };
    for cid in cids {
        let cand = candidate(db, &cid)?;
        let current_hit = hit.contains(&(cid.clone(), cand.current_version));
        if !current_hit && !rolled.contains(&cid) {
            continue;
        }
        if current_hit && !matches!(cand.state, CandidateState::Rejected) {
            let ver = version(db, &cid, cand.current_version)?;
            let msg = if support(db, &ver.observations)?.enough() {
                "needs review: a source observation was revoked and the text derived from it was withdrawn; revise it with new text to propose it again"
            } else {
                "lost its support: a source observation was revoked"
            };
            set_state(db, &cid, CandidateState::Rejected, msg)?;
        }
        touched.push(cid);
    }
    Ok(touched)
}

/// Memory record changed or forgotten: revoke observations that point at it.
pub fn on_memory_changed(db: &AssistDb, memory_id: &str, why: &str) -> Result<Vec<String>, String> {
    let ids: Vec<String> = db.with(|c| {
        let mut st = c.prepare(
            "SELECT id FROM learn_observations WHERE evidence_ref = ?1 AND revoked_ms IS NULL",
        )?;
        let rows = st.query_map(params![format!("memory:{memory_id}")], |r| r.get(0))?;
        rows.collect()
    })?;
    let mut touched = Vec::new();
    for id in ids {
        for c in revoke_with_label(
            db,
            &id,
            why,
            "revoked: the memory record it came from changed",
        )? {
            if !touched.contains(&c) {
                touched.push(c);
            }
        }
    }
    Ok(touched)
}

// ── Candidates ────────────────────────────────────────────────────────────

const CAND_COLS: &str =
    "id, bot_id, skill, scope, kind, title, state, current_version, reason, created_ms, updated_ms";

fn row_cand(r: &Row<'_>) -> rusqlite::Result<Candidate> {
    Ok(Candidate {
        id: r.get(0)?,
        bot_id: r.get(1)?,
        skill: r.get(2)?,
        scope: r.get(3)?,
        kind: r.get(4)?,
        title: r.get(5)?,
        state: CandidateState::parse(&r.get::<_, String>(6)?),
        current_version: r.get(7)?,
        reason: r.get(8)?,
        created_ms: r.get(9)?,
        updated_ms: r.get(10)?,
    })
}

pub fn candidate(db: &AssistDb, id: &str) -> Result<Candidate, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {CAND_COLS} FROM learn_candidates WHERE id = ?1"),
            params![id],
            row_cand,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_LEARN_NOT_FOUND}: candidate {id}"))
}

pub fn candidates(db: &AssistDb, bot: &str) -> Result<Vec<Candidate>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!("SELECT {CAND_COLS} FROM learn_candidates WHERE bot_id = ?1 ORDER BY updated_ms DESC LIMIT 200"))?;
        let rows = st.query_map(params![bot], row_cand)?;
        rows.collect()
    })
}

pub fn version(db: &AssistDb, cid: &str, v: i64) -> Result<CandidateVersion, String> {
    db.with(|c| {
        c.query_row(
            "SELECT candidate_id, version, overlay, diff, reason, observations, created_ms FROM learn_candidate_versions WHERE candidate_id = ?1 AND version = ?2",
            params![cid, v],
            |r| {
                Ok(CandidateVersion {
                    candidate_id: r.get(0)?,
                    version: r.get(1)?,
                    overlay: r.get(2)?,
                    diff: r.get(3)?,
                    reason: r.get(4)?,
                    observations: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                    created_ms: r.get(6)?,
                })
            },
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_LEARN_NOT_FOUND}: candidate {cid} v{v}"))
}

pub fn versions(db: &AssistDb, cid: &str) -> Result<Vec<CandidateVersion>, String> {
    let n = candidate(db, cid)?.current_version;
    (1..=n).map(|v| version(db, cid, v)).collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewCandidate {
    pub bot_id: String,
    pub skill: String,
    pub scope: Option<String>,
    pub title: String,
    pub overlay: String,
    pub reason: String,
    pub observations: Vec<String>,
    /// `overlay` or `executable`.
    pub kind: Option<String>,
}

/// Line diff between two overlay texts (`-`/`+` lines), for the UI.
pub fn line_diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let sa: HashSet<&str> = a.iter().copied().collect();
    let sb: HashSet<&str> = b.iter().copied().collect();
    let mut out = Vec::new();
    for l in &a {
        if !sb.contains(l) {
            out.push(format!("- {l}"));
        }
    }
    for l in &b {
        if !sa.contains(l) {
            out.push(format!("+ {l}"));
        }
    }
    out.join("\n")
}

/// Text an overlay must not carry: it would widen what the bot can do.
fn widens_capabilities(text: &str) -> Option<&'static str> {
    let low = text.to_lowercase();
    for (needle, why) in [
        ("allowed-tools", "declares tools"),
        ("allowed_tools", "declares tools"),
        ("#!/", "carries a script"),
        ("```sh", "carries a script"),
        ("```bash", "carries a script"),
        ("grant ", "asks for a grant"),
    ] {
        if low.contains(needle) {
            return Some(why);
        }
    }
    None
}

/// Proposes a candidate overlay from observations. Refused without enough
/// explicit support (model statements, duplicates and one task repeated do
/// not count), with learning off, or when the text would widen capabilities.
pub fn propose(db: &AssistDb, n: NewCandidate) -> Result<Candidate, String> {
    if !settings(db, &n.bot_id)?.enabled {
        return Err(format!(
            "{ERR_LEARN_DISABLED}: learning is off for this bot"
        ));
    }
    let overlay = n.overlay.trim();
    if overlay.is_empty() || overlay.len() > OVERLAY_MAX || n.title.trim().is_empty() {
        return Err(input_err(format!(
            "a candidate needs a title and an overlay of 1..{OVERLAY_MAX} bytes"
        )));
    }
    let kind = n.kind.clone().unwrap_or_else(|| "overlay".into());
    if kind == "overlay" {
        if let Some(why) = widens_capabilities(overlay) {
            return Err(format!("{ERR_LEARN_POLICY}: the overlay {why}; an overlay is text only and never widens what the bot may do"));
        }
    }
    let s = support(db, &n.observations)?;
    if !s.enough() {
        return Err(format!(
            "{ERR_LEARN_SUPPORT}: {} explicit observation(s) on distinct tasks (need {MIN_SUPPORT}); ignored: {} model statement(s), {} duplicate(s), {} revoked, {} contrary",
            s.counted.len(), s.model_statements.len(), s.duplicates.len(), s.revoked.len(), s.contrary.len()
        ));
    }
    let scope = n.scope.clone().unwrap_or_else(|| bot_scope(&n.bot_id));
    let base = active(db, &n.bot_id, &n.skill, &scope)?
        .map(|a| a.overlay)
        .unwrap_or_default();
    let id = new_id();
    let now = now_ms();
    db.tx(|tx| {
        tx.execute(
            &format!("INSERT INTO learn_candidates ({CAND_COLS}) VALUES (?1,?2,?3,?4,?5,?6,'proposed',1,?7,?8,?8)"),
            params![id, n.bot_id, n.skill, scope, kind, n.title.trim(), n.reason, now],
        )
        .map_err(err)?;
        insert_version_tx(tx, &id, 1, overlay, &line_diff(&base, overlay), &n.reason, &s.counted)?;
        Ok(())
    })?;
    candidate(db, &id)
}

fn insert_version_tx(
    tx: &Transaction,
    cid: &str,
    v: i64,
    overlay: &str,
    diff: &str,
    reason: &str,
    obs: &[String],
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO learn_candidate_versions (candidate_id, version, overlay, diff, reason, observations, created_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![cid, v, overlay, diff, reason, serde_json::to_string(obs).unwrap_or_default(), now_ms()],
    )
    .map_err(err)?;
    Ok(())
}

/// New version of a candidate (after feedback). Its evaluation starts over.
pub fn revise(
    db: &AssistDb,
    cid: &str,
    overlay: &str,
    reason: &str,
    observations: &[String],
) -> Result<Candidate, String> {
    let c = candidate(db, cid)?;
    if matches!(c.state, CandidateState::Active) {
        return Err(format!(
            "{ERR_LEARN_POLICY}: propose a new candidate instead of changing the active one"
        ));
    }
    if let Some(why) = widens_capabilities(overlay) {
        return Err(format!("{ERR_LEARN_POLICY}: the overlay {why}"));
    }
    let prev = version(db, cid, c.current_version)?;
    let mut obs = prev.observations.clone();
    obs.extend(observations.iter().cloned());
    obs.sort();
    obs.dedup();
    let v = c.current_version + 1;
    db.tx(|tx| {
        insert_version_tx(tx, cid, v, overlay.trim(), &line_diff(&prev.overlay, overlay), reason, &obs)?;
        tx.execute(
            "UPDATE learn_candidates SET current_version = ?2, state = 'proposed', reason = ?3, updated_ms = ?4 WHERE id = ?1",
            params![cid, v, reason, now_ms()],
        )
        .map_err(err)?;
        Ok(())
    })?;
    candidate(db, cid)
}

pub fn set_state(
    db: &AssistDb,
    cid: &str,
    s: CandidateState,
    reason: &str,
) -> Result<Candidate, String> {
    db.with(|c| {
        c.execute(
            "UPDATE learn_candidates SET state = ?2, reason = ?3, updated_ms = ?4 WHERE id = ?1",
            params![
                cid,
                s.as_str(),
                super::missions::clip(reason, 1000),
                now_ms()
            ],
        )
    })?;
    candidate(db, cid)
}

// ── Active overlays, promotion, rollback ──────────────────────────────────

pub fn active(
    db: &AssistDb,
    bot: &str,
    skill: &str,
    scope: &str,
) -> Result<Option<ActiveOverlay>, String> {
    let row: Option<(String, i64, i64)> = db.with(|c| {
        c.query_row(
            "SELECT candidate_id, version, since_ms FROM learn_active WHERE bot_id = ?1 AND skill = ?2 AND scope = ?3",
            params![bot, skill, scope],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
    })?;
    match row {
        None => Ok(None),
        Some((cid, v, since)) => {
            let ver = version(db, &cid, v)?;
            Ok(Some(ActiveOverlay {
                bot_id: bot.into(),
                skill: skill.into(),
                scope: scope.into(),
                candidate_id: cid,
                version: v,
                overlay: ver.overlay,
                since_ms: since,
            }))
        }
    }
}

pub fn active_for_bot(db: &AssistDb, bot: &str) -> Result<Vec<ActiveOverlay>, String> {
    let rows: Vec<(String, String)> = db.with(|c| {
        let mut st =
            c.prepare("SELECT skill, scope FROM learn_active WHERE bot_id = ?1 ORDER BY skill")?;
        let rows = st.query_map(params![bot], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })?;
    let mut out = Vec::new();
    for (skill, scope) in rows {
        if let Some(a) = active(db, bot, &skill, &scope)? {
            out.push(a);
        }
    }
    Ok(out)
}

/// Promotes `candidate` at its current version. `by` is `user` (a click),
/// or `auto` (only for a bot that opted in, a bot-local text overlay, and an
/// eligible evaluation of this exact version on the current case set).
pub fn promote(db: &AssistDb, cid: &str, eval_run_id: &str, by: &str) -> Result<Promotion, String> {
    let c = candidate(db, cid)?;
    if c.kind != "overlay" {
        return Err(format!("{ERR_LEARN_UNSUPPORTED}: executable candidates are not supported (no verified isolation to run them)"));
    }
    let run = eval::run(db, eval_run_id)?;
    if run.candidate_id != cid || run.candidate_version != c.current_version {
        return Err(format!(
            "{ERR_LEARN_POLICY}: that evaluation is of another version; evaluate v{} first",
            c.current_version
        ));
    }
    if run.decision != "eligible" {
        return Err(format!(
            "{ERR_LEARN_POLICY}: the evaluation did not make it eligible: {}",
            run.reasons.join("; ")
        ));
    }
    let current_set = eval::case_set_digest(db, &c.bot_id, &c.skill)?;
    if run.case_set != current_set {
        return Err(format!(
            "{ERR_LEARN_POLICY}: the case set changed since this evaluation; evaluate again"
        ));
    }
    let s = settings(db, &c.bot_id)?;
    if by == "auto" {
        if !s.auto_promote_local {
            return Err(format!(
                "{ERR_LEARN_POLICY}: automatic promotion is off for this bot"
            ));
        }
        if c.scope != bot_scope(&c.bot_id) {
            return Err(format!(
                "{ERR_LEARN_POLICY}: a shared procedure is never promoted automatically"
            ));
        }
    }
    let prev = active(db, &c.bot_id, &c.skill, &c.scope)?;
    let id = new_id();
    let now = now_ms();
    db.tx(|tx| {
        tx.execute(
            "INSERT INTO learn_active (bot_id, skill, scope, candidate_id, version, since_ms) VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT(bot_id, skill, scope) DO UPDATE SET candidate_id = excluded.candidate_id, version = excluded.version, since_ms = excluded.since_ms",
            params![c.bot_id, c.skill, c.scope, cid, c.current_version, now],
        )
        .map_err(err)?;
        if let Some(p) = &prev {
            if p.candidate_id != cid {
                tx.execute("UPDATE learn_candidates SET state = 'eligible', updated_ms = ?2 WHERE id = ?1 AND state = 'active'", params![p.candidate_id, now]).map_err(err)?;
            }
        }
        tx.execute("UPDATE learn_candidates SET state = 'active', updated_ms = ?2 WHERE id = ?1", params![cid, now]).map_err(err)?;
        tx.execute(
            "INSERT INTO learn_promotions (id, bot_id, skill, scope, action, candidate_id, version, previous, eval_run_id, by_whom, reason, created_ms) VALUES (?1,?2,?3,?4,'promote',?5,?6,?7,?8,?9,?10,?11)",
            params![id, c.bot_id, c.skill, c.scope, cid, c.current_version, prev.as_ref().map(|p| json!({ "candidate_id": p.candidate_id, "version": p.version }).to_string()), eval_run_id, by, run.reasons.join("; "), now],
        )
        .map_err(err)?;
        Ok(())
    })?;
    promotion(db, &id)
}

/// Restores what was active before the current overlay (or nothing). The
/// history keeps every row; a run that pinned the old version keeps it.
pub fn rollback(
    db: &AssistDb,
    bot: &str,
    skill: &str,
    scope: &str,
    by: &str,
    reason: &str,
) -> Result<Promotion, String> {
    let cur = active(db, bot, skill, scope)?
        .ok_or_else(|| format!("{ERR_LEARN_NOT_FOUND}: nothing active for {skill} in {scope}"))?;
    // The promotion that made the current one active tells what was before.
    let prev: Option<Value> = db.with(|c| {
        c.query_row(
            "SELECT previous FROM learn_promotions WHERE bot_id = ?1 AND skill = ?2 AND scope = ?3 AND action = 'promote' AND candidate_id = ?4 AND version = ?5 ORDER BY created_ms DESC LIMIT 1",
            params![bot, skill, scope, cur.candidate_id, cur.version],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
    })?
    .flatten()
    .and_then(|s| serde_json::from_str(&s).ok());
    let id = new_id();
    let now = now_ms();
    db.tx(|tx| {
        match prev.as_ref().and_then(|p| Some((p["candidate_id"].as_str()?.to_string(), p["version"].as_i64()?))) {
            Some((pc, pv)) => {
                tx.execute(
                    "UPDATE learn_active SET candidate_id = ?4, version = ?5, since_ms = ?6 WHERE bot_id = ?1 AND skill = ?2 AND scope = ?3",
                    params![bot, skill, scope, pc, pv, now],
                )
                .map_err(err)?;
                tx.execute("UPDATE learn_candidates SET state = 'active', updated_ms = ?2 WHERE id = ?1", params![pc, now]).map_err(err)?;
            }
            None => {
                tx.execute("DELETE FROM learn_active WHERE bot_id = ?1 AND skill = ?2 AND scope = ?3", params![bot, skill, scope]).map_err(err)?;
            }
        }
        tx.execute("UPDATE learn_candidates SET state = 'reverted', reason = ?2, updated_ms = ?3 WHERE id = ?1", params![cur.candidate_id, reason, now]).map_err(err)?;
        tx.execute(
            "INSERT INTO learn_promotions (id, bot_id, skill, scope, action, candidate_id, version, previous, eval_run_id, by_whom, reason, created_ms) VALUES (?1,?2,?3,?4,'rollback',?5,?6,?7,NULL,?8,?9,?10)",
            params![id, bot, skill, scope, cur.candidate_id, cur.version, prev.as_ref().map(|p| p.to_string()), by, reason, now],
        )
        .map_err(err)?;
        Ok(())
    })?;
    promotion(db, &id)
}

pub fn promotion(db: &AssistDb, id: &str) -> Result<Promotion, String> {
    db.with(|c| {
        c.query_row(
            "SELECT id, bot_id, skill, scope, action, candidate_id, version, previous, eval_run_id, by_whom, reason, created_ms FROM learn_promotions WHERE id = ?1",
            params![id],
            row_promo,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_LEARN_NOT_FOUND}: promotion {id}"))
}

fn row_promo(r: &Row<'_>) -> rusqlite::Result<Promotion> {
    Ok(Promotion {
        id: r.get(0)?,
        bot_id: r.get(1)?,
        skill: r.get(2)?,
        scope: r.get(3)?,
        action: r.get(4)?,
        candidate_id: r.get(5)?,
        version: r.get(6)?,
        previous: r
            .get::<_, Option<String>>(7)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        eval_run_id: r.get(8)?,
        by_whom: r.get(9)?,
        reason: r.get(10)?,
        created_ms: r.get(11)?,
    })
}

pub fn promotions(db: &AssistDb, bot: &str) -> Result<Vec<Promotion>, String> {
    db.with(|c| {
        let mut st = c.prepare("SELECT id, bot_id, skill, scope, action, candidate_id, version, previous, eval_run_id, by_whom, reason, created_ms FROM learn_promotions WHERE bot_id = ?1 ORDER BY created_ms DESC LIMIT 100")?;
        let rows = st.query_map(params![bot], row_promo)?;
        rows.collect()
    })
}

// ── Pins: a run keeps the version it started with (A16) ───────────────────

static PINS: RwLock<Option<HashMap<String, Vec<ActiveOverlay>>>> = RwLock::new(None);

/// Pins the bot's active overlays for `conversation` (a mission's run). Until
/// [`unpin`], turns of that conversation use exactly these versions.
pub fn pin(db: &AssistDb, conversation: &str, bot: &str) -> Result<Vec<ActiveOverlay>, String> {
    let now = active_for_bot(db, bot)?;
    PINS.write()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(conversation.to_string(), now.clone());
    Ok(now)
}

/// Pins exactly `overlays` for `conversation` (an evaluation variant), empty
/// meaning "no overlay at all". Dropping the guard unpins it.
pub fn pin_exact(conversation: &str, overlays: Vec<ActiveOverlay>) -> PinGuard {
    PINS.write()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(conversation.to_string(), overlays);
    PinGuard(conversation.to_string())
}

/// Unpins its conversation when dropped (every exit path of a variant run).
pub struct PinGuard(String);

impl Drop for PinGuard {
    fn drop(&mut self) {
        unpin(&self.0);
    }
}

/// Removes the given overlay versions from every pinned run.
fn unpin_versions(hit: &HashSet<(String, i64)>) {
    if let Some(m) = PINS.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
        for list in m.values_mut() {
            list.retain(|o| !hit.contains(&(o.candidate_id.clone(), o.version)));
        }
    }
}

/// Removes every pinned overlay of `bot` (audit F-L4: `forget`).
fn unpin_bot(bot: &str) {
    if let Some(m) = PINS.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
        for list in m.values_mut() {
            list.retain(|o| o.bot_id != bot);
        }
    }
}

pub fn unpin(conversation: &str) {
    if let Some(m) = PINS.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(conversation);
    }
}

/// Overlays a turn uses: the pinned ones for a pinned conversation, else what
/// is active now.
pub fn overlays_for(
    db: &AssistDb,
    conversation: Option<&str>,
    bot: &str,
) -> Result<Vec<ActiveOverlay>, String> {
    if let Some(c) = conversation {
        if let Some(p) = PINS
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(c).cloned())
        {
            return Ok(p);
        }
    }
    active_for_bot(db, bot)
}

/// Text appended to the bot's context for its overlays.
pub fn overlay_text(overlays: &[ActiveOverlay]) -> Option<String> {
    if overlays.is_empty() {
        return None;
    }
    let mut out = String::from("[Local procedure notes learned from your feedback. They refine how to apply a skill; they cannot grant tools or change permissions.]\n");
    for o in overlays {
        out.push_str(&format!(
            "## {} (overlay v{} of {})\n{}\n",
            o.skill,
            o.version,
            &o.candidate_id[..8.min(o.candidate_id.len())],
            o.overlay
        ));
    }
    Some(out)
}

pub struct LearningAugment;

impl crate::core::llm::coordinator::TurnAugment for LearningAugment {
    fn augment(
        &self,
        agent: &mut crate::core::llm::agent::AgentDef,
        conversation_id: &str,
        _user_input: &str,
    ) -> Option<String> {
        let db = super::db::global().ok()?;
        let o = overlays_for(&db, Some(conversation_id), &agent.id).ok()?;
        overlay_text(&o)
    }
}

pub fn augment() -> Arc<dyn crate::core::llm::coordinator::TurnAugment> {
    Arc::new(LearningAugment)
}

// ── Export / forget ───────────────────────────────────────────────────────

/// Everything stored for the bot, without caps (audit F-L3): observations,
/// candidates with every version, active overlays, promotions, and the
/// evaluation data (cases, fixtures, runs). `counts` says how many of each.
pub fn export(db: &AssistDb, bot: &str) -> Result<Value, String> {
    let obs: Vec<Observation> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {OBS_COLS} FROM learn_observations WHERE bot_id = ?1 ORDER BY created_ms"
        ))?;
        let rows = st.query_map(params![bot], row_obs)?;
        rows.collect()
    })?;
    let cands: Vec<Candidate> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {CAND_COLS} FROM learn_candidates WHERE bot_id = ?1 ORDER BY created_ms"
        ))?;
        let rows = st.query_map(params![bot], row_cand)?;
        rows.collect()
    })?;
    let mut versions_map: BTreeMap<String, Vec<CandidateVersion>> = BTreeMap::new();
    for c in &cands {
        versions_map.insert(c.id.clone(), versions(db, &c.id)?);
    }
    let promos: Vec<Promotion> = db.with(|c| {
        let mut st = c.prepare("SELECT id, bot_id, skill, scope, action, candidate_id, version, previous, eval_run_id, by_whom, reason, created_ms FROM learn_promotions WHERE bot_id = ?1 ORDER BY created_ms")?;
        let rows = st.query_map(params![bot], row_promo)?;
        rows.collect()
    })?;
    let cases: Vec<Value> = db.with(|c| {
        let mut st = c.prepare("SELECT id, skill, split, name, input, checks, created_ms FROM learn_eval_cases WHERE bot_id = ?1 ORDER BY created_ms")?;
        let rows = st.query_map(params![bot], |r| {
            Ok(json!({ "id": r.get::<_, String>(0)?, "skill": r.get::<_, String>(1)?, "split": r.get::<_, String>(2)?, "name": r.get::<_, String>(3)?, "input": r.get::<_, String>(4)?, "checks": serde_json::from_str::<Value>(&r.get::<_, String>(5)?).unwrap_or(Value::Null), "created_ms": r.get::<_, i64>(6)? }))
        })?;
        rows.collect()
    })?;
    let fixtures: Vec<Value> = db.with(|c| {
        let mut st = c.prepare("SELECT f.case_id, f.variant, f.output, f.recorded_ms FROM learn_fixtures f JOIN learn_eval_cases k ON k.id = f.case_id WHERE k.bot_id = ?1 ORDER BY f.recorded_ms")?;
        let rows = st.query_map(params![bot], |r| Ok(json!({ "case_id": r.get::<_, String>(0)?, "variant": r.get::<_, String>(1)?, "output": r.get::<_, String>(2)?, "recorded_ms": r.get::<_, i64>(3)? })))?;
        rows.collect()
    })?;
    let mut runs = Vec::new();
    for c in &cands {
        let ids: Vec<String> = db.with(|conn| {
            let mut st = conn.prepare(
                "SELECT id FROM learn_eval_runs WHERE candidate_id = ?1 ORDER BY created_ms",
            )?;
            let rows = st.query_map(params![c.id], |r| r.get(0))?;
            rows.collect()
        })?;
        for id in ids {
            runs.push(eval::run(db, &id)?);
        }
    }
    Ok(json!({
        "format": "omniget.learning/2",
        "bot_id": bot,
        "complete": true,
        "counts": { "observations": obs.len(), "candidates": cands.len(), "promotions": promos.len(), "eval_cases": cases.len(), "fixtures": fixtures.len(), "eval_runs": runs.len() },
        "settings": settings(db, bot)?,
        "observations": obs,
        "candidates": cands,
        "versions": versions_map,
        "active": active_for_bot(db, bot)?,
        "promotions": promos,
        "eval_cases": cases,
        "fixtures": fixtures,
        "eval_runs": runs,
    }))
}

/// Deletes everything learned for the bot that can hold personal text:
/// observations, candidates and their versions, evaluation cases, their
/// fixtures (model outputs) and the evaluation runs of its candidates, and
/// the overlays pinned by running conversations (audit F-L1/F-L4).
/// Promotion records keep ids and actions only (minimal metadata, no content).
pub fn forget(db: &AssistDb, bot: &str) -> Result<usize, String> {
    let n = db.tx(|tx| {
        let n = tx.execute("DELETE FROM learn_observations WHERE bot_id = ?1", params![bot]).map_err(err)?;
        tx.execute("DELETE FROM learn_active WHERE bot_id = ?1", params![bot]).map_err(err)?;
        tx.execute("DELETE FROM learn_eval_runs WHERE candidate_id IN (SELECT id FROM learn_candidates WHERE bot_id = ?1)", params![bot]).map_err(err)?;
        tx.execute("DELETE FROM learn_candidate_versions WHERE candidate_id IN (SELECT id FROM learn_candidates WHERE bot_id = ?1)", params![bot]).map_err(err)?;
        tx.execute("DELETE FROM learn_candidates WHERE bot_id = ?1", params![bot]).map_err(err)?;
        tx.execute("DELETE FROM learn_fixtures WHERE case_id IN (SELECT id FROM learn_eval_cases WHERE bot_id = ?1)", params![bot]).map_err(err)?;
        tx.execute("DELETE FROM learn_eval_cases WHERE bot_id = ?1", params![bot]).map_err(err)?;
        tx.execute("UPDATE learn_promotions SET reason = '', previous = NULL WHERE bot_id = ?1", params![bot]).map_err(err)?;
        Ok(n)
    })?;
    unpin_bot(bot);
    Ok(n)
}
