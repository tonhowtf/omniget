//! Amp (estudo 06, Amp §j; Parte 6 §8.1).
//!
//! Cópia local das threads: `${AMP_DATA_DIR:-$XDG_DATA_HOME/amp}/threads/T-*.json`
//! (`~/.local/share/amp/threads` em todos os SOs). Um JSON por thread: `id`
//! (`T-<uuid>`), `created` (ms), `title`, `env.initial.trees[].uri` (pasta),
//! `messages[]` com `role`, `messageId`, `content[]` (`text`, `thinking`,
//! `tool_use{id, name, input}`, `tool_result{toolUseID, run{status, result}}`),
//! `usage{model, inputTokens, outputTokens, cacheReadInputTokens,
//! cacheCreationInputTokens, credits}` e `meta.sentAt`; e `usageLedger.events[]`
//! (`timestamp, model, credits, tokens{input, output, cacheReadInputTokens,
//! cacheCreationInputTokens}, toMessageId`). O ledger completa o uso das
//! mensagens que não trazem `usage`. Amp cobra em créditos denominados em
//! dólar; o valor vai para `cost_usd`. Amp não está instalado aqui: só fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct AmpSource;

const TOOL: &str = "amp";

fn roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = env_dir("AMP_DATA_DIR") {
        v.push(d.join("threads"));
        v.push(d);
    }
    for d in xdg_data_homes() {
        v.push(d.join("amp").join("threads"));
    }
    dedup_paths(v).into_iter().filter(|p| p.is_dir()).collect()
}

fn files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for r in roots() {
        out.extend(find_files(&r, 1, |p| {
            let n = file_name(p);
            n.starts_with("T-") && n.ends_with(".json")
        }));
    }
    out
}

fn amp_usage(v: &Value) -> TokenUsage {
    TokenUsage {
        input: u(v, &["inputTokens", "input"]),
        output: u(v, &["outputTokens", "output"]),
        cache_read: u(v, &["cacheReadInputTokens", "cacheRead"]),
        cache_write: u(v, &["cacheCreationInputTokens", "cacheWrite"]),
        reasoning: 0,
    }
}

pub fn parse(doc: &Value, fallback_id: &str, source: &Path) -> Session {
    let id = s(doc, &["id"]).unwrap_or_else(|| fallback_id.to_string());
    let mut meta = new_meta(TOOL, id, source);
    meta.title = s(doc, &["title"]);
    let created = doc.get("created").and_then(ts_ms);
    meta.started = created.map(ms_to_rfc3339);
    meta.project_path = doc
        .pointer("/env/initial/trees/0/uri")
        .and_then(Value::as_str)
        .map(uri_to_path);
    meta.parent_session = doc
        .get("relationships")
        .and_then(Value::as_array)
        .and_then(|r| {
            r.iter()
                .find(|x| s(x, &["role"]).as_deref() == Some("child"))
        })
        .and_then(|x| s(x, &["threadID"]));

    // Ledger por mensagem de destino.
    let mut ledger: HashMap<i64, (TokenUsage, Option<String>, Option<f64>, Option<String>)> =
        HashMap::new();
    for ev in doc
        .pointer("/usageLedger/events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let to = ev.get("toMessageId").and_then(Value::as_i64).unwrap_or(-1);
        let us = ev.get("tokens").map(amp_usage).unwrap_or_default();
        let e = ledger
            .entry(to)
            .or_insert((TokenUsage::default(), None, None, None));
        e.0.add(&us);
        e.1 = s(ev, &["model"]).or(e.1.take());
        e.2 = Some(e.2.unwrap_or(0.0) + fnum(ev, &["credits"]).unwrap_or(0.0));
        e.3 = ev.get("timestamp").and_then(ts_str).or(e.3.take());
    }

    let base = created.unwrap_or(meta.mtime_ms);
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for m in doc
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let role = role_of(m.get("role").and_then(Value::as_str).unwrap_or(""));
        let mid = m.get("messageId").and_then(Value::as_i64);
        let ts = m
            .pointer("/meta/sentAt")
            .and_then(ts_str)
            .unwrap_or_else(|| ms_to_rfc3339(base + mid.unwrap_or(0).max(0) * 1000));
        let mut text = Vec::new();
        let mut calls = Vec::new();
        for b in m
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match b.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => {
                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                        text.push(t.to_string());
                    }
                }
                "tool_use" => {
                    let name = s(b, &["name"]).unwrap_or_else(|| "unknown".into());
                    calls.push(tool_call(
                        s(b, &["id"]).unwrap_or_default(),
                        &name,
                        b.get("input").cloned().unwrap_or(Value::Null),
                    ));
                }
                "tool_result" => {
                    let run = b.get("run").cloned().unwrap_or(Value::Null);
                    let status = s(&run, &["status"]).unwrap_or_default();
                    let body = run
                        .get("result")
                        .or_else(|| b.get("content"))
                        .map(result_text)
                        .or_else(|| run.get("error").map(result_text))
                        .unwrap_or_default();
                    results.push((
                        s(b, &["toolUseID", "tool_use_id", "toolUseId"]).unwrap_or_default(),
                        body,
                        matches!(status.as_str(), "error" | "cancelled" | "rejected-by-user"),
                    ));
                }
                _ => {}
            }
        }
        let text = text.join("\n");
        if role == Role::User && text.trim().is_empty() && calls.is_empty() {
            continue;
        }
        let mut t = turn(role, ts, text);
        t.tool_calls = calls;
        t.message_id = mid.map(|x| x.to_string());
        if role == Role::Assistant {
            let us = m.get("usage").cloned().unwrap_or(Value::Null);
            t.model = s(&us, &["model"]);
            t.usage = amp_usage(&us);
            t.cost_usd = fnum(&us, &["credits"]);
            if let Some((lu, lm, lc, _)) = mid.and_then(|x| ledger.remove(&x)) {
                if t.usage.total() == 0 {
                    t.usage = lu;
                }
                if t.model.is_none() {
                    t.model = lm;
                }
                if t.cost_usd.unwrap_or(0.0) == 0.0 {
                    t.cost_usd = lc;
                }
            }
        }
        turns.push(t);
    }
    // Eventos do ledger sem mensagem correspondente viram turnos só de uso.
    let mut rest: Vec<_> = ledger.into_iter().collect();
    rest.sort_by_key(|(k, _)| *k);
    for (_, (us, model, cost, ts)) in rest {
        if us.total() == 0 && cost.unwrap_or(0.0) == 0.0 {
            continue;
        }
        let mut t = turn(Role::Assistant, ts.unwrap_or_default(), "");
        t.usage = us;
        t.model = model;
        t.cost_usd = cost;
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

impl SessionSource for AmpSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        roots()
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
        Some(format!("amp threads continue {}", shell_quote(&meta.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amp_fixture() {
        let d: Value = serde_json::from_str(r#"{
          "v": 12, "id": "T-1234", "created": 1775304000000, "title": "Corrigir CI",
          "env": {"initial": {"trees": [{"displayName": "app", "uri": "file:///home/u/app"}]}},
          "messages": [
            {"role": "user", "messageId": 0, "content": [{"type": "text", "text": "por que o CI falha?"}]},
            {"role": "assistant", "messageId": 1, "content": [{"type": "text", "text": "Vendo."}, {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"cmd": "npm test"}}],
             "usage": {"model": "claude-sonnet-4-0", "inputTokens": 100, "outputTokens": 20, "cacheReadInputTokens": 7, "credits": 0.75}},
            {"role": "user", "messageId": 2, "content": [{"type": "tool_result", "toolUseID": "toolu_1", "run": {"status": "done", "result": {"output": "1 failed", "exitCode": 1}}}]},
            {"role": "assistant", "messageId": 3, "content": [{"type": "text", "text": "Falta variável."}]}
          ],
          "usageLedger": {"events": [
            {"timestamp": "2026-04-08T12:00:00Z", "model": "claude-sonnet-4-0", "credits": 0.75, "tokens": {"input": 100, "output": 20}, "toMessageId": 1},
            {"timestamp": "2026-04-08T12:01:00Z", "model": "claude-sonnet-4-0", "credits": 0.25, "tokens": {"input": 50, "output": 5}, "toMessageId": 3}
          ]}
        }"#).unwrap();
        let s = parse(&d, "T-1234", Path::new("/x/T-1234.json"));
        assert_eq!(s.meta.project_path.as_deref(), Some("/home/u/app"));
        assert_eq!(s.turns.len(), 3);
        assert_eq!(s.turns[1].usage.cache_read, 7);
        assert_eq!(s.turns[2].usage.input, 50);
        assert!((s.meta.cost_usd - 1.0).abs() < 1e-9);
        let c = &s.turns[1].tool_calls[0];
        assert_eq!(c.name_canonical, "Bash");
        assert!(c.result.as_deref().unwrap().contains("1 failed"));
    }
}
