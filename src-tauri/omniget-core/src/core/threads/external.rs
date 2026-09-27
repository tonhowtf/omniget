//! Sessions of CLIs run outside OmniGet as threads (plan §3.7/§3.8, T8).
//!
//! `import_external(tool, session_id)` reads the session through
//! `core::sessions` and makes a read-only "external" thread with its turns
//! (idempotent: the thread id comes from the session). `resume_external`
//! opens a new, normal thread on the right driver with the same history and
//! the session id as the resume cursor, so the driver continues it
//! (`--resume`).

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
    /// The thread existed already; nothing was imported again.
    pub existing: bool,
    pub resume_command: Option<String>,
}

/// `ext_<tool>_<12 hex of sha256(session id)>`.
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

/// The neutral turns of a session as import messages (tool calls ride on
/// the assistant message that made them).
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
            // A tool-only turn: attach its calls to the previous message.
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

/// The project of `path` (created when missing), or the shared "External"
/// project when the session has no folder.
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

/// Read-only thread mirroring `tool`/`session_id`.
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

/// The account instance (`<cli>-<account id>`) whose config dir is the one
/// the session came from; `None` = the target's default instance.
fn instance_for_target(t: &crate::core::sessions::api::ResumeTarget) -> Option<String> {
    let dir = t.config_dir.as_deref().filter(|d| !d.is_empty())?;
    let want = std::path::Path::new(dir);
    let store = crate::core::llm::cli_runtime::accounts::AccountStore::default_store()?;
    let list = store.list();
    let found = list.iter().find(|a| {
        a.cli.as_str() == t.driver
            && (a.config_dir == want
                || a.config_dir.canonicalize().ok() == want.canonicalize().ok())
    })?;
    Some(format!("{}-{}", found.cli.as_str(), found.id))
}

/// A new thread that continues the external one on `instance_id`/`driver`
/// (default: the tool's own driver and default instance). The session id is
/// the resume cursor the driver gets in `SessionStart.resume_cursor`.
pub async fn resume_external(
    engine: &Arc<ThreadsEngine>,
    thread_id: &str,
    instance_id: Option<String>,
    driver: Option<String>,
) -> Result<String, String> {
    let row = engine
        .read(|c| store::thread_row(c, thread_id))?
        .ok_or_else(|| format!("ERR_THREADS_NOT_FOUND: no thread {thread_id}"))?;
    let ext = row
        .external
        .clone()
        .ok_or_else(|| format!("ERR_THREADS_INVALID: {thread_id} is not an external thread"))?;
    let tool = ext["tool"].as_str().unwrap_or(&row.driver).to_string();
    let session_id = ext["sessionId"]
        .as_str()
        .ok_or_else(|| "ERR_THREADS_INVALID: external thread without a session id".to_string())?
        .to_string();
    // Where the session continues: the target `core::sessions` computes per
    // tool (driver, default instance and the cursor in that driver's shape).
    let explicit_driver = driver.filter(|d| !d.is_empty());
    let explicit_instance = instance_id.filter(|i| !i.is_empty());
    let target =
        crate::core::sessions::api::get(tool.clone(), session_id.clone(), Some(0), Some(1))
            .await
            .ok()
            .and_then(|p| p.resume_target);
    let same_driver = |t: &crate::core::sessions::api::ResumeTarget| {
        explicit_driver.as_deref().is_none_or(|d| d == t.driver)
    };
    let (driver, instance_id, cursor) = match (target.as_ref().filter(|t| same_driver(t)), explicit_driver.clone()) {
        (Some(t), _) => {
            let instance = explicit_instance
                .unwrap_or_else(|| instance_for_target(t).unwrap_or_else(|| t.instance_id.clone()));
            (t.driver.clone(), instance, t.cursor.clone())
        }
        // Another driver chosen by hand: it gets the plain session id.
        (None, Some(d)) => {
            let instance = explicit_instance.unwrap_or_else(|| d.clone());
            (d, instance, Value::String(session_id.clone()))
        }
        (None, None) => {
            return Err(format!(
                "ERR_THREADS_INVALID: {tool} session {session_id} cannot be resumed by a Central driver"
            ))
        }
    };
    let source = engine.read(|c| store::fork_source(c, thread_id, u32::MAX))?;
    let new_id = format!("thr_{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
    let mut messages = Vec::new();
    for t in &source.turns {
        for (mid, role, text, at) in &t.messages {
            messages.push(ImportMessage {
                message_id: Some(format!("{new_id}:{mid}")),
                role: role.clone(),
                text: text.clone(),
                created_at: Some(at.clone()),
                tools: Vec::new(),
            });
        }
    }
    dispatch(
        engine,
        Command::HistoryImport {
            thread_id: new_id.clone(),
            project_id: row.project_id.clone(),
            title: row.title.clone(),
            instance_id,
            driver,
            agent_id: None,
            created_at: None,
            messages,
        },
    )
    .await?;
    dispatch(
        engine,
        Command::HostRecord {
            event: DomainEvent::SessionSet {
                thread_id: new_id.clone(),
                status: None,
                last_error: None,
                resume_cursor: Some(cursor),
                provider_thread_id: Some(session_id),
                updated_at: now_iso(),
            },
        },
    )
    .await?;
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sessions::model::{TokenUsage, ToolCall};

    fn turn(role: Role, text: &str) -> Turn {
        Turn {
            role,
            ts: "2026-09-22T10:00:00Z".into(),
            text: text.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            cost_usd: None,
            model: None,
            message_id: None,
        }
    }

    #[test]
    fn tool_turns_ride_on_the_assistant_message() {
        let mut tool = turn(Role::Tool, "");
        tool.tool_calls.push(ToolCall {
            id: "c1".into(),
            name_canonical: "Bash".into(),
            name_raw: "Bash".into(),
            input: json!({"command": "ls"}),
            result: Some("a\nb".into()),
            status: ToolStatus::Error,
            ms: None,
            subagent: None,
        });
        let turns = vec![
            turn(Role::User, "list files"),
            turn(Role::Assistant, "running ls"),
            tool,
            turn(Role::Assistant, ""),
        ];
        let m = messages_of(&turns);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].role, "user");
        assert_eq!(m[1].tools.len(), 1);
        assert_eq!(m[1].tools[0]["isError"], json!(true));
        assert_eq!(
            external_thread_id("claude", "abc"),
            external_thread_id("claude", "abc")
        );
        assert!(external_thread_id("claude", "abc").starts_with("ext_claude_"));
    }
}
