use std::sync::Mutex;

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

use super::model::{new_id, now_rfc3339, GROUND_FLOOR_ID};
use super::persist;
use super::store::CanvasStore;

pub const HISTORY_MAX: usize = 50;
pub const MIN_INTERVAL_SECS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    Once {
        at: String,
    },
    #[serde(rename_all = "camelCase")]
    Interval {
        #[serde(default)]
        minutes: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_at: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Daily {
        #[serde(default)]
        time: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hour: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minute: Option<u8>,
    },
    #[serde(rename_all = "camelCase")]
    Weekly {
        #[serde(default)]
        days: Vec<u8>,
        #[serde(default)]
        time: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weekdays: Option<Vec<u8>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hour: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minute: Option<u8>,
    },
}

impl Schedule {
    pub fn interval_secs(&self) -> Option<u32> {
        match self {
            Schedule::Interval {
                minutes, seconds, ..
            } => Some(seconds.unwrap_or(minutes.saturating_mul(60))),
            _ => None,
        }
    }

    pub fn normalize(&mut self) -> Result<(), String> {
        fn hm(
            time: &mut String,
            hour: &mut Option<u8>,
            minute: &mut Option<u8>,
        ) -> Result<(), String> {
            if let Some(h) = hour.take() {
                let m = minute.take().unwrap_or(0);
                if h > 23 || m > 59 {
                    return Err(format!("CANVAS_INVALID: time {h}:{m:02}"));
                }
                *time = format!("{h:02}:{m:02}");
            }
            *minute = None;
            Ok(())
        }
        match self {
            Schedule::Interval {
                minutes, seconds, ..
            } => {
                if let Some(s) = *seconds {
                    *minutes = s / 60;
                }
            }
            Schedule::Daily { time, hour, minute } => hm(time, hour, minute)?,
            Schedule::Weekly {
                days,
                time,
                weekdays,
                hour,
                minute,
            } => {
                hm(time, hour, minute)?;
                if let Some(w) = weekdays.take() {
                    if w.iter().any(|d| !(1..=7).contains(d)) {
                        return Err("CANVAS_INVALID: weekdays are 1 (Sunday) to 7".into());
                    }
                    *days = w.iter().map(|d| d - 1).collect();
                }
                days.sort_unstable();
                days.dedup();
            }
            Schedule::Once { .. } => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum End {
    #[default]
    Never,
    AfterCount {
        count: u32,
    },
    OnDate {
        date: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub at: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub late_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub steps: u32,
    #[serde(default)]
    pub manual: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    #[serde(default = "ground")]
    pub floor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub prompt: String,
    pub schedule: Schedule,
    #[serde(default)]
    pub end: End,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_run: Option<String>,
    #[serde(default = "yes")]
    pub skip_if_busy: bool,
    #[serde(default)]
    pub notify: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    #[serde(default)]
    pub run_count: u32,
    #[serde(default)]
    pub history: Vec<RunRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_at: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
}

fn ground() -> String {
    GROUND_FLOOR_ID.to_string()
}
fn yes() -> bool {
    true
}

pub fn split_steps(prompt: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in prompt.lines() {
        if line.trim() == "&&" {
            out.push(cur.join("\n"));
            cur.clear();
        } else {
            cur.push(line);
        }
    }
    out.push(cur.join("\n"));
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn fill_output(step: &str, output: Option<&str>) -> String {
    match output {
        Some(o) => step.replace("{{output}}", o.trim_end()),
        None => step.to_string(),
    }
}

pub fn parse_time(s: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .map_err(|_| format!("CANVAS_INVALID: time `{s}` (HH:MM)"))
}

fn parse_at(s: &str) -> Result<DateTime<Local>, String> {
    DateTime::parse_from_rfc3339(s.trim())
        .map(|d| d.with_timezone(&Local))
        .map_err(|_| format!("CANVAS_INVALID: date `{s}` (RFC 3339)"))
}

fn at_local(date: chrono::NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&date.and_time(time)).earliest()
}

pub fn validate(r: &Routine) -> Result<(), String> {
    if r.name.trim().is_empty() {
        return Err("CANVAS_INVALID: routine name is empty".into());
    }
    if r.prompt.trim().is_empty() {
        return Err("CANVAS_INVALID: routine prompt is empty".into());
    }
    match &r.schedule {
        Schedule::Once { at } => {
            parse_at(at)?;
        }
        Schedule::Interval { start_at, .. } => {
            let secs = r.schedule.interval_secs().unwrap_or(0);
            if secs < MIN_INTERVAL_SECS {
                return Err(format!(
                    "CANVAS_INVALID: interval must be at least {MIN_INTERVAL_SECS} seconds"
                ));
            }
            if let Some(s) = start_at {
                parse_at(s)?;
            }
        }
        Schedule::Daily { time, .. } => {
            parse_time(time)?;
        }
        Schedule::Weekly { days, time, .. } => {
            parse_time(time)?;
            if days.is_empty() || days.iter().any(|d| *d > 6) {
                return Err("CANVAS_INVALID: weekly days are 0 (Sunday) to 6".into());
            }
        }
    }
    match &r.end {
        End::OnDate { date } => {
            parse_at(date)?;
        }
        End::AfterCount { count } if *count == 0 => {
            return Err("CANVAS_INVALID: end after 0 runs".into());
        }
        _ => {}
    }
    Ok(())
}

pub fn next_fire(r: &Routine) -> Option<DateTime<Local>> {
    if !r.enabled {
        return None;
    }
    if let End::AfterCount { count } = r.end {
        if r.run_count >= count {
            return None;
        }
    }
    if r.archived {
        return None;
    }
    let last = r.last_run_at.as_deref().and_then(|s| parse_at(s).ok());
    let anchor = r.anchor_at.as_deref().and_then(|s| parse_at(s).ok());
    let from = match (last, anchor) {
        (Some(l), Some(a)) => Some(l.max(a)),
        (l, a) => l.or(a),
    }
    .or_else(|| parse_at(&r.created_at).ok())
    .unwrap_or_else(Local::now);
    let next = match &r.schedule {
        Schedule::Once { at } => {
            if r.last_run_at.is_some() {
                return None;
            }
            parse_at(at).ok()?
        }
        Schedule::Interval { start_at, .. } => {
            let step = ChronoDuration::seconds(r.schedule.interval_secs().unwrap_or(60) as i64);
            match (
                last,
                anchor,
                start_at.as_deref().and_then(|s| parse_at(s).ok()),
            ) {
                (None, _, Some(start)) => start,
                (None, None, None) if step <= ChronoDuration::hours(1) => from,
                _ => from + step,
            }
        }
        Schedule::Daily { time, .. } => {
            let t = parse_time(time).ok()?;
            (0..3).find_map(|i| {
                at_local(from.date_naive() + ChronoDuration::days(i), t).filter(|c| *c > from)
            })?
        }
        Schedule::Weekly { days, time, .. } => {
            let t = parse_time(time).ok()?;
            (0..9).find_map(|i| {
                let d = from.date_naive() + ChronoDuration::days(i);
                if !days.contains(&(d.weekday().num_days_from_sunday() as u8)) {
                    return None;
                }
                at_local(d, t).filter(|c| *c > from)
            })?
        }
    };
    if let End::OnDate { date } = &r.end {
        if next > parse_at(date).ok()? {
            return None;
        }
    }
    Some(next)
}

static BOOK_LOCK: Mutex<()> = Mutex::new(());

fn book_path(store: &CanvasStore) -> std::path::PathBuf {
    store.root().join("routines.json")
}

fn load_all(store: &CanvasStore) -> Vec<Routine> {
    std::fs::read(book_path(store))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_all(store: &CanvasStore, all: &[Routine]) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(all).map_err(|e| e.to_string())?;
    persist::write_atomic(&book_path(store), &bytes)
}

fn edit<R>(
    store: &CanvasStore,
    f: impl FnOnce(&mut Vec<Routine>) -> Result<R, String>,
) -> Result<R, String> {
    let _g = BOOK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load_all(store);
    let r = f(&mut all)?;
    save_all(store, &all)?;
    Ok(r)
}

fn refresh_next(r: &mut Routine) {
    r.next_run_at = next_fire(r).map(|d| d.to_rfc3339());
}

pub fn list(store: &CanvasStore, ws: Option<&str>) -> Vec<Routine> {
    list_all(store, ws)
        .into_iter()
        .filter(|r| !r.archived)
        .collect()
}

pub fn list_all(store: &CanvasStore, ws: Option<&str>) -> Vec<Routine> {
    let _g = BOOK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load_all(store)
        .into_iter()
        .filter(|r| ws.is_none_or(|w| r.workspace_id == w))
        .collect()
}

pub fn get(store: &CanvasStore, id: &str) -> Result<Routine, String> {
    list_all(store, None)
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: routine {id}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub routine_id: String,
    pub routine_name: String,
    #[serde(default)]
    pub removed: bool,
    #[serde(flatten)]
    pub record: RunRecord,
}

pub fn history(store: &CanvasStore, ws: Option<&str>, id: Option<&str>) -> Vec<HistoryItem> {
    let mut out: Vec<HistoryItem> = list_all(store, ws)
        .into_iter()
        .filter(|r| id.is_none_or(|i| r.id == i))
        .flat_map(|r| {
            let (rid, name, removed) = (r.id.clone(), r.name.clone(), r.archived);
            r.history.into_iter().map(move |rec| HistoryItem {
                routine_id: rid.clone(),
                routine_name: name.clone(),
                removed,
                record: rec,
            })
        })
        .collect();
    out.sort_by(|a, b| b.record.at.cmp(&a.record.at));
    out
}

pub fn clear_history(
    store: &CanvasStore,
    ws: Option<&str>,
    id: Option<&str>,
) -> Result<usize, String> {
    edit(store, |all| {
        let mut n = 0;
        for r in all
            .iter_mut()
            .filter(|r| ws.is_none_or(|w| r.workspace_id == w) && id.is_none_or(|i| r.id == i))
        {
            n += r.history.len();
            r.history.clear();
        }
        all.retain(|r| {
            !(r.archived && ws.is_none_or(|w| r.workspace_id == w) && id.is_none_or(|i| r.id == i))
        });
        Ok(n)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub workspace_id: String,
    #[serde(default)]
    pub floor_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    pub prompt: String,
    pub schedule: Schedule,
    #[serde(default)]
    pub end: End,
    #[serde(default)]
    pub pre_run: Option<String>,
    #[serde(default)]
    pub skip_if_busy: Option<bool>,
    #[serde(default)]
    pub notify: bool,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub created_by: Option<String>,
}

pub fn save(store: &CanvasStore, input: RoutineInput) -> Result<Routine, String> {
    store.workspace(&input.workspace_id)?;
    edit(store, |all| {
        let existing = input
            .id
            .as_deref()
            .and_then(|id| all.iter().position(|r| r.id == id));
        let prev = existing.map(|i| all[i].clone());
        let mut schedule = input.schedule.clone();
        schedule.normalize()?;
        let schedule_changed = prev.as_ref().is_some_and(|p| p.schedule != schedule);
        let mut r = Routine {
            id: prev
                .as_ref()
                .map(|p| p.id.clone())
                .or(input.id.clone())
                .unwrap_or_else(new_id),
            name: input.name.trim().to_string(),
            workspace_id: input.workspace_id.clone(),
            floor_id: input.floor_id.clone().unwrap_or_else(ground),
            node_id: input.node_id.clone().filter(|n| !n.is_empty()),
            prompt: input.prompt.clone(),
            schedule,
            end: input.end.clone(),
            pre_run: input.pre_run.clone().filter(|s| !s.trim().is_empty()),
            skip_if_busy: input.skip_if_busy.unwrap_or(true),
            notify: input.notify,
            enabled: input.enabled.unwrap_or(true),
            created_at: prev
                .as_ref()
                .map(|p| p.created_at.clone())
                .unwrap_or_else(now_rfc3339),
            created_by: prev
                .as_ref()
                .and_then(|p| p.created_by.clone())
                .or(input.created_by.clone()),
            last_run_at: prev.as_ref().and_then(|p| p.last_run_at.clone()),
            next_run_at: None,
            run_count: prev.as_ref().map(|p| p.run_count).unwrap_or(0),
            history: prev.as_ref().map(|p| p.history.clone()).unwrap_or_default(),
            anchor_at: prev.as_ref().and_then(|p| p.anchor_at.clone()),
            archived: false,
        };
        if schedule_changed {
            r.anchor_at = Some(now_rfc3339());
            r.run_count = 0;
            if matches!(r.schedule, Schedule::Once { .. }) {
                r.last_run_at = None;
            }
        }
        validate(&r)?;
        refresh_next(&mut r);
        match existing {
            Some(i) => all[i] = r.clone(),
            None => all.push(r.clone()),
        }
        Ok(r)
    })
}

pub fn delete(store: &CanvasStore, id: &str) -> Result<(), String> {
    edit(store, |all| {
        let n = all.len();
        all.retain(|r| r.id != id);
        if all.len() == n {
            return Err(format!("CANVAS_NOT_FOUND: routine {id}"));
        }
        Ok(())
    })
}

pub fn replace_workspace(
    store: &CanvasStore,
    ws: &str,
    routines: Vec<Routine>,
) -> Result<usize, String> {
    edit(store, |all| {
        all.retain(|r| r.workspace_id != ws);
        let n = routines.len();
        for mut r in routines {
            r.workspace_id = ws.to_string();
            refresh_next(&mut r);
            all.retain(|x| x.id != r.id);
            all.push(r);
        }
        Ok(n)
    })
}

pub fn set_enabled(store: &CanvasStore, id: &str, enabled: bool) -> Result<Routine, String> {
    edit(store, |all| {
        let r = all
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("CANVAS_NOT_FOUND: routine {id}"))?;
        if enabled && !r.enabled {
            r.anchor_at = Some(now_rfc3339());
        }
        r.enabled = enabled;
        refresh_next(r);
        Ok(r.clone())
    })
}

pub fn due(store: &CanvasStore, now: DateTime<Local>) -> Vec<Routine> {
    list(store, None)
        .into_iter()
        .filter(|r| r.enabled)
        .filter(|r| next_fire(r).is_some_and(|n| n <= now))
        .collect()
}

pub fn record(store: &CanvasStore, id: &str, rec: RunRecord) -> Result<Routine, String> {
    edit(store, |all| {
        let r = all
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("CANVAS_NOT_FOUND: routine {id}"))?;
        if !rec.manual {
            r.last_run_at = Some(rec.at.clone());
        }
        if matches!(rec.status.as_str(), "fired" | "reminded") {
            r.run_count += 1;
        }
        let ran = matches!(rec.status.as_str(), "fired" | "reminded");
        let manual = rec.manual;
        r.history.insert(0, rec);
        r.history.truncate(HISTORY_MAX);
        refresh_next(r);
        if r.next_run_at.is_none() {
            r.enabled = false;
            if ran && !manual && matches!(r.schedule, Schedule::Once { .. }) {
                r.archived = true;
            }
        }
        Ok(r.clone())
    })
}

pub fn forget_node(store: &CanvasStore, node_id: &str) -> Result<usize, String> {
    edit(store, |all| {
        let n = all.len();
        all.retain(|r| r.node_id.as_deref() != Some(node_id));
        Ok(n - all.len())
    })
}
