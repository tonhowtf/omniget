use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::engine::ThreadsEngine;
use super::model::{Command, CommandEnvelope, DomainEvent, ImportMessage};
use super::store;
use crate::core::llm::drivers::now_iso;
use crate::core::sessions::model::{Role, ToolStatus, Turn};

pub const EXTERNAL_PROJECT: &str = "prj_external";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalImport {
    pub thread_id: String,
    pub project_id: String,
    pub turns: u32,
    pub existing: bool,
    pub resume_command: Option<String>,
}

pub fn external_thread_id(tool: &str, session_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(session_id.as_bytes());
    let hex: String = h
        .finalize()
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    let tool: String = tool
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(16)
        .collect();
    format!("ext_{tool}_{hex}")
}

fn role_of(r: Role) -> Option<&'static str> {
    match r {
        Role::User => Some("user"),
        Role::Assistant => Some("assistant"),
        Role::System => Some("system"),
        Role::Tool => None,
    }
}

pub fn messages_of(turns: &[Turn]) -> Vec<ImportMessage> {
    let mut out: Vec<ImportMessage> = Vec::new();
    for (i, t) in turns.iter().enumerate() {
        let tools: Vec<Value> = t
            .tool_calls
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "name": c.name_raw,
                    "input": c.input,
                    "output": c.result,
                    "isError": c.status == ToolStatus::Error,
                })
            })
            .collect();
        match role_of(t.role) {
            Some(role) => {
                if t.text.trim().is_empty() && tools.is_empty() {
                    continue;
                }
                out.push(ImportMessage {
                    message_id: Some(
                        t.message_id
                            .clone()
                            .map(|m| format!("{m}:{i}"))
                            .unwrap_or_else(|| format!("m{i}")),
                    ),
                    role: role.to_string(),
                    text: t.text.clone(),
                    created_at: Some(t.ts.clone()).filter(|s| !s.is_empty()),
                    tools,
                });
            }
            None => {
                if let Some(last) = out.last_mut() {
                    last.tools.extend(tools);
                }
            }
        }
    }
    out
}

async fn dispatch(engine: &ThreadsEngine, command: Command) -> Result<(), String> {
    engine
        .dispatch(CommandEnvelope {
            command_id: Some(format!("server:{}", uuid::Uuid::new_v4().simple())),
            command,
        })
        .await
        .map(|_| ())
}

async fn project_for(engine: &ThreadsEngine, path: Option<&str>) -> Result<String, String> {
    let snap = engine.read(store::snapshot)?;
    let root = path.map(str::trim).filter(|p| !p.is_empty()).unwrap_or("");
    if root.is_empty() {
        if !snap
            .projects
            .iter()
            .any(|p| p.project_id == EXTERNAL_PROJECT)
        {
            dispatch(
                engine,
                Command::ProjectCreate {
                    project_id: Some(EXTERNAL_PROJECT.into()),
                    title: "External".into(),
                    workspace_root: String::new(),
                },
            )
            .await?;
        }
        return Ok(EXTERNAL_PROJECT.into());
    }
    if let Some(p) = snap.projects.iter().find(|p| p.workspace_root == root) {
        return Ok(p.project_id.clone());
    }
    let id = format!("prj_{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
    dispatch(
        engine,
        Command::ProjectCreate {
            project_id: Some(id.clone()),
            title: String::new(),
            workspace_root: root.to_string(),
        },
    )
    .await?;
    Ok(id)
}

pub async fn import_external(
    engine: &Arc<ThreadsEngine>,
    tool: &str,
    session_id: &str,
) -> Result<ExternalImport, String> {
    let thread_id = external_thread_id(tool, session_id);
    if let Some(row) = engine.read(|c| store::thread_row(c, &thread_id))? {
        return Ok(ExternalImport {
            thread_id,
            project_id: row.project_id,
            turns: row.turn_count,
            existing: true,
            resume_command: row
                .external
                .as_ref()
                .and_then(|e| e["resumeCommand"].as_str().map(str::to_string)),
        });
    }
    let page =
        crate::core::sessions::api::get(tool.to_string(), session_id.to_string(), None, None)
            .await?;
    let project_id = project_for(engine, page.meta.project_path.as_deref()).await?;
    let messages = messages_of(&page.turns);
    let title = page
        .meta
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| format!("{tool} session"));
    dispatch(
        engine,
        Command::HistoryImport {
            thread_id: thread_id.clone(),
            project_id: project_id.clone(),
            title,
            instance_id: format!("external:{tool}"),
            driver: tool.to_string(),
            agent_id: None,
            created_at: page.meta.started.clone(),
            messages,
        },
    )
    .await?;
    dispatch(
        engine,
        Command::HostRecord {
            event: DomainEvent::ExternalLinked {
                thread_id: thread_id.clone(),
                tool: tool.to_string(),
                session_id: session_id.to_string(),
                source: Some(page.meta.source.to_string_lossy().into_owned()),
                resume_command: page.resume_command.clone(),
                linked_at: now_iso(),
            },
        },
    )
    .await?;
    let turns = engine
        .read(|c| store::thread_row(c, &thread_id))?
        .map(|r| r.turn_count)
        .unwrap_or(0);
    Ok(ExternalImport {
        thread_id,
        project_id,
        turns,
        existing: false,
        resume_command: page.resume_command,
    })
}
