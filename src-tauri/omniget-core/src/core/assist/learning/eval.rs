//! Baseline vs candidate on the same cases, conditions and budget.
//!
//! Cases are split into `dev` and `holdout`. Each case has an input and a
//! list of trusted checks (pure functions over the output text: contains,
//! excludes, regex, item counts, spoiler terms, JSON keys). An evaluation
//! produces, per case, the output of the baseline (what is active now) and of
//! the candidate version, and runs the checks on both.
//!
//! Outputs come from one of two places:
//! - `offline`: recorded fixtures. A missing fixture is an error for that
//!   case (`unknown`), never a reason to reach the network or a real tool
//!   (A17);
//! - `live`: an [`OutputRunner`] the app provides (a model call inside the
//!   mission budget), recorded as fixtures for replay. Each variant runs with
//!   an exact overlay set: the baseline with what is active, the candidate
//!   with its version *replacing* the active one of the same skill and scope
//!   (never stacked on it). The runner pins that set; nothing active leaks in.
//!
//! The lifecycle write at the end is a compare-and-set on the state the
//! evaluation put the candidate in: a promotion, a rollback, a revision or a
//! newer evaluation in the meantime makes this one stale. A stale run is
//! still recorded, but never changes the candidate's state.
//!
//! The promotion policy is fixed before the run and stored with it: minimum
//! cases per split, minimum improvement on dev, no regression on holdout,
//! no unknown results. Two successes are two successes, not a statistical
//! claim; the summary says the sample size.

use async_trait::async_trait;
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    candidate, err, input_err, set_state, version, ActiveOverlay, CandidateState,
    ERR_LEARN_NO_FIXTURE, ERR_LEARN_POLICY, ERR_LEARN_UNSUPPORTED,
};
use crate::core::assist::db::AssistDb;
use crate::core::assist::{new_id, now_ms};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromotionPolicy {
    pub min_dev_cases: usize,
    pub min_holdout_cases: usize,
    /// Candidate passes on dev minus baseline passes on dev, at least.
    pub min_dev_gain: i64,
    /// No holdout case that passed on the baseline may fail on the candidate.
    pub no_holdout_regression: bool,
    /// Any unknown result (missing fixture, runner error) blocks promotion.
    pub unknown_blocks: bool,
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            min_dev_cases: 2,
            min_holdout_cases: 2,
            min_dev_gain: 1,
            no_holdout_regression: true,
            unknown_blocks: true,
        }
    }
}

impl PromotionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_dev_cases == 0 || self.min_holdout_cases == 0 {
            return Err(input_err(
                "a policy needs at least one dev and one holdout case",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    Contains {
        text: String,
    },
    Excludes {
        text: String,
    },
    Regex {
        pattern: String,
    },
    /// Lines matching `pattern` counted within `[min, max]`.
    CountLines {
        pattern: String,
        min: usize,
        max: usize,
    },
    /// None of these terms (spoilers, banned words), case-insensitive.
    NoTerms {
        terms: Vec<String>,
    },
    /// The output is JSON with these keys (dot paths).
    JsonKeys {
        keys: Vec<String>,
    },
    MaxChars {
        max: usize,
    },
}

impl Check {
    pub fn run(&self, out: &str) -> (bool, String) {
        match self {
            Check::Contains { text } => (
                out.to_lowercase().contains(&text.to_lowercase()),
                format!("contains {text:?}"),
            ),
            Check::Excludes { text } => (
                !out.to_lowercase().contains(&text.to_lowercase()),
                format!("excludes {text:?}"),
            ),
            Check::Regex { pattern } => match Regex::new(pattern) {
                Ok(re) => (re.is_match(out), format!("matches /{pattern}/")),
                Err(e) => (false, format!("bad pattern: {e}")),
            },
            Check::CountLines { pattern, min, max } => match Regex::new(pattern) {
                Ok(re) => {
                    let n = out.lines().filter(|l| re.is_match(l)).count();
                    (
                        n >= *min && n <= *max,
                        format!("{n} line(s) match /{pattern}/, want {min}..={max}"),
                    )
                }
                Err(e) => (false, format!("bad pattern: {e}")),
            },
            Check::NoTerms { terms } => {
                let low = out.to_lowercase();
                let hit: Vec<&String> = terms
                    .iter()
                    .filter(|t| t.len() >= 3 && low.contains(&t.to_lowercase()))
                    .collect();
                (
                    hit.is_empty(),
                    if hit.is_empty() {
                        "no banned term".into()
                    } else {
                        format!("uses {hit:?}")
                    },
                )
            }
            Check::JsonKeys { keys } => match serde_json::from_str::<Value>(out) {
                Err(_) => (false, "not JSON".into()),
                Ok(v) => {
                    let missing: Vec<&String> = keys
                        .iter()
                        .filter(|k| k.split('.').try_fold(&v, |cur, p| cur.get(p)).is_none())
                        .collect();
                    (
                        missing.is_empty(),
                        if missing.is_empty() {
                            "has every key".into()
                        } else {
                            format!("missing {missing:?}")
                        },
                    )
                }
            },
            Check::MaxChars { max } => (out.chars().count() <= *max, format!("≤ {max} chars")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub bot_id: String,
    pub skill: String,
    pub split: String,
    pub name: String,
    pub input: String,
    pub checks: Vec<Check>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewCase {
    pub bot_id: String,
    pub skill: String,
    pub split: String,
    pub name: String,
    pub input: String,
    pub checks: Vec<Check>,
}

pub fn add_case(db: &AssistDb, c: NewCase) -> Result<EvalCase, String> {
    if !matches!(c.split.as_str(), "dev" | "holdout") {
        return Err(input_err("split is dev or holdout"));
    }
    if c.checks.is_empty() || c.name.trim().is_empty() {
        return Err(input_err("a case needs a name and at least one check"));
    }
    let id = new_id();
    db.with(|conn| {
        conn.execute(
            "INSERT INTO learn_eval_cases (id, bot_id, skill, split, name, input, checks, created_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, c.bot_id, c.skill, c.split, c.name.trim(), c.input, serde_json::to_string(&c.checks).unwrap_or_default(), now_ms()],
        )
    })?;
    Ok(cases(db, &c.bot_id, &c.skill)?
        .into_iter()
        .find(|x| x.id == id)
        .expect("inserted"))
}

pub fn delete_case(db: &AssistDb, id: &str) -> Result<(), String> {
    db.with(|c| c.execute("DELETE FROM learn_eval_cases WHERE id = ?1", params![id]))?;
    Ok(())
}

pub fn cases(db: &AssistDb, bot: &str, skill: &str) -> Result<Vec<EvalCase>, String> {
    db.with(|c| {
        let mut st = c.prepare("SELECT id, bot_id, skill, split, name, input, checks, created_ms FROM learn_eval_cases WHERE bot_id = ?1 AND skill = ?2 ORDER BY split, created_ms")?;
        let rows = st.query_map(params![bot, skill], |r| {
            Ok(EvalCase {
                id: r.get(0)?,
                bot_id: r.get(1)?,
                skill: r.get(2)?,
                split: r.get(3)?,
                name: r.get(4)?,
                input: r.get(5)?,
                checks: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                created_ms: r.get(7)?,
            })
        })?;
        rows.collect()
    })
}

/// Digest of the case set (ids + checks): an evaluation is only valid for
/// the set it ran on.
pub fn case_set_digest(db: &AssistDb, bot: &str, skill: &str) -> Result<String, String> {
    let cs = cases(db, bot, skill)?;
    let text: String = cs
        .iter()
        .map(|c| {
            format!(
                "{}|{}|{}\n",
                c.id,
                c.split,
                serde_json::to_string(&c.checks).unwrap_or_default()
            )
        })
        .collect();
    Ok(super::hash(&[&text]))
}

/// Variant key of a fixture: `baseline:<candidate>@<v>` or `baseline:none`,
/// and `candidate:<id>@<v>`.
pub fn variant_key(prefix: &str, overlay: Option<(&str, i64)>) -> String {
    match overlay {
        Some((c, v)) => format!("{prefix}:{c}@{v}"),
        None => format!("{prefix}:none"),
    }
}

pub fn record_fixture(
    db: &AssistDb,
    case_id: &str,
    variant: &str,
    output: &str,
) -> Result<(), String> {
    db.with(|c| {
        c.execute(
            "INSERT INTO learn_fixtures (case_id, variant, output, recorded_ms) VALUES (?1,?2,?3,?4) \
             ON CONFLICT(case_id, variant) DO UPDATE SET output = excluded.output, recorded_ms = excluded.recorded_ms",
            params![case_id, variant, super::super::missions::clip(output, 64 * 1024), now_ms()],
        )
    })?;
    Ok(())
}

fn fixture(db: &AssistDb, case_id: &str, variant: &str) -> Result<Option<String>, String> {
    db.with(|c| {
        c.query_row(
            "SELECT output FROM learn_fixtures WHERE case_id = ?1 AND variant = ?2",
            params![case_id, variant],
            |r| r.get(0),
        )
        .optional()
    })
}

/// Produces an output for a case under exactly `overlays` (empty = no
/// overlay at all). The runner must inject these and only these (the app pins
/// them on the evaluation's conversation), never whatever is active. The app
/// implements it with a budgeted model call; tests with a stub.
#[async_trait]
pub trait OutputRunner: Send + Sync {
    async fn output(&self, case: &EvalCase, overlays: &[ActiveOverlay]) -> Result<String, String>;
    /// Model/runtime/version label recorded with the run.
    fn label(&self) -> String;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Offline,
    Live,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub case_id: String,
    pub name: String,
    pub split: String,
    /// `pass|fail|unknown`
    pub baseline: String,
    pub candidate: String,
    pub baseline_notes: Vec<String>,
    pub candidate_notes: Vec<String>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SplitSummary {
    pub n: usize,
    pub baseline_pass: usize,
    pub candidate_pass: usize,
    pub unknown: usize,
    /// Cases that passed on the baseline and fail on the candidate.
    pub regressions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalRun {
    pub id: String,
    pub candidate_id: String,
    pub candidate_version: i64,
    pub baseline: String,
    pub case_set: String,
    pub mode: Mode,
    pub policy: PromotionPolicy,
    pub results: Vec<CaseResult>,
    pub dev: SplitSummary,
    pub holdout: SplitSummary,
    /// `eligible|rejected|insufficient|unsupported`
    pub decision: String,
    pub reasons: Vec<String>,
    pub meta: Value,
    pub created_ms: i64,
}

fn grade(checks: &[Check], out: Result<String, String>) -> (String, Vec<String>) {
    match out {
        Err(e) => ("unknown".into(), vec![e]),
        Ok(o) => {
            let mut notes = Vec::new();
            let mut ok = true;
            for c in checks {
                let (pass, why) = c.run(&o);
                if !pass {
                    ok = false;
                }
                notes.push(format!("{} {why}", if pass { "✓" } else { "✗" }));
            }
            (if ok { "pass" } else { "fail" }.into(), notes)
        }
    }
}

/// The overlay sets of both variants: the bot's active overlays (baseline),
/// and the same set with the candidate's version in place of the active
/// overlay of its skill and scope (candidate). Other skills' overlays are the
/// same on both sides.
pub fn variant_overlays(
    db: &AssistDb,
    c: &super::Candidate,
    ver: &super::CandidateVersion,
) -> Result<(Vec<ActiveOverlay>, Vec<ActiveOverlay>), String> {
    let active = super::active_for_bot(db, &c.bot_id)?;
    let same_slot = |o: &ActiveOverlay| o.skill == c.skill && o.scope == c.scope;
    // An active candidate is not its own baseline: the baseline is then "no
    // overlay" for that slot (as `base` below says).
    let baseline: Vec<ActiveOverlay> = active
        .iter()
        .filter(|o| !(same_slot(o) && o.candidate_id == c.id))
        .cloned()
        .collect();
    let mut cand: Vec<ActiveOverlay> = active.into_iter().filter(|o| !same_slot(o)).collect();
    cand.push(ActiveOverlay {
        bot_id: c.bot_id.clone(),
        skill: c.skill.clone(),
        scope: c.scope.clone(),
        candidate_id: c.id.clone(),
        version: ver.version,
        overlay: ver.overlay.clone(),
        since_ms: now_ms(),
    });
    Ok((baseline, cand))
}

/// Evaluates the candidate's current version against the active baseline.
/// `runner` is required for `live` and ignored for `offline` (which never
/// calls anything outside the database).
pub async fn evaluate(
    db: &AssistDb,
    cid: &str,
    mode: Mode,
    runner: Option<&dyn OutputRunner>,
) -> Result<EvalRun, String> {
    evaluate_as(db, cid, mode, runner, &new_id()).await
}

/// [`evaluate`] with the run id chosen by the caller (an app that returns the
/// id at once and evaluates in the background).
pub async fn evaluate_as(
    db: &AssistDb,
    cid: &str,
    mode: Mode,
    runner: Option<&dyn OutputRunner>,
    run_id: &str,
) -> Result<EvalRun, String> {
    let c = candidate(db, cid)?;
    let now = now_ms();
    if c.kind != "overlay" {
        let run = EvalRun {
            id: run_id.into(),
            candidate_id: cid.into(),
            candidate_version: c.current_version,
            baseline: "-".into(),
            case_set: String::new(),
            mode,
            policy: PromotionPolicy::default(),
            results: Vec::new(),
            dev: SplitSummary::default(),
            holdout: SplitSummary::default(),
            decision: "unsupported".into(),
            reasons: vec!["executable candidates need verified OS isolation, which this build does not have; nothing was run".into()],
            meta: json!({}),
            created_ms: now,
        };
        store(db, &run)?;
        set_state(
            db,
            cid,
            CandidateState::Rejected,
            &format!("{ERR_LEARN_UNSUPPORTED}: executable candidate"),
        )?;
        return Ok(run);
    }
    if mode == Mode::Live && runner.is_none() {
        return Err(format!(
            "{ERR_LEARN_POLICY}: a live evaluation needs a runner"
        ));
    }
    let settings = super::settings(db, &c.bot_id)?;
    let policy = settings.policy.clone();
    let ver = version(db, cid, c.current_version)?;
    let base = super::active(db, &c.bot_id, &c.skill, &c.scope)?.filter(|a| a.candidate_id != cid);
    let base_key = variant_key(
        "baseline",
        base.as_ref().map(|b| (b.candidate_id.as_str(), b.version)),
    );
    let cand_key = variant_key("candidate", Some((cid, c.current_version)));
    let (base_set, cand_set) = variant_overlays(db, &c, &ver)?;
    let cs = cases(db, &c.bot_id, &c.skill)?;
    let case_set = case_set_digest(db, &c.bot_id, &c.skill)?;
    // The state this evaluation owns: `evaluating`, tagged with its run id so
    // the final write can tell whether anything moved the candidate since.
    // An active candidate stays active while it is measured again.
    let token = format!("evaluating (run {run_id})");
    let owns_state = c.state != CandidateState::Active;
    if owns_state {
        set_state(db, cid, CandidateState::Evaluating, &token)?;
    }
    let mut results = Vec::new();
    for case in &cs {
        let started = std::time::Instant::now();
        let (b_out, c_out) = match mode {
            Mode::Offline => (
                fixture(db, &case.id, &base_key)?.ok_or_else(|| format!("{ERR_LEARN_NO_FIXTURE}: no recorded output for {base_key}; nothing was called")),
                fixture(db, &case.id, &cand_key)?.ok_or_else(|| format!("{ERR_LEARN_NO_FIXTURE}: no recorded output for {cand_key}; nothing was called")),
            ),
            Mode::Live => {
                let r = runner.expect("checked above");
                let b = r.output(case, &base_set).await;
                let k = r.output(case, &cand_set).await;
                if let Ok(o) = &b {
                    record_fixture(db, &case.id, &base_key, o)?;
                }
                if let Ok(o) = &k {
                    record_fixture(db, &case.id, &cand_key, o)?;
                }
                (b, k)
            }
        };
        let (bs, bn) = grade(&case.checks, b_out);
        let (cst, cn) = grade(&case.checks, c_out);
        results.push(CaseResult {
            case_id: case.id.clone(),
            name: case.name.clone(),
            split: case.split.clone(),
            baseline: bs,
            candidate: cst,
            baseline_notes: bn,
            candidate_notes: cn,
            latency_ms: (mode == Mode::Live).then(|| started.elapsed().as_millis() as u64),
        });
    }
    let summarize = |split: &str| -> SplitSummary {
        let mine: Vec<&CaseResult> = results.iter().filter(|r| r.split == split).collect();
        SplitSummary {
            n: mine.len(),
            baseline_pass: mine.iter().filter(|r| r.baseline == "pass").count(),
            candidate_pass: mine.iter().filter(|r| r.candidate == "pass").count(),
            unknown: mine
                .iter()
                .filter(|r| r.baseline == "unknown" || r.candidate == "unknown")
                .count(),
            regressions: mine
                .iter()
                .filter(|r| r.baseline == "pass" && r.candidate != "pass")
                .map(|r| r.name.clone())
                .collect(),
        }
    };
    let dev = summarize("dev");
    let holdout = summarize("holdout");
    let mut reasons = Vec::new();
    let mut insufficient = false;
    if dev.n < policy.min_dev_cases {
        reasons.push(format!(
            "{} dev case(s), the policy needs {}",
            dev.n, policy.min_dev_cases
        ));
        insufficient = true;
    }
    if holdout.n < policy.min_holdout_cases {
        reasons.push(format!(
            "{} holdout case(s), the policy needs {}",
            holdout.n, policy.min_holdout_cases
        ));
        insufficient = true;
    }
    let unknown = dev.unknown + holdout.unknown;
    if policy.unknown_blocks && unknown > 0 {
        reasons.push(format!("{unknown} case(s) have no result (missing fixture or runner error); unknown is not a pass"));
        insufficient = true;
    }
    let gain = dev.candidate_pass as i64 - dev.baseline_pass as i64;
    let mut rejected = false;
    if gain < policy.min_dev_gain {
        reasons.push(format!(
            "dev: candidate {}/{} vs baseline {}/{} (gain {gain}, need {})",
            dev.candidate_pass, dev.n, dev.baseline_pass, dev.n, policy.min_dev_gain
        ));
        rejected = true;
    } else {
        reasons.push(format!(
            "dev: candidate {}/{} vs baseline {}/{}",
            dev.candidate_pass, dev.n, dev.baseline_pass, dev.n
        ));
    }
    if policy.no_holdout_regression
        && (!holdout.regressions.is_empty() || holdout.candidate_pass < holdout.baseline_pass)
    {
        reasons.push(format!(
            "holdout regressed: candidate {}/{} vs baseline {}/{}; cases that got worse: {}",
            holdout.candidate_pass,
            holdout.n,
            holdout.baseline_pass,
            holdout.n,
            if holdout.regressions.is_empty() {
                "-".to_string()
            } else {
                holdout.regressions.join(", ")
            }
        ));
        rejected = true;
    } else {
        reasons.push(format!(
            "holdout: candidate {}/{} vs baseline {}/{}",
            holdout.candidate_pass, holdout.n, holdout.baseline_pass, holdout.n
        ));
    }
    reasons.push(format!("sample: {} dev + {} holdout case(s); this is evidence for these cases, not a general guarantee", dev.n, holdout.n));
    let decision = if insufficient {
        "insufficient"
    } else if rejected {
        "rejected"
    } else {
        "eligible"
    };
    let mut run = EvalRun {
        id: run_id.into(),
        candidate_id: cid.into(),
        candidate_version: c.current_version,
        baseline: base_key,
        case_set,
        mode,
        policy,
        results,
        dev,
        holdout,
        decision: decision.into(),
        reasons,
        meta: json!({ "runner": runner.map(|r| r.label()), "cost": "unknown unless the runner reported it" }),
        created_ms: now,
    };
    let next = match decision {
        "eligible" => CandidateState::Eligible,
        "rejected" => CandidateState::Rejected,
        _ => CandidateState::Proposed,
    };
    // Record the run and, only if the candidate is still in the state this
    // evaluation left it (same version, same `evaluating` token), move it.
    let applied = db.tx(|tx| {
        let applied = owns_state
            && tx
                .execute(
                    "UPDATE learn_candidates SET state = ?2, reason = ?3, updated_ms = ?4 \
                     WHERE id = ?1 AND state = 'evaluating' AND reason = ?5 AND current_version = ?6",
                    params![cid, next.as_str(), super::super::missions::clip(&run.reasons.join("; "), 1000), now_ms(), token, c.current_version],
                )
                .map_err(err)?
                == 1;
        let stale_why = if applied {
            None
        } else if !owns_state {
            Some("the candidate was active when this evaluation started; its state was left as is")
        } else {
            Some("the candidate changed during this evaluation (promoted, rolled back, revised or evaluated again); this run did not change its state")
        };
        if let (Some(why), Value::Object(m)) = (stale_why, &mut run.meta) {
            m.insert("lifecycle".into(), json!("stale"));
            m.insert("lifecycle_note".into(), json!(why));
        } else if let Value::Object(m) = &mut run.meta {
            m.insert("lifecycle".into(), json!("applied"));
        }
        store_conn(tx, &run)?;
        Ok(applied)
    })?;
    // Opt-in automatic promotion of a local, text-only overlay.
    if applied
        && decision == "eligible"
        && settings.auto_promote_local
        && c.scope == super::bot_scope(&c.bot_id)
    {
        super::promote(db, cid, &run.id, "auto")?;
    }
    Ok(run)
}

fn store(db: &AssistDb, r: &EvalRun) -> Result<(), String> {
    db.tx(|tx| store_conn(tx, r))
}

fn store_conn(c: &rusqlite::Connection, r: &EvalRun) -> Result<(), String> {
    {
        c.execute(
            "INSERT INTO learn_eval_runs (id, candidate_id, candidate_version, baseline, case_set, mode, policy, results, summary, decision, reasons, meta, created_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                r.id, r.candidate_id, r.candidate_version, r.baseline, r.case_set,
                serde_json::to_value(r.mode).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                serde_json::to_string(&r.policy).unwrap_or_default(),
                serde_json::to_string(&r.results).unwrap_or_default(),
                json!({ "dev": r.dev, "holdout": r.holdout }).to_string(),
                r.decision,
                serde_json::to_string(&r.reasons).unwrap_or_default(),
                r.meta.to_string(),
                r.created_ms
            ],
        )
    }
    .map_err(err)?;
    Ok(())
}

fn row_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<EvalRun> {
    let summary: Value = serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default();
    Ok(EvalRun {
        id: r.get(0)?,
        candidate_id: r.get(1)?,
        candidate_version: r.get(2)?,
        baseline: r.get(3)?,
        case_set: r.get(4)?,
        mode: serde_json::from_value(Value::String(r.get(5)?)).unwrap_or(Mode::Offline),
        policy: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
        results: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
        dev: serde_json::from_value(summary["dev"].clone()).unwrap_or_default(),
        holdout: serde_json::from_value(summary["holdout"].clone()).unwrap_or_default(),
        decision: r.get(9)?,
        reasons: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or_default(),
        meta: serde_json::from_str(&r.get::<_, String>(11)?).unwrap_or_default(),
        created_ms: r.get(12)?,
    })
}

pub fn run(db: &AssistDb, id: &str) -> Result<EvalRun, String> {
    db.with(|c| {
        c.query_row("SELECT id, candidate_id, candidate_version, baseline, case_set, mode, policy, results, summary, decision, reasons, meta, created_ms FROM learn_eval_runs WHERE id = ?1", params![id], row_run).optional()
    })?
    .ok_or_else(|| format!("{}: eval run {id}", super::ERR_LEARN_NOT_FOUND))
}

pub fn runs_of(db: &AssistDb, cid: &str) -> Result<Vec<EvalRun>, String> {
    db.with(|c| {
        let mut st = c.prepare("SELECT id, candidate_id, candidate_version, baseline, case_set, mode, policy, results, summary, decision, reasons, meta, created_ms FROM learn_eval_runs WHERE candidate_id = ?1 ORDER BY created_ms DESC LIMIT 20")?;
        let rows = st.query_map(params![cid], row_run)?;
        rows.collect()
    })
}
