use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, TimeZone, Timelike, Utc};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::issues::{issue_create_tx, NewIssue};
use super::org::get_staff;
use super::{audit, changed, invalid, new_id, not_found, now_ms, sql, Actor, CompanyStore};

pub const MAX_CATCH_UP: i64 = 25;

#[derive(Debug, Clone)]
struct CronSpec {
    min: Vec<bool>,
    hour: Vec<bool>,
    dom: Vec<bool>,
    month: Vec<bool>,
    dow: Vec<bool>,
    dom_any: bool,
    dow_any: bool,
}

fn names(field: &str, i: usize) -> String {
    let mut s = field.to_ascii_lowercase();
    let (list, base): (&[&str], usize) = match i {
        3 => (
            &[
                "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
            ],
            1,
        ),
        4 => (&["sun", "mon", "tue", "wed", "thu", "fri", "sat"], 0),
        _ => return s,
    };
    for (n, name) in list.iter().enumerate() {
        s = s.replace(name, &(n + base).to_string());
    }
    s
}

fn parse_field(raw: &str, lo: u32, hi: u32) -> Result<(Vec<bool>, bool), String> {
    let mut set = vec![false; hi as usize + 1];
    let mut any = false;
    for part in raw.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (
                r,
                s.parse::<u32>()
                    .map_err(|_| invalid(format!("bad step in `{raw}`")))?,
            ),
            None => (part, 1),
        };
        if step == 0 {
            return Err(invalid(format!("step 0 in `{raw}`")));
        }
        let (a, b) = if range == "*" {
            if step == 1 {
                any = true;
            }
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse()
                    .map_err(|_| invalid(format!("bad range in `{raw}`")))?,
                b.parse()
                    .map_err(|_| invalid(format!("bad range in `{raw}`")))?,
            )
        } else {
            let v: u32 = range
                .parse()
                .map_err(|_| invalid(format!("bad value `{range}`")))?;
            (v, if part.contains('/') { hi } else { v })
        };
        if a < lo || b > hi || a > b {
            return Err(invalid(format!("`{raw}` is outside {lo}-{hi}")));
        }
        let mut v = a;
        while v <= b {
            set[v as usize] = true;
            v += step;
        }
    }
    Ok((set, any))
}

fn parse(expr: &str) -> Result<CronSpec, String> {
    let expr = match expr.trim() {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        e => e,
    };
    let f: Vec<&str> = expr.split_whitespace().collect();
    if f.len() != 5 {
        return Err(invalid(
            "a cron has 5 fields: minute hour day month weekday",
        ));
    }
    let (min, _) = parse_field(&names(f[0], 0), 0, 59)?;
    let (hour, _) = parse_field(&names(f[1], 1), 0, 23)?;
    let (dom, dom_any) = parse_field(&names(f[2], 2), 1, 31)?;
    let (month, _) = parse_field(&names(f[3], 3), 1, 12)?;
    let (mut dow, dow_any) = parse_field(&names(f[4], 4).replace('7', "0"), 0, 6)?;
    dow.resize(7, false);
    Ok(CronSpec {
        min,
        hour,
        dom,
        month,
        dow,
        dom_any,
        dow_any,
    })
}

pub fn validate_cron(expr: &str) -> Result<(), String> {
    parse(expr).map(|_| ())
}

fn day_ok(c: &CronSpec, d: u32, wd: u32) -> bool {
    let dm = c.dom[d as usize];
    let dw = c.dow[wd as usize];
    match (c.dom_any, c.dow_any) {
        (true, true) => true,
        (true, false) => dw,
        (false, true) => dm,
        (false, false) => dm || dw,
    }
}

fn next_in<Tz: TimeZone>(c: &CronSpec, tz: &Tz, after_ms: i64) -> Option<i64> {
    let start = tz.timestamp_millis_opt(after_ms).single()?;
    let mut t = start.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
    for _ in 0..600_000 {
        if !c.month[t.month() as usize] {
            let (y, m) = if t.month() == 12 {
                (t.year() + 1, 1)
            } else {
                (t.year(), t.month() + 1)
            };
            t = tz.with_ymd_and_hms(y, m, 1, 0, 0, 0).earliest()?;
            continue;
        }
        if !day_ok(c, t.day(), t.weekday().num_days_from_sunday()) {
            t = (t + Duration::days(1)).with_hour(0)?.with_minute(0)?;
            continue;
        }
        if !c.hour[t.hour() as usize] {
            t = (t + Duration::hours(1)).with_minute(0)?;
            continue;
        }
        if !c.min[t.minute() as usize] {
            t += Duration::minutes(1);
            continue;
        }
        return Some(t.timestamp_millis());
    }
    None
}

pub fn next_after(expr: &str, tz: &str, after_ms: i64) -> Result<i64, String> {
    let c = parse(expr)?;
    let out = match tz.trim() {
        "" | "local" => next_in(&c, &Local, after_ms),
        "UTC" | "utc" | "Z" => next_in(&c, &Utc, after_ms),
        off => {
            let sign = if off.starts_with('-') { -1 } else { 1 };
            let body = off.trim_start_matches(['+', '-']);
            let (h, m) = body.split_once(':').unwrap_or((body, "0"));
            let secs =
                sign * (h.parse::<i32>().unwrap_or(0) * 3600 + m.parse::<i32>().unwrap_or(0) * 60);
            let fo = FixedOffset::east_opt(secs)
                .ok_or_else(|| invalid(format!("unknown time zone `{off}`")))?;
            next_in(&c, &fo, after_ms)
        }
    };
    out.ok_or_else(|| invalid("that cron never fires"))
}

pub(crate) fn schedule_staff_heartbeat(
    tx: &Transaction,
    company: &str,
    staff: &str,
    now: i64,
) -> Result<(), String> {
    let s = get_staff(tx, company, staff)?;
    let next = match (&s.heartbeat_cron, s.status.as_str()) {
        (Some(c), "idle" | "running" | "error" | "paused") => {
            next_after(c, &s.heartbeat_tz, now).ok()
        }
        _ => None,
    };
    tx.execute(
        "UPDATE staff SET heartbeat_next_ms = ?2 WHERE id = ?1",
        params![staff, next],
    )
    .map_err(sql)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    pub company_id: String,
    pub title: String,
    pub body: String,
    pub staff_id: String,
    pub project_id: Option<String>,
    pub priority: String,
    pub cron: String,
    pub tz: String,
    pub catch_up: String,
    pub concurrency: String,
    pub enabled: bool,
    pub next_run_ms: Option<i64>,
    pub last_run_ms: Option<i64>,
    pub last_issue_id: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewRoutine {
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub staff_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    pub cron: String,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub catch_up: Option<String>,
    #[serde(default)]
    pub concurrency: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

const ROUTINE_COLS: &str =
    "id, company_id, title, body, staff_id, project_id, priority, cron, tz, catch_up,
  concurrency, enabled, next_run_ms, last_run_ms, last_issue_id, created_ms, updated_ms";

fn routine_row(r: &Row) -> rusqlite::Result<Routine> {
    Ok(Routine {
        id: r.get(0)?,
        company_id: r.get(1)?,
        title: r.get(2)?,
        body: r.get(3)?,
        staff_id: r.get(4)?,
        project_id: r.get(5)?,
        priority: r.get(6)?,
        cron: r.get(7)?,
        tz: r.get(8)?,
        catch_up: r.get(9)?,
        concurrency: r.get(10)?,
        enabled: r.get::<_, i64>(11)? != 0,
        next_run_ms: r.get(12)?,
        last_run_ms: r.get(13)?,
        last_issue_id: r.get(14)?,
        created_ms: r.get(15)?,
        updated_ms: r.get(16)?,
    })
}

fn render_title(title: &str, at_ms: i64) -> String {
    let t: DateTime<Local> = Local
        .timestamp_millis_opt(at_ms)
        .single()
        .unwrap_or_else(Local::now);
    title
        .replace("{{date}}", &t.format("%Y-%m-%d").to_string())
        .replace("{{week}}", &t.format("%G-W%V").to_string())
        .replace("{{month}}", &t.format("%Y-%m").to_string())
}

fn fire_tx(
    tx: &Transaction,
    ch: &mut Vec<super::Change>,
    r: &Routine,
    slot_ms: i64,
    actor: &Actor,
) -> Result<Option<String>, String> {
    if r.concurrency == "skip_if_active" {
        if let Some(last) = &r.last_issue_id {
            let open: bool = tx
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM issues WHERE id = ?1 AND status NOT IN ('done','cancelled'))",
                    params![last],
                    |x| x.get(0),
                )
                .map_err(sql)?;
            if open {
                audit::record(
                    tx,
                    &r.company_id,
                    actor,
                    "routine.skipped",
                    "routine",
                    &r.id,
                    Some(json!({"reason": "previous issue still open"})),
                )?;
                return Ok(None);
            }
        }
    }
    let issue = issue_create_tx(
        tx,
        ch,
        &r.company_id,
        NewIssue {
            title: render_title(&r.title, slot_ms),
            body: r.body.clone(),
            project_id: r.project_id.clone(),
            priority: Some(r.priority.clone()),
            assignee_staff_id: Some(r.staff_id.clone()),
            origin_kind: Some("routine".into()),
            origin_id: Some(r.id.clone()),
            ..Default::default()
        },
        actor,
    )?;
    tx.execute(
        "UPDATE routines SET last_run_ms = ?2, last_issue_id = ?3, updated_ms = ?2 WHERE id = ?1",
        params![r.id, slot_ms, issue.id],
    )
    .map_err(sql)?;
    audit::record(
        tx,
        &r.company_id,
        actor,
        "routine.fired",
        "routine",
        &r.id,
        Some(json!({"issue": issue.identifier})),
    )?;
    changed(ch, &r.company_id, "routine", &r.id);
    Ok(Some(issue.id))
}

impl CompanyStore {
    pub fn routine_create(
        &self,
        company: &str,
        new: NewRoutine,
        actor: &Actor,
    ) -> Result<Routine, String> {
        if new.title.trim().is_empty() {
            return Err(invalid("a routine needs a title"));
        }
        let tz = new.tz.clone().unwrap_or_else(|| "local".into());
        let next = next_after(&new.cron, &tz, now_ms())?;
        let catch_up = new.catch_up.clone().unwrap_or_else(|| "once".into());
        let conc = new
            .concurrency
            .clone()
            .unwrap_or_else(|| "skip_if_active".into());
        self.write(|tx, ch| {
            get_staff(tx, company, &new.staff_id)?;
            let id = new_id();
            let now = now_ms();
            tx.execute(
                "INSERT INTO routines (id, company_id, title, body, staff_id, project_id, priority, cron, tz, catch_up,
                   concurrency, enabled, next_run_ms, created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14)",
                params![id, company, new.title.trim(), new.body, new.staff_id, new.project_id,
                    new.priority.clone().unwrap_or_else(|| "medium".into()), new.cron.trim(), tz, catch_up, conc,
                    new.enabled.unwrap_or(true) as i64, next, now],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                    invalid("catch_up is skip/once/all_capped and concurrency skip_if_active/always")
                }
                e => sql(e),
            })?;
            audit::record(tx, company, actor, "routine.created", "routine", &id, Some(json!({"title": new.title, "cron": new.cron})))?;
            changed(ch, company, "routine", &id);
            tx.query_row(&format!("SELECT {ROUTINE_COLS} FROM routines WHERE id = ?1"), params![id], routine_row).map_err(sql)
        })
    }

    pub fn routine_list(&self, company: &str) -> Result<Vec<Routine>, String> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {ROUTINE_COLS} FROM routines WHERE company_id = ?1 ORDER BY created_ms"
            ))?;
            let rows = st.query_map(params![company], routine_row)?;
            rows.collect()
        })
    }

    pub fn routine_update(
        &self,
        company: &str,
        id: &str,
        new: NewRoutine,
        actor: &Actor,
    ) -> Result<Routine, String> {
        let tz = new.tz.clone().unwrap_or_else(|| "local".into());
        let next = next_after(&new.cron, &tz, now_ms())?;
        self.write(|tx, ch| {
            get_staff(tx, company, &new.staff_id)?;
            let n = tx
                .execute(
                    "UPDATE routines SET title=?3, body=?4, staff_id=?5, project_id=?6, priority=?7, cron=?8, tz=?9,
                       catch_up=COALESCE(?10, catch_up), concurrency=COALESCE(?11, concurrency),
                       enabled=COALESCE(?12, enabled), next_run_ms=?13, updated_ms=?14
                     WHERE company_id=?1 AND id=?2",
                    params![company, id, new.title.trim(), new.body, new.staff_id, new.project_id,
                        new.priority.clone().unwrap_or_else(|| "medium".into()), new.cron.trim(), tz, new.catch_up,
                        new.concurrency, new.enabled.map(|b| b as i64), next, now_ms()],
                )
                .map_err(sql)?;
            if n == 0 {
                return Err(not_found("routine", id));
            }
            audit::record(tx, company, actor, "routine.updated", "routine", id, None)?;
            changed(ch, company, "routine", id);
            tx.query_row(&format!("SELECT {ROUTINE_COLS} FROM routines WHERE id = ?1"), params![id], routine_row).map_err(sql)
        })
    }

    pub fn routine_delete(&self, company: &str, id: &str, actor: &Actor) -> Result<(), String> {
        self.write(|tx, ch| {
            let n = tx
                .execute(
                    "DELETE FROM routines WHERE company_id = ?1 AND id = ?2",
                    params![company, id],
                )
                .map_err(sql)?;
            if n == 0 {
                return Err(not_found("routine", id));
            }
            audit::record(tx, company, actor, "routine.deleted", "routine", id, None)?;
            changed(ch, company, "routine", id);
            Ok(())
        })
    }

    pub fn routine_fire(
        &self,
        company: &str,
        id: &str,
        actor: &Actor,
    ) -> Result<Option<String>, String> {
        self.write(|tx, ch| {
            let r = tx
                .query_row(
                    &format!(
                        "SELECT {ROUTINE_COLS} FROM routines WHERE company_id = ?1 AND id = ?2"
                    ),
                    params![company, id],
                    routine_row,
                )
                .optional()
                .map_err(sql)?
                .ok_or_else(|| not_found("routine", id))?;
            fire_tx(tx, ch, &r, now_ms(), actor)
        })
    }

    pub fn tick_routines(&self, now: i64) -> Result<Vec<String>, String> {
        self.write(|tx, ch| {
            let mut st = tx
                .prepare(&format!(
                    "SELECT {ROUTINE_COLS} FROM routines r WHERE enabled = 1 AND next_run_ms IS NOT NULL AND next_run_ms <= ?1
                       AND EXISTS (SELECT 1 FROM companies c WHERE c.id = r.company_id AND c.status = 'active')"
                ))
                .map_err(sql)?;
            let due = st
                .query_map(params![now], routine_row)
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(st);
            let actor = Actor::system("routines");
            let mut filed = Vec::new();
            for r in due {
                let first = r.next_run_ms.unwrap_or(now);
                let mut slots = vec![first];
                let mut t = first;
                while slots.len() < (MAX_CATCH_UP + 1) as usize {
                    match next_after(&r.cron, &r.tz, t) {
                        Ok(n) if n <= now => {
                            slots.push(n);
                            t = n;
                        }
                        _ => break,
                    }
                }
                let mut next = next_after(&r.cron, &r.tz, now).ok();
                if next.is_some_and(|n| n <= now) {
                    next = None;
                }
                let moved = tx
                    .execute(
                        "UPDATE routines SET next_run_ms = ?2 WHERE id = ?1 AND next_run_ms = ?3",
                        params![r.id, next, r.next_run_ms],
                    )
                    .map_err(sql)?;
                if moved == 0 {
                    continue;
                }
                let missed = slots.iter().filter(|s| now - **s > 120_000).count() as i64;
                let fires: Vec<i64> = match r.catch_up.as_str() {
                    "skip" if missed == slots.len() as i64 => Vec::new(),
                    "skip" => slots.iter().copied().filter(|s| now - *s <= 120_000).take(1).collect(),
                    "all_capped" => slots.iter().copied().take(MAX_CATCH_UP as usize).collect(),
                    _ => vec![*slots.last().unwrap_or(&now)],
                };
                if missed > 0 {
                    audit::record(tx, &r.company_id, &actor, "routine.missed", "routine", &r.id,
                        Some(json!({"missed": missed, "policy": r.catch_up, "firing": fires.len()})))?;
                }
                for s in fires {
                    let cur = tx
                        .query_row(&format!("SELECT {ROUTINE_COLS} FROM routines WHERE id = ?1"), params![r.id], routine_row)
                        .map_err(sql)?;
                    if let Some(i) = fire_tx(tx, ch, &cur, s, &actor)? {
                        filed.push(i);
                    }
                }
            }
            Ok(filed)
        })
    }
}
