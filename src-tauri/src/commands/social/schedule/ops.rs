use std::collections::HashSet;
use std::fmt::Display;
use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone};
use rusqlite::Connection;
use serde_json::{json, Value};

use super::super::args::{opt_str, req_str};
use super::super::model::Platform;
use super::db::{self, Feed, Filter, Finish, Item, NewItem};
use super::feed;
use super::rules::{self, Content, CsvRow, Entry, IcsEvent, Raw, Reject, Slot};

const DAY_MS: i64 = 86_400_000;
const MAX_CSV_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CSV_ROWS: usize = 500;
const MAX_FEEDS: usize = 20;
const REPORT_ROWS: usize = 100;
pub const OPEN_ONLY: &str = "Posts are published only while OmniGet is open on this computer. One that came due while it was closed is sent on start if it is less than the grace period late; otherwise it is marked missed and waits for a decision.";

pub struct Ctx<Tz: TimeZone> {
    pub now: DateTime<Tz>,
    pub limit: usize,
    pub by: String,
    pub export_dir: PathBuf,
}

fn sql<T>(r: rusqlite::Result<T>) -> Result<T, String> {
    r.map_err(|e| format!("STORE_FAILED: {e}"))
}

fn strings(a: &Value, key: &str) -> Vec<String> {
    a[key]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn raw(a: &Value) -> Raw {
    Raw {
        text: opt_str(a, "text"),
        thread: strings(a, "thread"),
        reply_to: opt_str(a, "reply_to"),
        quote: opt_str(a, "quote"),
        repost: opt_str(a, "repost"),
        media: strings(a, "media"),
    }
}

fn check_media(paths: &[String]) -> Result<(), Reject> {
    for p in paths {
        let meta = std::fs::metadata(p).map_err(|_| {
            Reject::new(
                "MEDIA_NOT_FOUND",
                format!("{p} does not exist on this computer"),
                "pass the absolute path of an image file",
            )
        })?;
        rules::image_type(p, meta.len())?;
    }
    Ok(())
}

fn need_id(a: &Value) -> Result<i64, String> {
    a["id"]
        .as_i64()
        .or_else(|| a["id"].as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| "INVALID_ARGUMENTS: id is required".to_string())
}

fn parts_sent(it: &Item) -> usize {
    it.result["parts"].as_array().map(Vec::len).unwrap_or(0)
}

fn editable(it: &Item) -> Result<(), Reject> {
    if matches!(
        it.status.as_str(),
        "draft" | "scheduled" | "failed" | "missed"
    ) {
        return Ok(());
    }
    Err(Reject::new(
        "NOT_EDITABLE",
        format!("item {} is {}", it.id, it.status),
        match it.status.as_str() {
            "sending" => "it is being published right now; read it again in a few seconds",
            "sent" => "it was already published; delete the post on the network with x_delete_post if needed",
            _ => "create a new item with op=add",
        },
    ))
}

pub fn feed_json<Tz: TimeZone>(f: &Feed, now: &DateTime<Tz>) -> Value
where
    Tz::Offset: Display,
{
    let iso = |ms: i64| {
        now.timezone()
            .timestamp_millis_opt(ms)
            .single()
            .map(|d| d.to_rfc3339())
    };
    json!({
        "id": f.id,
        "platform": f.platform,
        "url": f.url,
        "template": f.template,
        "interval_minutes": f.interval_min,
        "monthly_cap": f.monthly_cap,
        "posted_this_month": rules::used_this_month(&f.month, f.month_count as u32, &rules::month_key(now)),
        "skipped_over_cap": f.skipped,
        "primed": f.primed,
        "last_checked_at": f.last_checked_ms.and_then(iso),
        "next_check_at": f.last_checked_ms.and_then(|t| iso(t + f.interval_min * 60_000)),
        "last_error": f.last_error,
    })
}

impl<Tz: TimeZone> Ctx<Tz>
where
    Tz::Offset: Display,
{
    fn now_ms(&self) -> i64 {
        self.now.timestamp_millis()
    }

    fn iso(&self, ms: i64) -> Option<String> {
        self.now
            .timezone()
            .timestamp_millis_opt(ms)
            .single()
            .map(|d| d.to_rfc3339())
    }

    pub fn item_json(&self, it: &Item) -> Value {
        let c = Content::from_json(&it.payload);
        json!({
            "id": it.id,
            "platform": it.platform,
            "kind": it.kind,
            "status": it.status,
            "text": c.parts.first(),
            "thread": c.parts.iter().skip(1).collect::<Vec<_>>(),
            "reply_to": c.reply_to,
            "quote": c.quote,
            "repost": c.repost,
            "media": c.media,
            "at": it.due_ms.and_then(|ms| self.iso(ms)),
            "sent_at": it.sent_ms.and_then(|ms| self.iso(ms)),
            "post_id": it.post_id,
            "post_url": it.post_url,
            "parts_total": c.parts.len().max(1),
            "parts_sent": parts_sent(it),
            "parts": it.result["parts"].as_array().cloned().unwrap_or_default(),
            "error": it.error,
            "created_by": it.created_by,
            "created_at": self.iso(it.created_ms),
            "updated_at": self.iso(it.updated_ms),
        })
    }

    fn one(&self, c: &Connection, p: Platform, id: i64) -> Result<Value, String> {
        let it = sql(db::get(c, p.id(), id))?.ok_or_else(|| not_found(id))?;
        Ok(json!({ "item": self.item_json(&it) }))
    }

    fn next_slot(&self, c: &Connection, p: Platform) -> Result<Option<i64>, String> {
        let slots = sql(db::slots(c, p.id()))?;
        let taken = sql(db::taken(c, p.id()))?;
        Ok(rules::next_slot(&slots, &taken, &self.now))
    }

    fn settings(&self, c: &Connection, p: Platform) -> Value {
        json!({ "grace_minutes": db::grace_min(c, p.id()), "long_posts": db::long_posts(c, p.id()) })
    }

    pub fn list(&self, c: &Connection, p: Platform, a: &Value) -> Result<Value, String> {
        let statuses = match &a["status"] {
            Value::String(s) => vec![s.clone()],
            Value::Array(_) => strings(a, "status"),
            _ => Vec::new(),
        };
        if let Some(bad) = statuses
            .iter()
            .find(|s| !db::STATUSES.contains(&s.as_str()))
        {
            return Err(format!(
                "INVALID_ARGUMENTS: status {bad} is not one of {}",
                db::STATUSES.join(", ")
            ));
        }
        let bound = |key: &str| -> Result<Option<i64>, String> {
            match opt_str(a, key) {
                None => Ok(None),
                Some(s) => rules::instant(&s).map(Some).ok_or_else(|| {
                    format!("INVALID_ARGUMENTS: {key} must be an ISO 8601 time with an offset, like 2026-11-12T00:00:00-03:00")
                }),
            }
        };
        let limit = a["limit"].as_u64().unwrap_or(50).clamp(1, 200) as usize;
        let (items, total) = sql(db::list(
            c,
            &Filter {
                platform: p.id(),
                statuses: &statuses,
                from_ms: bound("from")?,
                to_ms: bound("to")?,
                id: a["id"].as_i64(),
                newest_first: a["order"].as_str() == Some("desc"),
                limit,
            },
        ))?;
        let counts: serde_json::Map<String, Value> = sql(db::counts(c, p.id()))?
            .into_iter()
            .map(|(k, v)| (k, json!(v)))
            .collect();
        let feeds: Vec<Value> = sql(db::feeds(c, Some(p.id())))?
            .iter()
            .map(|f| feed_json(f, &self.now))
            .collect();
        let slots: Vec<Value> = sql(db::slots(c, p.id()))?
            .into_iter()
            .map(Slot::to_json)
            .collect();
        let mut out = json!({
            "platform": p.id(),
            "items": items.iter().map(|it| self.item_json(it)).collect::<Vec<_>>(),
            "total": total,
            "returned": items.len(),
            "truncated": total > items.len(),
            "counts": counts,
            "slots": slots,
            "next_slot": self.next_slot(c, p)?.and_then(|ms| self.iso(ms)),
            "feeds": feeds,
            "settings": self.settings(c, p),
            "limits": { "text": self.limit, "thread_parts": rules::MAX_PARTS, "images": rules::MAX_MEDIA },
            "supported": p == Platform::X,
            "now": self.iso(self.now_ms()),
            "publishing": OPEN_ONLY,
        });
        if total == 0 {
            out["empty_reason"] = json!(if p == Platform::X {
                "nothing matches this filter; add a post with social_schedule op=add or op=draft"
            } else {
                "scheduling is built for X only; see docs/social/schedule.md for what is missing on this network"
            });
        }
        Ok(out)
    }

    fn due(&self, c: &Connection, p: Platform, a: &Value) -> Result<Option<i64>, String> {
        match (opt_str(a, "at"), opt_str(a, "slot").as_deref()) {
            (Some(_), Some(_)) => Err(Reject::new(
                "INVALID_TIME",
                "at and slot were both given",
                "pass at for an exact time or slot=next for the next free posting slot, not both",
            )
            .wire()),
            (Some(at), None) => Ok(Some(rules::parse_at(&at, self.now_ms())?)),
            (None, Some("next")) => Ok(Some(self.next_slot(c, p)?.ok_or_else(no_slots)?)),
            (None, Some(_)) => Err("INVALID_ARGUMENTS: slot must be next".into()),
            (None, None) => Ok(None),
        }
    }

    fn no_duplicate(
        &self,
        c: &Connection,
        p: Platform,
        fp: &str,
        except: Option<i64>,
    ) -> Result<(), String> {
        match sql(db::duplicate(c, p.id(), fp, self.now_ms() - DAY_MS, except))? {
            Some(id) => Err(duplicate(id).wire()),
            None => Ok(()),
        }
    }

    pub fn write(&self, c: &mut Connection, p: Platform, a: &Value) -> Result<Value, String> {
        match a["op"].as_str().unwrap_or_default() {
            "add" => self.add(c, p, a, false),
            "draft" => self.add(c, p, a, true),
            "update" => self.update(c, p, a),
            "cancel" => self.cancel(c, p, a),
            "send_now" => self.send_now(c, p, a),
            "set_slots" => self.set_slots(c, p, a),
            "import_csv" => self.import_csv(c, p, a),
            "export_ics" => self.export_ics(c, p),
            _ => Err("INVALID_ARGUMENTS: op must be add, update, cancel, draft, send_now, set_slots, import_csv or export_ics".into()),
        }
    }

    fn add(&self, c: &Connection, p: Platform, a: &Value, draft: bool) -> Result<Value, String> {
        let content = rules::content(raw(a), self.limit)?;
        check_media(&content.media)?;
        let due = self.due(c, p, a)?;
        if draft && due.is_some() {
            return Err(
                "INVALID_ARGUMENTS: op=draft saves without a time; use op=add to schedule".into(),
            );
        }
        if !draft && due.is_none() {
            return Err(Reject::new(
                "TIME_REQUIRED",
                "no time was given",
                "pass at (ISO 8601 with offset) or slot=next, or use op=draft to save without a time",
            )
            .wire());
        }
        let fp = content.fingerprint();
        if !draft {
            self.no_duplicate(c, p, &fp, None)?;
        }
        let id = sql(db::insert(
            c,
            &NewItem {
                platform: p.id(),
                kind: content.kind(),
                payload: &content.to_json(),
                fp: &fp,
                due_ms: due,
                status: if draft { "draft" } else { "scheduled" },
                by: &self.by,
            },
            self.now_ms(),
        ))?;
        self.one(c, p, id)
    }

    fn update(&self, c: &Connection, p: Platform, a: &Value) -> Result<Value, String> {
        let id = need_id(a)?;
        let it = sql(db::get(c, p.id(), id))?.ok_or_else(|| not_found(id))?;
        editable(&it)?;
        let has_content = ["text", "thread", "reply_to", "quote", "repost", "media"]
            .iter()
            .any(|k| !a[*k].is_null());
        let due = self.due(c, p, a)?;
        if !has_content && due.is_none() {
            return Err("INVALID_ARGUMENTS: pass the new content, at or slot=next".into());
        }
        let mut content = Content::from_json(&it.payload);
        if has_content {
            if parts_sent(&it) > 0 {
                return Err(Reject::new(
                    "PARTIALLY_SENT",
                    format!("{} of {} parts of item {id} are already published", parts_sent(&it), content.parts.len()),
                    "use op=send_now to publish the remaining parts, or op=cancel and add the rest as a new item",
                )
                .wire());
            }
            content = rules::content(raw(a), self.limit)?;
            check_media(&content.media)?;
        }
        let fp = content.fingerprint();
        if due.is_some() || it.status == "scheduled" {
            self.no_duplicate(c, p, &fp, Some(id))?;
        }
        if has_content {
            sql(db::replace(
                c,
                id,
                content.kind(),
                &content.to_json(),
                &fp,
                self.now_ms(),
            ))?;
        }
        if let Some(d) = due {
            sql(db::reschedule(c, id, d, false, self.now_ms()))?;
        }
        self.one(c, p, id)
    }

    fn cancel(&self, c: &Connection, p: Platform, a: &Value) -> Result<Value, String> {
        let id = need_id(a)?;
        let it = sql(db::get(c, p.id(), id))?.ok_or_else(|| not_found(id))?;
        editable(&it)?;
        sql(db::cancel(c, id, self.now_ms()))?;
        self.one(c, p, id)
    }

    fn send_now(&self, c: &Connection, p: Platform, a: &Value) -> Result<Value, String> {
        let id = need_id(a)?;
        let it = sql(db::get(c, p.id(), id))?.ok_or_else(|| not_found(id))?;
        editable(&it)?;
        sql(db::reschedule(c, id, self.now_ms(), true, self.now_ms()))?;
        let mut out = self.one(c, p, id)?;
        out["queued"] = json!(true);
        out["next_step"] = json!(format!(
            "it is published within seconds while OmniGet is open; read it with social_schedule_list id={id} to get post_url or error"
        ));
        Ok(out)
    }

    fn set_slots(&self, c: &mut Connection, p: Platform, a: &Value) -> Result<Value, String> {
        let slots = match a["slots"].as_array() {
            Some(list) => Some(
                list.iter()
                    .map(Slot::parse)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            None => None,
        };
        let grace = a["grace_minutes"].as_i64();
        let long = opt_str(a, "long_posts");
        if slots.is_none() && grace.is_none() && long.is_none() {
            return Err("INVALID_ARGUMENTS: pass slots, grace_minutes or long_posts".into());
        }
        if grace.is_some_and(|g| !(0..=1440).contains(&g)) {
            return Err("INVALID_ARGUMENTS: grace_minutes must be between 0 and 1440".into());
        }
        if long
            .as_deref()
            .is_some_and(|m| !matches!(m, "auto" | "on" | "off"))
        {
            return Err("INVALID_ARGUMENTS: long_posts must be auto, on or off".into());
        }
        if let Some(s) = &slots {
            sql(db::set_slots(c, p.id(), s))?;
        }
        if let Some(g) = grace {
            sql(db::set_setting(
                c,
                &format!("grace_minutes:{}", p.id()),
                &g.to_string(),
            ))?;
        }
        if let Some(m) = long {
            sql(db::set_setting(c, &format!("long_posts:{}", p.id()), &m))?;
        }
        let slots: Vec<Value> = sql(db::slots(c, p.id()))?
            .into_iter()
            .map(Slot::to_json)
            .collect();
        Ok(json!({
            "slots": slots,
            "next_slot": self.next_slot(c, p)?.and_then(|ms| self.iso(ms)),
            "settings": self.settings(c, p),
        }))
    }

    fn import_csv(&self, c: &mut Connection, p: Platform, a: &Value) -> Result<Value, String> {
        let path = req_str(a, "path")?;
        let meta = std::fs::metadata(&path).map_err(|_| {
            Reject::new(
                "FILE_NOT_FOUND",
                format!("{path} does not exist on this computer"),
                "pass the absolute path of a .csv file",
            )
            .wire()
        })?;
        if meta.len() > MAX_CSV_BYTES {
            return Err(Reject::new(
                "FILE_TOO_LARGE",
                "the CSV is larger than 2 MB",
                "split it into smaller files",
            )
            .wire());
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("FILE_NOT_FOUND: {e}"))?;
        let rows = rules::csv_rows(&String::from_utf8_lossy(&bytes))?;
        if rows.len() > MAX_CSV_ROWS {
            return Err(Reject::new(
                "TOO_MANY_ROWS",
                format!(
                    "the CSV has {} rows, the limit is {MAX_CSV_ROWS}",
                    rows.len()
                ),
                "split it into smaller files",
            )
            .wire());
        }
        self.import(c, p, &rows, a["dry_run"].as_bool().unwrap_or(false))
    }

    fn row(
        &self,
        c: &Connection,
        p: Platform,
        r: &CsvRow,
        slots: &[Slot],
        taken: &HashSet<i64>,
        fps: &HashSet<String>,
    ) -> Result<(Content, i64), Reject> {
        let content = rules::content(r.raw.clone(), self.limit)?;
        check_media(&content.media)?;
        let due = if r.time.eq_ignore_ascii_case("next") {
            rules::next_slot(slots, taken, &self.now).ok_or_else(no_slots)?
        } else {
            rules::parse_local(&r.time, &self.now)?
        };
        let fp = content.fingerprint();
        if fps.contains(&fp) {
            return Err(Reject::new(
                "DUPLICATE",
                "same content as an earlier row of this file",
                "remove one of the rows or change its text",
            ));
        }
        match db::duplicate(c, p.id(), &fp, self.now_ms() - DAY_MS, None) {
            Ok(None) => Ok((content, due)),
            Ok(Some(id)) => Err(duplicate(id)),
            Err(e) => Err(Reject::new("STORE_FAILED", e.to_string(), "try again")),
        }
    }

    pub fn import(
        &self,
        c: &mut Connection,
        p: Platform,
        rows: &[CsvRow],
        dry_run: bool,
    ) -> Result<Value, String> {
        let slots = sql(db::slots(c, p.id()))?;
        let mut taken = sql(db::taken(c, p.id()))?;
        let mut fps = HashSet::new();
        let mut valid = Vec::new();
        let mut rejected = Vec::new();
        for r in rows {
            match self.row(c, p, r, &slots, &taken, &fps) {
                Ok((content, due)) => {
                    taken.insert(due);
                    fps.insert(content.fingerprint());
                    valid.push((r.row, content, due));
                }
                Err(e) => rejected.push(json!({ "row": r.row, "code": e.code, "reason": e.message, "next_step": e.next_step })),
            }
        }
        let mut ids = Vec::new();
        if !dry_run {
            let tx = sql(c.transaction())?;
            for (_, content, due) in &valid {
                ids.push(sql(db::insert(
                    &tx,
                    &NewItem {
                        platform: p.id(),
                        kind: content.kind(),
                        payload: &content.to_json(),
                        fp: &content.fingerprint(),
                        due_ms: Some(*due),
                        status: "scheduled",
                        by: &self.by,
                    },
                    self.now_ms(),
                ))?);
            }
            sql(tx.commit())?;
        }
        let preview: Vec<Value> = valid
            .iter()
            .take(REPORT_ROWS)
            .map(|(row, content, due)| {
                json!({ "row": row, "at": self.iso(*due), "kind": content.kind(), "text": content.parts.first(), "repost": content.repost })
            })
            .collect();
        Ok(json!({
            "dry_run": dry_run,
            "rows": rows.len(),
            "valid": valid.len(),
            "scheduled": ids.len(),
            "ids": ids,
            "rejected_count": rejected.len(),
            "rejected": rejected.iter().take(REPORT_ROWS).collect::<Vec<_>>(),
            "preview": preview,
            "truncated": rejected.len() > REPORT_ROWS || valid.len() > REPORT_ROWS,
        }))
    }

    fn export_ics(&self, c: &Connection, p: Platform) -> Result<Value, String> {
        let statuses = ["scheduled".to_string(), "sent".to_string()];
        let (items, _) = sql(db::list(
            c,
            &Filter {
                platform: p.id(),
                statuses: &statuses,
                from_ms: None,
                to_ms: None,
                id: None,
                newest_first: false,
                limit: 5000,
            },
        ))?;
        let events: Vec<IcsEvent> = items
            .iter()
            .filter_map(|it| {
                let content = Content::from_json(&it.payload);
                let text = match &content.repost {
                    Some(id) => format!("Repost of https://x.com/i/status/{id}"),
                    None => content.parts.join("\n\n"),
                };
                let head: String = text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(60)
                    .collect();
                Some(IcsEvent {
                    uid: format!("social-{}-{}@omniget", it.platform, it.id),
                    at_ms: it.sent_ms.or(it.due_ms)?,
                    summary: format!("{}: {head}", p.label()),
                    description: text,
                    url: it.post_url.clone(),
                    sent: it.status == "sent",
                })
            })
            .collect();
        if events.is_empty() {
            return Err(Reject::new(
                "NOTHING_TO_EXPORT",
                "there is no scheduled or sent post to put in a calendar",
                "schedule a post first with op=add",
            )
            .wire());
        }
        let body = rules::ics(&format!("{} posts", p.label()), &events, self.now_ms());
        std::fs::create_dir_all(&self.export_dir).map_err(|e| format!("EXPORT_FAILED: {e}"))?;
        let stamp = self.now.format("%Y%m%d-%H%M%S");
        for n in 0..50 {
            let suffix = if n == 0 {
                String::new()
            } else {
                format!("-{n}")
            };
            let path = self
                .export_dir
                .join(format!("schedule-{}-{stamp}{suffix}.ics", p.id()));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    f.write_all(body.as_bytes())
                        .map_err(|e| format!("EXPORT_FAILED: {e}"))?;
                    let sent = events.iter().filter(|e| e.sent).count();
                    return Ok(json!({
                        "path": path.to_string_lossy(),
                        "events": events.len(),
                        "sent": sent,
                        "scheduled": events.len() - sent,
                        "next_step": "import the file in Google Calendar or Outlook; it is a snapshot, export again after changes",
                    }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("EXPORT_FAILED: {e}")),
            }
        }
        Err("EXPORT_FAILED: could not create a new file".into())
    }

    pub fn feeds(&self, c: &Connection, p: Platform, a: &Value) -> Result<Value, String> {
        match a["op"].as_str().unwrap_or_default() {
            "list" => {
                let feeds: Vec<Value> = sql(db::feeds(c, Some(p.id())))?
                    .iter()
                    .map(|f| feed_json(f, &self.now))
                    .collect();
                Ok(
                    json!({ "total": feeds.len(), "returned": feeds.len(), "truncated": false, "feeds": feeds, "publishing": OPEN_ONLY }),
                )
            }
            "add" => {
                let url = feed::check_url(&req_str(a, "url")?)?;
                let template = opt_str(a, "template").unwrap_or_else(|| "{title} {link}".into());
                rules::check_template(&template)?;
                let interval = a["interval_minutes"].as_i64().unwrap_or(60);
                let cap = a["monthly_cap"].as_i64().unwrap_or(30);
                if !(5..=1440).contains(&interval) || !(1..=3000).contains(&cap) {
                    return Err("INVALID_ARGUMENTS: interval_minutes must be 5 to 1440 and monthly_cap 1 to 3000".into());
                }
                let existing = sql(db::feeds(c, Some(p.id())))?;
                if existing.iter().any(|f| f.url == url) {
                    return Err(Reject::new(
                        "DUPLICATE",
                        "this feed is already added",
                        "remove it first to change its settings",
                    )
                    .wire());
                }
                if existing.len() >= MAX_FEEDS {
                    return Err(Reject::new(
                        "TOO_MANY_FEEDS",
                        format!("at most {MAX_FEEDS} feeds per network"),
                        "remove a feed first",
                    )
                    .wire());
                }
                let id = sql(db::feed_add(
                    c,
                    p.id(),
                    &url,
                    &template,
                    interval,
                    cap,
                    self.now_ms(),
                ))?;
                let f = sql(db::feed_get(c, id))?.ok_or_else(|| not_found(id))?;
                Ok(json!({
                    "feed": feed_json(&f, &self.now),
                    "note": "the first check only records the items the feed has now; items published after it are posted",
                }))
            }
            "remove" => {
                let id = need_id(a)?;
                if !sql(db::feed_remove(c, p.id(), id))? {
                    return Err(not_found(id));
                }
                Ok(json!({ "ok": true, "removed": id }))
            }
            _ => Err("INVALID_ARGUMENTS: op must be add, list or remove".into()),
        }
    }

    pub fn apply_feed(
        &self,
        c: &Connection,
        feed_id: i64,
        fetched: Result<Vec<Entry>, String>,
    ) -> Result<usize, String> {
        let Some(mut f) = sql(db::feed_get(c, feed_id))? else {
            return Ok(0);
        };
        let entries = match fetched {
            Ok(e) => e,
            Err(e) => {
                sql(db::feed_checked(c, &f, Some(&e), self.now_ms()))?;
                return Ok(0);
            }
        };
        let month = rules::month_key(&self.now);
        let same_month = f.month == month;
        let used = i64::from(rules::used_this_month(
            &f.month,
            f.month_count as u32,
            &month,
        ));
        let plan = rules::plan_feed(
            &f.seen,
            &entries,
            f.primed,
            (f.monthly_cap - used).max(0) as usize,
        );
        let by = format!("feed:{}", f.id);
        let mut queued = 0;
        for e in &plan.post {
            let Some(text) = rules::render(&f.template, &e.title, &e.link, rules::SHORT_LIMIT)
            else {
                continue;
            };
            let Ok(content) = rules::content(
                Raw {
                    text: Some(text),
                    ..Default::default()
                },
                rules::SHORT_LIMIT,
            ) else {
                continue;
            };
            let fp = content.fingerprint();
            if sql(db::duplicate(
                c,
                &f.platform,
                &fp,
                self.now_ms() - DAY_MS,
                None,
            ))?
            .is_some()
            {
                continue;
            }
            sql(db::insert(
                c,
                &NewItem {
                    platform: &f.platform,
                    kind: content.kind(),
                    payload: &content.to_json(),
                    fp: &fp,
                    due_ms: Some(self.now_ms() + queued * 1000),
                    status: "scheduled",
                    by: &by,
                },
                self.now_ms(),
            ))?;
            queued += 1;
        }
        f.skipped = (if same_month { f.skipped } else { 0 }) + plan.skipped as i64;
        f.month = month;
        f.month_count = used + queued;
        f.primed = true;
        f.seen = plan.seen;
        sql(db::feed_checked(c, &f, None, self.now_ms()))?;
        Ok(queued as usize)
    }
}

fn not_found(id: i64) -> String {
    Reject::new(
        "NOT_FOUND",
        format!("there is no item {id} on this network"),
        "list the items with social_schedule_list (or the feeds with social_feeds op=list) to get a valid id",
    )
    .wire()
}

fn no_slots() -> Reject {
    Reject::new(
        "NO_SLOTS",
        "there is no free posting slot",
        r#"define posting slots first with op=set_slots, e.g. slots=[{"weekday":"mon","time":"09:00"}], or pass at"#,
    )
}

fn duplicate(id: i64) -> Reject {
    Reject::new(
        "DUPLICATE",
        format!(
            "the same content is already scheduled or was sent in the last 24 hours (item {id})"
        ),
        format!("change the text, or cancel item {id} first"),
    )
}

pub fn record(
    c: &Connection,
    id: i64,
    parts: &[Value],
    error: Option<&str>,
    now: i64,
) -> rusqlite::Result<&'static str> {
    let status = if error.is_some() { "failed" } else { "sent" };
    let one = |k: &str| parts.first().and_then(|p| p[k].as_str());
    db::finish(
        c,
        id,
        &Finish {
            status,
            result: &json!({ "parts": parts }),
            post_id: one("id"),
            post_url: one("url"),
            error,
        },
        now,
    )?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::social::store;
    use chrono::{FixedOffset, NaiveDateTime};

    const X: Platform = Platform::X;

    fn ctx_at(when: &str) -> Ctx<FixedOffset> {
        let tz = FixedOffset::west_opt(3 * 3600).unwrap();
        Ctx {
            now: tz
                .from_local_datetime(
                    &NaiveDateTime::parse_from_str(when, "%Y-%m-%d %H:%M").unwrap(),
                )
                .unwrap(),
            limit: rules::SHORT_LIMIT,
            by: "agent:test".into(),
            export_dir: std::env::temp_dir(),
        }
    }

    fn ctx() -> Ctx<FixedOffset> {
        ctx_at("2026-11-11 10:00")
    }

    fn code(e: String) -> String {
        e.split(':').next().unwrap_or_default().to_string()
    }

    fn ids(v: &Value) -> Vec<i64> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_i64().unwrap())
            .collect()
    }

    #[test]
    fn schedule_tool_end_to_end() {
        let mut c = store::memory(db::SCHEMA);
        let cx = ctx();
        let empty = cx.list(&c, X, &json!({})).unwrap();
        assert_eq!(
            (empty["total"].as_u64(), empty["truncated"].as_bool()),
            (Some(0), Some(false))
        );
        assert!(empty["empty_reason"].is_string());
        assert_eq!(
            empty["settings"],
            json!({ "grace_minutes": 10, "long_posts": "auto" })
        );

        assert_eq!(
            code(
                cx.write(
                    &mut c,
                    X,
                    &json!({ "op": "add", "text": "a", "slot": "next" })
                )
                .unwrap_err()
            ),
            "NO_SLOTS"
        );
        let slots = cx
            .write(&mut c, X, &json!({ "op": "set_slots", "slots": [{ "weekday": "wed", "time": "18:00" }, { "weekday": "thu", "time": "09:00" }], "grace_minutes": 30 }))
            .unwrap();
        assert_eq!(slots["next_slot"], "2026-11-11T18:00:00-03:00");
        assert_eq!(slots["settings"]["grace_minutes"], 30);

        let a = cx
            .write(
                &mut c,
                X,
                &json!({ "op": "add", "text": "first", "slot": "next" }),
            )
            .unwrap();
        assert_eq!(a["item"]["at"], "2026-11-11T18:00:00-03:00");
        assert_eq!(
            (
                a["item"]["status"].as_str(),
                a["item"]["kind"].as_str(),
                a["item"]["created_by"].as_str()
            ),
            (Some("scheduled"), Some("post"), Some("agent:test"))
        );
        let b = cx
            .write(
                &mut c,
                X,
                &json!({ "op": "add", "text": "one", "thread": ["two"], "slot": "next" }),
            )
            .unwrap();
        assert_eq!(
            (b["item"]["at"].as_str(), b["item"]["kind"].as_str()),
            (Some("2026-11-12T09:00:00-03:00"), Some("thread"))
        );
        let r = cx
            .write(&mut c, X, &json!({ "op": "add", "repost": "https://x.com/a/status/42", "at": "2026-11-11T12:00:00-03:00" }))
            .unwrap();
        assert_eq!(
            (r["item"]["kind"].as_str(), r["item"]["repost"].as_str()),
            (Some("repost"), Some("42"))
        );

        let listed = cx.list(&c, X, &json!({ "status": "scheduled" })).unwrap();
        assert_eq!(ids(&listed), vec![3, 1, 2]);
        assert_eq!(listed["next_slot"], "2026-11-18T18:00:00-03:00");
        assert_eq!(listed["counts"]["scheduled"], 3);
        let ranged = cx
            .list(
                &c,
                X,
                &json!({ "from": "2026-11-12T00:00:00-03:00", "to": "2026-11-13T00:00:00-03:00" }),
            )
            .unwrap();
        assert_eq!(ids(&ranged), vec![2]);
        let page = cx
            .list(&c, X, &json!({ "limit": 1, "order": "desc" }))
            .unwrap();
        assert_eq!(
            (
                ids(&page),
                page["total"].as_u64(),
                page["truncated"].as_bool()
            ),
            (vec![2], Some(3), Some(true))
        );

        let moved = cx
            .write(&mut c, X, &json!({ "op": "update", "id": 1, "text": "first, edited", "at": "2026-11-13T08:00:00-03:00" }))
            .unwrap();
        assert_eq!(
            (moved["item"]["text"].as_str(), moved["item"]["at"].as_str()),
            (Some("first, edited"), Some("2026-11-13T08:00:00-03:00"))
        );
        let gone = cx
            .write(&mut c, X, &json!({ "op": "cancel", "id": 3 }))
            .unwrap();
        assert_eq!(gone["item"]["status"], "cancelled");
        assert_eq!(
            code(
                cx.write(&mut c, X, &json!({ "op": "cancel", "id": 3 }))
                    .unwrap_err()
            ),
            "NOT_EDITABLE"
        );
        assert_eq!(
            code(
                cx.write(&mut c, X, &json!({ "op": "cancel", "id": 99 }))
                    .unwrap_err()
            ),
            "NOT_FOUND"
        );
        assert_eq!(
            code(
                cx.write(
                    &mut c,
                    Platform::Instagram,
                    &json!({ "op": "cancel", "id": 1 })
                )
                .unwrap_err()
            ),
            "NOT_FOUND"
        );
        assert_eq!(ids(&cx.list(&c, X, &json!({})).unwrap()), vec![2, 1]);
    }

    #[test]
    fn rejects_bad_schedules() {
        let mut c = store::memory(db::SCHEMA);
        let cx = ctx();
        let mut add = |a: Value| cx.write(&mut c, X, &a).map_err(code);
        assert_eq!(
            add(json!({ "op": "add", "text": "a" })).unwrap_err(),
            "TIME_REQUIRED"
        );
        assert_eq!(
            add(json!({ "op": "add", "text": "a", "at": "2026-11-11T09:00:00-03:00" }))
                .unwrap_err(),
            "PAST_TIME"
        );
        assert_eq!(
            add(json!({ "op": "add", "text": "a", "at": "2026-11-12 09:00" })).unwrap_err(),
            "INVALID_TIME"
        );
        assert_eq!(
            add(json!({ "op": "add", "text": "a", "at": "2026-11-12T09:00:00Z", "slot": "next" }))
                .unwrap_err(),
            "INVALID_TIME"
        );
        assert_eq!(
            add(json!({ "op": "add", "text": "x".repeat(281), "at": "2026-11-12T09:00:00Z" }))
                .unwrap_err(),
            "TOO_LONG"
        );
        assert_eq!(add(json!({ "op": "add", "text": "a", "media": ["/nope/missing.png"], "at": "2026-11-12T09:00:00Z" })).unwrap_err(), "MEDIA_NOT_FOUND");
        assert_eq!(
            add(json!({ "op": "draft", "text": "a", "at": "2026-11-12T09:00:00Z" })).unwrap_err(),
            "INVALID_ARGUMENTS"
        );
        assert_eq!(
            add(json!({ "op": "explode" })).unwrap_err(),
            "INVALID_ARGUMENTS"
        );
        assert!(
            add(json!({ "op": "add", "text": "Same  text", "at": "2026-11-12T09:00:00Z" })).is_ok()
        );
        assert_eq!(
            add(json!({ "op": "add", "text": "same text", "at": "2026-11-13T09:00:00Z" }))
                .unwrap_err(),
            "DUPLICATE"
        );
        assert_eq!(
            add(json!({ "op": "set_slots" })).unwrap_err(),
            "INVALID_ARGUMENTS"
        );
        assert_eq!(
            add(json!({ "op": "set_slots", "slots": [{ "weekday": "mon", "time": "25:00" }] }))
                .unwrap_err(),
            "INVALID_SLOT"
        );
        assert_eq!(
            add(json!({ "op": "set_slots", "grace_minutes": 5000 })).unwrap_err(),
            "INVALID_ARGUMENTS"
        );
    }

    #[test]
    fn long_posts_follow_the_limit_of_the_context() {
        let mut c = store::memory(db::SCHEMA);
        let mut cx = ctx();
        cx.limit = rules::LONG_LIMIT;
        let long = "x".repeat(1000);
        assert!(cx
            .write(
                &mut c,
                X,
                &json!({ "op": "add", "text": long, "at": "2026-11-12T09:00:00Z" })
            )
            .is_ok());
    }

    #[test]
    fn drafts_send_now_and_partial_threads() {
        let mut c = store::memory(db::SCHEMA);
        let cx = ctx();
        let d = cx
            .write(
                &mut c,
                X,
                &json!({ "op": "draft", "text": "one", "thread": ["two", "three"] }),
            )
            .unwrap();
        assert_eq!(
            (d["item"]["status"].as_str(), d["item"]["at"].as_str()),
            (Some("draft"), None)
        );
        assert!(db::next_due(&c).unwrap().is_none());
        let s = cx
            .write(&mut c, X, &json!({ "op": "send_now", "id": 1 }))
            .unwrap();
        assert_eq!(
            (s["queued"].as_bool(), s["item"]["status"].as_str()),
            (Some(true), Some("scheduled"))
        );
        let due = db::next_due(&c).unwrap().unwrap();
        assert!(due.forced);
        assert!(db::claim(&c, 1, cx.now_ms()).unwrap());
        assert!(!db::claim(&c, 1, cx.now_ms()).unwrap());
        assert_eq!(
            code(
                cx.write(&mut c, X, &json!({ "op": "cancel", "id": 1 }))
                    .unwrap_err()
            ),
            "NOT_EDITABLE"
        );
        let parts = [json!({ "id": "100", "url": "https://x.com/me/status/100" })];
        assert_eq!(
            record(&c, 1, &parts, Some("REJECTED: nope"), cx.now_ms()).unwrap(),
            "failed"
        );
        let failed = cx.list(&c, X, &json!({ "id": 1 })).unwrap()["items"][0].clone();
        assert_eq!(
            (
                failed["status"].as_str(),
                failed["parts_sent"].as_u64(),
                failed["parts_total"].as_u64(),
                failed["error"].as_str()
            ),
            (Some("failed"), Some(1), Some(3), Some("REJECTED: nope"))
        );
        assert_eq!(
            code(
                cx.write(
                    &mut c,
                    X,
                    &json!({ "op": "update", "id": 1, "text": "changed" })
                )
                .unwrap_err()
            ),
            "PARTIALLY_SENT"
        );
        cx.write(&mut c, X, &json!({ "op": "send_now", "id": 1 }))
            .unwrap();
        let again = db::next_due(&c).unwrap().unwrap();
        assert_eq!(
            (again.result["parts"].as_array().unwrap().len(), again.error),
            (1, None)
        );
        db::claim(&c, 1, cx.now_ms()).unwrap();
        let all = [
            parts[0].clone(),
            json!({ "id": "101", "url": "u" }),
            json!({ "id": "102", "url": "u" }),
        ];
        assert_eq!(record(&c, 1, &all, None, cx.now_ms()).unwrap(), "sent");
        let sent = cx.list(&c, X, &json!({ "status": ["sent"] })).unwrap()["items"][0].clone();
        assert_eq!(
            (
                sent["post_id"].as_str(),
                sent["post_url"].as_str(),
                sent["sent_at"].is_string()
            ),
            (Some("100"), Some("https://x.com/me/status/100"), true)
        );
    }

    #[test]
    fn interrupted_sends_are_failed_on_start_not_retried() {
        let mut c = store::memory(db::SCHEMA);
        let cx = ctx();
        cx.write(
            &mut c,
            X,
            &json!({ "op": "add", "text": "a", "at": "2026-11-11T10:01:00-03:00" }),
        )
        .unwrap();
        db::claim(&c, 1, cx.now_ms()).unwrap();
        assert_eq!(db::recover_interrupted(&c, cx.now_ms()).unwrap(), 1);
        let it = db::get(&c, "x", 1).unwrap().unwrap();
        assert_eq!(it.status, "failed");
        assert!(it.error.unwrap().starts_with("INTERRUPTED: {"));
        assert!(db::next_due(&c).unwrap().is_none());
    }

    #[test]
    fn csv_import_reports_rows_and_dry_run_schedules_nothing() {
        let mut c = store::memory(db::SCHEMA);
        let cx = ctx();
        cx.write(
            &mut c,
            X,
            &json!({ "op": "set_slots", "slots": [{ "weekday": "thu", "time": "09:00" }] }),
        )
        .unwrap();
        let rows = rules::csv_rows(
            "time,text,thread,repost\n2026-11-12 08:00,hello,,\nnext,with thread,two ||| three,\nnext,second slot,,\n2020-01-01 08:00,old,,\nsoon,bad time,,\n2026-11-12 10:00,hello,,\n2026-11-12 11:00,,,https://x.com/a/status/7\n2026-11-12 12:00,,,\n",
        )
        .unwrap();
        let dry = cx.import(&mut c, X, &rows, true).unwrap();
        assert_eq!(
            (
                dry["rows"].as_u64(),
                dry["valid"].as_u64(),
                dry["scheduled"].as_u64(),
                dry["rejected_count"].as_u64()
            ),
            (Some(8), Some(4), Some(0), Some(4))
        );
        let reasons: Vec<(u64, &str)> = dry["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["row"].as_u64().unwrap(), r["code"].as_str().unwrap()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (5, "PAST_TIME"),
                (6, "INVALID_TIME"),
                (7, "DUPLICATE"),
                (9, "EMPTY_TEXT")
            ]
        );
        assert_eq!(dry["preview"][1]["at"], "2026-11-12T09:00:00-03:00");
        assert_eq!(dry["preview"][2]["at"], "2026-11-19T09:00:00-03:00");
        assert_eq!(cx.list(&c, X, &json!({})).unwrap()["total"], 0);

        let real = cx.import(&mut c, X, &rows, false).unwrap();
        assert_eq!(
            (
                real["scheduled"].as_u64(),
                real["ids"].as_array().map(Vec::len)
            ),
            (Some(4), Some(4))
        );
        let listed = cx.list(&c, X, &json!({})).unwrap();
        assert_eq!(listed["total"], 4);
        assert_eq!(listed["items"][1]["thread"], json!(["two", "three"]));
        assert_eq!(listed["items"][2]["kind"], "repost");
        let twice = cx.import(&mut c, X, &rows, false).unwrap();
        assert_eq!(
            (
                twice["scheduled"].as_u64(),
                twice["rejected_count"].as_u64()
            ),
            (Some(0), Some(8))
        );
    }

    #[test]
    fn csv_import_reads_a_file_and_refuses_a_missing_one() {
        let mut c = store::memory(db::SCHEMA);
        let cx = ctx();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("posts.csv");
        std::fs::write(&path, "time;text\n2026-11-12 08:00;\"olá; mundo\"\n").unwrap();
        let out = cx
            .write(
                &mut c,
                X,
                &json!({ "op": "import_csv", "path": path.to_string_lossy(), "dry_run": true }),
            )
            .unwrap();
        assert_eq!(
            (out["valid"].as_u64(), out["preview"][0]["text"].as_str()),
            (Some(1), Some("olá; mundo"))
        );
        assert_eq!(
            code(
                cx.write(
                    &mut c,
                    X,
                    &json!({ "op": "import_csv", "path": "/nope/x.csv" })
                )
                .unwrap_err()
            ),
            "FILE_NOT_FOUND"
        );
    }

    #[test]
    fn exports_a_new_ics_file_each_time() {
        let mut c = store::memory(db::SCHEMA);
        let dir = tempfile::tempdir().unwrap();
        let mut cx = ctx();
        cx.export_dir = dir.path().join("exports");
        assert_eq!(
            code(
                cx.write(&mut c, X, &json!({ "op": "export_ics" }))
                    .unwrap_err()
            ),
            "NOTHING_TO_EXPORT"
        );
        cx.write(
            &mut c,
            X,
            &json!({ "op": "add", "text": "calendar post", "at": "2026-11-12T09:00:00-03:00" }),
        )
        .unwrap();
        cx.write(
            &mut c,
            X,
            &json!({ "op": "draft", "text": "not in the calendar" }),
        )
        .unwrap();
        let one = cx.write(&mut c, X, &json!({ "op": "export_ics" })).unwrap();
        let two = cx.write(&mut c, X, &json!({ "op": "export_ics" })).unwrap();
        assert_ne!(one["path"], two["path"]);
        assert_eq!(
            (
                one["events"].as_u64(),
                one["scheduled"].as_u64(),
                one["sent"].as_u64()
            ),
            (Some(1), Some(1), Some(0))
        );
        let body = std::fs::read_to_string(one["path"].as_str().unwrap()).unwrap();
        assert!(body.contains("DTSTART:20261112T120000Z"));
        assert!(body.contains("SUMMARY:X: calendar post"));
        assert!(!body.contains("not in the calendar"));
    }

    fn entry(n: u32) -> Entry {
        Entry {
            id: format!("id-{n}"),
            title: format!("Post {n}"),
            link: format!("https://b.log/{n}"),
        }
    }

    #[test]
    fn feeds_tool_end_to_end() {
        let c = store::memory(db::SCHEMA);
        let cx = ctx();
        assert_eq!(
            code(
                cx.feeds(
                    &c,
                    X,
                    &json!({ "op": "add", "url": "http://localhost/feed" })
                )
                .unwrap_err()
            ),
            "INVALID_URL"
        );
        assert_eq!(
            code(
                cx.feeds(
                    &c,
                    X,
                    &json!({ "op": "add", "url": "https://b.log/feed", "template": "hello" })
                )
                .unwrap_err()
            ),
            "INVALID_TEMPLATE"
        );
        assert_eq!(
            code(
                cx.feeds(
                    &c,
                    X,
                    &json!({ "op": "add", "url": "https://b.log/feed", "interval_minutes": 1 })
                )
                .unwrap_err()
            ),
            "INVALID_ARGUMENTS"
        );
        let added = cx
            .feeds(&c, X, &json!({ "op": "add", "url": "https://b.log/feed", "template": "New: {title} {link}", "monthly_cap": 3 }))
            .unwrap();
        assert_eq!(
            (
                added["feed"]["id"].as_i64(),
                added["feed"]["interval_minutes"].as_i64(),
                added["feed"]["primed"].as_bool()
            ),
            (Some(1), Some(60), Some(false))
        );
        assert_eq!(
            code(
                cx.feeds(&c, X, &json!({ "op": "add", "url": "https://b.log/feed" }))
                    .unwrap_err()
            ),
            "DUPLICATE"
        );

        assert_eq!(
            cx.apply_feed(&c, 1, Err("FEED_UNREACHABLE: {}".into()))
                .unwrap(),
            0
        );
        let down = cx.feeds(&c, X, &json!({ "op": "list" })).unwrap();
        assert_eq!(
            (
                down["feeds"][0]["primed"].as_bool(),
                down["feeds"][0]["last_error"].is_string()
            ),
            (Some(false), true)
        );

        assert_eq!(
            cx.apply_feed(&c, 1, Ok(vec![entry(2), entry(1)])).unwrap(),
            0
        );
        assert_eq!(cx.list(&c, X, &json!({})).unwrap()["total"], 0);
        assert_eq!(
            cx.apply_feed(&c, 1, Ok(vec![entry(2), entry(1)])).unwrap(),
            0
        );

        let later = ctx_at("2026-11-11 11:00");
        let fresh = vec![
            entry(7),
            entry(6),
            entry(5),
            entry(4),
            entry(3),
            entry(2),
            entry(1),
        ];
        assert_eq!(later.apply_feed(&c, 1, Ok(fresh.clone())).unwrap(), 3);
        let queued = later.list(&c, X, &json!({})).unwrap();
        assert_eq!(queued["total"], 3);
        assert_eq!(queued["items"][0]["text"], "New: Post 3 https://b.log/3");
        assert_eq!(queued["items"][0]["created_by"], "feed:1");
        assert_eq!(queued["items"][2]["text"], "New: Post 5 https://b.log/5");
        let feed = &queued["feeds"][0];
        assert_eq!(
            (
                feed["posted_this_month"].as_u64(),
                feed["skipped_over_cap"].as_u64(),
                feed["last_error"].is_null()
            ),
            (Some(3), Some(2), true)
        );
        assert_eq!(later.apply_feed(&c, 1, Ok(fresh.clone())).unwrap(), 0);

        let december = ctx_at("2026-12-01 09:00");
        let mut next = fresh;
        next.insert(0, entry(8));
        assert_eq!(december.apply_feed(&c, 1, Ok(next)).unwrap(), 1);
        let listed = december.feeds(&c, X, &json!({ "op": "list" })).unwrap();
        assert_eq!(
            (
                listed["feeds"][0]["posted_this_month"].as_u64(),
                listed["feeds"][0]["skipped_over_cap"].as_u64()
            ),
            (Some(1), Some(0))
        );

        assert_eq!(
            december
                .feeds(&c, X, &json!({ "op": "remove", "id": 1 }))
                .unwrap()["ok"],
            true
        );
        assert_eq!(
            code(
                december
                    .feeds(&c, X, &json!({ "op": "remove", "id": 1 }))
                    .unwrap_err()
            ),
            "NOT_FOUND"
        );
        assert_eq!(
            december.feeds(&c, X, &json!({ "op": "list" })).unwrap()["total"],
            0
        );
    }
}
