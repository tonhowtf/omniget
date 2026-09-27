//! Augment Code / Auggie CLI (estudo 06, Augment §j; Parte 6 §8.2).
//!
//! `~/.augment/sessions/<sessionId>.json` (confirmado no bundle 0.36.0):
//! `{sessionId, created, modified, agentState{modelId}, chatHistory[]}`; cada
//! item tem `completed`, `finishedAt`, `sequenceId` e `exchange{request_message,
//! response_text, model_id, request_id, request_nodes[], response_nodes[]}`.
//! Nos `response_nodes`, `token_usage{input_tokens, output_tokens,
//! cache_read_input_tokens, cache_creation_input_tokens}` (vale o último não
//! zero do turno) e `tool_use{tool_use_id, tool_name, input_json}`; nos
//! `request_nodes`, `tool_result_node{tool_use_id, content, is_error}`.
//! Augment cobra em créditos e não grava custo: `cost_usd` fica `None` e o
//! preço sai da tabela. Auggie não está instalado aqui: só fixture.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct AuggieSource;

const TOOL: &str = "auggie";

fn root() -> Option<PathBuf> {
    env_dir("AUGMENT_HOME")
        .map(|h| h.join("sessions"))
        .or_else(|| home_join(&[".augment", "sessions"]))
}

fn files() -> Vec<PathBuf> {
    let Some(r) = root() else { return Vec::new() };
    find_files(&r, 1, |p| {
        p.extension().and_then(|e| e.to_str()) == Some("json")
    })
}

pub fn parse(doc: &Value, fallback_id: &str, source: &Path) -> Session {
    let id = s(doc, &["sessionId"]).unwrap_or_else(|| fallback_id.to_string());
    let mut meta = new_meta(TOOL, id, source);
    meta.started = doc.get("created").and_then(ts_str);
    meta.ended = doc.get("modified").and_then(ts_str);
    meta.title = s(doc, &["title", "name"]);
    meta.project_path = s(doc, &["workspaceRoot", "cwd", "rootPath"]).or_else(|| {
        doc.pointer("/agentState/workspaceRoot")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let default_model = doc
        .pointer("/agentState/modelId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for item in doc
        .get("chatHistory")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ex = item.get("exchange").cloned().unwrap_or(Value::Null);
        let ts = item
            .get("finishedAt")
            .or_else(|| ex.get("timestamp"))
            .and_then(ts_str)
            .unwrap_or_default();
        for rn in ex
            .get("request_nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(tr) = rn.get("tool_result_node") {
                results.push((
                    s(tr, &["tool_use_id"]).unwrap_or_default(),
                    tr.get("content").map(result_text).unwrap_or_default(),
                    tr.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                ));
            }
        }
        if let Some(msg) = s(&ex, &["request_message"]) {
            turns.push(turn(Role::User, ts.clone(), msg));
        }
        let mut t = turn(
            Role::Assistant,
            ts,
            s(&ex, &["response_text"]).unwrap_or_default(),
        );
        t.model = s(&ex, &["model_id"]).or_else(|| default_model.clone());
        t.message_id =
            s(&ex, &["request_id"]).or_else(|| item.get("sequenceId").map(|x| x.to_string()));
        let mut last_usage: Option<TokenUsage> = None;
        let mut texts = Vec::new();
        for n in ex
            .get("response_nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(tu) = n.get("token_usage") {
                let us = TokenUsage {
                    input: u(tu, &["input_tokens"]),
                    output: u(tu, &["output_tokens"]),
                    cache_read: u(tu, &["cache_read_input_tokens"]),
                    cache_write: u(tu, &["cache_creation_input_tokens"]),
                    reasoning: 0,
                };
                if us.total() > 0 {
                    last_usage = Some(us);
                }
            }
            if let Some(tu) = n.get("tool_use") {
                let name = s(tu, &["tool_name", "name"]).unwrap_or_else(|| "unknown".into());
                let input = tu
                    .get("input_json")
                    .and_then(Value::as_str)
                    .and_then(|x| serde_json::from_str(x).ok())
                    .or_else(|| tu.get("input").cloned())
                    .unwrap_or(Value::Null);
                t.tool_calls.push(tool_call(
                    s(tu, &["tool_use_id", "id"]).unwrap_or_default(),
                    &name,
                    input,
                ));
            }
            if t.text.is_empty() {
                if let Some(c) = n.get("content").and_then(Value::as_str) {
                    texts.push(c.to_string());
                }
            }
        }
        if t.text.is_empty() {
            t.text = texts.join("");
        }
        // Turno não concluído não tem uso confiável.
        if item
            .get("completed")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            t.usage = last_usage.unwrap_or_default();
        }
        turns.push(t);
    }
    attach_results(&mut turns, results);
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

impl SessionSource for AuggieSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        root().filter(|p| p.is_dir()).into_iter().collect()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = files()
            .iter()
            .filter_map(|f| read_json(f).map(|d| parse(&d, &file_stem(f), f).meta))
            .collect();
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        let f = files().into_iter().find(|f| file_stem(f) == id)?;
        let d = read_json(&f)?;
        Some(parse(&d, id, &f))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        Some(format!("auggie --resume {}", shell_quote(&meta.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auggie_fixture() {
        let d: Value = serde_json::from_str(r#"{"sessionId":"11111111-2222-3333-4444-555555555555","created":"2026-01-15T12:00:00.000Z","modified":"2026-01-15T12:10:00.000Z",
          "agentState":{"modelId":"claude-sonnet-4-5"},
          "chatHistory":[
            {"completed":true,"finishedAt":"2026-01-15T12:01:00.000Z","sequenceId":1,
             "exchange":{"request_message":"liste os testes","response_text":"Vou listar.","model_id":"","request_id":"req-1",
               "response_nodes":[{"type":5,"tool_use":{"tool_use_id":"tu1","tool_name":"launch-process","input_json":"{\"command\":\"ls tests\"}"}},
                                 {"type":10,"token_usage":{"input_tokens":1000,"output_tokens":50,"cache_read_input_tokens":200,"cache_creation_input_tokens":0}}]}},
            {"completed":true,"finishedAt":"2026-01-15T12:02:00.000Z","sequenceId":2,
             "exchange":{"request_nodes":[{"type":1,"tool_result_node":{"tool_use_id":"tu1","content":"a_test.py","is_error":false}}],"response_text":"Achei um.","request_id":"req-2",
               "response_nodes":[{"type":10,"token_usage":{"input_tokens":1300,"output_tokens":10}}]}}
          ]}"#).unwrap();
        let s = parse(&d, "x", Path::new("/x/s.json"));
        assert_eq!(s.turns.len(), 3);
        assert_eq!(s.turns[1].model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Bash");
        assert_eq!(
            s.turns[1].tool_calls[0].result.as_deref(),
            Some("a_test.py")
        );
        assert_eq!(s.meta.usage.input, 2300);
        assert_eq!(s.meta.cost_usd, 0.0);
    }
}
