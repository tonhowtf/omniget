//! C-5: the old chat conversations (`<app_data>/llm/conversations/*.jsonl`)
//! become native threads, once, idempotently. The thread id is the
//! conversation id, so the native driver keeps reading the same JSONL as the
//! model context and a migrated conversation continues where it stopped.
//! Idempotency is layered: a thread that exists is skipped before its file is
//! read, and every import command carries a deterministic command id
//! (`migrate:c5:…`) whose receipt dedupes a retry after a crash.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::engine::ThreadsEngine;
use super::model::{Command, CommandEnvelope, ImportMessage};
use crate::core::llm::coordinator::ConversationRecord;
use crate::core::llm::types::{ContentPart, Role};

/// Project that receives conversations with no folder.
pub const IMPORTED_PROJECT_ID: &str = "prj_imported";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub scanned: usize,
    pub imported: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
}

fn project_id_for(root: &str) -> String {
    let mut h = Sha256::new();
    h.update(root.as_bytes());
    format!("prj_{}", &hex::encode(h.finalize())[..16])
}

/// Read one JSONL into import messages. Tool calls ride on the assistant
/// message that asked for them, with their results attached.
pub fn read_conversation(path: &Path) -> (Vec<ImportMessage>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), None);
    };
    let records: Vec<ConversationRecord> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let agent = records.iter().rev().find_map(|r| r.agent.clone());
    let mut out: Vec<ImportMessage> = Vec::new();
    for rec in records {
        let texts: String = rec
            .message
            .parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        match rec.message.role {
            Role::Tool => {
                for part in &rec.message.parts {
                    if let ContentPart::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } = part
                    {
                        let found = out.iter_mut().rev().find_map(|m| {
                            m.tools
                                .iter_mut()
                                .find(|t| t["id"].as_str() == Some(tool_use_id.as_str()))
                        });
                        if let Some(tool) = found {
                            let clipped: String = content.chars().take(16_000).collect();
                            tool["output"] = json!(clipped);
                            tool["isError"] = json!(is_error);
                        }
                    }
                }
            }
            role => {
                let tools: Vec<serde_json::Value> = rec
                    .message
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::ToolUse { id, name, input } => {
                            Some(json!({ "id": id, "name": name, "input": input }))
                        }
                        _ => None,
                    })
                    .collect();
                let role = match role {
                    Role::System => "system",
                    Role::User => "user",
                    _ => "assistant",
                };
                if texts.is_empty() && tools.is_empty() {
                    continue;
                }
                out.push(ImportMessage {
                    message_id: None,
                    role: role.to_string(),
                    text: texts,
                    created_at: Some(rec.ts.clone()),
                    tools,
                });
            }
        }
    }
    (out, agent)
}

/// Import every conversation in `dir` that is not a thread yet. `workspace_of`
/// maps a conversation id to its folder (the project it lands in).
pub fn migrate_conversations(
    engine: &ThreadsEngine,
    dir: &Path,
    workspace_of: impl Fn(&str) -> Option<PathBuf>,
) -> MigrationReport {
    let mut report = MigrationReport::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return report;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
        .collect();
    files.sort();
    for path in files {
        let Some(id) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        // Throwaway conversations: the help sessions and the stateless bridge.
        if id.starts_with("help-") || id.starts_with("bridge-") {
            continue;
        }
        report.scanned += 1;
        let exists = engine
            .read(|c| super::store::thread_row(c, &id))
            .ok()
            .flatten()
            .is_some();
        if exists {
            report.skipped += 1;
            continue;
        }
        let (messages, agent) = read_conversation(&path);
        if !messages.iter().any(|m| m.role == "user") {
            report.skipped += 1;
            continue;
        }
        let root = workspace_of(&id)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let project_id = match ensure_project(engine, &root) {
            Ok(p) => p,
            Err(e) => {
                report.failed.push(format!("{id}: {e}"));
                continue;
            }
        };
        let title = messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| super::decider::title_from(&m.text))
            .unwrap_or_default();
        let created_at = messages.first().and_then(|m| m.created_at.clone());
        let env = CommandEnvelope {
            command_id: Some(format!("migrate:c5:thread:{id}")),
            command: Command::HistoryImport {
                thread_id: id.clone(),
                project_id,
                title,
                instance_id: "native".into(),
                driver: "native".into(),
                agent_id: agent,
                created_at,
                messages,
            },
        };
        match engine.dispatch_blocking(env) {
            Ok(r) if r.deduplicated => report.skipped += 1,
            Ok(_) => report.imported += 1,
            Err(e) => report.failed.push(format!("{id}: {e}")),
        }
    }
    report
}

fn ensure_project(engine: &ThreadsEngine, root: &str) -> Result<String, String> {
    let existing: Option<String> = engine.read(|c| {
        use rusqlite::OptionalExtension;
        c.query_row(
            "SELECT project_id FROM projects WHERE workspace_root = ?1 AND deleted_at IS NULL LIMIT 1",
            [root],
            |r| r.get(0),
        )
        .optional()
        .map_err(super::store::db_err)
    })?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let (id, title) = if root.is_empty() {
        (IMPORTED_PROJECT_ID.to_string(), "Conversations".to_string())
    } else {
        (project_id_for(root), String::new())
    };
    let env = CommandEnvelope {
        command_id: Some(format!("migrate:c5:project:{id}")),
        command: Command::ProjectCreate {
            project_id: Some(id.clone()),
            title,
            workspace_root: root.to_string(),
        },
    };
    engine.dispatch_blocking(env)?;
    Ok(id)
}
