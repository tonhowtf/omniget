//! JetBrains Junie CLI (estudo 06, Junie §j; Parte 6 §8.2).
//!
//! `${JUNIE_HOME:-~/.junie}/sessions/<id>/events.jsonl` (id tipo
//! `session-251209-172932-1ze8`, horário local no nome), mais `index.jsonl`
//! (registro das sessões) e `transcript.md`. Eventos: o prompt do usuário é
//! `kind == "UserPromptEvent"` (no topo ou em `event.agentEvent`) com
//! `prompt`; o uso vem em `event.agentEvent.kind == "LlmResponseMetadataEvent"`
//! com `modelUsage[{model, inputTokens, outputTokens, cacheInputTokens |
//! cacheReadInputTokens, cacheCreationInputTokens, cost, time}]` e
//! `timestampMs`. Os demais eventos do agente (texto, tools) são lidos de forma
//! tolerante pelo `kind`. Os eventos de estado (`AgentStateUpdatedEvent` etc.)
//! são grandes e ignorados. Formato documentado só por terceiros; Junie não
//! está instalado aqui: só fixture.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct JunieSource;

const TOOL: &str = "junie";

const SKIP: &[&str] = &[
    "AgentStateUpdatedEvent",
    "AgentCurrentStatusUpdatedEvent",
    "AgentPatchCreatedEvent",
];

fn home_dir() -> Option<PathBuf> {
    env_dir("JUNIE_HOME").or_else(|| home_join(&[".junie"]))
}

fn files() -> Vec<PathBuf> {
    let Some(h) = home_dir() else {
        return Vec::new();
    };
    find_files(&h.join("sessions"), 2, |p| file_name(p) == "events.jsonl")
}

/// `session-YYMMDD-HHMMSS-xxxx` → ms (horário local).
fn id_time(id: &str) -> Option<i64> {
    let mut parts = id.split('-');
    if parts.next()? != "session" {
        return None;
    }
    let (d, t) = (parts.next()?, parts.next()?);
    let n = chrono::NaiveDateTime::parse_from_str(&format!("{d}{t}"), "%y%m%d%H%M%S").ok()?;
    chrono::TimeZone::from_local_datetime(&chrono::Local, &n)
        .earliest()
        .map(|x| x.timestamp_millis())
}

fn kind_of(v: &Value) -> (String, Value) {
    if let Some(k) = v.get("kind").and_then(Value::as_str) {
        return (k.to_string(), v.clone());
    }
    if let Some(ae) = v.pointer("/event/agentEvent") {
        if let Some(k) = ae.get("kind").and_then(Value::as_str) {
            return (k.to_string(), ae.clone());
        }
    }
    if let Some(ev) = v.get("event") {
        if let Some(k) = ev.get("kind").and_then(Value::as_str) {
            return (k.to_string(), ev.clone());
        }
    }
    (String::new(), Value::Null)
}

fn assistant<'a>(turns: &'a mut Vec<Turn>, ts: &str) -> &'a mut Turn {
    if !matches!(turns.last(), Some(t) if t.role == Role::Assistant) {
        turns.push(turn(Role::Assistant, ts.to_string(), ""));
    }
    turns.last_mut().expect("existe")
}

pub fn parse(lines: &[Value], id: &str, source: &Path) -> Session {
    let mut meta = new_meta(TOOL, id, source);
    meta.started = id_time(id).map(ms_to_rfc3339);
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for l in lines {
        let (kind, ev) = kind_of(l);
        if kind.is_empty() || SKIP.contains(&kind.as_str()) {
            continue;
        }
        let ts = l
            .get("timestampMs")
            .or_else(|| l.get("timestamp"))
            .and_then(ts_str)
            .unwrap_or_default();
        match kind.as_str() {
            "UserPromptEvent" => {
                let text = s(&ev, &["prompt", "text", "message"]).unwrap_or_default();
                turns.push(turn(Role::User, ts, text));
            }
            "LlmResponseMetadataEvent" => {
                for mu in ev
                    .get("modelUsage")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let us = TokenUsage {
                        input: u(mu, &["inputTokens", "input"]),
                        output: u(mu, &["outputTokens", "output"]),
                        cache_read: u(
                            mu,
                            &["cacheInputTokens", "cacheReadInputTokens", "cacheRead"],
                        ),
                        cache_write: u(
                            mu,
                            &[
                                "cacheCreateTokens",
                                "cacheCreationInputTokens",
                                "cacheWrite",
                            ],
                        ),
                        reasoning: u(mu, &["reasoningTokens", "thinkingTokens"]),
                    };
                    let cost = fnum(mu, &["cost"]);
                    let t = assistant(&mut turns, &ts);
                    t.usage.add(&us);
                    if let Some(c) = cost {
                        t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + c);
                    }
                    if t.model.is_none() {
                        t.model = s(mu, &["model"]);
                    }
                }
            }
            k if k.contains("Tool")
                && (k.contains("Result") || k.contains("Finished") || k.contains("Completed")) =>
            {
                let id = s(&ev, &["toolCallId", "toolUseId", "id", "callId"]).unwrap_or_default();
                let body = ev
                    .get("result")
                    .or_else(|| ev.get("output"))
                    .or_else(|| ev.get("content"))
                    .map(result_text)
                    .unwrap_or_default();
                let err = ev.get("isError").and_then(Value::as_bool).unwrap_or(false)
                    || s(&ev, &["status"])
                        .map(|x| {
                            x.eq_ignore_ascii_case("error") || x.eq_ignore_ascii_case("failed")
                        })
                        .unwrap_or(false);
                results.push((id, body, err));
            }
            k if k.contains("Tool") => {
                let name = s(&ev, &["toolName", "name", "tool"])
                    .or_else(|| {
                        ev.pointer("/tool/name")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "unknown".into());
                let input = ev
                    .get("input")
                    .or_else(|| ev.get("arguments"))
                    .or_else(|| ev.get("params"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let id = s(&ev, &["toolCallId", "toolUseId", "id", "callId"]).unwrap_or_default();
                let t = assistant(&mut turns, &ts);
                if id.is_empty() || !t.tool_calls.iter().any(|c| c.id == id) {
                    t.tool_calls.push(tool_call(id, &name, input));
                }
            }
            k if k.contains("Message") || k.contains("Response") || k.contains("Text") => {
                if let Some(text) = s(&ev, &["text", "message", "content", "response"]) {
                    let t = assistant(&mut turns, &ts);
                    if !t.text.is_empty() {
                        t.text.push('\n');
                    }
                    t.text.push_str(&text);
                }
            }
            _ => {}
        }
    }
    attach_results(&mut turns, results);
    for t in turns.iter_mut() {
        for c in t.tool_calls.iter_mut() {
            if c.status == ToolStatus::Pending {
                c.status = ToolStatus::Ok;
            }
        }
    }
    finalize(&mut meta, &mut turns);
    Session { meta, turns }
}

/// Título e projeto do `index.jsonl`, quando existir.
fn index_info(id: &str) -> (Option<String>, Option<String>) {
    let Some(h) = home_dir() else {
        return (None, None);
    };
    for idx in [
        h.join("sessions").join("index.jsonl"),
        h.join("index.jsonl"),
    ] {
        for l in read_jsonl(&idx) {
            let lid = s(&l, &["id", "sessionId"]).unwrap_or_default();
            if lid == id || lid.ends_with(&format!("/{id}")) {
                return (
                    s(&l, &["title", "name", "task"]),
                    s(&l, &["projectPath", "project", "cwd", "workingDirectory"]),
                );
            }
        }
    }
    (None, None)
}

fn load_file(f: &Path) -> Session {
    let id = parent_name(f);
    let mut s = parse(&read_jsonl(f), &id, f);
    let (title, project) = index_info(&id);
    if title.is_some() {
        s.meta.title = title;
    }
    s.meta.project_path = project;
    s
}

impl SessionSource for JunieSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        home_dir()
            .map(|h| h.join("sessions"))
            .filter(|p| p.is_dir())
            .into_iter()
            .collect()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = files().iter().map(|f| load_file(f).meta).collect();
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        files()
            .into_iter()
            .find(|f| parent_name(f) == id)
            .map(|f| load_file(&f))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        Some(format!(
            "junie --session-id {}",
            shell_quote(&format!("junie://sessions/{}", meta.id))
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junie_fixture() {
        let l: Vec<Value> = [
            r#"{"kind":"UserPromptEvent","prompt":"revise o handler"}"#,
            r#"{"kind":"AgentStateUpdatedEvent","event":{"agentEvent":{"kind":"LlmResponseMetadataEvent","modelUsage":[{"model":"gpt-5","inputTokens":999,"outputTokens":999}]}}}"#,
            r#"{"timestampMs":1750000005000,"event":{"agentEvent":{"kind":"ToolCallEvent","toolCallId":"t1","toolName":"Bash","input":{"command":"ls"}}}}"#,
            r#"{"timestampMs":1750000006000,"event":{"agentEvent":{"kind":"ToolResultEvent","toolCallId":"t1","result":"ok"}}}"#,
            r#"{"timestampMs":1750000007000,"event":{"agentEvent":{"kind":"LlmResponseMetadataEvent","modelUsage":[{"model":"gpt-5","inputTokens":100,"outputTokens":50,"cacheInputTokens":20,"cost":0.01,"time":2000}]}}}"#,
        ]
        .iter()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
        let s = parse(
            &l,
            "session-250622-101010-abcd",
            Path::new("/x/events.jsonl"),
        );
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.meta.usage.input, 100);
        assert_eq!(s.meta.usage.cache_read, 20);
        assert!((s.meta.cost_usd - 0.01).abs() < 1e-9);
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("ok"));
        assert!(id_time("session-250622-101010-abcd").is_some());
    }
}
