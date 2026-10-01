use std::sync::Arc;

use rusqlite::params;

use super::super::db::AssistDb;
use super::retain::Llm;

pub const MIN_FACTS: i64 = 2;
pub const MAX_FACTS: usize = 40;
pub const PER_RUN: usize = 8;

pub const SYSTEM: &str = r#"You maintain short memory notes about one entity (a person, place, work, project, pet…) in someone's personal memory. You get the saved facts about it and the current note, if any. Write the new note.

RULES
- Use ONLY the facts given. Never invent names, dates or details.
- Language: write in the language of the facts (majority wins). Never translate names.
- Current state first, then history with dates when known ("Owned a 2019 Civic; sold it on 2025-03-15").
- One line per facet (relationship, role, preferences, events, plans). Merge facts that say the same thing; keep distinct facets distinct.
- When facts disagree, the more recent one wins; keep the older one only as dated history.
- NO arithmetic or deductions: never compute counts, ages or totals the facts do not state.
- Unconfirmed guesses stay marked as guesses ("possibly…").
- At most 6 lines, each starting with "- ". No heading, no preamble.
- If the current note is still right, return it unchanged.

Return ONLY JSON: {"note":"- ...\n- ...","used":["<fact id>", ...]}"#;

struct Fact {
    id: String,
    category: String,
    content: String,
    when: i64,
}

fn date(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

pub async fn run(db: Arc<AssistDb>, llm: Llm) -> Result<usize, String> {
    let dirty: Vec<(String, String)> = db.with(|c| {
        let mut s = c.prepare("SELECT entity_id, scope FROM memory_dirty_entities LIMIT ?1")?;
        let rows = s.query_map([PER_RUN as i64 * 4], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })?;
    let mut written = 0;
    for (entity, scope) in dirty {
        if written >= PER_RUN {
            break;
        }
        let (name, facts) = db.with(|c| {
            use rusqlite::OptionalExtension;
            let name: Option<String> = c
                .query_row(
                    "SELECT name FROM memory_entities WHERE id = ?1",
                    [&entity],
                    |r| r.get(0),
                )
                .optional()?;
            let mut s = c.prepare(
                "SELECT r.id, r.category, r.content, COALESCE(m.occurred_start, r.source_ms)
                 FROM memory_entity_refs x JOIN memory_records r ON r.id = x.record_id
                 LEFT JOIN memory_meta m ON m.record_id = r.id
                 WHERE x.entity_id = ?1 AND r.scope = ?2 AND r.status = 'active'
                 ORDER BY COALESCE(m.occurred_start, r.source_ms) DESC LIMIT ?3",
            )?;
            let rows = s.query_map(params![entity, scope, MAX_FACTS as i64], |r| {
                Ok(Fact {
                    id: r.get(0)?,
                    category: r.get(1)?,
                    content: r.get(2)?,
                    when: r.get(3)?,
                })
            })?;
            Ok((name, rows.collect::<rusqlite::Result<Vec<_>>>()?))
        })?;
        let clear = |db: &AssistDb| {
            db.with(|c| {
                c.execute(
                    "DELETE FROM memory_dirty_entities WHERE entity_id = ?1 AND scope = ?2",
                    params![entity, scope],
                )
            })
        };
        let Some(name) = name else {
            clear(&db)?;
            continue;
        };
        if (facts.len() as i64) < MIN_FACTS {
            db.with(|c| {
                c.execute(
                    "DELETE FROM memory_entity_notes WHERE entity_id = ?1 AND scope = ?2",
                    params![entity, scope],
                )
            })?;
            clear(&db)?;
            continue;
        }
        let current: Option<String> = db.with(|c| {
            use rusqlite::OptionalExtension;
            c.query_row(
                "SELECT text FROM memory_entity_notes WHERE entity_id = ?1 AND scope = ?2",
                params![entity, scope],
                |r| r.get(0),
            )
            .optional()
        })?;
        let mut user = format!("ENTITY: {name}\n\nFACTS (newest first):\n");
        for f in &facts {
            let label = if f.category == "inference" {
                " (unconfirmed guess)"
            } else {
                ""
            };
            user.push_str(&format!(
                "[{}] {} ({}){label}\n",
                f.id,
                f.content.replace('\n', " "),
                date(f.when)
            ));
        }
        user.push_str("\nCURRENT NOTE:\n");
        user.push_str(current.as_deref().unwrap_or("(none)"));
        let reply = match llm(SYSTEM.to_string(), user).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("[memory] consolidation of {name} failed: {e}");
                break;
            }
        };
        let parsed = super::retain::json_object(&reply);
        let note = parsed
            .as_ref()
            .and_then(|v| v.get("note"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(1200).collect::<String>());
        let Some(note) = note else {
            clear(&db)?;
            continue;
        };
        let ids: Vec<String> = facts.iter().map(|f| f.id.clone()).collect();
        let ids = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into());
        db.tx(|tx| {
            let e = |x: rusqlite::Error| format!("{}: {x}", super::super::db::ERR_ASSIST_DB);
            let still: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM memory_records r JOIN memory_entity_refs x ON x.record_id = r.id
                     WHERE x.entity_id = ?1 AND r.scope = ?2 AND r.status = 'active'",
                    params![entity, scope],
                    |r| r.get(0),
                )
                .map_err(e)?;
            if still < facts.len() as i64 {
                return Ok(());
            }
            tx.execute(
                "INSERT OR REPLACE INTO memory_entity_notes(entity_id, scope, text, ids, built_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![entity, scope, note, ids, super::super::now_ms()],
            )
            .map_err(e)?;
            tx.execute(
                "DELETE FROM memory_dirty_entities WHERE entity_id = ?1 AND scope = ?2",
                params![entity, scope],
            )
            .map_err(e)?;
            Ok(())
        })?;
        written += 1;
    }
    Ok(written)
}
