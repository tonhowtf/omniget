//! Importação do JSON neutro (`export::to_json`) para retomar a conversa.
//!
//! * Claude Code: grava `<config>/projects/<cwd codificado>/<novo-id>.jsonl`
//!   com a codificação real do diretório (a mesma do CLI), cadeia
//!   `parentUuid`, `sessionId`, `cwd` e `version`, para `claude --resume`
//!   achar a sessão rodando dentro do projeto.
//! * Codex: grava um rollout válido em
//!   `$CODEX_HOME/sessions/AAAA/MM/DD/rollout-…-<uuid>.jsonl` (`session_meta`,
//!   `turn_context`, `response_item`s e `event_msg`s) e registra o título no
//!   `session_index.jsonl`.
//! * Qualquer outra ferramenta: vira Markdown de contexto para colar.
//!
//! Sempre um id novo: importar nunca sobrescreve uma sessão existente. Os
//! tokens das mensagens importadas ficam zerados (o consumo já foi contado na
//! sessão de origem).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use super::export::{self, ExportOut, NeutralExport, FORMAT_ID};
use super::model::{Role, Session};
use super::parsers::group_a::{claude, codex};
use super::util;

pub const ERR_IMPORT: &str = "ERR_SESSIONS_IMPORT";

#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub source_tool: String,
    pub target_tool: String,
    pub session_id: Option<String>,
    pub written: Option<PathBuf>,
    pub resume_command: Option<String>,
    /// Para ferramentas sem importação nativa: o Markdown de contexto.
    pub context: Option<ExportOut>,
}

/// Lê um arquivo exportado (envelope neutro ou `Session` crua).
pub fn read_export(path: &Path) -> Result<Session, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{ERR_IMPORT}: {e}"))?;
    if let Ok(env) = serde_json::from_slice::<NeutralExport>(&bytes) {
        if env.format != FORMAT_ID {
            return Err(format!(
                "{ERR_IMPORT}: formato desconhecido `{}`",
                env.format
            ));
        }
        return Ok(env.session);
    }
    serde_json::from_slice::<Session>(&bytes)
        .map_err(|e| format!("{ERR_IMPORT}: JSON não é uma sessão exportada: {e}"))
}

pub fn import(path: &Path, target: Option<&str>) -> Result<ImportOutcome, String> {
    let s = read_export(path)?;
    import_session(&s, target, None)
}

/// `base`: diretório de configuração alternativo (testes, contas isoladas).
pub fn import_session(
    s: &Session,
    target: Option<&str>,
    base: Option<&Path>,
) -> Result<ImportOutcome, String> {
    let target = target.unwrap_or(&s.meta.tool).to_string();
    let mut out = ImportOutcome {
        source_tool: s.meta.tool.clone(),
        target_tool: target.clone(),
        session_id: None,
        written: None,
        resume_command: None,
        context: None,
    };
    match target.as_str() {
        "claude" => {
            let (id, file) = write_claude(s, base)?;
            let cwd = s.meta.project_path.clone();
            let env_dir = base.map(|b| b.to_path_buf());
            out.resume_command = Some(util::in_dir(
                cwd.as_deref(),
                util::with_env(
                    "CLAUDE_CONFIG_DIR",
                    env_dir.as_deref(),
                    format!("claude --resume {id}"),
                ),
            ));
            out.session_id = Some(id);
            out.written = Some(file);
        }
        "codex" => {
            let (id, file) = write_codex(s, base)?;
            let env_dir = base.map(|b| b.to_path_buf());
            out.resume_command = Some(util::in_dir(
                s.meta.project_path.as_deref(),
                util::with_env(
                    "CODEX_HOME",
                    env_dir.as_deref(),
                    format!("codex resume {id}"),
                ),
            ));
            out.session_id = Some(id);
            out.written = Some(file);
        }
        _ => {
            out.context = Some(export::to_context(s, Some(&target), 120_000));
        }
    }
    Ok(out)
}

fn cwd_of(s: &Session) -> String {
    s.meta
        .project_path
        .clone()
        .filter(|p| !p.is_empty())
        .or_else(|| util::home().map(|h| h.to_string_lossy().to_string()))
        .unwrap_or_else(|| ".".into())
}

fn ts_or_now(ts: &str) -> String {
    util::parse_ts_str(ts)
        .map(util::ms_to_rfc3339)
        .unwrap_or_else(|| util::ms_to_rfc3339(util::now_ms()))
}

/// Tool call de outra ferramenta vira texto (o histórico do Claude/Codex só
/// aceita tools que ele conhece).
fn describe_call(c: &super::model::ToolCall) -> String {
    let mut s = format!(
        "[tool {}] {}",
        c.name_raw,
        util::truncate_chars(&c.input.to_string(), 600)
    );
    if let Some(r) = &c.result {
        s.push_str(&format!("\n[result] {}", util::truncate_chars(r, 600)));
    }
    s
}

fn write_lines(file: &Path, lines: &[Value]) -> Result<(), String> {
    if let Some(p) = file.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{ERR_IMPORT}: {e}"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(file)
        .map_err(|e| format!("{ERR_IMPORT}: {e}"))?;
    for l in lines {
        writeln!(f, "{l}").map_err(|e| format!("{ERR_IMPORT}: {e}"))?;
    }
    Ok(())
}

pub fn write_claude(s: &Session, base: Option<&Path>) -> Result<(String, PathBuf), String> {
    let projects = match base {
        Some(b) => b.join("projects"),
        None => claude::default_roots()
            .into_iter()
            .next()
            .map(|r| r.projects)
            .ok_or_else(|| format!("{ERR_IMPORT}: sem diretório do Claude"))?,
    };
    let cwd = cwd_of(s);
    let id = uuid::Uuid::new_v4().to_string();
    let file = projects
        .join(util::claude_project_dir_name(&cwd))
        .join(format!("{id}.jsonl"));
    let native = s.meta.tool == "claude";
    let branch = s.meta.git_branch.clone().unwrap_or_default();
    let mut lines: Vec<Value> = Vec::new();
    let mut parent: Option<String> = None;
    let mut n = 0u32;
    let common = |parent: &Option<String>, uuid: &str, ts: &str| {
        json!({
            "parentUuid": parent,
            "isSidechain": false,
            "userType": "external",
            "cwd": cwd,
            "sessionId": id,
            "version": "2.1.0",
            "gitBranch": branch,
            "uuid": uuid,
            "timestamp": ts,
        })
    };
    for t in &s.turns {
        let ts = ts_or_now(&t.ts);
        match t.role {
            Role::User => {
                if t.text.trim().is_empty() {
                    continue;
                }
                let uuid = uuid::Uuid::new_v4().to_string();
                let mut l = common(&parent, &uuid, &ts);
                l["type"] = json!("user");
                l["message"] = json!({"role": "user", "content": t.text});
                lines.push(l);
                parent = Some(uuid);
            }
            Role::Assistant => {
                n += 1;
                let mut content: Vec<Value> = Vec::new();
                let mut text = t.text.clone();
                let mut results: Vec<Value> = Vec::new();
                for (j, c) in t.tool_calls.iter().enumerate() {
                    if native {
                        let tid = if c.id.is_empty() {
                            format!("toolu_imp_{n}_{j}")
                        } else {
                            c.id.clone()
                        };
                        content.push(json!({"type": "tool_use", "id": tid, "name": c.name_raw, "input": c.input}));
                        results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": tid,
                            "content": c.result.clone().unwrap_or_default(),
                            "is_error": c.status == super::model::ToolStatus::Error,
                        }));
                    } else {
                        if !text.is_empty() {
                            text.push_str("\n\n");
                        }
                        text.push_str(&describe_call(c));
                    }
                }
                if !text.trim().is_empty() {
                    content.insert(0, json!({"type": "text", "text": text}));
                }
                if content.is_empty() {
                    continue;
                }
                let uuid = uuid::Uuid::new_v4().to_string();
                let mut l = common(&parent, &uuid, &ts);
                l["type"] = json!("assistant");
                l["requestId"] = json!(format!("req_imported_{n}"));
                l["message"] = json!({
                    "id": format!("msg_imported_{}_{n}", &id[..8]),
                    "type": "message",
                    "role": "assistant",
                    "model": t.model.clone().unwrap_or_else(|| "<synthetic>".into()),
                    "content": content,
                    "stop_reason": if results.is_empty() { "end_turn" } else { "tool_use" },
                    "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
                });
                lines.push(l);
                parent = Some(uuid);
                if !results.is_empty() {
                    let ruuid = uuid::Uuid::new_v4().to_string();
                    let mut r = common(&parent, &ruuid, &ts);
                    r["type"] = json!("user");
                    r["message"] = json!({"role": "user", "content": results});
                    lines.push(r);
                    parent = Some(ruuid);
                }
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        return Err(format!("{ERR_IMPORT}: sessão sem mensagens"));
    }
    let title = s
        .meta
        .title
        .clone()
        .unwrap_or_else(|| format!("{} {}", s.meta.tool, s.meta.id));
    lines.push(
        json!({"type": "summary", "summary": format!("{title} (importada)"), "leafUuid": parent}),
    );
    write_lines(&file, &lines)?;
    Ok((id, file))
}

pub fn write_codex(s: &Session, base: Option<&Path>) -> Result<(String, PathBuf), String> {
    let home = match base {
        Some(b) => b.to_path_buf(),
        None => codex::default_roots()
            .into_iter()
            .next()
            .map(|r| r.home)
            .ok_or_else(|| format!("{ERR_IMPORT}: sem diretório do Codex"))?,
    };
    let cwd = cwd_of(s);
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now();
    let file = home
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string())
        .join(format!(
            "rollout-{}-{id}.jsonl",
            now.format("%Y-%m-%dT%H-%M-%S")
        ));
    let now_ts = util::ms_to_rfc3339(util::now_ms());
    let native = s.meta.tool == "codex";
    let model = s
        .meta
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| "gpt-5".into());
    let line = |ts: &str, kind: &str, payload: Value| json!({"timestamp": ts, "type": kind, "payload": payload});
    let mut lines: Vec<Value> = vec![
        line(
            &now_ts,
            "session_meta",
            json!({
                "id": id,
                "timestamp": now_ts,
                "cwd": cwd,
                "originator": "omniget_import",
                "cli_version": "0.0.0",
                "instructions": null,
                "source": "cli",
                "model_provider": "openai",
            }),
        ),
        line(
            &now_ts,
            "turn_context",
            json!({
                "cwd": cwd,
                "approval_policy": "on-request",
                "sandbox_policy": {"type": "read-only"},
                "model": model,
                "summary": "auto",
            }),
        ),
    ];
    let mut n = 0u32;
    for t in &s.turns {
        let ts = ts_or_now(&t.ts);
        match t.role {
            Role::User => {
                if t.text.trim().is_empty() {
                    continue;
                }
                lines.push(line(
                    &ts,
                    "response_item",
                    json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": t.text}]}),
                ));
                lines.push(line(
                    &ts,
                    "event_msg",
                    json!({"type": "user_message", "message": t.text, "images": []}),
                ));
            }
            Role::Assistant => {
                let mut text = t.text.clone();
                for (j, c) in t.tool_calls.iter().enumerate() {
                    n += 1;
                    if native {
                        let cid = if c.id.is_empty() {
                            format!("call_imp_{n}_{j}")
                        } else {
                            c.id.clone()
                        };
                        let args = match &c.input {
                            Value::String(s) => s.clone(),
                            v => v.to_string(),
                        };
                        lines.push(line(
                            &ts,
                            "response_item",
                            json!({"type": "function_call", "name": c.name_raw, "arguments": args, "call_id": cid}),
                        ));
                        lines.push(line(
                            &ts,
                            "response_item",
                            json!({"type": "function_call_output", "call_id": cid, "output": c.result.clone().unwrap_or_default()}),
                        ));
                    } else {
                        if !text.is_empty() {
                            text.push_str("\n\n");
                        }
                        text.push_str(&describe_call(c));
                    }
                }
                if !text.trim().is_empty() {
                    lines.push(line(
                        &ts,
                        "response_item",
                        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}),
                    ));
                    lines.push(line(
                        &ts,
                        "event_msg",
                        json!({"type": "agent_message", "message": text}),
                    ));
                }
            }
            _ => {}
        }
    }
    if lines.len() <= 2 {
        return Err(format!("{ERR_IMPORT}: sessão sem mensagens"));
    }
    write_lines(&file, &lines)?;
    // Título na lista de threads.
    let title = s
        .meta
        .title
        .clone()
        .unwrap_or_else(|| format!("{} {}", s.meta.tool, s.meta.id));
    let idx_line =
        json!({"id": id, "thread_name": format!("{title} (importada)"), "updated_at": now_ts});
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("session_index.jsonl"))
    {
        let _ = writeln!(f, "{idx_line}");
    }
    Ok((id, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sessions::model::{
        SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
    };

    fn sample(tool: &str) -> Session {
        Session {
            meta: SessionMeta {
                tool: tool.into(),
                account: None,
                id: "orig".into(),
                title: Some("Conserto".into()),
                project_path: Some("/w/meu.proj".into()),
                git_branch: Some("main".into()),
                started: Some("2026-09-20T10:00:00.000Z".into()),
                ended: Some("2026-09-20T10:01:00.000Z".into()),
                models: vec!["claude-opus-5".into()],
                parent_session: None,
                turn_count: 2,
                usage: TokenUsage::default(),
                cost_usd: 0.0,
                source: PathBuf::new(),
                mtime_ms: 0,
            },
            turns: vec![
                Turn {
                    role: Role::User,
                    ts: "2026-09-20T10:00:00.000Z".into(),
                    text: "liste".into(),
                    tool_calls: vec![],
                    usage: TokenUsage::default(),
                    cost_usd: None,
                    model: None,
                    message_id: None,
                },
                Turn {
                    role: Role::Assistant,
                    ts: "2026-09-20T10:00:05.000Z".into(),
                    text: "feito".into(),
                    tool_calls: vec![ToolCall {
                        id: "toolu_1".into(),
                        name_canonical: "Bash".into(),
                        name_raw: "Bash".into(),
                        input: json!({"command": "ls"}),
                        result: Some("a.txt".into()),
                        status: ToolStatus::Ok,
                        ms: None,
                        subagent: None,
                    }],
                    usage: TokenUsage::default(),
                    cost_usd: None,
                    model: Some("claude-opus-5".into()),
                    message_id: None,
                },
            ],
        }
    }

    #[test]
    fn claude_import_uses_the_real_dir_encoding_and_reparses() {
        let base =
            std::env::temp_dir().join(format!("omniget-import-claude-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let out = import_session(&sample("claude"), None, Some(&base)).unwrap();
        let file = out.written.unwrap();
        assert!(file.to_string_lossy().contains("-w-meu-proj"));
        let src = claude::ClaudeSource::with_roots(vec![claude::Root {
            account: "t".into(),
            config_dir: Some(base.clone()),
            projects: base.join("projects"),
        }]);
        let back = src.load(out.session_id.as_deref().unwrap()).unwrap();
        assert_eq!(back.turns.len(), 2);
        assert_eq!(back.turns[1].tool_calls[0].result.as_deref(), Some("a.txt"));
        assert_eq!(back.meta.project_path.as_deref(), Some("/w/meu.proj"));
        assert!(out.resume_command.unwrap().contains("claude --resume"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn codex_import_writes_a_rollout_the_parser_reads() {
        let base =
            std::env::temp_dir().join(format!("omniget-import-codex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let out = import_session(&sample("claude"), Some("codex"), Some(&base)).unwrap();
        let src = codex::CodexSource::with_roots(vec![codex::Root {
            account: "t".into(),
            home: base.clone(),
            is_default: false,
        }]);
        let back = src.load(out.session_id.as_deref().unwrap()).unwrap();
        assert_eq!(back.turns.len(), 2);
        assert!(back.turns[1].text.contains("[tool Bash]"));
        assert_eq!(back.meta.title.as_deref(), Some("Conserto (importada)"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn other_targets_get_context_markdown() {
        let out = import_session(&sample("claude"), Some("gemini"), None).unwrap();
        let ctx = out.context.unwrap();
        assert!(ctx.content.contains("Context from a previous session"));
        assert!(ctx.content.contains("liste"));
    }
}
