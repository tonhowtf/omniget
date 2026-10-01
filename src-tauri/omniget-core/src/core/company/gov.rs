use chrono::{Datelike, TimeZone, Utc};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::heartbeat::{enqueue_wake, WakeSource};
use super::org::{get_company, get_staff};
use super::{
    audit, changed, conflict, forbidden, invalid, new_id, not_found, now_ms, sql, Actor, Change,
    CompanyStore,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub company_id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub requested_by_staff_id: Option<String>,
    pub issue_id: Option<String>,
    pub run_id: Option<String>,
    pub subject_hash: String,
    pub payload: Value,
    pub options: Option<Vec<String>>,
    pub chosen: Option<String>,
    pub state: String,
    pub note: Option<String>,
    pub decided_channel: Option<String>,
    pub decided_ms: Option<i64>,
    pub effect_error: Option<String>,
    pub expires_ms: Option<i64>,
    pub version: i64,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewApproval {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub issue_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub expires_ms: Option<i64>,
}

const APPROVAL_COLS: &str =
    "id, company_id, kind, title, body, requested_by_staff_id, issue_id, run_id,
  subject_hash, payload, options, chosen, state, note, decided_channel, decided_ms, effect_error,
  expires_ms, version, created_ms, updated_ms";

fn approval_row(r: &Row) -> rusqlite::Result<Approval> {
    let payload: String = r.get(9)?;
    let options: Option<String> = r.get(10)?;
    Ok(Approval {
        id: r.get(0)?,
        company_id: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        body: r.get(4)?,
        requested_by_staff_id: r.get(5)?,
        issue_id: r.get(6)?,
        run_id: r.get(7)?,
        subject_hash: r.get(8)?,
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        options: options.and_then(|o| serde_json::from_str(&o).ok()),
        chosen: r.get(11)?,
        state: r.get(12)?,
        note: r.get(13)?,
        decided_channel: r.get(14)?,
        decided_ms: r.get(15)?,
        effect_error: r.get(16)?,
        expires_ms: r.get(17)?,
        version: r.get(18)?,
        created_ms: r.get(19)?,
        updated_ms: r.get(20)?,
    })
}

fn subject_hash(kind: &str, payload: &Value) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update([0]);
    h.update(payload.to_string().as_bytes());
    hex::encode(h.finalize())
}

pub(crate) fn open_approval(
    tx: &Transaction,
    company: &str,
    new: NewApproval,
    actor: &Actor,
    ch: &mut Vec<Change>,
) -> Result<Approval, String> {
    if !matches!(
        new.kind.as_str(),
        "hire_agent"
            | "strategy"
            | "budget_override"
            | "board_request"
            | "decision"
            | "agent_change"
            | "plan"
    ) {
        return Err(invalid(format!("unknown approval kind `{}`", new.kind)));
    }
    if new.title.trim().is_empty() {
        return Err(invalid("an approval needs a title"));
    }
    if new.kind == "decision" && new.options.as_ref().is_none_or(|o| o.len() < 2) {
        return Err(invalid("a decision needs at least two options"));
    }
    let hash = subject_hash(&new.kind, &new.payload);
    if let Some(a) = tx
        .query_row(
            &format!("SELECT {APPROVAL_COLS} FROM approvals WHERE company_id = ?1 AND subject_hash = ?2 AND state IN ('pending','revision_requested')"),
            params![company, hash],
            approval_row,
        )
        .optional()
        .map_err(sql)?
    {
        return Ok(a);
    }
    let id = new_id();
    let now = now_ms();
    tx.execute(
        "INSERT INTO approvals (id, company_id, kind, title, body, requested_by_staff_id, issue_id, run_id,
           subject_hash, payload, options, expires_ms, created_ms, updated_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
        params![
            id,
            company,
            new.kind,
            new.title.trim(),
            new.body,
            actor.staff_id(),
            new.issue_id,
            actor.run_id(),
            hash,
            new.payload.to_string(),
            new.options.as_ref().map(|o| serde_json::to_string(o).unwrap_or_default()),
            new.expires_ms,
            now
        ],
    )
    .map_err(sql)?;
    audit::record(
        tx,
        company,
        actor,
        "approval.requested",
        "approval",
        &id,
        Some(json!({"kind": new.kind, "title": new.title})),
    )?;
    changed(ch, company, "approval", &id);
    tx.query_row(
        &format!("SELECT {APPROVAL_COLS} FROM approvals WHERE id = ?1"),
        params![id],
        approval_row,
    )
    .map_err(sql)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPolicy {
    pub id: String,
    pub company_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub metric: String,
    pub window: String,
    pub amount: f64,
    pub warn_pct: i64,
    pub hard_stop: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewPolicy {
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub metric: String,
    pub window: String,
    pub amount: f64,
    #[serde(default)]
    pub warn_pct: Option<i64>,
    #[serde(default)]
    pub hard_stop: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    pub company_id: String,
    pub policy_id: String,
    pub window_key: String,
    pub kind: String,
    pub observed: f64,
    pub limit_value: f64,
    pub state: String,
    pub approval_id: Option<String>,
    pub created_ms: i64,
    pub resolved_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyStatus {
    pub policy: BudgetPolicy,
    pub window_key: String,
    pub used: f64,
    pub pct: f64,
    pub open_incident: Option<Incident>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostEvent {
    pub provider: String,
    pub model: String,
    pub billing_type: String,
    pub input_tokens: i64,
    pub cached_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Crossing {
    pub kind: String,
    pub scope_type: String,
    pub scope_id: String,
    pub incident_id: String,
}

const POLICY_COLS: &str =
    "id, company_id, scope_type, scope_id, metric, window, amount, warn_pct, hard_stop, enabled";

fn policy_row(r: &Row) -> rusqlite::Result<BudgetPolicy> {
    Ok(BudgetPolicy {
        id: r.get(0)?,
        company_id: r.get(1)?,
        scope_type: r.get(2)?,
        scope_id: r.get(3)?,
        metric: r.get(4)?,
        window: r.get(5)?,
        amount: r.get(6)?,
        warn_pct: r.get(7)?,
        hard_stop: r.get::<_, i64>(8)? != 0,
        enabled: r.get::<_, i64>(9)? != 0,
    })
}

const INCIDENT_COLS: &str = "id, company_id, policy_id, window_key, kind, observed, limit_value, state, approval_id, created_ms, resolved_ms";

fn incident_row(r: &Row) -> rusqlite::Result<Incident> {
    Ok(Incident {
        id: r.get(0)?,
        company_id: r.get(1)?,
        policy_id: r.get(2)?,
        window_key: r.get(3)?,
        kind: r.get(4)?,
        observed: r.get(5)?,
        limit_value: r.get(6)?,
        state: r.get(7)?,
        approval_id: r.get(8)?,
        created_ms: r.get(9)?,
        resolved_ms: r.get(10)?,
    })
}

pub fn window_key(window: &str, at_ms: i64) -> String {
    let t = Utc
        .timestamp_millis_opt(at_ms)
        .single()
        .unwrap_or_else(Utc::now);
    match window {
        "day" => format!("day:{:04}-{:02}-{:02}", t.year(), t.month(), t.day()),
        "month" => format!("month:{:04}-{:02}", t.year(), t.month()),
        _ => "lifetime".into(),
    }
}

fn scope_keys(
    company: &str,
    staff: Option<&str>,
    project: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut v = vec![("company", company.to_string())];
    if let Some(s) = staff {
        v.push(("agent", s.to_string()));
    }
    if let Some(p) = project {
        v.push(("project", p.to_string()));
    }
    v
}

fn bump_rollup(
    tx: &Transaction,
    company: &str,
    scope_key: &str,
    at: i64,
    micros: i64,
    tokens: i64,
    runs: i64,
) -> Result<(), String> {
    for w in ["day", "month", "lifetime"] {
        tx.execute(
            "INSERT INTO spend_rollup (company_id, scope_key, window_key, cost_micros, tokens, runs)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(scope_key, window_key) DO UPDATE SET
               cost_micros = cost_micros + ?4, tokens = tokens + ?5, runs = runs + ?6",
            params![company, scope_key, window_key(w, at), micros, tokens, runs],
        )
        .map_err(sql)?;
    }
    Ok(())
}

fn used(tx: &rusqlite::Connection, p: &BudgetPolicy, at: i64) -> Result<(String, f64), String> {
    let wk = window_key(&p.window, at);
    let key = format!("{}:{}", p.scope_type, p.scope_id);
    let (micros, tokens, runs): (i64, i64, i64) = tx
        .query_row(
            "SELECT cost_micros, tokens, runs FROM spend_rollup WHERE scope_key = ?1 AND window_key = ?2",
            params![key, wk],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or((0, 0, 0));
    let v = match p.metric.as_str() {
        "usd" => micros as f64 / 1e6,
        "tokens" => tokens as f64,
        _ => runs as f64,
    };
    Ok((wk, v))
}

fn applicable(
    tx: &rusqlite::Connection,
    company: &str,
    staff: Option<&str>,
    project: Option<&str>,
) -> Result<Vec<BudgetPolicy>, String> {
    let mut st = tx
        .prepare(&format!(
            "SELECT {POLICY_COLS} FROM budget_policies WHERE company_id = ?1 AND enabled = 1 AND (
               (scope_type = 'company' AND scope_id = ?1) OR (scope_type = 'agent' AND scope_id = ?2)
               OR (scope_type = 'project' AND scope_id = ?3))"
        ))
        .map_err(sql)?;
    let rows = st
        .query_map(params![company, staff, project], policy_row)
        .map_err(sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql)
}

pub(crate) fn invocation_block(
    tx: &rusqlite::Connection,
    company: &str,
    staff: &str,
    project: Option<&str>,
) -> Result<Option<String>, String> {
    let now = now_ms();
    for p in applicable(tx, company, Some(staff), project)? {
        if !p.hard_stop {
            continue;
        }
        let (wk, v) = used(tx, &p, now)?;
        let open: Option<String> = tx
            .query_row(
                "SELECT id FROM budget_incidents WHERE policy_id = ?1 AND window_key = ?2 AND kind = 'hard' AND state = 'open'",
                params![p.id, wk],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        if open.is_some() || v >= p.amount {
            return Ok(Some(format!(
                "budget: {} {} {} at {v:.4}/{}",
                p.scope_type, p.metric, p.window, p.amount
            )));
        }
    }
    Ok(None)
}

pub(crate) fn count_run(
    tx: &Transaction,
    company: &str,
    staff: &str,
    project: Option<&str>,
    at: i64,
) -> Result<(), String> {
    for (kind, id) in scope_keys(company, Some(staff), project) {
        bump_rollup(tx, company, &format!("{kind}:{id}"), at, 0, 0, 1)?;
    }
    Ok(())
}

fn evaluate(
    tx: &Transaction,
    ch: &mut Vec<Change>,
    company: &str,
    staff: Option<&str>,
    project: Option<&str>,
    at: i64,
) -> Result<Vec<Crossing>, String> {
    let mut out = Vec::new();
    let system = Actor::system("budget");
    for p in applicable(tx, company, staff, project)? {
        let (wk, v) = used(tx, &p, at)?;
        let soft_at = p.amount * p.warn_pct as f64 / 100.0;
        let mut kinds = Vec::new();
        if v >= soft_at && p.warn_pct < 100 {
            kinds.push("soft");
        }
        if v >= p.amount && p.hard_stop {
            kinds.push("hard");
        }
        for kind in kinds {
            let id = new_id();
            let inserted = tx
                .execute(
                    "INSERT INTO budget_incidents (id, company_id, policy_id, window_key, kind, observed, limit_value, created_ms)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(policy_id, window_key, kind) DO NOTHING",
                    params![id, company, p.id, wk, kind, v, p.amount, at],
                )
                .map_err(sql)?;
            if inserted == 0 {
                continue;
            }
            if kind == "hard" {
                let a = open_approval(
                    tx,
                    company,
                    NewApproval {
                        kind: "budget_override".into(),
                        title: format!("Budget reached: {} {} ({} per {})", p.scope_type, p.metric, p.amount, p.window),
                        body: format!("Used {v:.4} of {}. Work in this scope is paused until you raise the limit.", p.amount),
                        issue_id: None,
                        payload: json!({"incident_id": id, "policy_id": p.id, "window_key": wk}),
                        options: None,
                        expires_ms: None,
                    },
                    &system,
                    ch,
                )?;
                tx.execute(
                    "UPDATE budget_incidents SET approval_id = ?2 WHERE id = ?1",
                    params![id, a.id],
                )
                .map_err(sql)?;
            }
            audit::record(
                tx,
                company,
                &system,
                &format!("budget.{kind}"),
                "budget_policy",
                &p.id,
                Some(json!({"used": v, "limit": p.amount, "window": wk, "metric": p.metric})),
            )?;
            changed(ch, company, "budget", &p.id);
            out.push(Crossing {
                kind: kind.into(),
                scope_type: p.scope_type.clone(),
                scope_id: p.scope_id.clone(),
                incident_id: id,
            });
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct CostSummary {
    pub window_key: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub runs: i64,
    pub by_agent: Vec<Value>,
    pub by_project: Vec<Value>,
    pub by_model: Vec<Value>,
    pub unpriced_events: i64,
}

impl CompanyStore {
    pub fn approvals(
        &self,
        company: Option<&str>,
        state: Option<&str>,
    ) -> Result<Vec<Approval>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {APPROVAL_COLS} FROM approvals WHERE (?1 IS NULL OR company_id = ?1)
                   AND (?2 IS NULL OR state = ?2 OR (?2 = 'open' AND state IN ('pending','revision_requested')))
                 ORDER BY state NOT IN ('pending','revision_requested'), created_ms DESC LIMIT 500"
            ))?;
            let rows = st.query_map(params![company, state], approval_row)?;
            rows.collect()
        })
    }

    pub fn approval_get(&self, id: &str) -> Result<Approval, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {APPROVAL_COLS} FROM approvals WHERE id = ?1"),
                params![id],
                approval_row,
            )
            .optional()
        })?
        .ok_or_else(|| not_found("approval", id))
    }

    pub fn approval_request(
        &self,
        company: &str,
        new: NewApproval,
        actor: &Actor,
    ) -> Result<Approval, String> {
        self.write(|tx, ch| {
            get_company(tx, company)?;
            if new.kind == "budget_override" && !matches!(actor, Actor::System { .. }) {
                return Err(forbidden(
                    "budget overrides are opened by the budget itself",
                ));
            }
            let mut new = new;
            if new.kind == "strategy" {
                tx.execute(
                    "UPDATE companies SET strategy_state = 'pending' WHERE id = ?1",
                    params![company],
                )
                .map_err(sql)?;
            }
            if new.payload.is_null() {
                new.payload = json!({"title": new.title, "body": new.body});
            }
            open_approval(tx, company, new, actor, ch)
        })
    }

    pub fn approval_decide(
        &self,
        id: &str,
        decision: &str,
        chosen: Option<&str>,
        note: Option<&str>,
        expected_version: Option<i64>,
        actor: &Actor,
    ) -> Result<Approval, String> {
        if !actor.is_user() && decision != "cancel" {
            return Err(forbidden("only the owner decides approvals"));
        }
        self.write(|tx, ch| {
            let a = tx
                .query_row(&format!("SELECT {APPROVAL_COLS} FROM approvals WHERE id = ?1"), params![id], approval_row)
                .optional()
                .map_err(sql)?
                .ok_or_else(|| not_found("approval", id))?;
            if let Some(v) = expected_version {
                if v != a.version {
                    return Err(conflict("the approval changed; reload it"));
                }
            }
            if !matches!(a.state.as_str(), "pending" | "revision_requested") {
                return Err(conflict(format!("already {}", a.state)));
            }
            if decision == "cancel" && actor.staff_id().is_some() && actor.staff_id() != a.requested_by_staff_id.as_deref() {
                return Err(forbidden("only who asked can withdraw a request"));
            }
            let state = match decision {
                "approve" => "approved",
                "reject" => "rejected",
                "revise" => "revision_requested",
                "cancel" => "cancelled",
                _ => return Err(invalid("decide with approve, reject, revise or cancel")),
            };
            if a.kind == "decision" && state == "approved" {
                let opts = a.options.clone().unwrap_or_default();
                match chosen {
                    Some(c) if opts.iter().any(|o| o == c) => {}
                    _ => return Err(invalid("pick one of the options")),
                }
            }
            let now = now_ms();
            let n = tx
                .execute(
                    "UPDATE approvals SET state = ?2, chosen = ?3, note = ?4, decided_channel = ?5, decided_ms = ?6,
                       version = version + 1, updated_ms = ?6 WHERE id = ?1 AND version = ?7",
                    params![a.id, state, chosen, note, actor.channel(), now, a.version],
                )
                .map_err(sql)?;
            if n == 0 {
                return Err(conflict("decided somewhere else"));
            }
            let effect = if state == "approved" {
                apply_effect(tx, ch, &a, chosen)
            } else {
                reject_effect(tx, &a, state)
            };
            let final_state = match &effect {
                Ok(()) if state == "approved" => "applied",
                Ok(()) => state,
                Err(_) => "apply_failed",
            };
            tx.execute(
                "UPDATE approvals SET state = ?2, effect_error = ?3 WHERE id = ?1",
                params![a.id, final_state, effect.as_ref().err()],
            )
            .map_err(sql)?;
            if let Some(s) = &a.requested_by_staff_id {
                if state != "cancelled" {
                    let _ = enqueue_wake(
                        tx,
                        &a.company_id,
                        s,
                        a.issue_id.as_deref(),
                        WakeSource::Approval,
                        &format!("your request \"{}\" was {state}", a.title),
                        json!({"approval_id": a.id, "state": state, "chosen": chosen, "note": note}),
                        0,
                    );
                }
            }
            audit::record(
                tx,
                &a.company_id,
                actor,
                &format!("approval.{state}"),
                "approval",
                &a.id,
                Some(json!({"kind": a.kind, "chosen": chosen, "note": note, "effect": final_state})),
            )?;
            changed(ch, &a.company_id, "approval", &a.id);
            tx.query_row(&format!("SELECT {APPROVAL_COLS} FROM approvals WHERE id = ?1"), params![a.id], approval_row)
                .map_err(sql)
        })
    }

    pub fn expire_approvals(&self, now: i64) -> Result<u32, String> {
        self.write(|tx, ch| {
            let mut st = tx
                .prepare(&format!(
                    "SELECT {APPROVAL_COLS} FROM approvals WHERE state IN ('pending','revision_requested')
                       AND expires_ms IS NOT NULL AND expires_ms <= ?1"
                ))
                .map_err(sql)?;
            let due = st
                .query_map(params![now], approval_row)
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(st);
            for a in &due {
                tx.execute(
                    "UPDATE approvals SET state = 'expired', version = version + 1, updated_ms = ?2 WHERE id = ?1",
                    params![a.id, now],
                )
                .map_err(sql)?;
                if let Some(s) = &a.requested_by_staff_id {
                    let _ = enqueue_wake(tx, &a.company_id, s, a.issue_id.as_deref(), WakeSource::Approval,
                        &format!("your request \"{}\" expired without an answer", a.title),
                        json!({"approval_id": a.id, "state": "expired"}), 0);
                }
                if a.kind == "strategy" {
                    tx.execute("UPDATE companies SET strategy_state = 'none' WHERE id = ?1", params![a.company_id]).map_err(sql)?;
                }
                audit::record(tx, &a.company_id, &Actor::system("approvals"), "approval.expired", "approval", &a.id, None)?;
                changed(ch, &a.company_id, "approval", &a.id);
            }
            Ok(due.len() as u32)
        })
    }

    pub fn policy_upsert(
        &self,
        company: &str,
        p: NewPolicy,
        actor: &Actor,
    ) -> Result<BudgetPolicy, String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner sets budgets"));
        }
        if !matches!(p.scope_type.as_str(), "company" | "agent" | "project") {
            return Err(invalid("scope is company, agent or project"));
        }
        if !matches!(p.metric.as_str(), "usd" | "tokens" | "runs") {
            return Err(invalid("metric is usd, tokens or runs"));
        }
        if !matches!(p.window.as_str(), "day" | "month" | "lifetime") {
            return Err(invalid("window is day, month or lifetime"));
        }
        if !p.amount.is_finite() || p.amount <= 0.0 {
            return Err(invalid("the limit must be above zero"));
        }
        let scope_id = if p.scope_type == "company" {
            company.to_string()
        } else {
            p.scope_id
                .clone()
                .ok_or_else(|| invalid("which agent or project?"))?
        };
        self.write(|tx, ch| {
            get_company(tx, company)?;
            if p.scope_type == "agent" {
                get_staff(tx, company, &scope_id)?;
            }
            let id = new_id();
            tx.execute(
                "INSERT INTO budget_policies (id, company_id, scope_type, scope_id, metric, window, amount, warn_pct, hard_stop, created_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(company_id, scope_type, scope_id, metric, window) DO UPDATE SET
                   amount = ?7, warn_pct = ?8, hard_stop = ?9, enabled = 1",
                params![id, company, p.scope_type, scope_id, p.metric, p.window, p.amount,
                    p.warn_pct.unwrap_or(80).clamp(1, 100), p.hard_stop.unwrap_or(true) as i64, now_ms()],
            )
            .map_err(sql)?;
            let pol = tx
                .query_row(
                    &format!("SELECT {POLICY_COLS} FROM budget_policies WHERE company_id = ?1 AND scope_type = ?2 AND scope_id = ?3 AND metric = ?4 AND window = ?5"),
                    params![company, p.scope_type, scope_id, p.metric, p.window],
                    policy_row,
                )
                .map_err(sql)?;
            let (wk, v) = used(tx, &pol, now_ms())?;
            if v < pol.amount {
                tx.execute(
                    "UPDATE budget_incidents SET state = 'resolved', resolved_ms = ?3 WHERE policy_id = ?1 AND window_key = ?2 AND state = 'open' AND kind = 'hard'",
                    params![pol.id, wk, now_ms()],
                )
                .map_err(sql)?;
            }
            audit::record(tx, company, actor, "budget.policy_set", "budget_policy", &pol.id,
                Some(json!({"scope": pol.scope_type, "metric": pol.metric, "window": pol.window, "amount": pol.amount})))?;
            changed(ch, company, "budget", &pol.id);
            Ok(pol)
        })
    }

    pub fn policy_delete(&self, company: &str, id: &str, actor: &Actor) -> Result<(), String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner sets budgets"));
        }
        self.write(|tx, ch| {
            tx.execute(
                "DELETE FROM budget_policies WHERE company_id = ?1 AND id = ?2",
                params![company, id],
            )
            .map_err(sql)?;
            audit::record(
                tx,
                company,
                actor,
                "budget.policy_deleted",
                "budget_policy",
                id,
                None,
            )?;
            changed(ch, company, "budget", id);
            Ok(())
        })
    }

    pub fn budget_status(&self, company: &str) -> Result<Vec<PolicyStatus>, String> {
        let now = now_ms();
        self.read(|c| {
            let mut st = c.prepare(&format!("SELECT {POLICY_COLS} FROM budget_policies WHERE company_id = ?1 ORDER BY scope_type, metric"))?;
            let pols = st.query_map(params![company], policy_row)?.collect::<Result<Vec<_>, _>>()?;
            let mut out = Vec::new();
            for p in pols {
                let (wk, v) = used(c, &p, now).map_err(rusqlite::Error::InvalidParameterName)?;
                let inc = c
                    .query_row(
                        &format!("SELECT {INCIDENT_COLS} FROM budget_incidents WHERE policy_id = ?1 AND window_key = ?2 AND state = 'open' ORDER BY kind = 'hard' DESC LIMIT 1"),
                        params![p.id, wk],
                        incident_row,
                    )
                    .optional()?;
                out.push(PolicyStatus { pct: if p.amount > 0.0 { v / p.amount * 100.0 } else { 0.0 }, used: v, window_key: wk, open_incident: inc, policy: p });
            }
            Ok(out)
        })
    }

    pub fn incidents(&self, company: &str, open_only: bool) -> Result<Vec<Incident>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {INCIDENT_COLS} FROM budget_incidents WHERE company_id = ?1 AND (?2 = 0 OR state = 'open') ORDER BY created_ms DESC LIMIT 200"
            ))?;
            let rows = st.query_map(params![company, open_only as i64], incident_row)?;
            rows.collect()
        })
    }

    pub fn incident_resolve(
        &self,
        id: &str,
        raise_to: Option<f64>,
        actor: &Actor,
    ) -> Result<Incident, String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner resolves budget incidents"));
        }
        let (company, policy): (String, String) = self
            .read(|c| {
                c.query_row(
                    "SELECT company_id, policy_id FROM budget_incidents WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })?
            .ok_or_else(|| not_found("incident", id))?;
        if let Some(amount) = raise_to {
            let p = self.read(|c| {
                c.query_row(
                    &format!("SELECT {POLICY_COLS} FROM budget_policies WHERE id = ?1"),
                    params![policy],
                    policy_row,
                )
            })?;
            let (_, v) = self
                .read(|c| used(c, &p, now_ms()).map_err(rusqlite::Error::InvalidParameterName))?;
            if amount <= v {
                return Err(invalid(format!(
                    "the new limit must be above what is already used ({v:.4})"
                )));
            }
            self.policy_upsert(
                &company,
                NewPolicy {
                    scope_type: p.scope_type.clone(),
                    scope_id: Some(p.scope_id.clone()),
                    metric: p.metric.clone(),
                    window: p.window.clone(),
                    amount,
                    warn_pct: Some(p.warn_pct),
                    hard_stop: Some(p.hard_stop),
                },
                actor,
            )?;
        }
        self.write(|tx, ch| {
            let state = if raise_to.is_some() { "resolved" } else { "dismissed" };
            tx.execute(
                "UPDATE budget_incidents SET state = ?2, resolved_ms = ?3 WHERE id = ?1 AND state = 'open'",
                params![id, state, now_ms()],
            )
            .map_err(sql)?;
            let appr: Option<String> = tx
                .query_row("SELECT approval_id FROM budget_incidents WHERE id = ?1", params![id], |r| r.get(0))
                .map_err(sql)?;
            if let Some(a) = appr {
                tx.execute(
                    "UPDATE approvals SET state = ?2, decided_ms = ?3, decided_channel = ?4, version = version + 1 WHERE id = ?1 AND state = 'pending'",
                    params![a, if raise_to.is_some() { "applied" } else { "rejected" }, now_ms(), actor.channel()],
                )
                .map_err(sql)?;
                changed(ch, &company, "approval", &a);
            }
            audit::record(tx, &company, actor, &format!("budget.incident_{state}"), "budget_incident", id, raise_to.map(|r| json!({"raised_to": r})))?;
            changed(ch, &company, "budget", id);
            tx.query_row(&format!("SELECT {INCIDENT_COLS} FROM budget_incidents WHERE id = ?1"), params![id], incident_row)
                .map_err(sql)
        })
    }

    pub fn record_cost(&self, run_id: &str, ev: &CostEvent) -> Result<Vec<Crossing>, String> {
        self.write(|tx, ch| {
            let (company, staff, issue): (String, String, Option<String>) = tx
                .query_row("SELECT company_id, staff_id, issue_id FROM runs WHERE id = ?1", params![run_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()
                .map_err(sql)?
                .ok_or_else(|| not_found("run", run_id))?;
            let (project, billing): (Option<String>, Option<String>) = match &issue {
                Some(i) => tx
                    .query_row("SELECT project_id, billing_code FROM issues WHERE id = ?1", params![i], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()
                    .map_err(sql)?
                    .unwrap_or((None, None)),
                None => (None, None),
            };
            let micros = ev.cost_usd.map(|u| (u * 1e6).round() as i64).unwrap_or(0).max(0);
            let status = if ev.cost_usd.is_some() { "reported" } else { "unpriced" };
            let tokens = ev.input_tokens.max(0) + ev.output_tokens.max(0);
            let now = now_ms();
            tx.execute(
                "INSERT INTO cost_events (id, company_id, staff_id, project_id, issue_id, run_id, billing_code, provider, model,
                   billing_type, cost_status, input_tokens, cached_tokens, output_tokens, cost_micros, occurred_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![new_id(), company, staff, project, issue, run_id, billing, ev.provider, ev.model,
                    ev.billing_type, status, ev.input_tokens, ev.cached_tokens, ev.output_tokens, micros, now],
            )
            .map_err(sql)?;
            for (kind, id) in scope_keys(&company, Some(&staff), project.as_deref()) {
                bump_rollup(tx, &company, &format!("{kind}:{id}"), now, micros, tokens, 0)?;
            }
            tx.execute(
                "UPDATE runs SET input_tokens = input_tokens + ?2, cached_tokens = cached_tokens + ?3,
                   output_tokens = output_tokens + ?4, cost_micros = cost_micros + ?5 WHERE id = ?1",
                params![run_id, ev.input_tokens, ev.cached_tokens, ev.output_tokens, micros],
            )
            .map_err(sql)?;
            changed(ch, &company, "cost", run_id);
            evaluate(tx, ch, &company, Some(&staff), project.as_deref(), now)
        })
    }

    pub fn would_exceed(
        &self,
        company: &str,
        staff: &str,
        project: Option<&str>,
        extra_tokens: i64,
        extra_usd: f64,
    ) -> Result<Option<String>, String> {
        let now = now_ms();
        self.read(|c| {
            let pols = applicable(c, company, Some(staff), project)
                .map_err(rusqlite::Error::InvalidParameterName)?;
            for p in pols.into_iter().filter(|p| p.hard_stop) {
                let (_, v) = used(c, &p, now).map_err(rusqlite::Error::InvalidParameterName)?;
                let with = match p.metric.as_str() {
                    "usd" => v + extra_usd,
                    "tokens" => v + extra_tokens as f64,
                    _ => v,
                };
                if with > p.amount {
                    return Ok(Some(format!(
                        "{} {} per {} would pass {} ({with:.4})",
                        p.scope_type, p.metric, p.window, p.amount
                    )));
                }
            }
            Ok(None)
        })
    }

    pub fn budget_block(
        &self,
        company: &str,
        staff: &str,
        project: Option<&str>,
    ) -> Result<Option<String>, String> {
        self.read(|c| {
            invocation_block(c, company, staff, project)
                .map_err(rusqlite::Error::InvalidParameterName)
        })
    }

    pub fn cost_summary(&self, company: &str, window: &str) -> Result<CostSummary, String> {
        let now = now_ms();
        let wk = window_key(window, now);
        let since = match window {
            "day" => Utc::now()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .map(|d| d.and_utc().timestamp_millis())
                .unwrap_or(0),
            "month" => {
                let t = Utc::now();
                Utc.with_ymd_and_hms(t.year(), t.month(), 1, 0, 0, 0)
                    .single()
                    .map(|d| d.timestamp_millis())
                    .unwrap_or(0)
            }
            _ => 0,
        };
        self.read(|c| {
            let (micros, tokens, runs): (i64, i64, i64) = c
                .query_row(
                    "SELECT cost_micros, tokens, runs FROM spend_rollup WHERE scope_key = ?1 AND window_key = ?2",
                    params![format!("company:{company}"), wk],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?
                .unwrap_or((0, 0, 0));
            let group = |sql_text: &str| -> rusqlite::Result<Vec<Value>> {
                let mut st = c.prepare(sql_text)?;
                let rows = st.query_map(params![company, since], |r| {
                    Ok(json!({"key": r.get::<_, Option<String>>(0)?, "name": r.get::<_, Option<String>>(1)?,
                        "cost_usd": r.get::<_, i64>(2)? as f64 / 1e6, "tokens": r.get::<_, i64>(3)?, "events": r.get::<_, i64>(4)?}))
                })?;
                rows.collect()
            };
            let by_agent = group(
                "SELECT e.staff_id, s.name, SUM(e.cost_micros), SUM(e.input_tokens + e.output_tokens), COUNT(*)
                 FROM cost_events e LEFT JOIN staff s ON s.id = e.staff_id
                 WHERE e.company_id = ?1 AND e.occurred_ms >= ?2 GROUP BY e.staff_id ORDER BY 3 DESC, 4 DESC",
            )?;
            let by_project = group(
                "SELECT e.project_id, p.name, SUM(e.cost_micros), SUM(e.input_tokens + e.output_tokens), COUNT(*)
                 FROM cost_events e LEFT JOIN projects p ON p.id = e.project_id
                 WHERE e.company_id = ?1 AND e.occurred_ms >= ?2 GROUP BY e.project_id ORDER BY 3 DESC, 4 DESC",
            )?;
            let by_model = group(
                "SELECT e.model, e.provider, SUM(e.cost_micros), SUM(e.input_tokens + e.output_tokens), COUNT(*)
                 FROM cost_events e WHERE e.company_id = ?1 AND e.occurred_ms >= ?2 GROUP BY e.model ORDER BY 3 DESC, 4 DESC",
            )?;
            let unpriced: i64 = c.query_row(
                "SELECT COUNT(*) FROM cost_events WHERE company_id = ?1 AND occurred_ms >= ?2 AND cost_status = 'unpriced'",
                params![company, since],
                |r| r.get(0),
            )?;
            Ok(CostSummary { window_key: wk, cost_usd: micros as f64 / 1e6, tokens, runs, by_agent, by_project, by_model, unpriced_events: unpriced })
        })
    }
}

fn apply_effect(
    tx: &Transaction,
    ch: &mut Vec<Change>,
    a: &Approval,
    chosen: Option<&str>,
) -> Result<(), String> {
    let now = now_ms();
    match a.kind.as_str() {
        "hire_agent" => {
            let sid = a
                .payload
                .get("staff_id")
                .and_then(Value::as_str)
                .ok_or("no staff in the request")?;
            let n = tx
                .execute(
                    "UPDATE staff SET status = 'idle', updated_ms = ?2 WHERE id = ?1 AND status = 'pending_approval'",
                    params![sid, now],
                )
                .map_err(sql)?;
            if n == 0 {
                return Err("the hire is no longer pending".into());
            }
            super::routines::schedule_staff_heartbeat(tx, &a.company_id, sid, now)?;
            changed(ch, &a.company_id, "staff", sid);
        }
        "strategy" => {
            tx.execute(
                "UPDATE companies SET strategy_state = 'approved', updated_ms = ?2 WHERE id = ?1",
                params![a.company_id, now],
            )
            .map_err(sql)?;
            changed(ch, &a.company_id, "company", &a.company_id);
        }
        "plan" => {
            let iid = a
                .payload
                .get("issue_id")
                .and_then(Value::as_str)
                .ok_or("no issue in the request")?;
            let rev = a.payload.get("rev").and_then(Value::as_i64);
            let current: Option<i64> = tx
                .query_row(
                    "SELECT rev FROM documents WHERE issue_id = ?1 AND key = 'plan'",
                    params![iid],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?;
            if current != rev {
                return Err(format!(
                    "the plan changed since (now revision {})",
                    current.unwrap_or(0)
                ));
            }
            tx.execute(
                "UPDATE issues SET work_mode = 'standard', seq = seq + 1, updated_ms = ?2 WHERE id = ?1",
                params![iid, now],
            )
            .map_err(sql)?;
            changed(ch, &a.company_id, "issue", iid);
        }
        "budget_override" => {
            if let Some(inc) = a.payload.get("incident_id").and_then(Value::as_str) {
                tx.execute(
                    "UPDATE budget_incidents SET state = 'dismissed', resolved_ms = ?2 WHERE id = ?1 AND state = 'open'",
                    params![inc, now],
                )
                .map_err(sql)?;
            }
        }
        "decision" | "board_request" | "agent_change" => {
            let _ = chosen;
        }
        _ => {}
    }
    Ok(())
}

fn reject_effect(tx: &Transaction, a: &Approval, state: &str) -> Result<(), String> {
    if state == "revision_requested" {
        return Ok(());
    }
    match a.kind.as_str() {
        "hire_agent" => {
            if let Some(sid) = a.payload.get("staff_id").and_then(Value::as_str) {
                tx.execute(
                    "UPDATE staff SET status = 'terminated', pause_reason = 'hire rejected', updated_ms = ?2
                     WHERE id = ?1 AND status = 'pending_approval'",
                    params![sid, now_ms()],
                )
                .map_err(sql)?;
            }
        }
        "strategy" => {
            tx.execute(
                "UPDATE companies SET strategy_state = 'none' WHERE id = ?1",
                params![a.company_id],
            )
            .map_err(sql)?;
        }
        _ => {}
    }
    Ok(())
}

pub const SECRET_NS: &str = "company";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretBinding {
    pub staff_id: String,
    pub env_key: String,
    pub secret_name: String,
    pub has_value: bool,
}

fn valid_env(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 64
        && k.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !k.starts_with(|c: char| c.is_ascii_digit())
}

impl CompanyStore {
    pub fn secret_set(
        &self,
        company: &str,
        name: &str,
        value: &str,
        actor: &Actor,
    ) -> Result<(), String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner sets secret values"));
        }
        if name.trim().is_empty() {
            return Err(invalid("a secret needs a name"));
        }
        crate::core::secrets::set(SECRET_NS, &format!("{company}:{}", name.trim()), value)?;
        self.write(|tx, ch| {
            audit::record(
                tx,
                company,
                actor,
                "secret.set",
                "secret",
                name.trim(),
                None,
            )?;
            changed(ch, company, "secret", name.trim());
            Ok(())
        })
    }

    pub fn secret_bind(
        &self,
        company: &str,
        staff: &str,
        env_key: &str,
        name: &str,
        actor: &Actor,
    ) -> Result<(), String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner binds secrets"));
        }
        if !valid_env(env_key) {
            return Err(invalid("the variable is UPPER_SNAKE_CASE"));
        }
        self.write(|tx, ch| {
            get_staff(tx, company, staff)?;
            tx.execute(
                "INSERT OR REPLACE INTO secret_bindings (company_id, staff_id, env_key, secret_name) VALUES (?1,?2,?3,?4)",
                params![company, staff, env_key, name.trim()],
            )
            .map_err(sql)?;
            audit::record(tx, company, actor, "secret.bound", "staff", staff, Some(json!({"env": env_key, "secret": name})))?;
            changed(ch, company, "staff", staff);
            Ok(())
        })
    }

    pub fn secret_unbind(
        &self,
        company: &str,
        staff: &str,
        env_key: &str,
        actor: &Actor,
    ) -> Result<(), String> {
        if !actor.is_user() {
            return Err(forbidden("only the owner binds secrets"));
        }
        self.write(|tx, ch| {
            tx.execute(
                "DELETE FROM secret_bindings WHERE company_id = ?1 AND staff_id = ?2 AND env_key = ?3",
                params![company, staff, env_key],
            )
            .map_err(sql)?;
            audit::record(tx, company, actor, "secret.unbound", "staff", staff, Some(json!({"env": env_key})))?;
            changed(ch, company, "staff", staff);
            Ok(())
        })
    }

    pub fn secret_bindings(&self, company: &str) -> Result<Vec<SecretBinding>, String> {
        let rows: Vec<(String, String, String)> = self.read(|c| {
            let mut st = c.prepare("SELECT staff_id, env_key, secret_name FROM secret_bindings WHERE company_id = ?1 ORDER BY staff_id, env_key")?;
            let rows = st.query_map(params![company], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect()
        })?;
        Ok(rows
            .into_iter()
            .map(|(s, e, n)| SecretBinding {
                has_value: crate::core::secrets::get(SECRET_NS, &format!("{company}:{n}"))
                    .ok()
                    .flatten()
                    .is_some(),
                staff_id: s,
                env_key: e,
                secret_name: n,
            })
            .collect())
    }

    pub fn secret_env(&self, company: &str, staff: &str) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for b in self
            .secret_bindings(company)?
            .into_iter()
            .filter(|b| b.staff_id == staff)
        {
            if let Some(v) =
                crate::core::secrets::get(SECRET_NS, &format!("{company}:{}", b.secret_name))?
            {
                out.push((b.env_key.clone(), v));
            }
        }
        if !out.is_empty() {
            let keys: Vec<&String> = out.iter().map(|(k, _)| k).collect();
            let run = self.live_run_of(staff)?.map(|r| r.id);
            self.write(|tx, _| {
                audit::record(
                    tx,
                    company,
                    &Actor::system("secrets"),
                    "secret.resolved",
                    "staff",
                    staff,
                    Some(json!({"env": keys, "run": run})),
                )
            })?;
        }
        Ok(out)
    }
}
