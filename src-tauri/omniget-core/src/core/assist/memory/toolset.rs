//! The model's door to memory (tools) and the per-turn hook.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::ctx::{AssistCtx, Scope};
use super::super::db::{self, AssistDb};
use super::super::tools::{
    need_str, opt_str, spec, AssistToolset, ERR_ASSIST_SCOPE, ERR_ASSIST_TOOL,
};
use super::{
    correct, forget, get, profile, recall, remember, retract, search, source_line, Category,
    Correction, MemoryRecord, NewMemory, Source, ERR_MEMORY_INVALID,
};
use crate::core::llm::agent::{AgentDef, GrantMode, ToolSource};
use crate::core::llm::types::ToolSpec;

struct MemoryTools;

pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(MemoryTools)
}

fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "memory_recall",
            "Search what you remember about this person (only the scopes this conversation may read). Returns notes with their source.",
            json!({"type":"object","properties":{
                "query":{"type":"string","description":"Words to look for"},
                "limit":{"type":"integer","minimum":1,"maximum":20}
            },"required":["query"]}),
        ),
        spec(
            "memory_remember",
            "Save one note. kind: observation (something that happened, e.g. a movie watched and the reaction to it), inference (your guess; stays an unconfirmed candidate), temporary (holds only for now, e.g. 'today I want something light'), procedural (how to do things for this person), declared (ONLY when the person explicitly stated it; set user_said_explicitly). Record a reaction to one item as an observation about that item, not as a preference about a whole genre.",
            json!({"type":"object","properties":{
                "content":{"type":"string"},
                "kind":{"type":"string","enum":["declared","observation","inference","temporary","procedural"]},
                "subject":{"type":"string","description":"Short stable key for what this is about (e.g. 'pace', 'book.progress'); a newer note with the same subject and kind replaces the older one"},
                "scope":{"type":"string","enum":["user","bot","room"],"description":"user: the person's profile; bot: your private notes; room: this group's shared memory"},
                "user_said_explicitly":{"type":"boolean"},
                "evidence":{"type":"string","description":"For inferences: what the person said or did"},
                "confidence":{"type":"number","minimum":0,"maximum":1},
                "valid_hours":{"type":"number","description":"For temporary context; default: until the end of today"},
                "source_message_id":{"type":"string"},
                "data":{"type":"object","description":"Optional structured fields (e.g. {\"movie\":\"…\",\"liked\":[\"humor\"]})"}
            },"required":["content","kind"]}),
        ),
        spec(
            "memory_correct",
            "Replace or withdraw a note the person corrected ('actually…'). With content: a new version replaces it. With retract=true: it is withdrawn with no replacement (e.g. a wrong guess).",
            json!({"type":"object","properties":{
                "id":{"type":"string"},
                "content":{"type":"string"},
                "retract":{"type":"boolean"},
                "kind":{"type":"string","enum":["declared","observation","inference","temporary","procedural"]},
                "user_said_explicitly":{"type":"boolean"},
                "source_message_id":{"type":"string"}
            },"required":["id"]}),
        ),
        spec(
            "memory_forget",
            "Forget a note completely when the person asks: every version, the search index and the summary. It cannot come back unless the person adds it again.",
            json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
        ),
        spec(
            "memory_profile",
            "The compact profile of this person for this conversation, with sources.",
            json!({"type":"object","properties":{}}),
        ),
    ]
}

#[async_trait]
impl AssistToolset for MemoryTools {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn specs(&self) -> Vec<ToolSpec> {
        specs()
    }

    async fn call(&self, ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String> {
        let db = db::global()?;
        call_tool(&db, ctx, tool, input, super::super::now_ms())
    }
}

fn view(rec: &MemoryRecord) -> Value {
    json!({
        "id": rec.id,
        "kind": rec.category,
        "content": rec.content,
        "subject": rec.subject,
        "scope": match &rec.scope { Scope::User => "user", Scope::Bot { .. } => "bot", Scope::Room { .. } => "room" },
        "status": rec.status,
        "confidence": rec.confidence,
        "evidence": rec.evidence,
        "data": rec.data,
        "valid_until_ms": rec.valid_until,
        "source": source_line(rec),
    })
}

fn tool_source(ctx: &AssistCtx, input: &Value, now: i64) -> Source {
    let msg = opt_str(input, "source_message_id").map(str::to_string);
    Source {
        kind: if msg.is_some() {
            "message"
        } else {
            "conversation"
        }
        .into(),
        id: msg.or_else(|| ctx.run_id.clone()),
        conversation: ctx.conversation_id.clone(),
        author: format!("bot:{}", ctx.bot_id.as_deref().unwrap_or("unknown")),
        at_ms: now,
    }
}

fn pick_scope(ctx: &AssistCtx, requested: Option<&str>, cat: Category) -> Result<Scope, String> {
    let of = |kind: &str| -> Option<Scope> {
        match kind {
            "user" => Some(Scope::User),
            "bot" => ctx.bot_id.clone().map(|bot| Scope::Bot { bot }),
            "room" => ctx
                .writable
                .iter()
                .find(|s| matches!(s, Scope::Room { .. }))
                .cloned(),
            _ => None,
        }
    };
    if let Some(r) = requested {
        return of(r)
            .filter(|s| ctx.can_write(s))
            .ok_or_else(|| format!("{ERR_ASSIST_SCOPE}: this conversation cannot save to `{r}`"));
    }
    let order: &[&str] = match cat {
        Category::Declared | Category::Inference | Category::Temporary => &["user", "room", "bot"],
        Category::Observation | Category::Procedural => &["bot", "room", "user"],
    };
    order
        .iter()
        .filter_map(|k| of(k))
        .find(|s| ctx.can_write(s))
        .ok_or_else(|| format!("{ERR_ASSIST_SCOPE}: this conversation cannot save memories"))
}

fn parse_kind(input: &Value) -> Result<Option<Category>, String> {
    match opt_str(input, "kind") {
        None => Ok(None),
        Some(k) => Category::parse(k)
            .map(Some)
            .ok_or_else(|| format!("{ERR_MEMORY_INVALID}: unknown kind `{k}`")),
    }
}

/// Runs one memory tool against `db` (the trait impl passes the app's).
pub fn call_tool(
    db: &AssistDb,
    ctx: &AssistCtx,
    tool: &str,
    input: Value,
    now: i64,
) -> Result<Value, String> {
    let explicit = input
        .get("user_said_explicitly")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match tool {
        "memory_recall" => {
            let q = need_str(&input, "query")?;
            let limit = input
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(8)
                .min(20) as usize;
            let found = recall(db, ctx, q, limit, now)?;
            Ok(json!({ "results": found.iter().map(view).collect::<Vec<_>>() }))
        }
        "memory_remember" => {
            let content = need_str(&input, "content")?;
            let mut cat = parse_kind(&input)?
                .ok_or_else(|| format!("{ERR_ASSIST_TOOL}: `kind` is required"))?;
            let mut note = None;
            if cat == Category::Declared && !explicit {
                cat = Category::Inference;
                note = Some("downgraded_to_candidate");
            }
            let scope = pick_scope(ctx, opt_str(&input, "scope"), cat)?;
            let mut new = NewMemory::new(scope, cat, content, tool_source(ctx, &input, now));
            new.subject = opt_str(&input, "subject").map(str::to_string);
            new.evidence = opt_str(&input, "evidence").map(str::to_string);
            new.confidence = input.get("confidence").and_then(Value::as_f64).map(|c| {
                if cat == Category::Inference {
                    c.min(0.9)
                } else {
                    c
                }
            });
            new.data = input.get("data").filter(|d| d.is_object()).cloned();
            if let Some(h) = input.get("valid_hours").and_then(Value::as_f64) {
                if h > 0.0 && h.is_finite() {
                    new.valid_until = Some(now + (h.min(24.0 * 30.0) * 3_600_000.0) as i64);
                }
            }
            if cat == Category::Inference && new.evidence.is_none() {
                new.evidence = Some("inferred in conversation".into());
            }
            let w = remember(db, ctx, new, now)?;
            let mut out = view(&w.record);
            out["created"] = json!(w.created);
            out["note"] = json!(w.note.or(note.map(str::to_string)));
            Ok(out)
        }
        "memory_correct" => {
            let id = need_str(&input, "id")?;
            if input
                .get("retract")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let rec = retract(db, ctx, id, now)?;
                return Ok(view(&rec));
            }
            let content = need_str(&input, "content")?;
            let old = get(db, ctx, id)?;
            let mut cat = parse_kind(&input)?.unwrap_or(old.category);
            if cat == Category::Declared && !explicit {
                if old.category == Category::Declared {
                    // Rewording a stated preference without the person
                    // saying so turns it into a guess.
                    cat = Category::Inference;
                } else {
                    return Err(format!(
                        "{ERR_MEMORY_INVALID}: only the person can make this a stated preference"
                    ));
                }
            }
            let rec = correct(
                db,
                ctx,
                id,
                Correction {
                    content: content.to_string(),
                    category: Some(cat),
                    ..Default::default()
                },
                tool_source(ctx, &input, now),
                now,
            )?;
            Ok(view(&rec))
        }
        "memory_forget" => {
            let id = need_str(&input, "id")?;
            let n = forget(db, ctx, id, now)?;
            Ok(json!({ "forgotten": n > 0 }))
        }
        "memory_profile" => {
            let p = profile(db, ctx, now)?;
            Ok(json!({ "profile": p.text }))
        }
        other => Err(format!("{ERR_ASSIST_TOOL}: unknown tool `{other}`")),
    }
}

/// True when the agent (after the bots hook) may call `memory_recall`.
pub fn has_recall_grant(agent: &AgentDef) -> bool {
    agent.tools.iter().any(|g| {
        g.mode != GrantMode::Deny
            && matches!(&g.source, ToolSource::Internal { name } if name == "memory_recall")
    })
}

struct MemoryAugment;

impl crate::core::llm::coordinator::TurnAugment for MemoryAugment {
    fn augment(
        &self,
        agent: &mut AgentDef,
        conversation_id: &str,
        user_input: &str,
    ) -> Option<String> {
        if !has_recall_grant(agent) {
            return None;
        }
        let db = db::global().ok()?;
        let ctx = super::super::ctx::resolve(&agent.id, Some(conversation_id));
        search::augment_text(&db, &ctx, user_input, super::super::now_ms())
    }
}

/// Per-turn hook: profile + relevant recollections, only for agents that
/// hold the `memory_recall` grant.
pub fn augment() -> Arc<dyn crate::core::llm::coordinator::TurnAugment> {
    Arc::new(MemoryAugment)
}
