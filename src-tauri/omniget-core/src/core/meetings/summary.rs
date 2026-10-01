//! Summary, decisions and action items from a transcript, through the AI
//! provider the person configured (Ollama by default for local setups).
//!
//! Small local models have small context windows, so a long meeting goes
//! map-reduce: each chunk of the transcript becomes dense notes, and the
//! notes become the final JSON. A short meeting goes straight to JSON.

use anyhow::anyhow;
use serde_json::Value;

use super::transcript::{to_plain, Segment};
use super::{ActionItem, Meeting, Summary};

/// ~3-4k tokens: fits the default context of the usual Ollama models.
const CHUNK_CHARS: usize = 12_000;

const JSON_SHAPE: &str = r#"{
  "title": "short descriptive title of the meeting (max 8 words)",
  "overview": "2-4 sentences: what the meeting was about and its outcome",
  "key_points": ["main topics and facts discussed"],
  "decisions": ["decisions that were actually made"],
  "action_items": [{"task": "concrete next step", "owner": "person or null", "due": "deadline as said, or null"}],
  "open_questions": ["unresolved questions or risks"]
}"#;

fn language_rule(lang: Option<&str>) -> String {
    match lang
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "auto")
    {
        Some(l) => format!("Write every string in the language with ISO code \"{l}\"."),
        None => "Write every string in the same language the meeting was held in.".to_string(),
    }
}

fn final_system(lang: Option<&str>) -> String {
    format!(
        "You turn meeting transcripts into structured minutes. Reply with ONLY a JSON object, no prose, no code fences, in exactly this shape:\n{JSON_SHAPE}\n\
Rules: only include what the transcript supports; never invent owners, dates or decisions; use the speaker names as they appear; an empty list is fine. {}",
        language_rule(lang)
    )
}

fn notes_system(lang: Option<&str>) -> String {
    format!(
        "You take dense notes of one part of a longer meeting transcript. Output plain bullet points covering: topics, facts and numbers, decisions made, action items with owner and deadline when stated, and open questions. Keep speaker names. No preamble. {}",
        language_rule(lang)
    )
}

/// Splits the plain transcript on line boundaries into chunks of at most
/// `max` characters (a single longer line is kept whole).
pub fn chunk(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if !cur.is_empty() && cur.len() + line.len() + 1 > max {
            out.push(std::mem::take(&mut cur));
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn user_context(m: &Meeting) -> String {
    let mut ctx = format!("Meeting: {}\n", m.title);
    if !m.speakers.is_empty() {
        ctx.push_str(&format!("Speakers: {}\n", m.speakers.join(", ")));
    }
    if !m.notes.trim().is_empty() {
        ctx.push_str(
            "\nNotes the attendee wrote during the meeting (treat them as important hints):\n",
        );
        ctx.push_str(m.notes.trim());
        ctx.push('\n');
    }
    ctx
}

/// Runs the summary for `m` over `segments`. `lang` forces the output
/// language; `None` follows the meeting.
pub async fn summarize(
    m: &Meeting,
    segments: &[Segment],
    lang: Option<&str>,
) -> anyhow::Result<Summary> {
    let cfg = crate::core::ai::get();
    if !cfg.is_configured() {
        return Err(anyhow!(
            "No AI provider configured: set one (Ollama works offline) in Settings > AI to summarize meetings"
        ));
    }
    let lang = lang.or(m.language.as_deref());
    let plain = to_plain(segments);
    if plain.trim().is_empty() {
        return Err(anyhow!("The transcript is empty: nothing to summarize"));
    }
    let chunks = chunk(&plain, CHUNK_CHARS);
    let ctx = user_context(m);
    let body = if chunks.len() == 1 {
        format!("{ctx}\nTranscript:\n{}", chunks[0])
    } else {
        let mut notes = String::new();
        for (i, c) in chunks.iter().enumerate() {
            let part = crate::core::ai::chat(
                &notes_system(lang),
                &format!("{ctx}\nPart {} of {}:\n{c}", i + 1, chunks.len()),
            )
            .await
            .map_err(|e| anyhow!(e))?;
            notes.push_str(&format!("## Part {}\n{}\n\n", i + 1, part.trim()));
        }
        format!("{ctx}\nNotes taken over the whole meeting, in order:\n{notes}")
    };
    let raw = crate::core::ai::chat(&final_system(lang), &body)
        .await
        .map_err(|e| anyhow!(e))?;
    let mut summary = parse_summary(&raw).ok_or_else(|| {
        anyhow!(
            "The model did not answer with the expected JSON: {}",
            raw.chars().take(200).collect::<String>()
        )
    })?;
    summary.model = format!("{}:{}", cfg.provider_id(), cfg.model);
    summary.created_at_ms = super::now_ms();
    Ok(summary)
}

/// Lenient parse: the first `{` to the last `}`, code fences ignored, list
/// items accepted as strings or objects, action items as strings too.
pub fn parse_summary(raw: &str) -> Option<Summary> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    let v: Value = serde_json::from_str(&raw[start..=end]).ok()?;
    if !v.is_object() {
        return None;
    }
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let list = |k: &str| -> Vec<String> {
        v[k].as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| match x {
                        Value::String(s) => Some(s.trim().to_string()),
                        Value::Object(o) => o
                            .values()
                            .find_map(|v| v.as_str())
                            .map(|s| s.trim().to_string()),
                        _ => None,
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let opt = |x: &Value| {
        x.as_str()
            .map(str::trim)
            .filter(|s| {
                !s.is_empty() && !s.eq_ignore_ascii_case("null") && !s.eq_ignore_ascii_case("none")
            })
            .map(str::to_string)
    };
    let action_items = v["action_items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| match x {
                    Value::String(t) => Some(ActionItem {
                        task: t.trim().to_string(),
                        ..Default::default()
                    }),
                    Value::Object(_) => {
                        let task = opt(&x["task"]).or_else(|| opt(&x["description"]))?;
                        Some(ActionItem {
                            task,
                            owner: opt(&x["owner"]).or_else(|| opt(&x["assignee"])),
                            due: opt(&x["due"]).or_else(|| opt(&x["deadline"])),
                            done: false,
                        })
                    }
                    _ => None,
                })
                .filter(|a| !a.task.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let summary = Summary {
        title: s("title"),
        overview: s("overview"),
        key_points: list("key_points"),
        decisions: list("decisions"),
        action_items,
        open_questions: list("open_questions"),
        model: String::new(),
        created_at_ms: 0,
    };
    if summary.overview.is_empty()
        && summary.key_points.is_empty()
        && summary.action_items.is_empty()
    {
        return None;
    }
    Some(summary)
}
