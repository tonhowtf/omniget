//! date, entities, vector and links. Adapted from Hindsight's concise

use std::sync::Arc;

use chrono::{Local, NaiveDate, TimeZone};
use futures::future::BoxFuture;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::ctx::{AssistCtx, Scope};
use super::super::db::AssistDb;
use super::graph::{self, EntityRef};
use super::retrieve::{embed_passages, vector_source};
use super::{remember, sha, Category, NewMemory, Source};

pub type Llm =
    Arc<dyn Fn(String, String) -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

pub fn default_llm() -> Llm {
    Arc::new(|system, user| {
        Box::pin(async move {
            static ONE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
            let _turn = ONE.acquire().await.map_err(|e| e.to_string())?;
            if !crate::core::ai::get().model.is_empty() {
                return crate::core::ai::chat(&system, &user).await;
            }
            if let Some(out) = ollama(&system, &user).await {
                return out;
            }
            if let Some(out) = claude_cli(&system, &user).await {
                return out;
            }
            Err("ERR_MEMORY_NO_MODEL: no AI model is set up for memory (Settings → AI, a local Ollama model, or the Claude Code CLI)".into())
        })
    })
}

async fn ollama(system: &str, user: &str) -> Option<Result<String, String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let tags: Value = client
        .get("http://127.0.0.1:11434/api/tags")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let model = tags["models"]
        .as_array()?
        .iter()
        .filter_map(|m| m["name"].as_str())
        .find(|n| !n.contains("embed"))?
        .to_string();
    let body = serde_json::json!({
        "model": model,
        "stream": false,
        "format": "json",
        "options": {"temperature": 0},
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    let long = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .ok()?;
    let out = async {
        let v: Value = long
            .post("http://127.0.0.1:11434/api/chat")
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        v["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "ollama: empty reply".to_string())
    }
    .await;
    Some(out)
}

fn find_claude() -> Option<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    let name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

pub(crate) async fn claude_cli(system: &str, user: &str) -> Option<Result<String, String>> {
    use tokio::io::AsyncWriteExt;
    let bin = find_claude()?;
    let cwd = std::env::temp_dir();
    let mut child = tokio::process::Command::new(bin)
        .args([
            "-p",
            "--model",
            "haiku",
            "--tools",
            "",
            "--no-session-persistence",
            "--setting-sources",
            "project",
            "--output-format",
            "text",
            "--system-prompt",
            system,
        ])
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let out = async {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(user.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
        }
        let done = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| "claude: timed out".to_string())?
        .map_err(|e| e.to_string())?;
        if !done.status.success() {
            let err = String::from_utf8_lossy(&done.stderr);
            return Err(format!(
                "claude: {}",
                err.chars().take(300).collect::<String>()
            ));
        }
        Ok(String::from_utf8_lossy(&done.stdout).trim().to_string())
    }
    .await;
    Some(out)
}

pub const CHUNK_CHARS: usize = 3500;
pub const MAX_CHUNKS: usize = 12;
pub const MAX_FACTS: usize = 30;

pub const SYSTEM: &str = r#"You extract long-term memories from a conversation between a person and their AI assistant. Be SELECTIVE: only facts worth remembering in six months.

LANGUAGE: write every fact in the same language as the input. Never translate. Names, identifiers, code and quotes stay verbatim.

EXTRACT (when in doubt about a concrete event, detail or plan, keep it: a missing fact cannot be recalled, an extra one costs little)
- About the person: names, relationships, roles, background, skills, projects, constraints.
- Preferences, likes, dislikes, habits, and how they want things done.
- Significant events: milestones, decisions, purchases, trips, health, changes of state.
- Plans, goals, deadlines, commitments.
- Details that characterize an experience (how a meal tasted, how a film felt).
- What the assistant itself did or learned that it should remember (about = "assistant").
- Every distinct event gets its own fact with its date (look relative days up in the Calendar): what was done, made, bought, visited, read, watched, and with whom. Sensory and emotional details that characterize an experience or a person ARE worth keeping (what a painting showed, how a trip felt, what a gift reminds them of).

DO NOT EXTRACT
- Greetings, thanks, "ok", process chatter ("let me check"), the assistant's general knowledge, questions without an answer, anything already obvious.
- Content from documents, web pages or tool output unless the person said it matters to them.
- Anything the person asked to keep private or to forget.
- What the assistant only repeated from its saved memory about the person (it is already stored).

FORMAT — one fact = one or two sentences, self-contained (resolve "he", "my roommate" to names when both appear: "Emily (the person's roommate)"). Refer to the person as "The person" / "A pessoa" in the input's language, never "the user". When the material is between named people (a transcript, a group chat, a document), name each one instead.

TIME — the input gives the Event Date. Convert every relative time ("yesterday", "last week", "ontem") to an absolute date in the fact text AND in occurred_start/occurred_end (YYYY-MM-DD). A year only or a month only spans the whole period ("in 2015" → 2015-01-01..2015-12-31). States and preferences have no dates.

KIND
- "declared": the person explicitly stated it about themselves (a preference, a fact, a rule). Put their exact words in "quote".
- "observation": something that happened (an event, an experience, a reaction to one specific item).
- "inference": your guess from indirect evidence; give "confidence" 0..1 and never more than 0.8.
- "temporary": holds only for now ("today I want something light"); give "valid_hours".
- "procedural": how to do things for this person (workflow, format, tool they prefer).
A reaction to one film is an observation about that film, not a preference about a genre.

SUBJECT — a short stable key for what the fact is about when a newer fact on the same key should replace it ("home.city", "diet", "job.employer"); null for events.

ENTITIES — people, places, organizations, works (films, books, songs), projects, pets, products. Each {"name", "kind"} with kind one of person|place|org|work|project|pet|product|concept. Never include the person themselves.

CAUSES — "caused_by": indexes of earlier facts in this list that caused this one (at most 2).

Return ONLY JSON:
{"facts":[{"text":"...","kind":"declared|observation|inference|temporary|procedural","about":"person|assistant|other","subject":null,"quote":null,"confidence":null,"valid_hours":null,"occurred_start":null,"occurred_end":null,"entities":[],"caused_by":[]}]}
Return {"facts":[]} when nothing is worth keeping."#;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Extracted {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub valid_hours: Option<f64>,
    #[serde(default)]
    pub occurred_start: Option<String>,
    #[serde(default)]
    pub occurred_end: Option<String>,
    #[serde(default)]
    pub entities: Vec<Value>,
    #[serde(default)]
    pub caused_by: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RetainReport {
    pub extracted: usize,
    pub created: Vec<String>,
    pub reinforced: Vec<String>,
    pub skipped: usize,
    pub duplicate: bool,
}

#[derive(Debug, Clone)]
pub struct RetainInput {
    pub person: String,
    pub other: String,
    pub event_ms: i64,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub context: Option<String>,
}

const FILLER: &[&str] = &[
    "ok",
    "okay",
    "obrigado",
    "obrigada",
    "valeu",
    "thanks",
    "thank you",
    "thx",
    "sim",
    "nao",
    "yes",
    "no",
    "beleza",
    "blz",
    "legal",
    "certo",
    "entendi",
    "perfeito",
    "show",
    "top",
    "oi",
    "ola",
    "hi",
    "hello",
    "hey",
    "bom dia",
    "boa tarde",
    "boa noite",
    "good morning",
    "continue",
    "continua",
    "segue",
    "go on",
    "next",
    "proximo",
];

pub fn worth_retaining(person: &str) -> bool {
    let t = graph::fold(person.trim());
    let t = t.trim_matches(|c: char| !c.is_alphanumeric());
    if t.is_empty() || t.starts_with('/') {
        return false;
    }
    if FILLER.contains(&t) {
        return false;
    }
    t.split_whitespace().count() >= 3 || t.chars().count() >= 18
}

pub fn setting(db: &AssistDb, key: &str) -> bool {
    let v: Option<String> = db
        .with(|c| {
            use rusqlite::OptionalExtension;
            c.query_row(
                "SELECT value FROM memory_settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()
        })
        .ok()
        .flatten();
    v.map(|v| v != "off").unwrap_or(true)
}

pub fn set_setting(db: &AssistDb, key: &str, on: bool) -> Result<(), String> {
    db.with(|c| {
        c.execute(
            "INSERT OR REPLACE INTO memory_settings(key, value) VALUES (?1, ?2)",
            params![key, if on { "on" } else { "off" }],
        )
    })?;
    Ok(())
}

pub fn json_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

pub fn parse_facts(text: &str) -> Vec<Extracted> {
    let Some(v) = json_object(text) else {
        return Vec::new();
    };
    let list = v.get("facts").cloned().unwrap_or(Value::Array(Vec::new()));
    serde_json::from_value::<Vec<Extracted>>(list)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| !f.text.trim().is_empty())
        .take(MAX_FACTS)
        .collect()
}

fn day_ms(s: &str, end: bool) -> Option<i64> {
    let s = s.trim();
    let day = s
        .get(..10)
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let month = || {
        s.get(..7)
            .and_then(|m| NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d").ok())
    };
    let (date, whole) = match day {
        Some(d) => (d, 1),
        None => (month()?, 2),
    };
    let date = if end {
        match whole {
            1 => date.succ_opt()?,
            _ => {
                let (y, m) = if date.month0() == 11 {
                    (date.year() + 1, 1)
                } else {
                    (date.year(), date.month0() + 2)
                };
                NaiveDate::from_ymd_opt(y, m, 1)?
            }
        }
    } else {
        date
    };
    let ms = date
        .and_hms_opt(0, 0, 0)
        .and_then(|n| Local.from_local_datetime(&n).earliest())
        .map(|d| d.timestamp_millis())?;
    Some(if end { ms - 1 } else { ms })
}

use chrono::Datelike;

fn year_only(s: &str) -> Option<(i64, i64)> {
    let y: i32 = s.trim().get(..4)?.parse().ok()?;
    if s.trim().len() != 4 {
        return None;
    }
    let a = NaiveDate::from_ymd_opt(y, 1, 1)?;
    let b = NaiveDate::from_ymd_opt(y + 1, 1, 1)?;
    let f = |d: NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .and_then(|n| Local.from_local_datetime(&n).earliest())
            .map(|d| d.timestamp_millis())
    };
    Some((f(a)?, f(b)? - 1))
}

pub fn occurred(f: &Extracted) -> (Option<i64>, Option<i64>) {
    let s = f.occurred_start.as_deref().filter(|s| !s.trim().is_empty());
    let e = f.occurred_end.as_deref().filter(|s| !s.trim().is_empty());
    if let Some((a, b)) = s.and_then(year_only) {
        let end = e.and_then(year_only).map(|x| x.1).unwrap_or(b);
        return (Some(a), Some(end));
    }
    let start = s.and_then(|s| day_ms(s, false));
    let end = e
        .and_then(|e| day_ms(e, true))
        .or_else(|| s.and_then(|s| day_ms(s, true)));
    (start, if start.is_some() { end } else { None })
}

fn entities_of(f: &Extracted) -> Vec<EntityRef> {
    f.entities
        .iter()
        .filter_map(|e| match e {
            Value::String(s) => Some(EntityRef {
                name: s.clone(),
                kind: "thing".into(),
            }),
            Value::Object(_) => serde_json::from_value::<EntityRef>(e.clone()).ok(),
            _ => None,
        })
        .filter(|e| {
            let n = graph::entity_norm(&e.name);
            !n.is_empty()
                && !matches!(
                    n.as_str(),
                    "user"
                        | "usuario"
                        | "the person"
                        | "person"
                        | "pessoa"
                        | "a pessoa"
                        | "eu"
                        | "me"
                        | "i"
                )
        })
        .collect()
}

fn said_by_person(quote: Option<&str>, person: &str) -> bool {
    let Some(q) = quote
        .map(super::normalize)
        .filter(|q| q.chars().count() >= 4)
    else {
        return false;
    };
    let hay = super::normalize(person);
    if hay.contains(&q) {
        return true;
    }
    let words: Vec<&str> = q
        .split_whitespace()
        .filter(|w| w.chars().count() > 2)
        .collect();
    if words.len() < 3 {
        return false;
    }
    let hits = words.iter().filter(|w| hay.contains(*w)).count();
    hits * 10 >= words.len() * 8
}

fn category_of(f: &Extracted, person: &str) -> Category {
    match Category::parse(&f.kind.to_lowercase()).unwrap_or(Category::Observation) {
        Category::Declared if !said_by_person(f.quote.as_deref(), person) => Category::Inference,
        c => c,
    }
}

fn scope_for(ctx: &AssistCtx, cat: Category, about: Option<&str>) -> Option<Scope> {
    let bot = ctx.bot_id.clone().map(|bot| Scope::Bot { bot });
    let room = ctx
        .writable
        .iter()
        .find(|s| matches!(s, Scope::Room { .. }))
        .cloned();
    let order: Vec<Option<Scope>> = if about == Some("assistant") || cat == Category::Procedural {
        vec![bot, room, Some(Scope::User)]
    } else {
        vec![Some(Scope::User), room, bot]
    };
    order.into_iter().flatten().find(|s| ctx.can_write(s))
}

fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for para in text.split_inclusive('\n') {
        if cur.len() + para.len() > CHUNK_CHARS && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if para.len() > CHUNK_CHARS {
            let chars: Vec<char> = para.chars().collect();
            for piece in chars.chunks(CHUNK_CHARS) {
                out.push(piece.iter().collect());
            }
            continue;
        }
        cur.push_str(para);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.truncate(MAX_CHUNKS);
    out
}

fn calendar(event_ms: i64) -> String {
    let Some(day) = Local
        .timestamp_millis_opt(event_ms)
        .single()
        .map(|d| d.date_naive())
    else {
        return String::new();
    };
    let mut s = String::from("Calendar (for relative dates):");
    for back in (0..14).rev() {
        let d = day - chrono::Duration::days(back);
        let tag = match back {
            0 => " (event date)",
            1 => " (yesterday)",
            _ => "",
        };
        s.push_str(&format!("\n  {}{tag}", d.format("%a %Y-%m-%d")));
    }
    s.push('\n');
    s
}

fn user_message(input: &RetainInput, person: &str, chunk: &str, i: usize, n: usize) -> String {
    let date = Local
        .timestamp_millis_opt(input.event_ms)
        .single()
        .map(|d| d.format("%A, %Y-%m-%d").to_string())
        .unwrap_or_else(|| "Unknown".into());
    let mut s = format!(
        "Extract facts from the following material.\n\nChunk: {}/{}\nEvent Date: {date}\nContext: {}\n{}\n",
        i + 1,
        n,
        input.context.as_deref().unwrap_or("none"),
        calendar(input.event_ms)
    );
    if !person.trim().is_empty() {
        s.push_str("THE PERSON SAID:\n");
        s.push_str(person.trim());
        s.push_str("\n\n");
    }
    if !chunk.trim().is_empty() {
        s.push_str(if input.source_kind == "document" {
            "DOCUMENT:\n"
        } else {
            "THE ASSISTANT REPLIED (context only; not the person's words):\n"
        });
        s.push_str(chunk.trim());
    }
    s
}

pub async fn retain(
    db: Arc<AssistDb>,
    ctx: AssistCtx,
    input: RetainInput,
    llm: Llm,
) -> Result<RetainReport, String> {
    let key = sha(&[
        "retain",
        &ctx.principal,
        ctx.bot_id.as_deref().unwrap_or(""),
        &super::normalize(&input.person),
        &super::normalize(&input.other),
    ]);
    let seen: bool = db.with(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM memory_retain_log WHERE key = ?1",
            [&key],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
    })?;
    if seen {
        return Ok(RetainReport {
            duplicate: true,
            ..Default::default()
        });
    }
    let person_pieces = if input.person.len() > CHUNK_CHARS {
        chunks(&input.person)
    } else {
        vec![input.person.clone()]
    };
    let other_pieces = chunks(&input.other);
    let mut pieces: Vec<(String, String)> = person_pieces
        .into_iter()
        .map(|p| (p, String::new()))
        .collect();
    for (i, o) in other_pieces.into_iter().enumerate() {
        match pieces.get_mut(i) {
            Some(slot) => slot.1 = o,
            None => pieces.push((String::new(), o)),
        }
    }
    pieces.truncate(MAX_CHUNKS);
    let n = pieces.len();
    let mut facts: Vec<Extracted> = Vec::new();
    let mut failure: Option<String> = None;
    for (i, (person, chunk)) in pieces.iter().enumerate() {
        let msg = user_message(&input, person, chunk, i, n);
        let first = llm(SYSTEM.to_string(), msg.clone()).await;
        let reply = match first {
            Ok(t) => Ok(t),
            Err(_) => llm(SYSTEM.to_string(), msg).await,
        };
        match reply {
            Ok(text) => facts.extend(parse_facts(&text)),
            Err(e) => failure = Some(e),
        }
    }
    if facts.is_empty() {
        if let Some(e) = &failure {
            log(&db, &key, &ctx, 0, 0, "failed", Some(e));
            return Err(e.clone());
        }
    }
    let report = store(&db, &ctx, &input, &facts, super::super::now_ms())?;
    log(
        &db,
        &key,
        &ctx,
        report.created.len(),
        report.reinforced.len(),
        "done",
        failure.as_deref(),
    );
    Ok(report)
}

fn log(
    db: &AssistDb,
    key: &str,
    ctx: &AssistCtx,
    facts: usize,
    reinforced: usize,
    status: &str,
    detail: Option<&str>,
) {
    let _ = db.with(|c| {
        c.execute(
            "INSERT OR REPLACE INTO memory_retain_log(key, conversation, bot, at_ms, facts, reinforced, status, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                key,
                ctx.conversation_id,
                ctx.bot_id,
                super::super::now_ms(),
                facts as i64,
                reinforced as i64,
                status,
                detail.map(|d| d.chars().take(300).collect::<String>())
            ],
        )
    });
}

pub fn store(
    db: &AssistDb,
    ctx: &AssistCtx,
    input: &RetainInput,
    facts: &[Extracted],
    now: i64,
) -> Result<RetainReport, String> {
    let mut report = RetainReport {
        extracted: facts.len(),
        ..Default::default()
    };
    let texts: Vec<&str> = facts.iter().map(|f| f.text.trim()).collect();
    let (source, vectors): (&str, Vec<Option<Vec<f32>>>) = match embed_passages(&texts) {
        Some((src, v)) if v.len() == texts.len() => (src, v.into_iter().map(Some).collect()),
        _ => ("", vec![None; texts.len()]),
    };
    let mut ids: Vec<Option<String>> = vec![None; facts.len()];
    for (i, f) in facts.iter().enumerate() {
        let cat = category_of(f, &input.person);
        let Some(scope) = scope_for(ctx, cat, f.about.as_deref()) else {
            report.skipped += 1;
            continue;
        };
        let vec = vectors.get(i).cloned().flatten();
        if let Some(v) = &vec {
            if let Some(twin) = twin(db, ctx, &scope, cat, source, v)? {
                let from_person = input.person.trim().is_empty()
                    || said_by_person(f.quote.as_deref(), &input.person)
                    || said_by_person(Some(&f.text), &input.person);
                if !from_person {
                    ids[i] = Some(twin);
                    continue;
                }
                db.with(|c| graph::reinforce(c, &twin))?;
                if cat == Category::Inference {
                    let _ = db.with(|c| {
                        c.execute(
                            "UPDATE memory_records SET confidence = MIN(0.9, confidence + (1.0 - confidence) * 0.25) WHERE id = ?1 AND category = 'inference'",
                            [&twin],
                        )
                    });
                }
                report.reinforced.push(twin.clone());
                ids[i] = Some(twin);
                continue;
            }
        }
        let mut new = NewMemory::new(
            scope.clone(),
            cat,
            f.text.trim(),
            Source {
                kind: input.source_kind.clone(),
                id: input.source_id.clone(),
                conversation: ctx.conversation_id.clone(),
                author: format!("bot:{}", ctx.bot_id.as_deref().unwrap_or("memory")),
                at_ms: input.event_ms,
            },
        );
        new.subject = f.subject.clone().filter(|s| !s.trim().is_empty());
        match cat {
            Category::Inference => {
                new.confidence = Some(f.confidence.unwrap_or(0.6).clamp(0.1, 0.8));
                new.evidence = Some(
                    f.quote
                        .clone()
                        .filter(|q| !q.trim().is_empty())
                        .unwrap_or_else(|| "inferred from the conversation".into()),
                );
            }
            Category::Temporary => {
                let h = f.valid_hours.filter(|h| h.is_finite() && *h > 0.0);
                if let Some(h) = h {
                    new.valid_until = Some(now + (h.min(24.0 * 30.0) * 3_600_000.0) as i64);
                }
            }
            _ => {}
        }
        let written = match remember(db, ctx, new, now) {
            Ok(w) => w,
            Err(_) => {
                report.skipped += 1;
                continue;
            }
        };
        let id = written.record.id.clone();
        ids[i] = Some(id.clone());
        if !written.created {
            continue;
        }
        let scope_key = scope.key();
        let entities = entities_of(f);
        let when = occurred(f);
        db.tx(|tx| {
            let e = |x: rusqlite::Error| format!("{}: {x}", super::super::db::ERR_ASSIST_DB);
            graph::set_meta(tx, &id, when).map_err(e)?;
            graph::attach_entities(tx, &ctx.principal, &id, &scope_key, &entities, now)
                .map_err(e)?;
            if let Some(v) = &vec {
                graph::store_vector(tx, &id, source, v).map_err(e)?;
                graph::link_semantic(tx, &ctx.principal, &id, source, v).map_err(e)?;
            }
            Ok(())
        })?;
        report.created.push(id);
    }
    db.with(|c| {
        for (i, f) in facts.iter().enumerate() {
            let Some(src) = &ids[i] else { continue };
            for cause in f.caused_by.iter().filter_map(Value::as_u64).take(2) {
                let j = cause as usize;
                if j < i {
                    if let Some(dst) = &ids[j] {
                        graph::add_link(c, src, dst, "causal", 1.0)?;
                        graph::add_link(c, dst, src, "causal", 0.8)?;
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(report)
}

fn twin(
    db: &AssistDb,
    ctx: &AssistCtx,
    scope: &Scope,
    cat: Category,
    source: &str,
    v: &[f32],
) -> Result<Option<String>, String> {
    let key = scope.key();
    let all = db.with(|c| graph::active_vectors(c, &ctx.principal, source))?;
    let mut best: Option<(f32, String)> = None;
    for (id, s, w) in all {
        if s != key {
            continue;
        }
        let sim = graph::dot(v, &w);
        if sim >= super::retrieve::cutoffs(source).same_fact
            && best.as_ref().map(|b| sim > b.0).unwrap_or(true)
        {
            best = Some((sim, id));
        }
    }
    let Some((_, id)) = best else { return Ok(None) };
    let their: Option<String> = db.with(|c| {
        use rusqlite::OptionalExtension;
        c.query_row(
            "SELECT category FROM memory_records WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .optional()
    })?;
    let compatible = match (their.as_deref().and_then(Category::parse), cat) {
        (Some(a), b) if a == b => true,
        (Some(Category::Declared), Category::Inference) => true,
        _ => false,
    };
    Ok(compatible.then_some(id))
}

pub fn backfill_vectors(db: &AssistDb, max: usize) -> Result<usize, String> {
    let Some(src) = vector_source() else {
        return Ok(0);
    };
    let rows: Vec<(String, String, String)> = db.with(|c| {
        let mut s = c.prepare(
            "SELECT r.id, r.principal, r.content FROM memory_records r
             LEFT JOIN memory_vectors v ON v.record_id = r.id
             WHERE r.status = 'active' AND (v.record_id IS NULL OR v.source != ?1) LIMIT ?2",
        )?;
        let rows = s.query_map(rusqlite::params![src, max as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })?;
    let mut done = 0;
    for batch in rows.chunks(32) {
        let texts: Vec<&str> = batch.iter().map(|r| r.2.as_str()).collect();
        let Some((src, vecs)) = embed_passages(&texts) else {
            break;
        };
        db.tx(|tx| {
            let e = |x: rusqlite::Error| format!("{}: {x}", super::super::db::ERR_ASSIST_DB);
            for ((id, principal, _), v) in batch.iter().zip(vecs.iter()) {
                graph::store_vector(tx, id, src, v).map_err(e)?;
                graph::link_semantic(tx, principal, id, src, v).map_err(e)?;
            }
            Ok(())
        })?;
        done += batch.len();
    }
    Ok(done)
}
