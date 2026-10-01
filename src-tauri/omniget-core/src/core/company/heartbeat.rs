use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::issues::{get_issue, issue_row, Issue, ISSUE_COLS};
use super::org::{get_company, get_staff, invokable, staff_row, Staff, STAFF_COLS};
use super::{audit, changed, invalid, new_id, not_found, now_ms, sql, Actor, Change, CompanyStore};

pub const MAX_CONTINUATIONS: i64 = 3;
pub const MAX_RETRIES: i64 = 4;
pub const RETRY_BASE_MS: i64 = 15_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeSource {
    Timer,
    Assignment,
    Mention,
    Comment,
    Review,
    Dependency,
    ChildrenDone,
    Approval,
    Routine,
    Manual,
    Retry,
    Continuation,
}

impl WakeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            WakeSource::Timer => "timer",
            WakeSource::Assignment => "assignment",
            WakeSource::Mention => "mention",
            WakeSource::Comment => "comment",
            WakeSource::Review => "review",
            WakeSource::Dependency => "dependency",
            WakeSource::ChildrenDone => "children_done",
            WakeSource::Approval => "approval",
            WakeSource::Routine => "routine",
            WakeSource::Manual => "manual",
            WakeSource::Retry => "retry",
            WakeSource::Continuation => "continuation",
        }
    }
    fn weight(self) -> i64 {
        match self {
            WakeSource::Manual | WakeSource::Comment | WakeSource::Approval => 0,
            WakeSource::Mention | WakeSource::Review => 10,
            WakeSource::Assignment | WakeSource::Dependency | WakeSource::ChildrenDone => 20,
            WakeSource::Routine | WakeSource::Retry | WakeSource::Continuation => 30,
            WakeSource::Timer => 40,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wake {
    pub id: String,
    pub company_id: String,
    pub staff_id: String,
    pub issue_id: Option<String>,
    pub source: String,
    pub reason: String,
    pub context: Value,
    pub priority: i64,
    pub state: String,
    pub run_id: Option<String>,
    pub not_before_ms: i64,
    pub skip_reason: Option<String>,
    pub created_ms: i64,
}

const WAKE_COLS: &str =
    "id, company_id, staff_id, issue_id, source, reason, context, priority, state,
  run_id, not_before_ms, skip_reason, created_ms";

fn wake_row(r: &Row) -> rusqlite::Result<Wake> {
    let ctx: String = r.get(6)?;
    Ok(Wake {
        id: r.get(0)?,
        company_id: r.get(1)?,
        staff_id: r.get(2)?,
        issue_id: r.get(3)?,
        source: r.get(4)?,
        reason: r.get(5)?,
        context: serde_json::from_str(&ctx).unwrap_or(Value::Null),
        priority: r.get(7)?,
        state: r.get(8)?,
        run_id: r.get(9)?,
        not_before_ms: r.get(10)?,
        skip_reason: r.get(11)?,
        created_ms: r.get(12)?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub company_id: String,
    pub staff_id: String,
    pub issue_id: Option<String>,
    pub source: String,
    pub wake_count: i64,
    pub job_id: Option<String>,
    pub state: String,
    pub failure_class: Option<String>,
    pub error: Option<String>,
    pub attempt: i64,
    pub prompt_chars: i64,
    pub input_tokens: i64,
    pub cached_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
    pub summary: Option<String>,
    pub liveness: Option<String>,
    pub created_ms: i64,
    pub started_ms: Option<i64>,
    pub finished_ms: Option<i64>,
}

pub(crate) const RUN_COLS: &str = "id, company_id, staff_id, issue_id, source, wake_count, job_id, state,
  failure_class, error, attempt, prompt_chars, input_tokens, cached_tokens, output_tokens, cost_micros,
  summary, liveness, created_ms, started_ms, finished_ms";

pub(crate) fn run_row(r: &Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: r.get(0)?,
        company_id: r.get(1)?,
        staff_id: r.get(2)?,
        issue_id: r.get(3)?,
        source: r.get(4)?,
        wake_count: r.get(5)?,
        job_id: r.get(6)?,
        state: r.get(7)?,
        failure_class: r.get(8)?,
        error: r.get(9)?,
        attempt: r.get(10)?,
        prompt_chars: r.get(11)?,
        input_tokens: r.get(12)?,
        cached_tokens: r.get(13)?,
        output_tokens: r.get(14)?,
        cost_micros: r.get(15)?,
        summary: r.get(16)?,
        liveness: r.get(17)?,
        created_ms: r.get(18)?,
        started_ms: r.get(19)?,
        finished_ms: r.get(20)?,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Claim {
    pub run: Run,
    pub staff: Staff,
    pub issue: Option<Issue>,
    pub wakes: Vec<Wake>,
    pub first_run: bool,
    pub last_run_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunOutcome {
    pub state: String,
    pub failure_class: Option<String>,
    pub error: Option<String>,
    pub summary: Option<String>,
    pub job_id: Option<String>,
}

pub fn retryable(class: Option<&str>) -> bool {
    matches!(
        class,
        Some("stalled" | "no_start" | "spawn" | "rate_limit" | "interrupted" | "network")
    )
}

pub fn backoff_ms(attempt: i64, seed: &str) -> i64 {
    let base = RETRY_BASE_MS.saturating_mul(1 << (attempt.clamp(1, 10) - 1));
    let h = seed
        .bytes()
        .fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
    let jitter = (h % 21) as i64 - 10;
    base + base * jitter / 100
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue_wake(
    tx: &Transaction,
    company: &str,
    staff: &str,
    issue: Option<&str>,
    source: WakeSource,
    reason: &str,
    context: Value,
    priority: i64,
) -> Result<String, String> {
    enqueue_wake_at(
        tx, company, staff, issue, source, reason, context, priority, 0,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue_wake_at(
    tx: &Transaction,
    company: &str,
    staff: &str,
    issue: Option<&str>,
    source: WakeSource,
    reason: &str,
    context: Value,
    priority: i64,
    not_before_ms: i64,
) -> Result<String, String> {
    let now = now_ms();
    let prio = priority * 100 + source.weight();
    let id = new_id();
    let survivor: Option<(String, i64)> = tx
        .query_row(
            "SELECT id, priority FROM wakeups
             WHERE staff_id = ?1 AND state = 'queued' AND issue_id IS ?2
             ORDER BY created_ms LIMIT 1",
            params![staff, issue],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    let ctx = context.to_string();
    if let Some((sid, sprio)) = survivor {
        tx.execute(
            "INSERT INTO wakeups (id, company_id, staff_id, issue_id, source, reason, context, priority, state,
               skip_reason, not_before_ms, created_ms, updated_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'coalesced',?9,?10,?11,?11)",
            params![id, company, staff, issue, source.as_str(), reason, ctx, prio, sid, not_before_ms, now],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE wakeups SET priority = MIN(priority, ?2), not_before_ms = MIN(not_before_ms, ?3), updated_ms = ?4 WHERE id = ?1",
            params![sid, prio.min(sprio), not_before_ms, now],
        )
        .map_err(sql)?;
        return Ok(sid);
    }
    tx.execute(
        "INSERT INTO wakeups (id, company_id, staff_id, issue_id, source, reason, context, priority, state,
           not_before_ms, created_ms, updated_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'queued',?9,?10,?10)",
        params![id, company, staff, issue, source.as_str(), reason, ctx, prio, not_before_ms, now],
    )
    .map_err(sql)?;
    Ok(id)
}

fn skip(tx: &Transaction, id: &str, why: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE wakeups SET state = 'skipped', skip_reason = ?2, updated_ms = ?3 WHERE id = ?1",
        params![id, why, now_ms()],
    )
    .map_err(sql)?;
    Ok(())
}

fn running_count(tx: &Transaction, col: &str, id: &str) -> Result<i64, String> {
    tx.query_row(
        &format!("SELECT COUNT(*) FROM runs WHERE {col} = ?1 AND state IN ('queued','running')"),
        params![id],
        |r| r.get(0),
    )
    .map_err(sql)
}

impl CompanyStore {
    pub fn wake(
        &self,
        company: &str,
        staff: &str,
        issue: Option<&str>,
        reason: &str,
        actor: &Actor,
    ) -> Result<String, String> {
        self.write(|tx, ch| {
            let s = get_staff(tx, company, staff)?;
            if matches!(s.status.as_str(), "terminated" | "pending_approval") {
                return Err(invalid(format!("{} is {}", s.name, s.status)));
            }
            let issue_id = match issue {
                Some(i) => Some(get_issue(tx, company, i)?.id),
                None => None,
            };
            let id = enqueue_wake(
                tx,
                company,
                staff,
                issue_id.as_deref(),
                WakeSource::Manual,
                reason,
                json!({}),
                0,
            )?;
            audit::record(
                tx,
                company,
                actor,
                "agent.woken",
                "staff",
                staff,
                Some(json!({"issue": issue_id, "reason": reason})),
            )?;
            changed(ch, company, "wake", &id);
            Ok(id)
        })
    }

    pub fn wakes(
        &self,
        company: &str,
        state: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Wake>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {WAKE_COLS} FROM wakeups WHERE company_id = ?1 AND (?2 IS NULL OR state = ?2)
                 ORDER BY created_ms DESC LIMIT ?3"
            ))?;
            let rows = st.query_map(params![company, state, limit.clamp(1, 2000) as i64], wake_row)?;
            rows.collect()
        })
    }

    pub fn claim(&self, now: i64, global_free: usize) -> Result<Vec<Claim>, String> {
        if global_free == 0 {
            return Ok(Vec::new());
        }
        self.write(|tx, ch| {
            let mut st = tx
                .prepare(&format!(
                    "SELECT {WAKE_COLS} FROM wakeups WHERE state = 'queued' AND not_before_ms <= ?1
                     ORDER BY priority, created_ms LIMIT 500"
                ))
                .map_err(sql)?;
            let due = st
                .query_map(params![now], wake_row)
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(st);
            let mut out = Vec::new();
            let mut busy_staff: std::collections::HashSet<String> = std::collections::HashSet::new();
            for w in due {
                if out.len() >= global_free {
                    break;
                }
                if busy_staff.contains(&w.staff_id) {
                    continue;
                }
                let still: bool = tx
                    .query_row("SELECT state = 'queued' FROM wakeups WHERE id = ?1", params![w.id], |r| r.get(0))
                    .map_err(sql)?;
                if !still {
                    continue;
                }
                let staff = match get_staff(tx, &w.company_id, &w.staff_id) {
                    Ok(s) => s,
                    Err(_) => {
                        skip(tx, &w.id, "agent_gone")?;
                        continue;
                    }
                };
                if let Err(why) = invokable(tx, &staff) {
                    if why.starts_with("company_") || staff.status == "terminated" {
                        skip(tx, &w.id, &why)?;
                    }
                    busy_staff.insert(staff.id.clone());
                    continue;
                }
                if running_count(tx, "staff_id", &staff.id)? >= staff.max_concurrent {
                    busy_staff.insert(staff.id.clone());
                    continue;
                }
                let company = get_company(tx, &w.company_id)?;
                if running_count(tx, "company_id", &company.id)? >= company.max_parallel {
                    continue;
                }
                let issue = match &w.issue_id {
                    Some(i) => match get_issue(tx, &w.company_id, i) {
                        Ok(i) => Some(i),
                        Err(_) => {
                            skip(tx, &w.id, "issue_gone")?;
                            continue;
                        }
                    },
                    None => None,
                };
                if let Some(i) = &issue {
                    let talk = matches!(w.source.as_str(), "mention" | "comment" | "manual" | "review");
                    if matches!(i.status.as_str(), "done" | "cancelled") && !talk {
                        skip(tx, &w.id, "issue_closed")?;
                        continue;
                    }
                    if i.assignee_staff_id.as_deref().is_some_and(|a| a != staff.id) && !talk {
                        skip(tx, &w.id, "reassigned")?;
                        continue;
                    }
                    if let (Some(l), Some(until)) = (&i.lease_run_id, i.lease_until_ms) {
                        if until > now {
                            let live: bool = tx
                                .query_row(
                                    "SELECT EXISTS (SELECT 1 FROM runs WHERE id = ?1 AND state IN ('queued','running'))",
                                    params![l],
                                    |r| r.get(0),
                                )
                                .map_err(sql)?;
                            if live {
                                continue;
                            }
                        }
                    }
                    if company.require_strategy_approval
                        && company.strategy_state != "approved"
                        && i.parent_id.is_some()
                    {
                        continue;
                    }
                }
                if let Some(block) = super::gov::invocation_block(
                    tx,
                    &company.id,
                    &staff.id,
                    issue.as_ref().and_then(|i| i.project_id.as_deref()),
                )? {
                    let _ = block;
                    busy_staff.insert(staff.id.clone());
                    continue;
                }
                let mut st = tx
                    .prepare(&format!(
                        "SELECT {WAKE_COLS} FROM wakeups WHERE staff_id = ?1 AND issue_id IS ?2
                           AND state IN ('queued') AND not_before_ms <= ?3
                         UNION ALL
                         SELECT {WAKE_COLS} FROM wakeups WHERE state = 'coalesced' AND skip_reason IN
                           (SELECT id FROM wakeups WHERE staff_id = ?1 AND issue_id IS ?2 AND state = 'queued')
                         ORDER BY created_ms"
                    ))
                    .map_err(sql)?;
                let group = st
                    .query_map(params![staff.id, w.issue_id, now], wake_row)
                    .map_err(sql)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(sql)?;
                drop(st);
                let prev: Option<i64> = tx
                    .query_row(
                        "SELECT MAX(created_ms) FROM runs WHERE staff_id = ?1",
                        params![staff.id],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                let attempt: i64 = w
                    .context
                    .get("attempt")
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                let run_id = new_id();
                let queued: Vec<&Wake> = group.iter().filter(|g| g.state == "queued").collect();
                tx.execute(
                    "INSERT INTO runs (id, company_id, staff_id, issue_id, source, wake_count, state, attempt, created_ms, started_ms)
                     VALUES (?1,?2,?3,?4,?5,?6,'running',?7,?8,?8)",
                    params![run_id, company.id, staff.id, w.issue_id, w.source, group.len() as i64, attempt, now],
                )
                .map_err(sql)?;
                for g in &queued {
                    tx.execute(
                        "UPDATE wakeups SET state = 'dispatched', run_id = ?2, updated_ms = ?3 WHERE id = ?1",
                        params![g.id, run_id, now],
                    )
                    .map_err(sql)?;
                }
                tx.execute(
                    "UPDATE wakeups SET run_id = ?2 WHERE state = 'coalesced' AND skip_reason IN (SELECT value FROM json_each(?1))",
                    params![serde_json::to_string(&queued.iter().map(|g| g.id.clone()).collect::<Vec<_>>()).unwrap_or_default(), run_id],
                )
                .map_err(sql)?;
                tx.execute(
                    "UPDATE staff SET status = 'running', last_heartbeat_ms = ?2, updated_ms = ?2 WHERE id = ?1",
                    params![staff.id, now],
                )
                .map_err(sql)?;
                super::gov::count_run(tx, &company.id, &staff.id, issue.as_ref().and_then(|i| i.project_id.as_deref()), now)?;
                let issue = match issue {
                    Some(i) if matches!(i.status.as_str(), "todo" | "backlog" | "in_progress")
                        && i.assignee_staff_id.as_deref().is_none_or(|a| a == staff.id) =>
                    {
                        Some(super::issues::checkout_tx(tx, ch, &company.id, &i.id, &staff.id, &run_id, None).unwrap_or(i))
                    }
                    other => other,
                };
                let run = tx
                    .query_row(&format!("SELECT {RUN_COLS} FROM runs WHERE id = ?1"), params![run_id], run_row)
                    .map_err(sql)?;
                let staff = get_staff(tx, &company.id, &staff.id)?;
                changed(ch, &company.id, "run", &run_id);
                changed(ch, &company.id, "staff", &staff.id);
                busy_staff.insert(staff.id.clone());
                out.push(Claim {
                    first_run: prev.is_none(),
                    last_run_ms: prev,
                    run,
                    staff,
                    issue,
                    wakes: group,
                });
            }
            Ok(out)
        })
    }

    pub fn run_set_job(
        &self,
        run_id: &str,
        job_id: &str,
        prompt_chars: usize,
    ) -> Result<(), String> {
        self.write(|tx, ch| {
            let company: String = tx
                .query_row(
                    "SELECT company_id FROM runs WHERE id = ?1",
                    params![run_id],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            tx.execute(
                "UPDATE runs SET job_id = ?2, prompt_chars = ?3 WHERE id = ?1",
                params![run_id, job_id, prompt_chars as i64],
            )
            .map_err(sql)?;
            changed(ch, &company, "run", run_id);
            Ok(())
        })
    }

    pub fn run_renew_lease(&self, run_id: &str) -> Result<(), String> {
        self.write(|tx, _| {
            tx.execute(
                "UPDATE issues SET lease_until_ms = ?2 WHERE lease_run_id = ?1",
                params![run_id, now_ms() + super::issues::LEASE_MS],
            )
            .map_err(sql)?;
            Ok(())
        })
    }

    pub fn run_get(&self, id: &str) -> Result<Run, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {RUN_COLS} FROM runs WHERE id = ?1"),
                params![id],
                run_row,
            )
            .optional()
        })?
        .ok_or_else(|| not_found("run", id))
    }

    pub fn runs(
        &self,
        company: &str,
        staff: Option<&str>,
        issue: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Run>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {RUN_COLS} FROM runs WHERE company_id = ?1 AND (?2 IS NULL OR staff_id = ?2)
                   AND (?3 IS NULL OR issue_id = ?3)
                 ORDER BY created_ms DESC LIMIT ?4"
            ))?;
            let rows = st.query_map(
                params![company, staff, issue, limit.clamp(1, 1000) as i64],
                run_row,
            )?;
            rows.collect()
        })
    }

    pub fn live_run_of(&self, staff: &str) -> Result<Option<Run>, String> {
        self.read(|c| {
            c.query_row(
                &format!("SELECT {RUN_COLS} FROM runs WHERE staff_id = ?1 AND state IN ('queued','running') ORDER BY created_ms DESC LIMIT 1"),
                params![staff],
                run_row,
            )
            .optional()
        })
    }

    pub fn live_runs(&self) -> Result<Vec<Run>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {RUN_COLS} FROM runs WHERE state IN ('queued','running')"
            ))?;
            let rows = st.query_map([], run_row)?;
            rows.collect()
        })
    }

    pub fn run_finish(&self, run_id: &str, out: RunOutcome) -> Result<Run, String> {
        if !matches!(
            out.state.as_str(),
            "succeeded" | "failed" | "cancelled" | "timed_out"
        ) {
            return Err(invalid(
                "a run ends succeeded, failed, cancelled or timed_out",
            ));
        }
        self.write(|tx, ch| finish_tx(tx, ch, run_id, out))
    }

    pub fn heartbeat_stats(&self, company: &str) -> Result<Value, String> {
        self.read(|c| {
            let wakes: Vec<(String, i64)> = {
                let mut st = c.prepare("SELECT state, COUNT(*) FROM wakeups WHERE company_id = ?1 GROUP BY state")?;
                let rows = st.query_map(params![company], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            let runs: Vec<(String, i64)> = {
                let mut st = c.prepare("SELECT state, COUNT(*) FROM runs WHERE company_id = ?1 GROUP BY state")?;
                let rows = st.query_map(params![company], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            Ok(json!({
                "wakes": wakes.into_iter().map(|(k, v)| (k, json!(v))).collect::<serde_json::Map<String, Value>>(),
                "runs": runs.into_iter().map(|(k, v)| (k, json!(v))).collect::<serde_json::Map<String, Value>>(),
            }))
        })
    }

    pub fn tick_timers(&self, now: i64) -> Result<u32, String> {
        self.write(|tx, ch| {
            let mut st = tx
                .prepare(&format!(
                    "SELECT {STAFF_COLS} FROM staff WHERE heartbeat_cron IS NOT NULL AND heartbeat_next_ms IS NOT NULL
                       AND heartbeat_next_ms <= ?1 AND status IN ('idle','running','error')"
                ))
                .map_err(sql)?;
            let due = st
                .query_map(params![now], staff_row)
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(st);
            let mut n = 0;
            for s in due {
                let next = s
                    .heartbeat_cron
                    .as_deref()
                    .and_then(|c| super::routines::next_after(c, &s.heartbeat_tz, now).ok());
                let moved = tx
                    .execute(
                        "UPDATE staff SET heartbeat_next_ms = ?2 WHERE id = ?1 AND heartbeat_next_ms = ?3",
                        params![s.id, next, s.heartbeat_next_ms],
                    )
                    .map_err(sql)?;
                if moved == 0 {
                    continue;
                }
                let c = get_company(tx, &s.company_id)?;
                if c.status != "active" {
                    continue;
                }
                let already: bool = tx
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM wakeups WHERE staff_id = ?1 AND state = 'queued' AND source = 'timer')",
                        params![s.id],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                if !already {
                    enqueue_wake(tx, &s.company_id, &s.id, None, WakeSource::Timer, "scheduled heartbeat", json!({}), 3)?;
                    changed(ch, &s.company_id, "wake", &s.id);
                    n += 1;
                }
            }
            Ok(n)
        })
    }

    pub fn reconcile(&self, now: i64) -> Result<u32, String> {
        self.write(|tx, ch| {
            let freed = tx
                .execute(
                    "UPDATE issues SET lease_run_id = NULL, lease_until_ms = NULL
                     WHERE lease_run_id IS NOT NULL AND (lease_until_ms < ?1 OR lease_run_id NOT IN
                       (SELECT id FROM runs WHERE state IN ('queued','running')))",
                    params![now],
                )
                .map_err(sql)?;
            let mut st = tx
                .prepare(&format!(
                    "SELECT {ISSUE_COLS} FROM issues i WHERE status = 'in_progress' AND assignee_staff_id IS NOT NULL
                       AND lease_run_id IS NULL
                       AND NOT EXISTS (SELECT 1 FROM wakeups w WHERE w.issue_id = i.id AND w.state = 'queued')
                       AND NOT EXISTS (SELECT 1 FROM runs r WHERE r.issue_id = i.id AND r.state IN ('queued','running'))
                       AND updated_ms < ?1"
                ))
                .map_err(sql)?;
            let stranded = st
                .query_map(params![now - 60_000], issue_row)
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(st);
            let mut n = freed as u32;
            for i in stranded {
                let Some(a) = &i.assignee_staff_id else { continue };
                let attempts = continuation_count(tx, &i.id)?;
                if attempts >= MAX_CONTINUATIONS {
                    continue;
                }
                enqueue_wake(
                    tx,
                    &i.company_id,
                    a,
                    Some(&i.id),
                    WakeSource::Continuation,
                    "your issue is in progress with nothing running",
                    json!({"attempt": attempts + 1}),
                    super::issues::priority_rank(&i.priority),
                )?;
                changed(ch, &i.company_id, "issue", &i.id);
                n += 1;
            }
            Ok(n)
        })
    }
}

fn continuation_count(tx: &Transaction, issue: &str) -> Result<i64, String> {
    let mut st = tx
        .prepare("SELECT liveness FROM runs WHERE issue_id = ?1 AND state <> 'running' ORDER BY created_ms DESC LIMIT 20")
        .map_err(sql)?;
    let rows = st
        .query_map(params![issue], |r| r.get::<_, Option<String>>(0))
        .map_err(sql)?;
    let mut n = 0;
    for l in rows {
        match l.map_err(sql)?.as_deref() {
            Some("no_progress") => n += 1,
            _ => break,
        }
    }
    Ok(n)
}

fn progress_evidence(tx: &Transaction, run: &Run) -> Result<i64, String> {
    tx.query_row(
        "SELECT COUNT(*) FROM audit WHERE company_id = ?1 AND run_id = ?2
           AND action IN ('issue.status','comment.added','document.saved','work_product.added','issue.created',
                          'issue.updated','approval.requested','staff.hire_requested','routine.created')",
        params![run.company_id, run.id],
        |r| r.get(0),
    )
    .map_err(sql)
}

fn finish_tx(
    tx: &Transaction,
    ch: &mut Vec<Change>,
    run_id: &str,
    out: RunOutcome,
) -> Result<Run, String> {
    let run = tx
        .query_row(
            &format!("SELECT {RUN_COLS} FROM runs WHERE id = ?1"),
            params![run_id],
            run_row,
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| not_found("run", run_id))?;
    if !matches!(run.state.as_str(), "queued" | "running") {
        return Ok(run);
    }
    let now = now_ms();
    let evidence = progress_evidence(tx, &run)?;
    let issue = match &run.issue_id {
        Some(i) => get_issue(tx, &run.company_id, i).ok(),
        None => None,
    };
    let mut liveness = if evidence > 0 {
        "progress"
    } else {
        "no_progress"
    }
    .to_string();
    let system = Actor::system("heartbeat");
    if out.state == "succeeded" {
        if let Some(i) = &issue {
            let mine = i.assignee_staff_id.as_deref() == Some(run.staff_id.as_str());
            let open = matches!(i.status.as_str(), "todo" | "in_progress");
            if mine && open {
                if evidence == 0 {
                    let attempts = continuation_count(tx, &i.id)? + 1;
                    if attempts >= MAX_CONTINUATIONS {
                        liveness = "needs_owner".into();
                        tx.execute(
                            "UPDATE issues SET status = 'blocked', blocked_reason = ?2, lease_run_id = NULL,
                               lease_until_ms = NULL, seq = seq + 1, updated_ms = ?3 WHERE id = ?1",
                            params![i.id, format!("{attempts} runs without progress; the agent needs attention"), now],
                        )
                        .map_err(sql)?;
                        tx.execute(
                            "INSERT INTO comments (id, company_id, issue_id, author_kind, body, run_id, created_ms)
                             VALUES (?1,?2,?3,'system',?4,?5,?6)",
                            params![new_id(), run.company_id, i.id,
                                format!("@owner {attempts} runs in a row ended without moving this issue. Check the last run, then reassign, comment or unblock."),
                                run.id, now],
                        )
                        .map_err(sql)?;
                        audit::record(
                            tx,
                            &run.company_id,
                            &system,
                            "issue.needs_owner",
                            "issue",
                            &i.id,
                            Some(json!({"attempts": attempts})),
                        )?;
                        changed(ch, &run.company_id, "issue", &i.id);
                    } else {
                        liveness = "no_progress".into();
                        enqueue_wake_at(
                            tx,
                            &run.company_id,
                            &run.staff_id,
                            Some(&i.id),
                            WakeSource::Continuation,
                            "keep going: the issue is still open and the last run left no trace",
                            json!({"attempt": attempts + 1, "previous_run": run.id}),
                            super::issues::priority_rank(&i.priority),
                            now + backoff_ms(attempts, &run.id) / 3,
                        )?;
                    }
                } else if i.status == "in_progress" {
                    liveness = "continuing".into();
                    enqueue_wake_at(
                        tx,
                        &run.company_id,
                        &run.staff_id,
                        Some(&i.id),
                        WakeSource::Continuation,
                        "continue where you stopped",
                        json!({"previous_run": run.id}),
                        super::issues::priority_rank(&i.priority),
                        now + 2_000,
                    )?;
                }
            }
        }
    } else if out.state == "failed"
        && retryable(out.failure_class.as_deref())
        && run.attempt < MAX_RETRIES
    {
        liveness = "retrying".into();
        enqueue_wake_at(
            tx,
            &run.company_id,
            &run.staff_id,
            run.issue_id.as_deref(),
            WakeSource::Retry,
            &format!(
                "retry after {}",
                out.failure_class.clone().unwrap_or_default()
            ),
            json!({"attempt": run.attempt + 1, "previous_run": run.id}),
            2,
            now + backoff_ms(run.attempt, &run.id),
        )?;
    } else if out.state == "failed" {
        liveness = "failed".into();
        if let Some(i) = &issue {
            tx.execute(
                "INSERT INTO comments (id, company_id, issue_id, author_kind, body, run_id, created_ms)
                 VALUES (?1,?2,?3,'system',?4,?5,?6)",
                params![new_id(), run.company_id, i.id,
                    format!("The run failed ({}): {}", out.failure_class.clone().unwrap_or_else(|| "error".into()),
                        out.error.clone().unwrap_or_default().chars().take(500).collect::<String>()),
                    run.id, now],
            )
            .map_err(sql)?;
        }
    }
    tx.execute(
        "UPDATE runs SET state = ?2, failure_class = ?3, error = ?4, summary = COALESCE(?5, summary),
           job_id = COALESCE(?6, job_id), liveness = ?7, finished_ms = ?8 WHERE id = ?1",
        params![run.id, out.state, out.failure_class, out.error, out.summary, out.job_id, liveness, now],
    )
    .map_err(sql)?;
    tx.execute(
        "UPDATE issues SET lease_run_id = NULL, lease_until_ms = NULL WHERE lease_run_id = ?1",
        params![run.id],
    )
    .map_err(sql)?;
    let still: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE staff_id = ?1 AND state IN ('queued','running')",
            params![run.staff_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let status = if still > 0 {
        "running"
    } else if out.state == "failed" && !retryable(out.failure_class.as_deref()) {
        "error"
    } else {
        "idle"
    };
    tx.execute(
        "UPDATE staff SET status = ?2, updated_ms = ?3 WHERE id = ?1 AND status IN ('running','idle','error')",
        params![run.staff_id, status, now],
    )
    .map_err(sql)?;
    audit::record(
        tx,
        &run.company_id,
        &system,
        &format!("run.{}", out.state),
        "run",
        &run.id,
        Some(
            json!({"staff": run.staff_id, "issue": run.issue_id, "liveness": liveness,
                    "failure": out.failure_class, "cost_micros": run.cost_micros}),
        ),
    )?;
    changed(ch, &run.company_id, "run", &run.id);
    changed(ch, &run.company_id, "staff", &run.staff_id);
    tx.query_row(
        &format!("SELECT {RUN_COLS} FROM runs WHERE id = ?1"),
        params![run.id],
        run_row,
    )
    .map_err(sql)
}

#[derive(Debug, Clone, Serialize)]
pub struct Scorecard {
    pub staff_id: String,
    pub name: String,
    pub role: String,
    pub runs: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub with_progress: i64,
    pub issues_done: i64,
    pub reviews_sent_back: i64,
    pub cost_usd: f64,
    pub tokens: i64,
    pub avg_run_s: f64,
    pub cost_per_done_usd: f64,
}

impl CompanyStore {
    pub fn scorecards(&self, company: &str, since_ms: i64) -> Result<Vec<Scorecard>, String> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT s.id, s.name, s.role,
                   (SELECT COUNT(*) FROM runs r WHERE r.staff_id = s.id AND r.created_ms >= ?2),
                   (SELECT COUNT(*) FROM runs r WHERE r.staff_id = s.id AND r.created_ms >= ?2 AND r.state = 'succeeded'),
                   (SELECT COUNT(*) FROM runs r WHERE r.staff_id = s.id AND r.created_ms >= ?2 AND r.state IN ('failed','timed_out')),
                   (SELECT COUNT(*) FROM runs r WHERE r.staff_id = s.id AND r.created_ms >= ?2 AND r.liveness IN ('progress','continuing')),
                   (SELECT COUNT(*) FROM issues i WHERE i.assignee_staff_id = s.id AND i.status = 'done' AND i.completed_ms >= ?2),
                   (SELECT COUNT(*) FROM audit a WHERE a.company_id = ?1 AND a.action = 'issue.status' AND a.ts_ms >= ?2
                      AND json_extract(a.details, '$.from') = 'in_review' AND json_extract(a.details, '$.to') = 'todo'
                      AND a.entity_id IN (SELECT id FROM issues WHERE assignee_staff_id = s.id)),
                   (SELECT COALESCE(SUM(cost_micros), 0) FROM cost_events e WHERE e.staff_id = s.id AND e.occurred_ms >= ?2),
                   (SELECT COALESCE(SUM(input_tokens + output_tokens), 0) FROM cost_events e WHERE e.staff_id = s.id AND e.occurred_ms >= ?2),
                   (SELECT COALESCE(AVG(finished_ms - started_ms), 0) FROM runs r WHERE r.staff_id = s.id AND r.created_ms >= ?2 AND finished_ms IS NOT NULL)
                 FROM staff s WHERE s.company_id = ?1 AND s.status <> 'terminated' ORDER BY s.created_ms",
            )?;
            let rows = st.query_map(params![company, since_ms], |r| {
                let micros: i64 = r.get(9)?;
                let done: i64 = r.get(7)?;
                Ok(Scorecard {
                    staff_id: r.get(0)?,
                    name: r.get(1)?,
                    role: r.get(2)?,
                    runs: r.get(3)?,
                    succeeded: r.get(4)?,
                    failed: r.get(5)?,
                    with_progress: r.get(6)?,
                    issues_done: done,
                    reviews_sent_back: r.get(8)?,
                    cost_usd: micros as f64 / 1e6,
                    tokens: r.get(10)?,
                    avg_run_s: r.get::<_, f64>(11)? / 1000.0,
                    cost_per_done_usd: if done > 0 { micros as f64 / 1e6 / done as f64 } else { 0.0 },
                })
            })?;
            rows.collect()
        })
    }
}
