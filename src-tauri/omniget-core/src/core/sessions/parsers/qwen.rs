//! Qwen Code: `~/.qwen/projects/<cwd sanitizado>/chats/<sessionId>.jsonl`
//! (`QWEN_RUNTIME_DIR`, `QWEN_HOME` ou `QWEN_DATA_DIR` sobrepõem a base).
//!
//! Registro parecido com o do Claude (`uuid`, `parentUuid`, `sessionId`,
//! `timestamp`, `cwd`, `gitBranch`, `type: user|assistant|tool_result|system`,
//! `subtype`), mas `message` é um `Content` do Gemini (`role`, `parts[]` com
//! `text`, `functionCall{id,name,args}`, `functionResponse{id,name,response}`)
//! e o uso vem em `usageMetadata` (`promptTokenCount` inclui o cache).
//! Subagentes gravam no mesmo arquivo com `agentId`/`isSidechain`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolCall, ToolStatus, Turn,
};
use crate::core::sessions::util::{self, canonical_tool, truncate_chars, value_text, RESULT_MAX};

pub const TOOL: &str = "qwen";

pub struct QwenSource {
    roots: Vec<PathBuf>,
}

impl Default for QwenSource {
    fn default() -> Self {
        Self::new()
    }
}

impl QwenSource {
    pub fn new() -> Self {
        let mut roots = Vec::new();
        for var in ["QWEN_RUNTIME_DIR", "QWEN_HOME", "QWEN_DATA_DIR"] {
            for d in util::env_paths(var) {
                roots.push(d.join("projects"));
            }
        }
        if let Some(h) = util::home() {
            roots.push(h.join(".qwen").join("projects"));
        }
        roots.dedup();
        Self { roots }
    }

    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for root in &self.roots {
            let Ok(projects) = std::fs::read_dir(root) else {
                continue;
            };
            for p in projects.flatten() {
                let Ok(chats) = std::fs::read_dir(p.path().join("chats")) else {
                    continue;
                };
                for c in chats.flatten() {
                    let cp = c.path();
                    if cp.is_file() && cp.extension().map(|e| e == "jsonl").unwrap_or(false) {
                        out.push(cp);
                    }
                }
            }
        }
        out
    }
}

#[derive(Default)]
struct Parser {
    turns: Vec<Turn>,
    calls: HashMap<String, (usize, usize)>,
    cwd: Option<String>,
    branch: Option<String>,
    session_id: Option<String>,
    first_user: Option<String>,
    title: Option<String>,
}

fn usage_of(u: &Value) -> TokenUsage {
    let prompt = util::num(u, "promptTokenCount");
    let cached = util::num(u, "cachedContentTokenCount").min(prompt);
    TokenUsage {
        input: prompt - cached,
        output: util::num(u, "candidatesTokenCount"),
        cache_read: cached,
        cache_write: 0,
        reasoning: util::num(u, "thoughtsTokenCount"),
    }
}

impl Parser {
    fn feed(&mut self, raw: &[u8]) {
        let Ok(v) = serde_json::from_slice::<Value>(raw) else {
            return;
        };
        if self.cwd.is_none() {
            self.cwd = util::str_of(&v, "cwd").map(|s| s.to_string());
        }
        if let Some(b) = util::str_of(&v, "gitBranch") {
            self.branch = Some(b.to_string());
        }
        if self.session_id.is_none() {
            self.session_id = util::str_of(&v, "sessionId").map(|s| s.to_string());
        }
        let ts = util::ts_ms_of(v.get("timestamp"))
            .map(util::ms_to_rfc3339)
            .unwrap_or_default();
        let ts_ms = util::parse_ts_str(&ts);
        let kind = util::str_of(&v, "type").unwrap_or("");
        let subtype = util::str_of(&v, "subtype").unwrap_or("");
        let parts: Vec<Value> = v
            .get("message")
            .and_then(|m| m.get("parts"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        let mut text = Vec::new();
        let mut calls = Vec::new();
        let mut responses = Vec::new();
        for p in &parts {
            if p.get("thought").and_then(|t| t.as_bool()) == Some(true) {
                continue;
            }
            if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                text.push(t.to_string());
            }
            if let Some(fc) = p.get("functionCall") {
                calls.push(fc.clone());
            }
            if let Some(fr) = p.get("functionResponse") {
                responses.push(fr.clone());
            }
        }
        // Resultado de tool: por `functionResponse` e por `toolCallResult`.
        for fr in &responses {
            let id = util::str_of(fr, "id").unwrap_or("").to_string();
            let resp = fr.get("response").cloned().unwrap_or(Value::Null);
            let is_err = resp.get("error").is_some();
            let out = resp.get("output").cloned().unwrap_or(resp);
            self.attach(&id, value_text(&out, RESULT_MAX), is_err, ts_ms);
        }
        if let Some(tcr) = v.get("toolCallResult") {
            let id = util::str_of(tcr, "callId").unwrap_or("").to_string();
            let is_err = tcr.get("error").map(|e| !e.is_null()).unwrap_or(false)
                || util::str_of(tcr, "status") == Some("error");
            if responses.is_empty() {
                let disp = tcr.get("resultDisplay").cloned().unwrap_or(Value::Null);
                self.attach(&id, value_text(&disp, RESULT_MAX), is_err, ts_ms);
            } else if is_err {
                self.attach_status(&id, ToolStatus::Error);
            }
        }
        match kind {
            "user" => {
                let t = text.join("\n");
                if t.trim().is_empty() {
                    return;
                }
                if self.first_user.is_none() && !t.starts_with('<') {
                    self.first_user = Some(t.clone());
                }
                self.turns.push(Turn {
                    role: Role::User,
                    ts,
                    text: t,
                    tool_calls: Vec::new(),
                    usage: TokenUsage::default(),
                    cost_usd: None,
                    model: None,
                    message_id: util::str_of(&v, "uuid").map(|s| s.to_string()),
                });
            }
            "assistant" => {
                let mut tcs = Vec::new();
                for fc in calls {
                    let name = util::str_of(&fc, "name").unwrap_or("?").to_string();
                    let input = fc.get("args").cloned().unwrap_or(Value::Null);
                    let canon = canonical_tool(&name);
                    tcs.push(ToolCall {
                        id: util::str_of(&fc, "id").unwrap_or("").to_string(),
                        name_canonical: canon.to_string(),
                        subagent: (canon == "Agent").then(|| {
                            util::str_of(&input, "subagent_type")
                                .or_else(|| util::str_of(&input, "agent"))
                                .or_else(|| util::str_of(&input, "name"))
                                .unwrap_or("agent")
                                .to_string()
                        }),
                        name_raw: name,
                        input,
                        result: None,
                        status: ToolStatus::Pending,
                        ms: None,
                    });
                }
                let t = text.join("\n");
                let usage = v.get("usageMetadata").map(usage_of).unwrap_or_default();
                if t.trim().is_empty() && tcs.is_empty() && usage.total() == 0 {
                    return;
                }
                self.turns.push(Turn {
                    role: Role::Assistant,
                    ts,
                    text: t,
                    tool_calls: tcs,
                    usage,
                    cost_usd: None,
                    model: util::str_of(&v, "model").map(|s| s.to_string()),
                    message_id: util::str_of(&v, "uuid").map(|s| s.to_string()),
                });
                let idx = self.turns.len() - 1;
                for (ci, c) in self.turns[idx].tool_calls.iter().enumerate() {
                    if !c.id.is_empty() {
                        self.calls.insert(c.id.clone(), (idx, ci));
                    }
                }
            }
            "system" => {
                if subtype == "slash_command" {
                    let t = text.join("\n");
                    let cmd = v
                        .get("systemPayload")
                        .and_then(|p| {
                            util::str_of(p, "rawCommand").or_else(|| util::str_of(p, "command"))
                        })
                        .map(|s| s.to_string())
                        .unwrap_or(t);
                    if !cmd.is_empty() {
                        let cmd = if cmd.starts_with('/') {
                            cmd
                        } else {
                            format!("/{cmd}")
                        };
                        self.turns.push(Turn {
                            role: Role::System,
                            ts,
                            text: format!(
                                "<command-name>{}</command-name>",
                                cmd.split_whitespace().next().unwrap_or(&cmd)
                            ),
                            tool_calls: Vec::new(),
                            usage: TokenUsage::default(),
                            cost_usd: None,
                            model: None,
                            message_id: util::str_of(&v, "uuid").map(|s| s.to_string()),
                        });
                    }
                } else if subtype == "chat_compression" || subtype == "session_title" {
                    if let Some(t) = v
                        .get("systemPayload")
                        .and_then(|p| util::str_of(p, "title"))
                    {
                        self.title = Some(t.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    fn attach(&mut self, id: &str, result: String, is_err: bool, ts_ms: Option<i64>) {
        if let Some(&(ti, ci)) = self.calls.get(id) {
            let start = util::parse_ts_str(&self.turns[ti].ts);
            let c = &mut self.turns[ti].tool_calls[ci];
            if c.result.is_none() || !result.is_empty() {
                c.result = Some(result);
            }
            c.status = if is_err {
                ToolStatus::Error
            } else {
                ToolStatus::Ok
            };
            if let (Some(a), Some(b)) = (start, ts_ms) {
                if b >= a {
                    c.ms = Some((b - a) as u64);
                }
            }
        }
    }

    fn attach_status(&mut self, id: &str, st: ToolStatus) {
        if let Some(&(ti, ci)) = self.calls.get(id) {
            self.turns[ti].tool_calls[ci].status = st;
        }
    }
}

fn load_file(path: &Path) -> Session {
    let mut p = Parser::default();
    let _ = util::for_each_line(path, 0, |l| p.feed(l));
    let id = p.session_id.clone().unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let mut usage = TokenUsage::default();
    let mut models = Vec::new();
    for t in &p.turns {
        usage.add(&t.usage);
        if let Some(m) = &t.model {
            if !models.contains(m) {
                models.push(m.clone());
            }
        }
    }
    let first = p
        .turns
        .iter()
        .filter_map(|t| util::parse_ts_str(&t.ts))
        .min();
    let last = p
        .turns
        .iter()
        .filter_map(|t| util::parse_ts_str(&t.ts))
        .max();
    Session {
        meta: SessionMeta {
            tool: TOOL.into(),
            account: None,
            id,
            title: p.title.clone().or_else(|| {
                p.first_user
                    .as_ref()
                    .map(|t| truncate_chars(t.lines().next().unwrap_or(t).trim(), 80))
            }),
            project_path: p.cwd.clone(),
            git_branch: p.branch.clone(),
            started: first.map(util::ms_to_rfc3339),
            ended: last.map(util::ms_to_rfc3339),
            models,
            parent_session: None,
            turn_count: p.turns.len() as u32,
            usage,
            cost_usd: 0.0,
            source: path.to_path_buf(),
            mtime_ms: util::file_mtime_ms(path),
        },
        turns: p.turns,
    }
}

impl SessionSource for QwenSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    fn list(&self) -> Vec<SessionMeta> {
        self.files().iter().map(|p| load_file(p).meta).collect()
    }

    fn load(&self, id: &str) -> Option<Session> {
        let files = self.files();
        if let Some(p) = files.iter().find(|p| {
            p.file_stem()
                .map(|s| s.to_string_lossy() == id)
                .unwrap_or(false)
        }) {
            return Some(load_file(p));
        }
        files.iter().map(|p| load_file(p)).find(|s| s.meta.id == id)
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        Some(util::in_dir(
            meta.project_path.as_deref(),
            format!("qwen --resume {}", meta.id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_content_records_become_turns() {
        let dir = std::env::temp_dir().join(format!("omniget-qwen-{}", std::process::id()));
        let chats = dir.join("-w-p").join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        let lines = [
            r#"{"uuid":"u1","parentUuid":null,"sessionId":"q1","timestamp":"2026-09-20T10:00:00.000Z","type":"user","cwd":"/w/p","message":{"role":"user","parts":[{"text":"leia o README"}]}}"#,
            r#"{"uuid":"a1","parentUuid":"u1","sessionId":"q1","timestamp":"2026-09-20T10:00:02.000Z","type":"assistant","cwd":"/w/p","model":"qwen3-coder-plus","message":{"role":"model","parts":[{"functionCall":{"id":"c1","name":"read_file","args":{"absolute_path":"/w/p/README.md"}}}]},"usageMetadata":{"promptTokenCount":500,"candidatesTokenCount":30,"cachedContentTokenCount":200,"thoughtsTokenCount":7,"totalTokenCount":537}}"#,
            r#"{"uuid":"t1","parentUuid":"a1","sessionId":"q1","timestamp":"2026-09-20T10:00:03.000Z","type":"tool_result","cwd":"/w/p","message":{"role":"user","parts":[{"functionResponse":{"id":"c1","name":"read_file","response":{"output":"Proj"}}}]},"toolCallResult":{"callId":"c1","status":"success"}}"#,
        ];
        std::fs::write(chats.join("q1.jsonl"), lines.join("\n") + "\n").unwrap();
        let src = QwenSource::with_roots(vec![dir.clone()]);
        let s = src.load("q1").unwrap();
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.meta.usage.input, 300);
        assert_eq!(s.meta.usage.cache_read, 200);
        let c = &s.turns[1].tool_calls[0];
        assert_eq!(c.name_canonical, "Read");
        assert_eq!(c.result.as_deref(), Some("Proj"));
        assert_eq!(c.status, ToolStatus::Ok);
        assert_eq!(c.ms, Some(1000));
        std::fs::remove_dir_all(&dir).ok();
    }
}
