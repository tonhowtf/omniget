//! Structured checkpoints and the context rebuilt from them.
//!
//! A checkpoint is state, not a summary: the original objective, the
//! criteria version, each task's progress, decisions with their origin, what
//! is pending, the digest of every artifact a criterion depends on, the
//! grants and skills (with hashes) in force, the memory records selected, the
//! account/cwd/runtime pins, the last known effect, the budget and the
//! learning overlays pinned for the run. The readable text a resumed turn
//! gets is a projection of that state ([`rebuild_context`]), with an explicit
//! size budget and references instead of logs.
//!
//! Checkpoints are written by the domain at task boundaries and before a
//! compaction (the default policies) — never left to the model to remember.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::diag::redact;
use super::{
    append_event_tx, clip, criteria, effects, get, tasks, verify, EffectState, NewEvent, TaskState,
};
use crate::core::assist::db::AssistDb;
use crate::core::assist::{new_id, now_ms};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskProgress {
    pub id: String,
    pub title: String,
    pub state: String,
    pub attempts: i64,
    pub result_head: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Decision {
    pub at_ms: i64,
    pub what: String,
    /// `user`, `domain`, `policy`.
    pub origin: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillPin {
    pub skill: String,
    pub hash: String,
}

/// What the caller knows that the mission tables do not (grants of the bot,
/// memory selected for the turn, learning overlays pinned for the run).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extras {
    pub grants: Vec<String>,
    pub memory_ids: Vec<String>,
    pub overlays: Vec<Value>,
    pub resume_kind: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointState {
    pub objective: String,
    pub criteria_version: i64,
    pub criteria: Vec<Value>,
    pub progress: Vec<TaskProgress>,
    pub decisions: Vec<Decision>,
    pub pending: Vec<String>,
    pub artifacts: BTreeMap<String, String>,
    pub grants: Vec<String>,
    pub memory_ids: Vec<String>,
    pub skills: Vec<SkillPin>,
    pub overlays: Vec<Value>,
    pub pins: Option<Value>,
    pub last_effect: Option<Value>,
    pub budget: Value,
    pub notes: Vec<String>,
    pub verdict: Option<String>,
    pub resume_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub mission_id: String,
    pub seq: i64,
    pub reason: String,
    pub state: CheckpointState,
    pub created_ms: i64,
}

/// Builds the state from the tables (plus `extras`) and stores it.
pub fn create(
    db: &AssistDb,
    mission_id: &str,
    reason: &str,
    digests: &dyn Fn(&super::Criterion) -> Option<String>,
    extras: &Extras,
) -> Result<Checkpoint, String> {
    let wrapped = super::with_revision(db, mission_id, digests);
    let digests: &dyn Fn(&super::Criterion) -> Option<String> = &wrapped;
    let m = get(db, mission_id)?;
    let crit = criteria(db, mission_id, Some(m.criteria_version))?;
    let ts = tasks(db, mission_id)?;
    let fx = effects(db, mission_id)?;
    let recs = super::receipts(db, mission_id)?;
    let verdict = verify::verdict(&crit, &recs, digests);
    let evs = super::recent_events(db, mission_id, 1000)?;
    let mut decisions = Vec::new();
    let mut notes = Vec::new();
    for ev in &evs {
        let (what, origin) = match ev.kind.as_str() {
            "criteria_revised" => (format!("criteria → v{}", ev.payload["version"]), "user"),
            "task_unblocked" => (
                format!(
                    "task {} unblocked ({})",
                    ev.payload["task"].as_str().unwrap_or(""),
                    ev.payload["note"].as_str().unwrap_or("")
                ),
                "user",
            ),
            "replay_allowed" => ("replay on new pins allowed".to_string(), "user"),
            "context_note" => {
                notes.push(redact(ev.payload["text"].as_str().unwrap_or("")));
                continue;
            }
            "diagnosis" => (
                format!("blocked: {}", ev.payload["summary"].as_str().unwrap_or("")),
                "domain",
            ),
            "policy_blocked" => (
                format!(
                    "policy {} blocked",
                    ev.payload["policy"].as_str().unwrap_or("")
                ),
                "policy",
            ),
            _ => continue,
        };
        decisions.push(Decision {
            at_ms: ev.ts_ms,
            what: clip(&redact(&what), 300),
            origin: origin.into(),
        });
    }
    let skills: Vec<SkillPin> = match &m.bot_id {
        Some(bot) => db.with(|c| {
            let mut st = c.prepare(
                "SELECT skill, hash FROM bots_skill_bindings WHERE bot_id = ?1 ORDER BY skill",
            )?;
            let rows = st.query_map(params![bot], |r| {
                Ok(SkillPin {
                    skill: r.get(0)?,
                    hash: r.get(1)?,
                })
            })?;
            rows.collect()
        })?,
        None => Vec::new(),
    };
    let artifacts: BTreeMap<String, String> = crit
        .iter()
        .map(|c| (c.id.clone(), digests(c).unwrap_or_else(|| "unknown".into())))
        .collect();
    let last_effect = fx
        .iter()
        .max_by_key(|x| x.updated_ms)
        .map(|x| json!({ "key": x.key, "kind": x.kind, "state": x.state, "task": x.task_id }));
    let state = CheckpointState {
        objective: m.objective.clone(),
        criteria_version: m.criteria_version,
        criteria: crit
            .iter()
            .map(|c| json!({ "id": c.id, "title": c.title, "kind": c.kind, "severity": c.severity, "acceptance": c.acceptance }))
            .collect(),
        progress: ts
            .iter()
            .map(|t| TaskProgress {
                id: t.id.clone(),
                title: t.title.clone(),
                state: t.state.as_str().into(),
                attempts: t.attempts,
                // Checkpoints feed other tasks' prompts and bots: redacted
                // before they are stored (C11).
                result_head: t.result.as_deref().map(|r| clip(&redact(r), 400)),
                error: t.error.as_deref().map(|r| clip(&redact(r), 300)),
            })
            .collect(),
        decisions,
        pending: ts
            .iter()
            .filter(|t| !t.state.is_terminal())
            .map(|t| format!("{} [{}]", t.title, t.state.as_str()))
            .collect(),
        artifacts,
        grants: extras.grants.clone(),
        memory_ids: extras.memory_ids.clone(),
        skills,
        overlays: extras.overlays.clone(),
        pins: m.pins.clone(),
        last_effect,
        budget: json!({ "limits": m.budget, "spent": m.spent }),
        notes,
        verdict: Some(verdict.summary()),
        resume_kind: extras.resume_kind.clone(),
    };
    let id = new_id();
    let now = now_ms();
    db.tx(|tx| {
        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM missions_checkpoints WHERE mission_id = ?1",
                params![mission_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO missions_checkpoints (id, mission_id, seq, reason, state, created_ms) VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, mission_id, seq, clip(reason, 200), serde_json::to_string(&state).map_err(|e| e.to_string())?, now],
        )
        .map_err(|e| e.to_string())?;
        append_event_tx(tx, mission_id, NewEvent::new("checkpoint_created", json!({ "seq": seq, "reason": clip(reason, 200) })))?;
        Ok(Checkpoint { id: id.clone(), mission_id: mission_id.to_string(), seq, reason: reason.to_string(), state: state.clone(), created_ms: now })
    })
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Checkpoint> {
    Ok(Checkpoint {
        id: r.get(0)?,
        mission_id: r.get(1)?,
        seq: r.get(2)?,
        reason: r.get(3)?,
        state: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
        created_ms: r.get(5)?,
    })
}

pub fn latest(db: &AssistDb, mission_id: &str) -> Result<Option<Checkpoint>, String> {
    db.with(|c| {
        c.query_row(
            "SELECT id, mission_id, seq, reason, state, created_ms FROM missions_checkpoints WHERE mission_id = ?1 ORDER BY seq DESC LIMIT 1",
            params![mission_id],
            row,
        )
        .optional()
    })
}

pub fn list(db: &AssistDb, mission_id: &str) -> Result<Vec<Checkpoint>, String> {
    db.with(|c| {
        let mut st = c.prepare("SELECT id, mission_id, seq, reason, state, created_ms FROM missions_checkpoints WHERE mission_id = ?1 ORDER BY seq DESC LIMIT 50")?;
        let rows = st.query_map(params![mission_id], row)?;
        rows.collect()
    })
}

/// The user corrects or adds context. It lands in the next checkpoint and
/// in every rebuilt context, marked as coming from the user.
pub fn add_note(db: &AssistDb, mission_id: &str, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("{}: the note is empty", super::ERR_MISSION_INPUT));
    }
    get(db, mission_id)?;
    super::append_event(
        db,
        mission_id,
        NewEvent::new(
            "context_note",
            json!({ "text": clip(text, 2000), "origin": "user" }),
        ),
    )?;
    Ok(())
}

/// Deterministic context for a resumed (or next) turn, within `max_chars`.
/// Order of priority: objective, criteria and their status, user notes,
/// pending tasks, decisions, artifacts, pins; what does not fit is replaced
/// by a reference (`checkpoint <seq>`), never silently dropped.
pub fn rebuild_context(cp: &Checkpoint, max_chars: usize) -> String {
    let s = &cp.state;
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    sections.push(("Objective (verbatim)".into(), vec![s.objective.clone()]));
    let mut crit = vec![format!("criteria version {}", s.criteria_version)];
    for c in &s.criteria {
        crit.push(format!(
            "- [{}] {} ({}, {})",
            c["id"].as_str().unwrap_or(""),
            c["title"].as_str().unwrap_or(""),
            c["kind"].as_str().unwrap_or(""),
            c["severity"].as_str().unwrap_or("")
        ));
    }
    if let Some(v) = &s.verdict {
        crit.push(format!("status: {v}"));
    }
    sections.push((
        "Completion criteria (only these finish the mission)".into(),
        crit,
    ));
    if !s.notes.is_empty() {
        sections.push((
            "Corrections from the user".into(),
            s.notes.iter().map(|n| format!("- {n}")).collect(),
        ));
    }
    let mut tasks = Vec::new();
    for t in &s.progress {
        let mut line = format!("- {} [{}] attempts {}", t.title, t.state, t.attempts);
        if let Some(e) = &t.error {
            line.push_str(&format!(" — {e}"));
        } else if let Some(r) = &t.result_head {
            if t.state == TaskState::Done.as_str() {
                line.push_str(&format!(" — {}", clip(r, 200)));
            }
        }
        tasks.push(line);
    }
    sections.push(("Tasks".into(), tasks));
    if !s.decisions.is_empty() {
        sections.push((
            "Decisions".into(),
            s.decisions
                .iter()
                .rev()
                .take(10)
                .map(|d| format!("- {} (by {})", d.what, d.origin))
                .collect(),
        ));
    }
    if let Some(e) = &s.last_effect {
        let state = e["state"].as_str().unwrap_or("");
        let mut line = format!(
            "last effect {} ({}) is {state}",
            e["key"].as_str().unwrap_or(""),
            e["kind"].as_str().unwrap_or("")
        );
        if state == EffectState::Unknown.as_str() {
            line.push_str(" — check what exists before acting; do not repeat it blindly");
        }
        sections.push(("Effects".into(), vec![line]));
    }
    if !s.skills.is_empty() {
        sections.push((
            "Skills in force".into(),
            s.skills
                .iter()
                .map(|k| format!("- {}@{}", k.skill, &k.hash[..k.hash.len().min(8)]))
                .collect(),
        ));
    }
    let mut out = String::new();
    let mut cut: Vec<String> = Vec::new();
    for (title, lines) in sections {
        let block = format!("## {title}\n{}\n\n", lines.join("\n"));
        if out.len() + block.len() <= max_chars {
            out.push_str(&block);
        } else {
            cut.push(title);
        }
    }
    if !cut.is_empty() {
        let note = format!(
            "(omitted for size: {}; full state in checkpoint {} of mission {})\n",
            cut.join(", "),
            cp.seq,
            cp.mission_id
        );
        if out.len() + note.len() > max_chars {
            let keep = max_chars.saturating_sub(note.len());
            out = clip(&out, keep);
        }
        out.push_str(&note);
    }
    out
}
