//! Gemini CLI: `~/.gemini/tmp/<slug>/chats/session-*.jsonl` (antigo:
//! `.json` com `messages[]`); subagentes em `chats/<sessão-mãe>/<id>.jsonl`.
//! Sandbox do macOS: `~/.cache/.gemini/tmp`. `GEMINI_DATA_DIR` sobrepõe.
//!
//! Registro: metadados `{sessionId, projectHash, startTime, lastUpdated,
//! summary?, kind}`, atualizações `{"$set": {...}}`, `{"$rewindTo": id}`
//! (descarta a mensagem `id` e o que veio depois) e mensagens
//! `{id, timestamp, type: user|gemini|info|error|warning, content,
//! toolCalls[], thoughts[], model, tokens{input, output, cached, thoughts,
//! tool, total}}`. Uma mensagem com id repetido substitui a anterior.
//! `tokens.input` inclui o cache: sai separado aqui.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
};
use crate::core::sessions::util::{self, canonical_tool, truncate_chars, value_text, RESULT_MAX};

pub const TOOL: &str = "gemini";

pub struct GeminiSource {
    roots: Vec<PathBuf>,
}

impl Default for GeminiSource {
    fn default() -> Self {
        Self::new()
    }
}

impl GeminiSource {
    pub fn new() -> Self {
        let mut roots = Vec::new();
        for d in util::env_paths("GEMINI_DATA_DIR") {
            roots.push(if d.ends_with("tmp") { d } else { d.join("tmp") });
        }
        if let Some(h) = util::home() {
            roots.push(h.join(".gemini").join("tmp"));
            roots.push(h.join(".cache").join(".gemini").join("tmp"));
        }
        Self { roots }
    }

    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// (arquivo, sessão-mãe, diretório do slug)
    fn files(&self) -> Vec<(PathBuf, Option<String>, PathBuf)> {
        let mut out = Vec::new();
        for root in &self.roots {
            let Ok(slugs) = std::fs::read_dir(root) else {
                continue;
            };
            for slug in slugs.flatten() {
                let sdir = slug.path();
                let chats = sdir.join("chats");
                let Ok(entries) = std::fs::read_dir(&chats) else {
                    continue;
                };
                for e in entries.flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    if p.is_file() && (name.ends_with(".jsonl") || name.ends_with(".json")) {
                        out.push((p, None, sdir.clone()));
                    } else if p.is_dir() {
                        if let Ok(subs) = std::fs::read_dir(&p) {
                            for s in subs.flatten() {
                                let sp = s.path();
                                let sn = s.file_name().to_string_lossy().to_string();
                                if sp.is_file() && (sn.ends_with(".jsonl") || sn.ends_with(".json"))
                                {
                                    out.push((sp, Some(name.clone()), sdir.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

fn project_root(slug_dir: &Path) -> Option<String> {
    let t = std::fs::read_to_string(slug_dir.join(".project_root")).ok()?;
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn content_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| match p {
                Value::String(s) => Some(s.clone()),
                Value::Object(_) => p
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Object(_)) => v
            .and_then(|o| o.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

#[derive(Default)]
struct Doc {
    session_id: Option<String>,
    start: Option<i64>,
    updated: Option<i64>,
    summary: Option<String>,
    dirs: Vec<String>,
    kind: Option<String>,
    messages: Vec<Value>,
}

impl Doc {
    fn apply_meta(&mut self, v: &Value) {
        if let Some(s) = util::str_of(v, "sessionId") {
            self.session_id = Some(s.to_string());
        }
        if let Some(t) = util::ts_ms_of(v.get("startTime")) {
            self.start = Some(t);
        }
        if let Some(t) = util::ts_ms_of(v.get("lastUpdated")) {
            self.updated = Some(t);
        }
        if let Some(s) = util::str_of(v, "summary") {
            self.summary = Some(s.to_string());
        }
        if let Some(k) = util::str_of(v, "kind") {
            self.kind = Some(k.to_string());
        }
        if let Some(Value::Array(d)) = v.get("directories") {
            self.dirs = d
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
    }

    fn push_message(&mut self, m: Value) {
        let id = util::str_of(&m, "id").map(|s| s.to_string());
        if let Some(id) = &id {
            if let Some(pos) = self
                .messages
                .iter()
                .position(|x| util::str_of(x, "id") == Some(id))
            {
                self.messages[pos] = m;
                return;
            }
        }
        self.messages.push(m);
    }

    fn feed(&mut self, v: Value) {
        if let Some(set) = v.get("$set") {
            self.apply_meta(set);
            if let Some(Value::Array(ms)) = set.get("messages") {
                self.messages.clear();
                for m in ms {
                    self.push_message(m.clone());
                }
            }
            return;
        }
        if let Some(to) = v.get("$rewindTo").and_then(|x| x.as_str()) {
            if let Some(pos) = self
                .messages
                .iter()
                .position(|x| util::str_of(x, "id") == Some(to))
            {
                self.messages.truncate(pos);
            }
            return;
        }
        if v.get("type").and_then(|t| t.as_str()).is_some() && v.get("id").is_some() {
            self.push_message(v);
            return;
        }
        if v.get("sessionId").is_some() || v.get("startTime").is_some() {
            self.apply_meta(&v);
            if let Some(Value::Array(ms)) = v.get("messages") {
                for m in ms {
                    self.push_message(m.clone());
                }
            }
        }
    }
}

fn read_doc(path: &Path) -> Doc {
    let mut doc = Doc::default();
    let is_json = path.extension().map(|e| e == "json").unwrap_or(false);
    if is_json {
        if let Some(bytes) = util::read_maybe_zst(path) {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                match v {
                    Value::Array(ms) => {
                        for m in ms {
                            doc.push_message(m);
                        }
                    }
                    other => doc.feed(other),
                }
            }
        }
    } else {
        let _ = util::for_each_line(path, 0, |l| {
            if let Ok(v) = serde_json::from_slice::<Value>(l) {
                doc.feed(v);
            }
        });
    }
    doc
}

fn usage_of(t: &Value) -> TokenUsage {
    let input = util::num(t, "input");
    let cached = util::num(t, "cached").min(input);
    TokenUsage {
        input: input - cached + util::num(t, "tool"),
        output: util::num(t, "output"),
        cache_read: cached,
        cache_write: 0,
        reasoning: util::num(t, "thoughts"),
    }
}

fn to_session(path: &Path, parent: Option<String>, slug_dir: &Path, doc: Doc) -> Session {
    let id = doc.session_id.clone().unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let mut turns = Vec::new();
    let mut first_user: Option<String> = None;
    for m in &doc.messages {
        let ts = util::ts_ms_of(m.get("timestamp"))
            .map(util::ms_to_rfc3339)
            .unwrap_or_default();
        let kind = util::str_of(m, "type").unwrap_or("");
        let text = content_text(m.get("displayContent").or(m.get("content")));
        let role = match kind {
            "user" => Role::User,
            "gemini" | "model" | "assistant" => Role::Assistant,
            _ => Role::System,
        };
        if role == Role::User && first_user.is_none() && !text.trim().is_empty() {
            first_user = Some(text.clone());
        }
        let mut calls = Vec::new();
        if let Some(Value::Array(tcs)) = m.get("toolCalls") {
            for c in tcs {
                let name = util::str_of(c, "name").unwrap_or("?").to_string();
                let input = c.get("args").cloned().unwrap_or(Value::Null);
                let status = match util::str_of(c, "status") {
                    Some("success") | Some("completed") | Some("ok") => ToolStatus::Ok,
                    Some("error") | Some("cancelled") | Some("failed") => ToolStatus::Error,
                    _ => {
                        if c.get("result").is_some() {
                            ToolStatus::Ok
                        } else {
                            ToolStatus::Pending
                        }
                    }
                };
                let result = c.get("result").map(|r| {
                    // `result` é uma lista de `Part` com `functionResponse`.
                    let inner = r
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|p| p.get("functionResponse"))
                        .and_then(|f| f.get("response"))
                        .cloned()
                        .unwrap_or_else(|| r.clone());
                    let v = inner.get("output").cloned().unwrap_or(inner);
                    value_text(&v, RESULT_MAX)
                });
                let ms = match (util::ts_ms_of(c.get("timestamp")), util::parse_ts_str(&ts)) {
                    (Some(a), Some(b)) if a >= b => Some((a - b) as u64),
                    _ => None,
                };
                let canon = canonical_tool(&name);
                calls.push(ToolCall {
                    id: util::str_of(c, "id").unwrap_or("").to_string(),
                    name_canonical: canon.to_string(),
                    subagent: if canon == "Agent" || c.get("agentId").is_some() {
                        util::str_of(c, "agentId")
                            .map(|s| s.to_string())
                            .or(Some(name.clone()))
                    } else {
                        None
                    },
                    name_raw: name,
                    input,
                    result,
                    status,
                    ms,
                });
            }
        }
        if text.trim().is_empty() && calls.is_empty() && m.get("tokens").is_none() {
            continue;
        }
        turns.push(Turn {
            role,
            ts,
            text,
            tool_calls: calls,
            usage: m.get("tokens").map(usage_of).unwrap_or_default(),
            cost_usd: None,
            model: util::str_of(m, "model").map(|s| s.to_string()),
            message_id: util::str_of(m, "id").map(|s| s.to_string()),
        });
    }
    let mut usage = TokenUsage::default();
    let mut models = Vec::new();
    for t in &turns {
        usage.add(&t.usage);
        if let Some(m) = &t.model {
            if !models.contains(m) {
                models.push(m.clone());
            }
        }
    }
    let first = turns
        .iter()
        .filter_map(|t| util::parse_ts_str(&t.ts))
        .min()
        .or(doc.start);
    let last = turns
        .iter()
        .filter_map(|t| util::parse_ts_str(&t.ts))
        .max()
        .or(doc.updated);
    let project = project_root(slug_dir).or_else(|| doc.dirs.first().cloned());
    Session {
        meta: SessionMeta {
            tool: TOOL.into(),
            account: None,
            id,
            title: doc.summary.clone().or_else(|| {
                first_user.map(|t| truncate_chars(t.lines().next().unwrap_or(&t).trim(), 80))
            }),
            project_path: project,
            git_branch: None,
            started: first.map(util::ms_to_rfc3339),
            ended: last.map(util::ms_to_rfc3339),
            models,
            parent_session: parent,
            turn_count: turns.len() as u32,
            usage,
            cost_usd: 0.0,
            source: path.to_path_buf(),
            mtime_ms: util::file_mtime_ms(path),
        },
        turns,
    }
}

impl SessionSource for GeminiSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    fn list(&self) -> Vec<SessionMeta> {
        self.files()
            .into_iter()
            .map(|(p, parent, slug)| {
                let doc = read_doc(&p);
                to_session(&p, parent, &slug, doc).meta
            })
            .collect()
    }

    fn load(&self, id: &str) -> Option<Session> {
        for (p, parent, slug) in self.files() {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let short = id.get(..8).unwrap_or(id);
            if stem == id || stem.ends_with(short) || stem.contains(id) {
                let s = to_session(&p, parent.clone(), &slug, read_doc(&p));
                if s.meta.id == id {
                    return Some(s);
                }
            }
        }
        // Nome do arquivo não bate (formato antigo): procura pelo conteúdo.
        self.files()
            .into_iter()
            .map(|(p, parent, slug)| to_session(&p, parent, &slug, read_doc(&p)))
            .find(|s| s.meta.id == id)
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        if meta.parent_session.is_some() {
            return None;
        }
        Some(util::in_dir(
            meta.project_path.as_deref(),
            format!("gemini --resume {}", meta.id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_with_set_rewind_and_replacement() {
        let dir = std::env::temp_dir().join(format!("omniget-gemini-{}", std::process::id()));
        let chats = dir.join("myrepo").join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        std::fs::write(dir.join("myrepo").join(".project_root"), "/w/myrepo\n").unwrap();
        let f = chats.join("session-2026-09-20T10-00-abcdef12.jsonl");
        let lines = [
            r#"{"sessionId":"abcdef12-0000-0000-0000-000000000000","projectHash":"h","startTime":"2026-09-20T10:00:00.000Z","lastUpdated":"2026-09-20T10:00:00.000Z","kind":"main"}"#,
            r#"{"id":"m1","timestamp":"2026-09-20T10:00:01.000Z","type":"user","content":[{"text":"oi"}]}"#,
            r#"{"id":"m2","timestamp":"2026-09-20T10:00:02.000Z","type":"gemini","content":"ola","model":"gemini-3-pro","tokens":{"input":1000,"output":20,"cached":600,"thoughts":5,"tool":0,"total":1025},"toolCalls":[{"id":"t1","name":"run_shell_command","args":{"command":"ls"},"status":"success","result":[{"functionResponse":{"id":"t1","name":"run_shell_command","response":{"output":"a.txt"}}}],"timestamp":"2026-09-20T10:00:03.000Z"}]}"#,
            r#"{"id":"m3","timestamp":"2026-09-20T10:00:04.000Z","type":"user","content":"descartada"}"#,
            r#"{"$rewindTo":"m3"}"#,
            r#"{"$set":{"summary":"Listar arquivos","lastUpdated":"2026-09-20T10:05:00.000Z"}}"#,
        ];
        std::fs::write(&f, lines.join("\n") + "\n").unwrap();
        let src = GeminiSource::with_roots(vec![dir.clone()]);
        let metas = src.list();
        assert_eq!(metas.len(), 1);
        let s = src.load("abcdef12-0000-0000-0000-000000000000").unwrap();
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.meta.title.as_deref(), Some("Listar arquivos"));
        assert_eq!(s.meta.project_path.as_deref(), Some("/w/myrepo"));
        assert_eq!(s.meta.usage.input, 400);
        assert_eq!(s.meta.usage.cache_read, 600);
        assert_eq!(s.meta.usage.reasoning, 5);
        let c = &s.turns[1].tool_calls[0];
        assert_eq!(c.name_canonical, "Bash");
        assert_eq!(c.result.as_deref(), Some("a.txt"));
        assert_eq!(c.ms, Some(1000));
        std::fs::remove_dir_all(&dir).ok();
    }
}
