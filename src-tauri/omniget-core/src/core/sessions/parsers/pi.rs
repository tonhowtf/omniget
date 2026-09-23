//! Pi coding agent (estudo 06, Pi §j; Parte 6 §8.1).
//!
//! `${PI_AGENT_DIR:-~/.pi/agent}/sessions/--<cwd com / \ : trocados por ->--/
//! <timestamp>_<uuid>.jsonl`, JSONL em árvore (v3): linhas `title` opcionais,
//! cabeçalho `{type:"session", version, id, timestamp, cwd, parentSession?}`,
//! depois entradas com `id`, `parentId`, `timestamp`: `message` (`message.role`
//! = `user|assistant|toolResult|…`; assistente com `model`, `provider`,
//! `content[]` com `text|thinking|toolCall{id, name, arguments}` e
//! `usage{input, output, cacheRead, cacheWrite, reasoning?, totalTokens,
//! cost{total}}`; `toolResult` com `toolCallId`, `content`, `isError`),
//! `model_change`, `session_info{name}`, `compaction` etc.
//! O ramo ativo é o caminho da última entrada até a raiz pelos `parentId`.
//! O custo em dólar vem gravado por mensagem (`usage.cost.total`).
//! `reasoning` já está dentro de `output`; aqui ele é separado para o total
//! não contar duas vezes. Pi não está instalado aqui: só fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct PiSource;

const TOOL: &str = "pi";

fn roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = env_dir("PI_AGENT_DIR") {
        v.push(d.join("sessions"));
        v.push(d);
    }
    if let Some(d) = home_join(&[".pi", "agent", "sessions"]) {
        v.push(d);
    }
    dedup_paths(v).into_iter().filter(|p| p.is_dir()).collect()
}

fn files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for r in roots() {
        out.extend(find_files(&r, 3, |p| {
            p.extension().and_then(|e| e.to_str()) == Some("jsonl")
        }));
    }
    out.sort();
    out.dedup();
    out
}

/// Linhas do ramo ativo, na ordem.
fn active_branch(entries: &[Value]) -> Vec<&Value> {
    let with_ids = entries.iter().filter(|e| e.get("id").is_some()).count();
    let has_parents = entries.iter().any(|e| e.get("parentId").is_some());
    if with_ids == 0 || !has_parents {
        return entries.iter().collect();
    }
    let by_id: HashMap<String, &Value> = entries
        .iter()
        .filter_map(|e| s(e, &["id"]).map(|id| (id, e)))
        .collect();
    let Some(mut cur) = entries.iter().rev().find(|e| e.get("id").is_some()) else {
        return entries.iter().collect();
    };
    let mut chain = vec![cur];
    let mut guard = 0;
    while let Some(pid) = s(cur, &["parentId"]) {
        guard += 1;
        if guard > entries.len() {
            break;
        }
        match by_id.get(&pid) {
            Some(p) => {
                chain.push(p);
                cur = p;
            }
            None => break,
        }
    }
    chain.reverse();
    chain
}

pub fn parse(lines: &[Value], fallback_id: &str, source: &Path) -> Session {
    let mut meta = new_meta(TOOL, fallback_id, source);
    let mut entries = Vec::new();
    for l in lines {
        match l.get("type").and_then(Value::as_str).unwrap_or("") {
            "title" => {
                meta.title = s(l, &["title"]);
            }
            "session" => {
                if let Some(id) = s(l, &["id"]) {
                    meta.id = id;
                }
                meta.project_path = s(l, &["cwd"]);
                meta.started = l.get("timestamp").and_then(ts_str);
                meta.parent_session = s(l, &["parentSession"]).map(|p| {
                    // Pode ser caminho de arquivo: fica só o id.
                    let st = Path::new(&p)
                        .file_stem()
                        .and_then(|x| x.to_str())
                        .unwrap_or(&p)
                        .to_string();
                    st.rsplit('_').next().unwrap_or(&st).to_string()
                });
            }
            "session_info" => {
                if meta.title.is_none() {
                    meta.title = s(l, &["name"]);
                }
            }
            _ => entries.push(l.clone()),
        }
    }
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for e in active_branch(&entries) {
        if e.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(m) = e.get("message") else { continue };
        let role_s = m.get("role").and_then(Value::as_str).unwrap_or("");
        let ts = e
            .get("timestamp")
            .or_else(|| m.get("timestamp"))
            .and_then(ts_str)
            .unwrap_or_default();
        if role_s == "toolResult" {
            results.push((
                s(m, &["toolCallId"]).unwrap_or_default(),
                m.get("content").map(result_text).unwrap_or_default(),
                m.get("isError").and_then(Value::as_bool).unwrap_or(false),
            ));
            continue;
        }
        let role = role_of(role_s);
        if !matches!(role, Role::User | Role::Assistant) {
            continue;
        }
        let content = m.get("content").cloned().unwrap_or(Value::Null);
        let (text, mut calls, res) = anthropic_blocks(&content);
        results.extend(res);
        // Pi usa `toolCall` com `arguments`; anthropic_blocks já cobre, mas
        // garante o formato caso venha só `name` + `arguments` string.
        for c in calls.iter_mut() {
            if let Value::String(st) = &c.input {
                if let Ok(v) = serde_json::from_str(st) {
                    c.input = v;
                }
            }
        }
        let mut t = turn(role, ts, text);
        t.tool_calls = calls;
        t.message_id = s(e, &["id"]).or_else(|| s(m, &["responseId"]));
        if role == Role::Assistant {
            t.model = s(m, &["model"]);
            if let Some(us) = m.get("usage") {
                let reasoning = u(us, &["reasoning"]);
                t.usage = TokenUsage {
                    input: u(us, &["input"]),
                    output: u(us, &["output"]).saturating_sub(reasoning),
                    cache_read: u(us, &["cacheRead"]),
                    cache_write: u(us, &["cacheWrite"]) + u(us, &["cacheWrite1h"]),
                    reasoning,
                };
                t.cost_usd = us.get("cost").and_then(|c| fnum(c, &["total"]));
            }
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

fn load_file(f: &Path) -> Session {
    parse(&read_jsonl(f), &file_stem(f), f)
}

/// Id do cabeçalho sem ler o arquivo inteiro.
fn header_id(f: &Path) -> Option<String> {
    let file = std::fs::File::open(f).ok()?;
    use std::io::BufRead;
    for line in std::io::BufReader::new(file).lines().take(20) {
        let v: Value = serde_json::from_str(&line.ok()?).ok()?;
        if v.get("type").and_then(Value::as_str) == Some("session") {
            return s(&v, &["id"]);
        }
    }
    None
}

impl SessionSource for PiSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        roots()
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = files().iter().map(|f| load_file(f).meta).collect();
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        files()
            .into_iter()
            .find(|f| header_id(f).as_deref() == Some(id) || file_stem(f) == id)
            .map(|f| load_file(&f))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        Some(format!(
            "pi --session {}",
            shell_quote(&meta.source.to_string_lossy())
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_fixture() {
        let l: Vec<Value> = [
            r#"{"type":"title","v":1,"title":"Comentar issue","source":"auto"}"#,
            r#"{"type":"session","version":3,"id":"pi_ses_001","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}"#,
            r#"{"type":"message","id":"a1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","content":"leia a"}}"#,
            r#"{"type":"message","id":"a2","parentId":"a1","timestamp":"2026-01-01T00:00:02.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","provider":"anthropic","content":[{"type":"toolCall","id":"c1","name":"read","arguments":{"path":"a"}}],"usage":{"input":100,"output":50,"cacheRead":10,"cacheWrite":5,"reasoning":8,"totalTokens":165,"cost":{"input":0.0003,"output":0.00075,"total":0.0011}}}}"#,
            r#"{"type":"message","id":"a3","parentId":"a2","timestamp":"2026-01-01T00:00:03.000Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"read","content":[{"type":"text","text":"conteúdo"}],"isError":false}}"#,
            r#"{"type":"message","id":"b2","parentId":"a1","timestamp":"2026-01-01T00:00:04.000Z","message":{"role":"assistant","model":"x","content":[{"type":"text","text":"ramo abandonado"}]}}"#,
            r#"{"type":"message","id":"a4","parentId":"a3","timestamp":"2026-01-01T00:00:05.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","content":[{"type":"text","text":"feito"}]}}"#,
        ]
        .iter()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
        let s = parse(&l, "f", Path::new("/x/f.jsonl"));
        assert_eq!(s.meta.id, "pi_ses_001");
        assert_eq!(s.meta.title.as_deref(), Some("Comentar issue"));
        assert_eq!(s.turns.len(), 3);
        assert!(s.turns.iter().all(|t| t.text != "ramo abandonado"));
        assert_eq!(s.turns[1].usage.output, 42);
        assert_eq!(s.turns[1].usage.reasoning, 8);
        assert_eq!(s.turns[1].cost_usd, Some(0.0011));
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Read");
        assert_eq!(s.turns[1].tool_calls[0].result.as_deref(), Some("conteúdo"));
    }
}
