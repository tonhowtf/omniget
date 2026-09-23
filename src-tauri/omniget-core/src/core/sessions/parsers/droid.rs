//! Factory Droid (estudo 06, Droid §j; Parte 6 §8.1).
//!
//! `${DROID_SESSIONS_DIR:-~/.factory/sessions}/<slug-do-projeto>/<uuid>.jsonl`:
//! 1ª linha `{type:"session_start", id, title, cwd}`, depois
//! `{type:"message", id?, timestamp, message:{role, content[]}}` com blocos
//! Anthropic (`text|thinking|tool_use|tool_result|image`) e
//! `{type:"compaction_state"}`. Ao lado, `<uuid>.settings.json` com `model` e
//! `tokenUsage{inputTokens, outputTokens, cacheCreationTokens,
//! cacheReadTokens, thinkingTokens}` acumulado da sessão, sem custo. Como o
//! uso só existe acumulado, ele vai inteiro para o último turno do assistente
//! (o total da sessão fica certo; o dia a dia usa o horário desse turno).
//! Droid não está instalado aqui: só fixture.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{Role, Session, SessionMeta, SessionSource, TokenUsage, Turn};

pub struct DroidSource;

const TOOL: &str = "droid";

fn root() -> Option<PathBuf> {
    env_dir("DROID_SESSIONS_DIR").or_else(|| home_join(&[".factory", "sessions"]))
}

fn files() -> Vec<PathBuf> {
    let Some(r) = root() else { return Vec::new() };
    find_files(&r, 2, |p| {
        p.extension().and_then(|e| e.to_str()) == Some("jsonl")
    })
}

fn settings_of(f: &Path) -> Option<Value> {
    read_json(&f.with_file_name(format!("{}.settings.json", file_stem(f))))
}

/// `custom:Claude-Opus-4.5-[Anthropic]-0` → `Claude-Opus-4.5-0`.
fn clean_model(m: &str) -> String {
    let m = m.strip_prefix("custom:").unwrap_or(m);
    let mut out = String::new();
    let mut depth = 0;
    for c in m.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let mut out = out.replace("--", "-");
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn parse(
    lines: &[Value],
    settings: Option<&Value>,
    fallback_id: &str,
    source: &Path,
) -> Session {
    let mut meta = new_meta(TOOL, fallback_id, source);
    let model = settings
        .and_then(|s| s.get("model"))
        .and_then(Value::as_str)
        .map(clean_model);
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for l in lines {
        match l.get("type").and_then(Value::as_str).unwrap_or("") {
            "session_start" => {
                if let Some(id) = s(l, &["id", "sessionId"]) {
                    meta.id = id;
                }
                meta.title = s(l, &["title", "sessionTitle"]);
                meta.project_path = s(l, &["cwd", "workingDirectory"]);
                meta.started = l.get("timestamp").and_then(ts_str);
                meta.parent_session = s(l, &["parentSessionId", "parent"]);
            }
            "message" => {
                let msg = l.get("message").cloned().unwrap_or(Value::Null);
                let role = role_of(msg.get("role").and_then(Value::as_str).unwrap_or(""));
                let (text, calls, res) =
                    anthropic_blocks(msg.get("content").unwrap_or(&Value::Null));
                results.extend(res);
                if role == Role::User && text.trim().is_empty() && calls.is_empty() {
                    continue;
                }
                let mut t = turn(
                    role,
                    l.get("timestamp").and_then(ts_str).unwrap_or_default(),
                    text,
                );
                t.tool_calls = calls;
                t.message_id = s(l, &["id"]).or_else(|| s(&msg, &["id"]));
                if role == Role::Assistant {
                    t.model = s(&msg, &["model"])
                        .map(|m| clean_model(&m))
                        .or_else(|| model.clone());
                }
                turns.push(t);
            }
            _ => {}
        }
    }
    attach_results(&mut turns, results);
    if let Some(tu) = settings.and_then(|s| s.get("tokenUsage")) {
        let us = TokenUsage {
            input: u(tu, &["inputTokens"]),
            output: u(tu, &["outputTokens"]),
            cache_read: u(tu, &["cacheReadTokens"]),
            cache_write: u(tu, &["cacheCreationTokens"]),
            reasoning: u(tu, &["thinkingTokens"]),
        };
        if us.total() > 0 {
            match turns.iter().rposition(|t| t.role == Role::Assistant) {
                Some(i) => turns[i].usage = us,
                None => {
                    let mut t = turn(Role::Assistant, "", "");
                    t.usage = us;
                    t.model = model.clone();
                    turns.push(t);
                }
            }
        }
    }
    finalize(&mut meta, &mut turns);
    if meta.models.is_empty() {
        meta.models.extend(model);
    }
    Session { meta, turns }
}

fn load_file(f: &Path) -> Session {
    let settings = settings_of(f);
    parse(&read_jsonl(f), settings.as_ref(), &file_stem(f), f)
}

impl SessionSource for DroidSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        root().filter(|p| p.is_dir()).into_iter().collect()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = files().iter().map(|f| load_file(f).meta).collect();
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        files()
            .into_iter()
            .find(|f| file_stem(f) == id)
            .map(|f| load_file(&f))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        Some(format!("droid --resume {}", shell_quote(&meta.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn droid_fixture() {
        let lines: Vec<Value> = [
            r#"{"type":"session_start","id":"11111111-2222-3333-4444-555555555555","title":"Fix login","cwd":"/home/u/app","timestamp":"2026-08-07T11:59:00Z"}"#,
            r#"{"type":"message","timestamp":"2026-08-07T12:00:00Z","message":{"role":"user","content":[{"type":"text","text":"conserte o login"}]}}"#,
            r#"{"type":"message","timestamp":"2026-08-07T12:00:05Z","message":{"role":"assistant","content":[{"type":"text","text":"Vou rodar os testes."},{"type":"tool_use","id":"tu1","name":"Execute","input":{"command":"npm test"}}]}}"#,
            r#"{"type":"message","timestamp":"2026-08-07T12:00:09Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"1 falhou","is_error":true}]}}"#,
            r#"{"type":"compaction_state","timestamp":"2026-08-08T12:00:00Z"}"#,
        ]
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
        let settings: Value = serde_json::from_str(r#"{"model":"custom:Claude-Opus-4.5-Thinking-[Anthropic]-0","tokenUsage":{"inputTokens":1234,"outputTokens":567,"cacheCreationTokens":89,"cacheReadTokens":12,"thinkingTokens":34}}"#).unwrap();
        let s = parse(&lines, Some(&settings), "x", Path::new("/x/a.jsonl"));
        assert_eq!(s.meta.id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(s.meta.project_path.as_deref(), Some("/home/u/app"));
        assert_eq!(s.turns.len(), 2);
        let c = &s.turns[1].tool_calls[0];
        assert_eq!(c.name_canonical, "Bash");
        assert_eq!(c.status, crate::core::sessions::model::ToolStatus::Error);
        assert_eq!(s.meta.usage.input, 1234);
        assert_eq!(s.meta.usage.reasoning, 34);
        assert_eq!(
            s.meta.models,
            vec!["Claude-Opus-4.5-Thinking-0".to_string()]
        );
    }
}
