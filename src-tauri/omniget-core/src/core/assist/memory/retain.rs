//! Retain: after a turn (or when the model hands over a document), one model
//! call extracts the facts worth keeping, and each becomes a record with its
//! date, entities, vector and links. Adapted from Hindsight's concise
//! extraction prompt (estudos/91-hindsight/prompts.md §1).
//!
//! What this does that Hindsight does not:
//! - a record is `declared` only when its quote is really in the person's own
//!   words; anything else the model claims the person said becomes an
//!   unconfirmed guess (the memory module's rule, now enforced on extraction);
//! - a fact that arrives again reinforces the record it matches (one more
//!   proof, a guess gains confidence) instead of becoming a sibling;
//! - a turn that says nothing personal never reaches a model (cheap filter),
//!   and the same turn is never extracted twice (log keyed by content);
//! - forgetting wins: a fact the person asked to forget is refused by the
//!   tombstones on the way in, whatever the extractor says.

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

/// A model call: `(system, user) -> text`.
pub type Llm =
    Arc<dyn Fn(String, String) -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

/// The model memory talks to, first one available:
/// 1. the app's configured model (`ai::chat`);
/// 2. a local Ollama with a model pulled (nothing leaves the machine);
/// 3. the Claude Code CLI on the person's subscription (`claude -p`, Haiku,
///    no tools, no session saved).
///
/// One call at a time: retains after several turns queue instead of opening
/// several model processes at once.
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
    // Chat models only; embedding models cannot answer.
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

/// Characters of one extraction chunk (Hindsight uses 3000; a smaller chunk
/// gets more of its details extracted).
pub const CHUNK_CHARS: usize = 3500;
/// Chunks one retain call may send.
pub const MAX_CHUNKS: usize = 12;
/// Facts kept from one chunk.
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

/// What one retain call did.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RetainReport {
    pub extracted: usize,
    pub created: Vec<String>,
    pub reinforced: Vec<String>,
    /// Refused by a tombstone, a scope, or invalid.
    pub skipped: usize,
    /// True when the log already had this input.
    pub duplicate: bool,
}

/// The material to retain.
#[derive(Debug, Clone)]
pub struct RetainInput {
    /// What the person said (the only text that can make a fact `declared`).
    pub person: String,
    /// Everything else: the assistant's reply, or a document.
    pub other: String,
    /// When it was said.
    pub event_ms: i64,
    /// `conversation` | `document` | `message`.
    pub source_kind: String,
    pub source_id: Option<String>,
    /// Free context for the extractor ("chat with the reading bot").
    pub context: Option<String>,
}

// ── Cheap gate ───────────────────────────────────────────────────────────

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

/// Whether the person's message could hold something worth remembering.
/// Short acknowledgements and slash commands never reach the extractor.
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

// ── Settings ─────────────────────────────────────────────────────────────

/// `auto_retain` (on/off, default on), `consolidate` (on/off, default on).
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

// ── Parsing ──────────────────────────────────────────────────────────────

/// The first JSON object in a model reply (fences and prose around it are
/// tolerated).
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

/// `(start, end)` of an extracted fact, in ms.
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

/// True when `quote` (or, failing that, the fact) is in the person's words.
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
    // Tolerate small edits: most of the quote's words, in the person's text.
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

// ── The pipeline ─────────────────────────────────────────────────────────

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

/// The two weeks up to the event date with weekday names, so "last
/// Friday" or "the weekend before" is a lookup instead of arithmetic the
/// model gets wrong (Hindsight only gives the day name of the event date).
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

/// Extracts and stores. Runs off the hot path; every write goes through
/// [`remember`], so scopes, idempotency and tombstones hold.
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
    // A long message from the person (a pasted diary, a transcript) is cut
    // like any document; a short one rides with the first chunk of the rest.
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
        // One retry: a slow model call should not cost the chunk its facts.
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

/// Writes extracted facts (split out so tests can skip the model).
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
    // Index in `facts` → record id, for causal links.
    let mut ids: Vec<Option<String>> = vec![None; facts.len()];
    for (i, f) in facts.iter().enumerate() {
        let cat = category_of(f, &input.person);
        let Some(scope) = scope_for(ctx, cat, f.about.as_deref()) else {
            report.skipped += 1;
            continue;
        };
        let vec = vectors.get(i).cloned().flatten();
        // Same fact again → one more proof, not a sibling.
        if let Some(v) = &vec {
            if let Some(twin) = twin(db, ctx, &scope, cat, source, v)? {
                // The assistant repeating a recalled memory is not a new
                // proof: only the person saying it again (or a document)
                // counts. Otherwise memory would confirm itself (Hindsight
                // strips its injected block for the same reason).
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
    // Causal links, now that every fact has an id.
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

/// An active record in `scope` saying the same thing as `v`, of a compatible
/// kind.
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
        // A stated fact covers a later guess of the same thing.
        (Some(Category::Declared), Category::Inference) => true,
        _ => false,
    };
    Ok(compatible.then_some(id))
}

/// Embeds active records that have no vector yet (written while the local
/// model was missing, or before this version). Returns how many.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_and_parse() {
        assert!(!worth_retaining("ok"));
        assert!(!worth_retaining("valeu!"));
        assert!(!worth_retaining("/clear"));
        assert!(worth_retaining("minha filha Ana começou a faculdade"));
        let f = parse_facts("```json\n{\"facts\":[{\"text\":\"A pessoa mora em Recife\",\"kind\":\"declared\",\"quote\":\"moro em Recife\",\"entities\":[\"Recife\"]}]}\n```");
        assert_eq!(f.len(), 1);
        assert_eq!(
            category_of(&f[0], "eu moro em Recife faz 3 anos"),
            Category::Declared
        );
        assert_eq!(category_of(&f[0], "gosto de praia"), Category::Inference);
        assert_eq!(entities_of(&f[0])[0].name, "Recife");
    }

    #[test]
    fn dates() {
        let f = Extracted {
            occurred_start: Some("2025-03".into()),
            ..Default::default()
        };
        let (s, e) = occurred(&f);
        let s = s.unwrap();
        let e = e.unwrap();
        assert!(e - s > 27 * 86_400_000 && e - s < 32 * 86_400_000);
        let y = Extracted {
            occurred_start: Some("2015".into()),
            ..Default::default()
        };
        let (s, e) = occurred(&y);
        assert!(e.unwrap() - s.unwrap() > 364 * 86_400_000);
    }
}
