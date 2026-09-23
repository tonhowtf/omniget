//! Kimi Code (Moonshot) e o kimi-cli legado (estudo 06, Kimi §j; Parte 6 §8.1).
//!
//! Kimi Code: `${KIMI_CODE_HOME:-~/.kimi-code}/sessions/<wd_slug_hash>/<sessão>/
//! agents/<agente>/wire.jsonl` (`main` = sessão; `agent-N` = subagente, vira
//! sessão filha `<sessão>:<agente>`), `state.json` (título, `lastPrompt`),
//! `workspaces.json` (`workspaces.<slug>.root`) e `session_index.jsonl`
//! (`sessionId`, `workDir`). Linhas planas `{type, time(ms), …}`:
//! `config.update{cwd}`, `turn.prompt{input[]}`, `llm.request{model}`,
//! `usage.record{model, usage{inputOther, output, inputCacheRead,
//! inputCacheCreation}, usageScope}` (só `usageScope == "turn"` conta; o
//! `step.end` repete o mesmo uso), e `context.append_loop_event{event}` com o
//! que o agente fez (texto, tool call, resultado; lido de forma tolerante).
//!
//! Legado kimi-cli: `${KIMI_DATA_DIR:-~/.kimi}/sessions/<hash>/<sessão>/wire.jsonl`,
//! linhas `{timestamp(s), message:{type, payload}}`: `TurnBegin{user_input}`,
//! `ContentPart{type:"text", text}`, `ToolCall{id, function{name, arguments}}`,
//! `ToolResult{tool_call_id, return_value}`, `StatusUpdate{token_usage{input_other,
//! output, input_cache_read, input_cache_creation}, message_id}` (o mesmo
//! `message_id` pode repetir: fica o maior). O modelo do legado mora no
//! `config.json`, que também guarda chave de API: não é lido.
//! Kimi não está instalado aqui: só fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct KimiSource;

const TOOL: &str = "kimi";

fn code_home() -> Option<PathBuf> {
    env_dir("KIMI_CODE_HOME").or_else(|| home_join(&[".kimi-code"]))
}

fn legacy_home() -> Option<PathBuf> {
    env_dir("KIMI_DATA_DIR").or_else(|| home_join(&[".kimi"]))
}

/// (arquivo, id, pai, é legado).
fn wires() -> Vec<(PathBuf, String, Option<String>, bool)> {
    let mut out = Vec::new();
    if let Some(h) = code_home() {
        for f in find_files(&h.join("sessions"), 5, |p| file_name(p) == "wire.jsonl") {
            // sessions/<ws>/<sessão>/agents/<agente>/wire.jsonl
            let agent_dir = f.parent();
            let agents = agent_dir.and_then(Path::parent);
            if agents.map(file_name).as_deref() != Some("agents") {
                continue;
            }
            let agent = agent_dir.map(file_name).unwrap_or_default();
            let sess = agents
                .and_then(Path::parent)
                .map(file_name)
                .unwrap_or_default();
            if agent == "main" {
                out.push((f, sess, None, false));
            } else {
                out.push((f, format!("{sess}:{agent}"), Some(sess), false));
            }
        }
    }
    if let Some(h) = legacy_home() {
        for f in find_files(&h.join("sessions"), 3, |p| file_name(p) == "wire.jsonl") {
            let id = parent_name(&f);
            out.push((f, id, None, true));
        }
    }
    out
}

fn kimi_usage(u_: &Value) -> TokenUsage {
    TokenUsage {
        input: u(u_, &["inputOther", "input_other"]),
        output: u(u_, &["output"]),
        cache_read: u(u_, &["inputCacheRead", "input_cache_read"]),
        cache_write: u(u_, &["inputCacheCreation", "input_cache_creation"]),
        reasoning: 0,
    }
}

fn clean_model(m: &str) -> Option<String> {
    let m = m.trim().trim_start_matches("kimi-code/");
    (!m.is_empty() && !m.starts_with("__")).then(|| m.to_string())
}

fn assistant<'a>(turns: &'a mut Vec<Turn>, ts: &str, model: &Option<String>) -> &'a mut Turn {
    if !matches!(turns.last(), Some(t) if t.role == Role::Assistant) {
        let mut t = turn(Role::Assistant, ts.to_string(), "");
        t.model = model.clone();
        turns.push(t);
    }
    turns.last_mut().expect("existe")
}

fn input_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => content_text(other),
    }
}

/// Evento do loop do agente (formato aberto: nomes variam por versão).
fn loop_event(ev: &Value, ts: &str, turns: &mut Vec<Turn>, model: &Option<String>) {
    let ty = ev
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if ty == "step.end" || ty.contains("usage") {
        return;
    }
    let is_tool = ty.contains("tool");
    if is_tool && (ty.contains("result") || ty.contains("output") || ty.contains("end")) {
        let id = s(ev, &["toolCallId", "tool_call_id", "id", "callId"]).unwrap_or_default();
        let body = ev
            .get("output")
            .or_else(|| ev.get("result"))
            .or_else(|| ev.get("returnValue"))
            .or_else(|| ev.get("content"))
            .map(result_text)
            .unwrap_or_default();
        let err = ev
            .get("isError")
            .or_else(|| ev.get("is_error"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        attach_results(turns, vec![(id, body, err)]);
        return;
    }
    if is_tool {
        let name = s(ev, &["name", "toolName"])
            .or_else(|| {
                ev.pointer("/function/name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                ev.pointer("/toolCall/name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "unknown".into());
        let args = ev
            .get("input")
            .or_else(|| ev.get("arguments"))
            .or_else(|| ev.pointer("/function/arguments"))
            .or_else(|| ev.pointer("/toolCall/input"))
            .cloned()
            .unwrap_or(Value::Null);
        let args = match args {
            Value::String(ref st) => serde_json::from_str(st).unwrap_or(args),
            other => other,
        };
        let id = s(ev, &["toolCallId", "tool_call_id", "id", "callId"]).unwrap_or_default();
        let t = assistant(turns, ts, model);
        if !id.is_empty() && t.tool_calls.iter().any(|c| c.id == id) {
            return;
        }
        t.tool_calls.push(tool_call(id, &name, args));
        return;
    }
    if ty.contains("think") || ty.contains("reason") {
        return;
    }
    let text = ev
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| ev.get("content").map(content_text))
        .unwrap_or_default();
    if !text.is_empty()
        && (ty.contains("text")
            || ty.contains("content")
            || ty.contains("message")
            || ty.contains("delta"))
    {
        let t = assistant(turns, ts, model);
        t.text.push_str(&text);
    }
}

/// Parser do `wire.jsonl` do Kimi Code.
pub fn parse_code(lines: &[Value], id: &str, source: &Path) -> Session {
    let mut meta = new_meta(TOOL, id, source);
    let mut turns: Vec<Turn> = Vec::new();
    let mut model: Option<String> = None;
    for l in lines {
        let ty = l.get("type").and_then(Value::as_str).unwrap_or("");
        let ts = l.get("time").and_then(ts_str).unwrap_or_default();
        match ty {
            "metadata" => {
                if meta.started.is_none() {
                    meta.started = l.get("created_at").and_then(ts_str);
                }
            }
            "config.update" => {
                if meta.project_path.is_none() {
                    meta.project_path = s(l, &["cwd"]);
                }
            }
            "turn.prompt" => {
                let origin = l
                    .pointer("/origin/kind")
                    .and_then(Value::as_str)
                    .unwrap_or("user");
                if origin != "user" {
                    continue;
                }
                let text = l.get("input").map(input_text).unwrap_or_default();
                turns.push(turn(Role::User, ts, text));
            }
            "llm.request" => {
                if let Some(m) = l.get("model").and_then(Value::as_str).and_then(clean_model) {
                    model = Some(m);
                }
            }
            "usage.record" => {
                if l.get("usageScope")
                    .and_then(Value::as_str)
                    .unwrap_or("turn")
                    != "turn"
                {
                    continue;
                }
                let m = l
                    .get("model")
                    .and_then(Value::as_str)
                    .and_then(clean_model)
                    .or_else(|| model.clone());
                let us = l.get("usage").map(kimi_usage).unwrap_or_default();
                let t = assistant(&mut turns, &ts, &m);
                t.usage.add(&us);
                if m.is_some() {
                    t.model = m;
                }
            }
            "context.append_loop_event" => {
                if let Some(ev) = l.get("event") {
                    loop_event(ev, &ts, &mut turns, &model);
                }
            }
            _ => {
                if let Some(msg) = l.get("message").filter(|m| m.get("role").is_some()) {
                    let role = role_of(msg.get("role").and_then(Value::as_str).unwrap_or(""));
                    if role == Role::Assistant {
                        let (text, calls, res) =
                            anthropic_blocks(msg.get("content").unwrap_or(&Value::Null));
                        let t = assistant(&mut turns, &ts, &model);
                        t.text.push_str(&text);
                        t.tool_calls.extend(calls);
                        attach_results(&mut turns, res);
                    }
                }
            }
        }
    }
    for t in turns.iter_mut() {
        for c in t.tool_calls.iter_mut() {
            if c.status == ToolStatus::Pending && c.result.is_some() {
                c.status = ToolStatus::Ok;
            }
        }
    }
    finalize(&mut meta, &mut turns);
    Session { meta, turns }
}

/// Parser do `wire.jsonl` do kimi-cli legado.
pub fn parse_legacy(lines: &[Value], id: &str, source: &Path) -> Session {
    let mut meta = new_meta(TOOL, id, source);
    meta.account = Some("kimi-cli".into());
    let mut turns: Vec<Turn> = Vec::new();
    let model: Option<String> = None;
    // message_id → (turno, uso) para ficar com o maior.
    let mut status: HashMap<String, (usize, TokenUsage)> = HashMap::new();
    let mut results = Vec::new();
    for l in lines {
        let Some(msg) = l.get("message") else {
            continue;
        };
        let ts = l.get("timestamp").and_then(ts_str).unwrap_or_default();
        let ty = msg.get("type").and_then(Value::as_str).unwrap_or("");
        let p = msg.get("payload").cloned().unwrap_or(Value::Null);
        match ty {
            "TurnBegin" => {
                let text = p.get("user_input").map(input_text).unwrap_or_default();
                turns.push(turn(Role::User, ts, text));
            }
            "ContentPart" => {
                if p.get("type").and_then(Value::as_str) == Some("text") {
                    let text = s(&p, &["text"]).unwrap_or_default();
                    assistant(&mut turns, &ts, &model).text.push_str(&text);
                }
            }
            "ToolCall" => {
                let name = p
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let args = p
                    .pointer("/function/arguments")
                    .cloned()
                    .unwrap_or(Value::Null);
                let args = match args {
                    Value::String(ref st) => serde_json::from_str(st).unwrap_or(args),
                    other => other,
                };
                let c = tool_call(s(&p, &["id"]).unwrap_or_default(), &name, args);
                assistant(&mut turns, &ts, &model).tool_calls.push(c);
            }
            "ToolResult" => {
                let id = s(&p, &["tool_call_id", "id"]).unwrap_or_default();
                let rv = p.get("return_value").cloned().unwrap_or(Value::Null);
                let body = rv
                    .get("output")
                    .or_else(|| rv.get("message"))
                    .map(result_text)
                    .unwrap_or_else(|| result_text(&rv));
                let err = rv.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                results.push((id, body, err));
            }
            "StatusUpdate" => {
                let Some(tu) = p.get("token_usage") else {
                    continue;
                };
                let us = kimi_usage(tu);
                let key = s(&p, &["message_id"]).unwrap_or_else(|| format!("#{}", status.len()));
                let _ = assistant(&mut turns, &ts, &model);
                let idx = turns.len() - 1;
                let keep_old = status
                    .get(&key)
                    .map(|(_, prev)| prev.total() >= us.total())
                    .unwrap_or(false);
                if !keep_old {
                    status.insert(key, (idx, us));
                }
            }
            _ => {}
        }
    }
    for (_, (i, us)) in status {
        if let Some(t) = turns.get_mut(i) {
            t.usage.add(&us);
        }
    }
    attach_results(&mut turns, results);
    finalize(&mut meta, &mut turns);
    Session { meta, turns }
}

fn session_extras(wire: &Path, sess_id: &str) -> (Option<String>, Option<String>) {
    // <home>/sessions/<ws>/<sessão>/agents/<agente>/wire.jsonl
    let sess_dir = wire.parent().and_then(Path::parent).and_then(Path::parent);
    let ws_slug = sess_dir
        .and_then(Path::parent)
        .map(file_name)
        .unwrap_or_default();
    let home = sess_dir
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::parent);
    let mut title = None;
    let mut project = None;
    if let Some(st) = sess_dir.and_then(|d| read_json(&d.join("state.json"))) {
        title = s(&st, &["title", "lastPrompt"]).map(|t| title_from(&t));
    }
    if let Some(h) = home {
        if let Some(ws) = read_json(&h.join("workspaces.json")) {
            project = ws
                .pointer(&format!("/workspaces/{}/root", ws_slug))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if project.is_none() {
            for l in read_jsonl(&h.join("session_index.jsonl")) {
                if s(&l, &["sessionId"]).as_deref() == Some(sess_id) {
                    project = s(&l, &["workDir"]);
                    break;
                }
            }
        }
    }
    (title, project)
}

fn load_one(f: &Path, id: &str, parent: Option<String>, legacy: bool) -> Session {
    let lines = read_jsonl(f);
    if legacy {
        return parse_legacy(&lines, id, f);
    }
    let mut s = parse_code(&lines, id, f);
    let sess = parent.clone().unwrap_or_else(|| id.to_string());
    let (title, project) = session_extras(f, &sess);
    if title.is_some() && parent.is_none() {
        s.meta.title = title;
    }
    if project.is_some() {
        s.meta.project_path = project;
    }
    s.meta.parent_session = parent;
    s
}

impl SessionSource for KimiSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        [
            code_home().map(|h| h.join("sessions")),
            legacy_home().map(|h| h.join("sessions")),
        ]
        .into_iter()
        .flatten()
        .filter(|p| p.is_dir())
        .collect()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = wires()
            .into_iter()
            .map(|(f, id, parent, legacy)| load_one(&f, &id, parent, legacy).meta)
            .collect();
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        wires()
            .into_iter()
            .find(|(_, wid, _, _)| wid == id)
            .map(|(f, wid, parent, legacy)| load_one(&f, &wid, parent, legacy))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        if meta.parent_session.is_some() {
            return None;
        }
        Some(format!("kimi -S {}", shell_quote(&meta.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(src: &[&str]) -> Vec<Value> {
        src.iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn code_fixture() {
        let l = lines(&[
            r#"{"type":"metadata","protocol_version":"1.1","created_at":1779256791085}"#,
            r#"{"type":"config.update","cwd":"/tmp/work","profileName":"agent","time":1779256791100}"#,
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"hi"}],"origin":{"kind":"user"},"time":1779256792000}"#,
            r#"{"type":"llm.request","model":"kimi-code/k3-256k","time":1779256792100}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c1","name":"Bash","input":{"command":"ls"}},"time":1779256793000}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"tool.result","toolCallId":"c1","output":"a.txt"},"time":1779256794000}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"step.end","turnId":"t1","usage":{"inputOther":10,"output":5,"inputCacheRead":0,"inputCacheCreation":0},"finishReason":"end_turn"},"time":1779256795000}"#,
            r#"{"type":"usage.record","model":"kimi-k2","usage":{"inputOther":10,"output":5,"inputCacheRead":3,"inputCacheCreation":0},"usageScope":"turn","time":1779256795001}"#,
            r#"{"type":"usage.record","model":"kimi-k2","usage":{"inputOther":999,"output":999},"usageScope":"session","time":1779256795002}"#,
        ]);
        let s = parse_code(&l, "sess-1", Path::new("/x/wire.jsonl"));
        assert_eq!(s.meta.project_path.as_deref(), Some("/tmp/work"));
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.meta.usage.input, 10);
        assert_eq!(s.meta.usage.cache_read, 3);
        assert_eq!(s.turns[1].model.as_deref(), Some("kimi-k2"));
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("a.txt"));
    }

    #[test]
    fn legacy_fixture() {
        let l = lines(&[
            r#"{"type": "metadata", "protocol_version": "1.3"}"#,
            r#"{"timestamp": 1770983400.0, "message": {"type": "TurnBegin", "payload": {"user_input": "hello"}}}"#,
            r#"{"timestamp": 1770983410.0, "message": {"type": "ContentPart", "payload": {"type": "text", "text": "Oi"}}}"#,
            r#"{"timestamp": 1770983415.0, "message": {"type": "ToolCall", "payload": {"type": "function", "id": "tool_1", "function": {"name": "ReadFile", "arguments": "{\"path\":\"a\"}"}}}}"#,
            r#"{"timestamp": 1770983416.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 20, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "m1"}}}"#,
            r#"{"timestamp": 1770983417.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 1508, "output": 205, "input_cache_read": 4864, "input_cache_creation": 0}, "message_id": "m1"}}}"#,
        ]);
        let s = parse_legacy(&l, "legacy-1", Path::new("/x/wire.jsonl"));
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.turns[0].ts, "2026-02-13T11:50:00.000Z");
        assert_eq!(s.meta.usage.input, 1508);
        assert_eq!(s.meta.usage.cache_read, 4864);
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Read");
    }
}
