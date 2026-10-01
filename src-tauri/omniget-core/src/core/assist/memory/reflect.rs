use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::ctx::{AssistCtx, Scope};
use super::super::db::AssistDb;
use super::retain::{json_object, Llm};
use super::retrieve::{embed_query, hybrid, notes_for};
use super::{remember, Category, NewMemory, Source};

pub const REFLECT_FACTS: usize = 30;

pub const SYSTEM: &str = r#"You answer a question using ONLY the memories given below: saved facts about a person and short notes that summarize them. NEVER make up names, people, events or details.

How to reason
- Facts are ground truth; notes are summaries and may lag behind newer facts. When they disagree, the newer fact wins.
- Temporal reasoning: each fact has a date. Answer "when" questions with dates; for "how long ago" compute from Today. For "current" state use the most recent fact on that facet.
- Unconfirmed guesses are not facts: say "possibly" if you rely on one.
- If the memories do not contain the answer, say so plainly. A partial answer is fine if you say what is missing.
- Write in the language of the question.

Also note any NEW insight the facts jointly support but no single fact states (a pattern, a preference shown by several events). Only strong ones; none is fine.

Return ONLY JSON:
{"answer":"...","used":["<fact id>", ...],"confidence":0.0,"insights":[{"text":"...","evidence":["<fact id>", ...],"confidence":0.0}]}"#;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Reflection {
    pub answer: String,
    pub used: Vec<String>,
    pub confidence: f64,
    pub saved_insights: Vec<String>,
}

fn date(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

pub async fn reflect(
    db: Arc<AssistDb>,
    ctx: AssistCtx,
    question: String,
    keep_insights: bool,
    llm: Llm,
) -> Result<Reflection, String> {
    let now = super::super::now_ms();
    let q = question.clone();
    let qvec = tokio::task::spawn_blocking(move || embed_query(&q))
        .await
        .ok()
        .flatten();
    let probe = hybrid(&db, &ctx, &question, qvec.as_deref(), REFLECT_FACTS, now)?;
    let notes = notes_for(&db, &ctx, &question)?;
    if probe.hits.is_empty() && notes.is_empty() {
        return Ok(Reflection {
            answer: "Nothing in memory is about this.".into(),
            ..Default::default()
        });
    }
    let mut user = format!("Today: {}\n\nQUESTION: {question}\n\nFACTS:\n", date(now));
    for h in &probe.hits {
        let when = h.why.occurred_start.unwrap_or(h.record.source.at_ms);
        let guess = if h.record.category == Category::Inference {
            format!(
                " (unconfirmed guess, confidence {:.1})",
                h.record.confidence
            )
        } else {
            String::new()
        };
        user.push_str(&format!(
            "[{}] {} ({}){guess}\n",
            h.record.id,
            h.record.content.replace('\n', " "),
            date(when)
        ));
    }
    if !notes.is_empty() {
        user.push_str("\nNOTES:\n");
        for (name, text) in &notes {
            user.push_str(&format!("{name}:\n{text}\n"));
        }
    }
    let reply = llm(SYSTEM.to_string(), user).await?;
    let v = json_object(&reply).unwrap_or(Value::Null);
    let known: std::collections::HashSet<&str> =
        probe.hits.iter().map(|h| h.record.id.as_str()).collect();
    let ids = |key: &str, v: &Value| -> Vec<String> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|id| known.contains(id))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let answer = v
        .get("answer")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| reply.trim().to_string());
    let mut out = Reflection {
        answer,
        used: ids("used", &v),
        confidence: v.get("confidence").and_then(Value::as_f64).unwrap_or(0.5),
        saved_insights: Vec::new(),
    };
    let bot_scope = ctx.bot_id.clone().map(|bot| Scope::Bot { bot });
    if keep_insights {
        if let (Some(scope), Some(list)) = (
            bot_scope.filter(|s| ctx.can_write(s)),
            v.get("insights").and_then(Value::as_array),
        ) {
            for ins in list.iter().take(3) {
                let text = ins.get("text").and_then(Value::as_str).unwrap_or("").trim();
                let evidence = ids("evidence", ins);
                if text.is_empty() || evidence.len() < 2 {
                    continue;
                }
                let conf = ins
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5)
                    .clamp(0.1, 0.8);
                let mut new = NewMemory::new(
                    scope.clone(),
                    Category::Inference,
                    text,
                    Source {
                        kind: "reflection".into(),
                        id: None,
                        conversation: ctx.conversation_id.clone(),
                        author: format!("bot:{}", ctx.bot_id.as_deref().unwrap_or("memory")),
                        at_ms: now,
                    },
                );
                new.confidence = Some(conf);
                new.evidence = Some(format!("from facts {}", evidence.join(", ")));
                if let Ok(w) = remember(&db, &ctx, new, now) {
                    if w.created {
                        let id = w.record.id.clone();
                        let _ = db.with(|c| {
                            for e in &evidence {
                                super::graph::add_link(c, &id, e, "causal", 0.9)?;
                            }
                            Ok(())
                        });
                        out.saved_insights.push(id);
                    }
                }
            }
        }
    }
    Ok(out)
}
