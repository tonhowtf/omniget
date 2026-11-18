use std::collections::HashSet;

use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone, Utc};
use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;

use super::super::shortpost::tweet_id;

pub const SHORT_LIMIT: usize = 280;
pub const LONG_LIMIT: usize = 4000;
pub const MAX_PARTS: usize = 25;
pub const MAX_MEDIA: usize = 4;
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
pub const URL_WEIGHT: usize = 23;
const HORIZON_DAYS: i64 = 366;

#[derive(Debug, Clone, PartialEq)]
pub struct Reject {
    pub code: &'static str,
    pub message: String,
    pub next_step: String,
}

impl Reject {
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        next_step: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            next_step: next_step.into(),
        }
    }

    pub fn wire(&self) -> String {
        format!(
            "{}: {}",
            self.code,
            json!({ "message": self.message, "retryable": false, "next_step": self.next_step })
        )
    }
}

impl From<Reject> for String {
    fn from(r: Reject) -> Self {
        r.wire()
    }
}

fn light(c: char) -> bool {
    matches!(c as u32, 0..=4351 | 8192..=8205 | 8208..=8223 | 8242..=8247)
}

fn emoji_base(c: char) -> bool {
    matches!(c as u32, 0x2190..=0x2BFF | 0x1F000..=0x1FAFF)
}

fn regional(c: char) -> bool {
    matches!(c as u32, 0x1F1E6..=0x1F1FF)
}

fn emoji_modifier(c: char) -> bool {
    matches!(c as u32, 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

fn plain_weight(s: &str) -> usize {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut w = 0;
    while i < cs.len() {
        let c = cs[i];
        let keycap = (c.is_ascii_digit() || c == '#' || c == '*')
            && (cs.get(i + 1) == Some(&'\u{20E3}')
                || (cs.get(i + 1) == Some(&'\u{FE0F}') && cs.get(i + 2) == Some(&'\u{20E3}')));
        if keycap {
            w += 2;
            i += 1;
            while cs.get(i).is_some_and(|m| emoji_modifier(*m)) {
                i += 1;
            }
            continue;
        }
        if !emoji_base(c) {
            w += if light(c) { 1 } else { 2 };
            i += 1;
            continue;
        }
        w += 2;
        i += 1;
        if regional(c) && cs.get(i).is_some_and(|n| regional(*n)) {
            i += 1;
            continue;
        }
        loop {
            match cs.get(i) {
                Some(m) if emoji_modifier(*m) => i += 1,
                Some('\u{200D}') if cs.get(i + 1).is_some_and(|n| emoji_base(*n)) => i += 2,
                _ => break,
            }
        }
    }
    w
}

fn url_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)https?://[^\s<>"']+|\b(?:[a-z0-9-]+\.)+(?:com|org|net|io|co|dev|app|me|tv|ai|br|uk|de|fr|es|it|jp|ru|in|info|xyz|gg|ly|to|edu|gov|us|ca|au|nl|ch|se|pt|mx|ar)\b(?:/[^\s<>"']*)?"#,
        )
        .expect("static regex")
    })
}

pub fn weighted_len(text: &str) -> usize {
    let t: String = text.nfc().collect();
    let mut total = 0;
    let mut last = 0;
    for m in url_re().find_iter(&t) {
        let url = m
            .as_str()
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'']);
        let end = m.start() + url.len();
        total += plain_weight(&t[last..m.start()]) + URL_WEIGHT;
        last = end;
    }
    total + plain_weight(&t[last..])
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Raw {
    pub text: Option<String>,
    pub thread: Vec<String>,
    pub reply_to: Option<String>,
    pub quote: Option<String>,
    pub repost: Option<String>,
    pub media: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Content {
    pub parts: Vec<String>,
    pub reply_to: Option<String>,
    pub quote: Option<String>,
    pub repost: Option<String>,
    pub media: Vec<String>,
}

impl Content {
    pub fn kind(&self) -> &'static str {
        if self.repost.is_some() {
            "repost"
        } else if self.parts.len() > 1 {
            "thread"
        } else if self.reply_to.is_some() {
            "reply"
        } else if self.quote.is_some() {
            "quote"
        } else {
            "post"
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "parts": self.parts, "reply_to": self.reply_to, "quote": self.quote, "repost": self.repost, "media": self.media })
    }

    pub fn from_json(v: &Value) -> Self {
        let list = |k: &str| -> Vec<String> {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let one = |k: &str| v[k].as_str().map(String::from);
        Self {
            parts: list("parts"),
            reply_to: one("reply_to"),
            quote: one("quote"),
            repost: one("repost"),
            media: list("media"),
        }
    }

    pub fn fingerprint(&self) -> String {
        let norm = |s: &str| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        let parts: Vec<String> = self.parts.iter().map(|p| norm(p)).collect();
        format!(
            "{}|{}|{}|{}|{}",
            parts.join("\u{1f}"),
            self.reply_to.as_deref().unwrap_or_default(),
            self.quote.as_deref().unwrap_or_default(),
            self.repost.as_deref().unwrap_or_default(),
            self.media.join("\u{1f}")
        )
    }
}

fn target(raw: &Option<String>, field: &str) -> Result<Option<String>, Reject> {
    let Some(r) = raw.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    tweet_id(r).map(Some).ok_or_else(|| {
        Reject::new(
            "INVALID_TARGET",
            format!("{field} is not a post id or an x.com/.../status/... link"),
            format!("pass {field} as the numeric post id or the full link of the post"),
        )
    })
}

pub fn content(raw: Raw, limit: usize) -> Result<Content, Reject> {
    let repost = target(&raw.repost, "repost")?;
    let reply_to = target(&raw.reply_to, "reply_to")?;
    let quote = target(&raw.quote, "quote")?;
    let text = raw.text.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if repost.is_some() {
        if text.is_some()
            || !raw.thread.is_empty()
            || reply_to.is_some()
            || quote.is_some()
            || !raw.media.is_empty()
        {
            return Err(Reject::new(
                "CONFLICTING_TARGETS",
                "a repost carries no text, thread, reply, quote or media",
                "send repost alone, or use quote with text to comment on the post",
            ));
        }
        return Ok(Content {
            repost,
            ..Default::default()
        });
    }
    let Some(text) = text else {
        return Err(Reject::new(
            "EMPTY_TEXT",
            "text is required",
            "pass text, or repost with the post to repost",
        ));
    };
    let mut parts = vec![text.to_string()];
    for (i, t) in raw.thread.iter().enumerate() {
        let t = t.trim();
        if t.is_empty() {
            return Err(Reject::new(
                "EMPTY_TEXT",
                format!("thread part {} is empty", i + 2),
                "remove the empty part or give it text",
            ));
        }
        parts.push(t.to_string());
    }
    if parts.len() > MAX_PARTS {
        return Err(Reject::new(
            "TOO_MANY_PARTS",
            format!(
                "a thread has at most {MAX_PARTS} parts, this one has {}",
                parts.len()
            ),
            "merge parts or split the thread into two scheduled threads",
        ));
    }
    for (i, p) in parts.iter().enumerate() {
        let w = weighted_len(p);
        let chars = p.chars().count();
        if w > limit || chars > LONG_LIMIT {
            return Err(Reject::new(
                "TOO_LONG",
                format!(
                    "part {} counts {w} of {limit} on Short Posts (links count 23, emoji and CJK count 2)",
                    i + 1
                ),
                if limit < LONG_LIMIT {
                    "shorten it, split it into thread parts, or if the account has Short Posts Premium set long_posts=on with op=set_slots"
                } else {
                    "shorten it or split it into thread parts"
                },
            ));
        }
    }
    if raw.media.len() > MAX_MEDIA {
        return Err(Reject::new(
            "TOO_MANY_MEDIA",
            format!("a post carries at most {MAX_MEDIA} images"),
            "remove images or move them to another post",
        ));
    }
    Ok(Content {
        parts,
        reply_to,
        quote,
        repost: None,
        media: raw
            .media
            .iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect(),
    })
}

pub fn image_type(path: &str, bytes: u64) -> Result<&'static str, Reject> {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => {
            return Err(Reject::new(
                "UNSUPPORTED",
                format!("{path}: only JPEG, PNG and WebP images can be attached; GIF and video upload are not built"),
                "attach a still image, or publish the video by hand on Short Posts",
            ))
        }
    };
    if bytes == 0 || bytes > MAX_IMAGE_BYTES {
        return Err(Reject::new(
            "MEDIA_TOO_LARGE",
            format!("{path}: an image must be between 1 byte and 5 MB"),
            "export a smaller image",
        ));
    }
    Ok(mime)
}

fn check_range(ms: i64, now_ms: i64) -> Result<i64, Reject> {
    if ms <= now_ms {
        return Err(Reject::new(
            "PAST_TIME",
            "the time is in the past",
            "pass a future time, or use op=send_now to publish immediately",
        ));
    }
    if ms > now_ms + Duration::days(HORIZON_DAYS).num_milliseconds() {
        return Err(Reject::new(
            "TOO_FAR",
            "the time is more than a year ahead",
            "pass a time within the next 366 days",
        ));
    }
    Ok(ms)
}

pub fn instant(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M%:z"))
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%d %H:%M%:z"))
        .ok()
        .map(|d| d.timestamp_millis())
}

pub fn parse_at(s: &str, now_ms: i64) -> Result<i64, Reject> {
    let ms = instant(s.trim()).ok_or_else(|| {
        Reject::new(
            "INVALID_TIME",
            format!("{s} is not an ISO 8601 time with an offset"),
            "write the time like 2026-11-12T09:30:00-03:00 (or with Z for UTC)",
        )
    })?;
    check_range(ms, now_ms)
}

pub fn parse_local<Tz: TimeZone>(s: &str, now: &DateTime<Tz>) -> Result<i64, Reject> {
    let s = s.trim();
    if let Some(ms) = instant(s) {
        return check_range(ms, now.timestamp_millis());
    }
    let naive = ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .and_then(|n| now.timezone().from_local_datetime(&n).earliest())
        .ok_or_else(|| {
            Reject::new(
                "INVALID_TIME",
                format!("{s} is not a time"),
                "write the time as 2026-11-12 09:30 (this computer's time zone), as ISO 8601 with an offset, or as next",
            )
        })?;
    check_range(naive.timestamp_millis(), now.timestamp_millis())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot {
    pub weekday: u8,
    pub minute: u16,
}

pub const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl Slot {
    pub fn parse(v: &Value) -> Result<Self, Reject> {
        let bad = || {
            Reject::new(
                "INVALID_SLOT",
                format!("{v} is not a posting slot"),
                r#"write each slot as {"weekday":"mon","time":"09:30"} with a 24 hour time"#,
            )
        };
        let weekday = match &v["weekday"] {
            Value::String(s) => WEEKDAYS
                .iter()
                .position(|d| s.trim().to_ascii_lowercase().starts_with(d))
                .ok_or_else(bad)? as u8,
            Value::Number(n) => n.as_u64().filter(|n| *n < 7).ok_or_else(bad)? as u8,
            _ => return Err(bad()),
        };
        let time = v["time"].as_str().ok_or_else(bad)?;
        let (h, m) = time.trim().split_once(':').ok_or_else(bad)?;
        let h: u16 = h.parse().map_err(|_| bad())?;
        let m: u16 = m.parse().map_err(|_| bad())?;
        if h > 23 || m > 59 {
            return Err(bad());
        }
        Ok(Self {
            weekday,
            minute: h * 60 + m,
        })
    }

    pub fn to_json(self) -> Value {
        json!({ "weekday": WEEKDAYS[self.weekday as usize % 7], "time": format!("{:02}:{:02}", self.minute / 60, self.minute % 60) })
    }
}

pub fn next_slot<Tz: TimeZone>(
    slots: &[Slot],
    taken: &HashSet<i64>,
    now: &DateTime<Tz>,
) -> Option<i64> {
    let mut sorted = slots.to_vec();
    sorted.sort();
    let now_ms = now.timestamp_millis();
    let today = now.naive_local().date();
    for d in 0..=HORIZON_DAYS {
        let date = today + Duration::days(d);
        let weekday = date.weekday().num_days_from_monday() as u8;
        for s in sorted.iter().filter(|s| s.weekday == weekday) {
            let Some(naive) =
                date.and_hms_opt(u32::from(s.minute / 60), u32::from(s.minute % 60), 0)
            else {
                continue;
            };
            let Some(at) = now.timezone().from_local_datetime(&naive).earliest() else {
                continue;
            };
            let ms = at.timestamp_millis();
            if ms > now_ms && !taken.contains(&ms) {
                return Some(ms);
            }
        }
    }
    None
}

pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, Reject> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let head = text.lines().next().unwrap_or_default();
    let sep = if head.contains(';') && !head.contains(',') {
        ';'
    } else {
        ','
    };
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !was_quoted => {
                quoted = true;
                was_quoted = true;
            }
            c if c == sep => {
                row.push(std::mem::take(&mut field));
                was_quoted = false;
            }
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                was_quoted = false;
                if row.iter().any(|f| !f.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => field.push(c),
        }
    }
    if quoted {
        return Err(Reject::new(
            "INVALID_CSV",
            "a quoted field never closes",
            "close every \" and write a literal quote as \"\"",
        ));
    }
    row.push(field);
    if row.iter().any(|f| !f.trim().is_empty()) {
        rows.push(row);
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CsvRow {
    pub row: usize,
    pub time: String,
    pub raw: Raw,
}

pub const THREAD_SEPARATOR: &str = "|||";

pub fn csv_rows(text: &str) -> Result<Vec<CsvRow>, Reject> {
    let rows = parse_csv(text)?;
    let Some(header) = rows.first() else {
        return Err(Reject::new(
            "INVALID_CSV",
            "the file is empty",
            "the first line names the columns: time,text and optionally thread,reply,quote,repost,media",
        ));
    };
    let col = |names: &[&str]| {
        header
            .iter()
            .position(|h| names.contains(&h.trim().to_ascii_lowercase().as_str()))
    };
    let time = col(&["time", "at", "date"]);
    let text_col = col(&["text"]);
    let (Some(time), Some(text_col)) = (time, text_col) else {
        return Err(Reject::new(
            "INVALID_CSV",
            "the header needs the columns time and text",
            "the first line names the columns: time,text and optionally thread,reply,quote,repost,media",
        ));
    };
    let thread = col(&["thread"]);
    let reply = col(&["reply", "reply_to"]);
    let quote = col(&["quote"]);
    let repost = col(&["repost"]);
    let media = col(&["media"]);
    let cell = |r: &Vec<String>, i: Option<usize>| -> Option<String> {
        i.and_then(|i| r.get(i))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let split = |s: Option<String>, sep: &str| -> Vec<String> {
        s.map(|s| {
            s.split(sep)
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
    };
    Ok(rows
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, r)| CsvRow {
            row: i + 1,
            time: cell(r, Some(time)).unwrap_or_default(),
            raw: Raw {
                text: cell(r, Some(text_col)),
                thread: split(cell(r, thread), THREAD_SEPARATOR),
                reply_to: cell(r, reply),
                quote: cell(r, quote),
                repost: cell(r, repost),
                media: split(cell(r, media), ";"),
            },
        })
        .collect())
}

pub struct IcsEvent {
    pub uid: String,
    pub at_ms: i64,
    pub summary: String,
    pub description: String,
    pub url: Option<String>,
    pub sent: bool,
}

fn ics_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

fn ics_fold(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if width + n > 73 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out
}

fn ics_stamp(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_default()
}

pub fn ics(calendar: &str, events: &[IcsEvent], now_ms: i64) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//OmniGet//Social schedule//EN".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        format!("X-WR-CALNAME:{}", ics_escape(calendar)),
    ];
    for e in events {
        lines.push("BEGIN:VEVENT".into());
        lines.push(format!("UID:{}", e.uid));
        lines.push(format!("DTSTAMP:{}", ics_stamp(now_ms)));
        lines.push(format!("DTSTART:{}", ics_stamp(e.at_ms)));
        lines.push(format!("DTEND:{}", ics_stamp(e.at_ms + 15 * 60_000)));
        lines.push(format!("SUMMARY:{}", ics_escape(&e.summary)));
        lines.push(format!("DESCRIPTION:{}", ics_escape(&e.description)));
        if let Some(u) = &e.url {
            lines.push(format!("URL:{u}"));
        }
        lines.push(format!(
            "STATUS:{}",
            if e.sent { "CONFIRMED" } else { "TENTATIVE" }
        ));
        lines.push("END:VEVENT".into());
    }
    lines.push("END:VCALENDAR".into());
    let mut out: String = lines
        .iter()
        .map(|l| ics_fold(l))
        .collect::<Vec<_>>()
        .join("\r\n");
    out.push_str("\r\n");
    out
}

pub fn check_template(t: &str) -> Result<(), Reject> {
    if t.chars().count() > 500 || !(t.contains("{link}") || t.contains("{title}")) {
        return Err(Reject::new(
            "INVALID_TEMPLATE",
            "the template must contain {title} or {link} and be at most 500 characters",
            "use a template such as \"{title} {link}\"",
        ));
    }
    Ok(())
}

pub fn render(template: &str, title: &str, link: &str, limit: usize) -> Option<String> {
    let fill = |title: &str| {
        template
            .replace("{title}", title)
            .replace("{link}", link)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = fill(&title);
    if full.is_empty() {
        return None;
    }
    if weighted_len(&full) <= limit {
        return Some(full);
    }
    if !template.contains("{title}") {
        return None;
    }
    let chars: Vec<char> = title.chars().collect();
    for keep in (0..chars.len()).rev() {
        let short: String = chars[..keep].iter().collect();
        let cut = format!("{}…", short.trim_end());
        let out = fill(if keep == 0 { "" } else { &cut });
        if !out.is_empty() && weighted_len(&out) <= limit {
            return Some(out);
        }
    }
    None
}

pub fn month_key<Tz: TimeZone>(now: &DateTime<Tz>) -> String {
    let d = now.naive_local().date();
    format!("{:04}-{:02}", d.year(), d.month())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub link: String,
}

#[derive(Debug, PartialEq)]
pub struct FeedPlan {
    pub post: Vec<Entry>,
    pub skipped: usize,
    pub seen: Vec<String>,
}

pub const SEEN_KEEP: usize = 500;

pub fn plan_feed(seen: &[String], entries: &[Entry], primed: bool, remaining: usize) -> FeedPlan {
    let known: HashSet<&str> = seen.iter().map(String::as_str).collect();
    let mut fresh: Vec<Entry> = Vec::new();
    for e in entries.iter().rev() {
        if !known.contains(e.id.as_str()) && !fresh.iter().any(|f| f.id == e.id) {
            fresh.push(e.clone());
        }
    }
    let mut all: Vec<String> = seen.to_vec();
    all.extend(fresh.iter().map(|e| e.id.clone()));
    if all.len() > SEEN_KEEP {
        all.drain(..all.len() - SEEN_KEEP);
    }
    if !primed {
        return FeedPlan {
            post: Vec::new(),
            skipped: 0,
            seen: all,
        };
    }
    let skipped = fresh.len().saturating_sub(remaining);
    fresh.truncate(remaining);
    FeedPlan {
        post: fresh,
        skipped,
        seen: all,
    }
}

pub fn used_this_month(stored_month: &str, stored_count: u32, month: &str) -> u32 {
    if stored_month == month {
        stored_count
    } else {
        0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Due {
    Send,
    Missed,
}

pub fn decide(now_ms: i64, due_ms: i64, grace_ms: i64, exempt: bool) -> Due {
    if !exempt && now_ms - due_ms > grace_ms {
        Due::Missed
    } else {
        Due::Send
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn raw(text: &str) -> Raw {
        Raw {
            text: Some(text.into()),
            ..Default::default()
        }
    }

    #[test]
    fn counts_like_x() {
        assert_eq!(weighted_len("hello"), 5);
        assert_eq!(weighted_len("olá, São Paulo"), 14);
        assert_eq!(weighted_len("日本語"), 6);
        assert_eq!(weighted_len("a 🔥"), 4);
        assert_eq!(weighted_len("👨‍👩‍👧"), 2);
        assert_eq!(weighted_len("👍🏽"), 2);
        assert_eq!(weighted_len("🇧🇷"), 2);
        assert_eq!(weighted_len("1️⃣"), 2);
        assert_eq!(
            weighted_len("read https://example.com/a/very/long/path/that/goes/on/and/on?x=1"),
            5 + 23
        );
        assert_eq!(weighted_len("see example.com."), 4 + 23 + 1);
        assert_eq!(weighted_len("e\u{301}"), 1);
    }

    #[test]
    fn length_limit_is_weighted_and_capped() {
        assert!(content(raw(&"a".repeat(280)), SHORT_LIMIT).is_ok());
        assert_eq!(
            content(raw(&"a".repeat(281)), SHORT_LIMIT)
                .unwrap_err()
                .code,
            "TOO_LONG"
        );
        assert_eq!(
            content(raw(&"日".repeat(141)), SHORT_LIMIT)
                .unwrap_err()
                .code,
            "TOO_LONG"
        );
        assert!(content(raw(&"a".repeat(4000)), LONG_LIMIT).is_ok());
        assert_eq!(
            content(raw(&"a".repeat(4001)), LONG_LIMIT)
                .unwrap_err()
                .code,
            "TOO_LONG"
        );
        let long_link = format!(
            "{} https://example.com/{}",
            "a".repeat(256),
            "b".repeat(300)
        );
        assert!(content(raw(&long_link), SHORT_LIMIT).is_ok());
    }

    #[test]
    fn builds_each_kind() {
        assert_eq!(content(raw("oi"), 280).unwrap().kind(), "post");
        let mut r = raw("one");
        r.thread = vec!["two".into(), " three ".into()];
        let c = content(r, 280).unwrap();
        assert_eq!(c.kind(), "thread");
        assert_eq!(c.parts, vec!["one", "two", "three"]);
        let mut r = raw("re");
        r.reply_to = Some("https://x.com/a/status/123".into());
        let c = content(r, 280).unwrap();
        assert_eq!((c.kind(), c.reply_to.as_deref()), ("reply", Some("123")));
        let mut r = raw("look");
        r.quote = Some("456".into());
        assert_eq!(content(r, 280).unwrap().kind(), "quote");
        let r = Raw {
            repost: Some("https://x.com/a/status/789".into()),
            ..Default::default()
        };
        let c = content(r, 280).unwrap();
        assert_eq!(
            (c.kind(), c.repost.as_deref(), c.parts.len()),
            ("repost", Some("789"), 0)
        );
        assert_eq!(Content::from_json(&c.to_json()), c);
    }

    #[test]
    fn rejects_bad_content() {
        assert_eq!(content(raw("  "), 280).unwrap_err().code, "EMPTY_TEXT");
        let mut r = raw("a");
        r.thread = vec!["".into()];
        assert_eq!(content(r, 280).unwrap_err().code, "EMPTY_TEXT");
        let mut r = raw("a");
        r.thread = vec!["b".to_string(); 25];
        assert_eq!(content(r, 280).unwrap_err().code, "TOO_MANY_PARTS");
        let mut r = raw("a");
        r.thread = vec!["b".to_string(); 24];
        assert!(content(r, 280).is_ok());
        let mut r = raw("a");
        r.reply_to = Some("not a post".into());
        assert_eq!(content(r, 280).unwrap_err().code, "INVALID_TARGET");
        let mut r = raw("a");
        r.repost = Some("1".into());
        assert_eq!(content(r, 280).unwrap_err().code, "CONFLICTING_TARGETS");
        let mut r = raw("a");
        r.media = vec!["a.png".to_string(); 5];
        assert_eq!(content(r, 280).unwrap_err().code, "TOO_MANY_MEDIA");
    }

    #[test]
    fn fingerprint_ignores_case_and_spacing() {
        let a = content(raw("Hello   World"), 280).unwrap();
        let b = content(raw("hello world"), 280).unwrap();
        let c = content(raw("hello world!"), 280).unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_ne!(a.fingerprint(), c.fingerprint());
    }

    #[test]
    fn image_rules() {
        assert_eq!(image_type("/a/b.JPG", 10).unwrap(), "image/jpeg");
        assert_eq!(image_type("/a/b.webp", 10).unwrap(), "image/webp");
        assert_eq!(image_type("/a/b.mp4", 10).unwrap_err().code, "UNSUPPORTED");
        assert_eq!(image_type("/a/b.gif", 10).unwrap_err().code, "UNSUPPORTED");
        assert_eq!(
            image_type("/a/b.png", MAX_IMAGE_BYTES + 1)
                .unwrap_err()
                .code,
            "MEDIA_TOO_LARGE"
        );
    }

    const NOW: i64 = 1_794_000_000_000;

    #[test]
    fn parses_times_with_an_offset_only() {
        let at = parse_at("2026-11-12T09:30:00-03:00", NOW).unwrap();
        assert_eq!(at, parse_at("2026-11-12T12:30:00Z", NOW).unwrap());
        assert_eq!(at, parse_at("2026-11-12T09:30-03:00", NOW).unwrap());
        assert_eq!(
            parse_at("2026-11-12T09:30:00", NOW).unwrap_err().code,
            "INVALID_TIME"
        );
        assert_eq!(parse_at("tomorrow", NOW).unwrap_err().code, "INVALID_TIME");
        assert_eq!(
            parse_at("2020-01-01T00:00:00Z", NOW).unwrap_err().code,
            "PAST_TIME"
        );
        assert_eq!(
            parse_at("2029-01-01T00:00:00Z", NOW).unwrap_err().code,
            "TOO_FAR"
        );
    }

    #[test]
    fn csv_times_without_offset_use_the_local_zone() {
        let tz = FixedOffset::west_opt(3 * 3600).unwrap();
        let now = tz.timestamp_millis_opt(NOW).unwrap();
        let local = parse_local("2026-11-12 09:30", &now).unwrap();
        assert_eq!(local, parse_at("2026-11-12T09:30:00-03:00", NOW).unwrap());
        assert_eq!(parse_local("2026-11-12T12:30:00Z", &now).unwrap(), local);
        assert_eq!(
            parse_local("12/11/2026", &now).unwrap_err().code,
            "INVALID_TIME"
        );
        assert_eq!(
            parse_local("2020-01-01 10:00", &now).unwrap_err().code,
            "PAST_TIME"
        );
    }

    #[test]
    fn parses_slots() {
        let s = Slot::parse(&json!({ "weekday": "Wed", "time": "09:05" })).unwrap();
        assert_eq!(
            s,
            Slot {
                weekday: 2,
                minute: 545
            }
        );
        assert_eq!(s.to_json(), json!({ "weekday": "wed", "time": "09:05" }));
        assert_eq!(
            Slot::parse(&json!({ "weekday": 6, "time": "23:59" }))
                .unwrap()
                .weekday,
            6
        );
        for bad in [
            json!({ "weekday": "xyz", "time": "09:00" }),
            json!({ "weekday": 7, "time": "09:00" }),
            json!({ "weekday": "mon", "time": "24:00" }),
            json!({ "weekday": "mon", "time": "9" }),
            json!({ "weekday": "mon" }),
        ] {
            assert_eq!(Slot::parse(&bad).unwrap_err().code, "INVALID_SLOT");
        }
    }

    fn at(tz: &FixedOffset, s: &str) -> DateTime<FixedOffset> {
        tz.from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
            .unwrap()
    }

    #[test]
    fn next_slot_walks_weekdays_and_skips_taken() {
        let tz = FixedOffset::west_opt(3 * 3600).unwrap();
        let slots = [
            Slot {
                weekday: 0,
                minute: 9 * 60,
            },
            Slot {
                weekday: 2,
                minute: 18 * 60,
            },
            Slot {
                weekday: 2,
                minute: 8 * 60,
            },
        ];
        let now = at(&tz, "2026-11-11 10:00");
        let mut taken = HashSet::new();
        let first = next_slot(&slots, &taken, &now).unwrap();
        assert_eq!(first, at(&tz, "2026-11-11 18:00").timestamp_millis());
        taken.insert(first);
        let second = next_slot(&slots, &taken, &now).unwrap();
        assert_eq!(second, at(&tz, "2026-11-16 09:00").timestamp_millis());
        taken.insert(second);
        assert_eq!(
            next_slot(&slots, &taken, &now).unwrap(),
            at(&tz, "2026-11-18 08:00").timestamp_millis()
        );
        assert_eq!(next_slot(&[], &taken, &now), None);
        let exact = at(&tz, "2026-11-11 18:00");
        assert_eq!(
            next_slot(&slots, &HashSet::new(), &exact).unwrap(),
            at(&tz, "2026-11-16 09:00").timestamp_millis()
        );
    }

    #[test]
    fn next_slot_uses_the_weekday_of_the_local_zone() {
        let slots = [Slot {
            weekday: 3,
            minute: 30,
        }];
        let instant = Utc.with_ymd_and_hms(2026, 11, 11, 23, 0, 0).unwrap();
        let tokyo = FixedOffset::east_opt(9 * 3600).unwrap();
        let sao_paulo = FixedOffset::west_opt(3 * 3600).unwrap();
        let in_tokyo = next_slot(&slots, &HashSet::new(), &instant.with_timezone(&tokyo)).unwrap();
        let in_sp = next_slot(&slots, &HashSet::new(), &instant.with_timezone(&sao_paulo)).unwrap();
        assert_eq!(
            in_tokyo,
            Utc.with_ymd_and_hms(2026, 11, 18, 15, 30, 0)
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(
            in_sp,
            Utc.with_ymd_and_hms(2026, 11, 12, 3, 30, 0)
                .unwrap()
                .timestamp_millis()
        );
    }

    #[test]
    fn csv_handles_quotes_newlines_and_semicolons() {
        let rows = parse_csv("\u{feff}time,text\r\n2026-11-12 09:00,\"Hello, \"\"world\"\"\nline two\"\r\n\r\n2026-11-13 09:00,plain\n").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1][1], "Hello, \"world\"\nline two");
        assert_eq!(rows[2], vec!["2026-11-13 09:00", "plain"]);
        let rows = parse_csv("time;text\n2026-11-12 09:00;a, b").unwrap();
        assert_eq!(rows[1], vec!["2026-11-12 09:00", "a, b"]);
        assert_eq!(
            parse_csv("time,text\nx,\"open").unwrap_err().code,
            "INVALID_CSV"
        );
    }

    #[test]
    fn csv_rows_map_columns_by_name() {
        let rows = csv_rows("Text,Time,thread,reply,quote,repost,media\nfirst,next,two ||| three,,99,,a.png; b.jpg\n,2026-11-12 09:00,,,,https://x.com/a/status/5,\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].row, 2);
        assert_eq!(rows[0].time, "next");
        assert_eq!(rows[0].raw.thread, vec!["two", "three"]);
        assert_eq!(rows[0].raw.quote.as_deref(), Some("99"));
        assert_eq!(rows[0].raw.media, vec!["a.png", "b.jpg"]);
        assert_eq!(
            rows[1].raw.repost.as_deref(),
            Some("https://x.com/a/status/5")
        );
        assert_eq!(rows[1].raw.text, None);
        assert_eq!(csv_rows("when,text\n").unwrap_err().code, "INVALID_CSV");
        assert_eq!(csv_rows("").unwrap_err().code, "INVALID_CSV");
    }

    #[test]
    fn ics_escapes_and_folds() {
        let out = ics(
            "Short Posts",
            &[IcsEvent {
                uid: "social-1@omniget".into(),
                at_ms: Utc
                    .with_ymd_and_hms(2026, 11, 12, 12, 30, 0)
                    .unwrap()
                    .timestamp_millis(),
                summary: "Hello, world; ok".into(),
                description: format!("line one\nline two {}", "x".repeat(100)),
                url: Some("https://x.com/i/status/1".into()),
                sent: true,
            }],
            NOW,
        );
        assert!(out.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"));
        assert!(out.contains("DTSTART:20261112T123000Z\r\n"));
        assert!(out.contains("DTEND:20261112T124500Z\r\n"));
        assert!(out.contains("SUMMARY:Hello\\, world\\; ok\r\n"));
        assert!(out.contains("DESCRIPTION:line one\\nline two"));
        assert!(out.contains("STATUS:CONFIRMED\r\n"));
        assert!(out.ends_with("END:VCALENDAR\r\n"));
        assert!(out.split("\r\n").all(|l| l.len() <= 75));
        assert!(out.contains("\r\n x"));
    }

    #[test]
    fn renders_templates_within_the_limit() {
        assert_eq!(
            render("{title} {link}", "A  post", "https://b.log/1", 280).unwrap(),
            "A post https://b.log/1"
        );
        let long = "word ".repeat(100);
        let out = render("New: {title} {link}", &long, "https://b.log/1", 280).unwrap();
        assert!(weighted_len(&out) <= 280);
        assert!(out.ends_with("… https://b.log/1"));
        assert!(out.starts_with("New: word"));
        assert_eq!(render("{link}", "t", "https://b.log/1", 10), None);
        assert_eq!(render("{title}", "", "", 280), None);
        assert!(check_template("{title} {link}").is_ok());
        assert_eq!(
            check_template("no placeholders").unwrap_err().code,
            "INVALID_TEMPLATE"
        );
    }

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            title: id.into(),
            link: format!("https://b.log/{id}"),
        }
    }

    #[test]
    fn first_feed_check_only_records() {
        let plan = plan_feed(&[], &[entry("c"), entry("b"), entry("a")], false, 30);
        assert!(plan.post.is_empty());
        assert_eq!(plan.seen, vec!["a", "b", "c"]);
    }

    #[test]
    fn later_checks_post_new_items_oldest_first_under_the_cap() {
        let seen = vec!["a".to_string(), "b".to_string()];
        let entries = [entry("e"), entry("d"), entry("c"), entry("b"), entry("a")];
        let plan = plan_feed(&seen, &entries, true, 30);
        assert_eq!(
            plan.post.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "d", "e"]
        );
        assert_eq!(plan.skipped, 0);
        let capped = plan_feed(&seen, &entries, true, 2);
        assert_eq!(
            capped
                .post
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "d"]
        );
        assert_eq!(capped.skipped, 1);
        assert_eq!(capped.seen, vec!["a", "b", "c", "d", "e"]);
        assert!(plan_feed(&capped.seen, &entries, true, 30).post.is_empty());
    }

    #[test]
    fn monthly_counter_resets_on_a_new_month() {
        assert_eq!(used_this_month("2026-11", 12, "2026-11"), 12);
        assert_eq!(used_this_month("2026-10", 30, "2026-11"), 0);
        let tz = FixedOffset::west_opt(3 * 3600).unwrap();
        let now = Utc
            .with_ymd_and_hms(2026, 12, 1, 1, 0, 0)
            .unwrap()
            .with_timezone(&tz);
        assert_eq!(month_key(&now), "2026-11");
    }

    #[test]
    fn late_items_are_missed_past_the_grace() {
        let grace = 10 * 60_000;
        assert_eq!(decide(NOW, NOW - 5_000, grace, false), Due::Send);
        assert_eq!(decide(NOW, NOW - grace, grace, false), Due::Send);
        assert_eq!(decide(NOW, NOW - grace - 1, grace, false), Due::Missed);
        assert_eq!(decide(NOW, NOW - grace - 1, grace, true), Due::Send);
    }

    #[test]
    fn rejections_carry_a_next_step() {
        let w = Reject::new("PAST_TIME", "m", "n").wire();
        let (code, rest) = w.split_once(": ").unwrap();
        assert_eq!(code, "PAST_TIME");
        let v: Value = serde_json::from_str(rest).unwrap();
        assert_eq!(
            v,
            json!({ "message": "m", "retryable": false, "next_step": "n" })
        );
    }
}
