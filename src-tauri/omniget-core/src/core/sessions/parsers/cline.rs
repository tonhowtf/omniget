//! Cline e a família que herdou o formato: Roo Code e o Kilo Code legado
//! (extensão VS Code antes de abril de 2026). Estudo 06, Cline §j, Roo §j,
//! Kilo §j; Parte 6 §8.3.
//!
//! Extensão (qualquer editor da família VS Code):
//! `<User>/globalStorage/<ext>/tasks/<taskId>/ui_messages.json` (lista de
//! `ClineMessage {ts, type:"ask"|"say", ask?, say?, text?, modelInfo?}`; o uso
//! de cada chamada vem em `say:"api_req_started"`, cujo `text` é JSON
//! `{tokensIn, tokensOut, cacheWrites, cacheReads, cost, apiProtocol?}`),
//! `api_conversation_history.json` (mensagens Anthropic) e o histórico
//! (`state/taskHistory.json` no Cline, `tasks/_index.json` ou
//! `tasks/<id>/history_item.json` no Roo) com `task`, `ts`, `tokensIn/Out`,
//! `totalCost`, `cwdOnTaskInitialization`/`workspace`, `modelId`.
//!
//! Cline CLI/SDK: `~/.cline/data/sessions/<id>/<id>.messages.json`
//! (`messages[{id, role, ts, content[], modelInfo{id}, metrics{inputTokens,
//! outputTokens, cacheReadTokens, cacheWriteTokens, cost}}]`) e o manifesto
//! `<id>.json` (`cwd`, `workspace_root`, `model`, `metadata.title`).
//!
//! Nenhuma dessas extensões está instalada nesta máquina: só fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::util::*;
use crate::core::sessions::model::{
    Role, Session, SessionMeta, SessionSource, TokenUsage, ToolStatus, Turn,
};

pub struct ClineSource {
    tool: &'static str,
    ext_id: &'static str,
    with_cli: bool,
}

impl ClineSource {
    pub fn cline() -> Self {
        Self {
            tool: "cline",
            ext_id: "saoudrizwan.claude-dev",
            with_cli: true,
        }
    }
    pub fn roo() -> Self {
        Self {
            tool: "roo",
            ext_id: "rooveterinaryinc.roo-cline",
            with_cli: false,
        }
    }
    /// Kilo Code legado (extensão VS Code). O Kilo atual (CLI, `kilo.db`) é do
    /// grupo a; os dois usam o id `kilo`.
    pub fn kilo_legacy() -> Self {
        Self {
            tool: "kilo",
            ext_id: "kilocode.kilo-code",
            with_cli: false,
        }
    }

    /// (rótulo, pasta da extensão).
    fn ext_dirs(&self) -> Vec<(String, PathBuf)> {
        let mut v: Vec<(String, PathBuf)> = vscode_user_dirs()
            .into_iter()
            .map(|(l, u)| (l, u.join("globalStorage").join(self.ext_id)))
            .filter(|(_, p)| p.is_dir())
            .collect();
        // Roo CLI roda a extensão num shim de VS Code.
        if let Some(p) = home_join(&[".vscode-mock", "global-storage", self.ext_id]) {
            if p.is_dir() {
                v.push(("cli".into(), p));
            }
        }
        v
    }

    fn task_dirs(&self) -> Vec<(String, PathBuf, PathBuf)> {
        let mut out = Vec::new();
        for (label, ext) in self.ext_dirs() {
            let Ok(rd) = std::fs::read_dir(ext.join("tasks")) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.join("ui_messages.json").is_file()
                    || p.join("api_conversation_history.json").is_file()
                {
                    out.push((label.clone(), ext.clone(), p));
                }
            }
        }
        out
    }

    fn cli_roots(&self) -> Vec<PathBuf> {
        if !self.with_cli {
            return Vec::new();
        }
        let mut v = Vec::new();
        if let Some(p) = env_dir("CLINE_SESSION_DATA_DIR") {
            v.push(p);
        }
        if let Some(p) = env_dir("CLINE_DATA_DIR") {
            v.push(p.join("sessions"));
        }
        if let Some(p) = env_dir("CLINE_DIR") {
            v.push(p.join("data").join("sessions"));
        }
        if let Some(p) = home_join(&[".cline", "data", "sessions"]) {
            v.push(p);
        }
        dedup_paths(v).into_iter().filter(|p| p.is_dir()).collect()
    }

    fn cli_files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for r in self.cli_roots() {
            out.extend(find_files(&r, 2, |p| {
                file_name(p).ends_with(".messages.json")
            }));
        }
        out
    }
}

/// Itens de histórico indexados pelo id da tarefa.
fn history_index(ext: &Path) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    let mut push_all = |v: Option<Value>| {
        let arr = match v {
            Some(Value::Array(a)) => a,
            Some(Value::Object(o)) => o
                .get("entries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        for it in arr {
            if let Some(id) = s(&it, &["id"]) {
                map.insert(id, it);
            }
        }
    };
    push_all(read_json(&ext.join("state").join("taskHistory.json")));
    push_all(read_json(&ext.join("tasks").join("_index.json")));
    map
}

fn tool_name_of_ui(tool: &str) -> &str {
    match tool {
        "readFile" | "fetchInstructions" => "read_file",
        "editedExistingFile" | "appliedDiff" | "insertContent" | "searchAndReplace" => {
            "replace_in_file"
        }
        "newFileCreated" => "write_to_file",
        "listFilesTopLevel" | "listFilesRecursive" => "list_files",
        "searchFiles" | "codebaseSearch" | "listCodeDefinitionNames" => "search_files",
        "webFetch" => "web_fetch",
        "webSearch" => "web_search",
        "newTask" | "subagent" | "useSubagents" => "new_task",
        "useSkill" | "skill" => "use_skill",
        "updateTodoList" => "update_todo_list",
        other => other,
    }
}

fn api_req_usage(info: &Value) -> TokenUsage {
    let cache_read = u(info, &["cacheReads"]);
    let cache_write = u(info, &["cacheWrites"]);
    let mut input = u(info, &["tokensIn"]);
    // Roo com protocolo OpenAI conta o cache dentro de tokensIn.
    if info.get("apiProtocol").and_then(Value::as_str) == Some("openai") {
        input = input.saturating_sub(cache_read + cache_write);
    }
    TokenUsage {
        input,
        output: u(info, &["tokensOut"]),
        cache_read,
        cache_write,
        reasoning: 0,
    }
}

/// `ui_messages.json` em turnos. Cada `api_req_started` abre um turno do
/// assistente com o uso/custo daquela chamada.
pub fn parse_ui_messages(msgs: &[Value], default_model: Option<&str>) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut model = default_model.map(str::to_string);
    let mut n = 0usize;
    fn cur_assistant<'a>(
        turns: &'a mut Vec<Turn>,
        ts: &str,
        model: &Option<String>,
    ) -> &'a mut Turn {
        if !matches!(turns.last(), Some(t) if t.role == Role::Assistant) {
            let mut t = turn(Role::Assistant, ts.to_string(), "");
            t.model = model.clone();
            turns.push(t);
        }
        turns.last_mut().expect("existe")
    }
    for m in msgs {
        let ts = m.get("ts").and_then(ts_str).unwrap_or_default();
        let ty = m.get("type").and_then(Value::as_str).unwrap_or("");
        let say = m.get("say").and_then(Value::as_str).unwrap_or("");
        let ask = m.get("ask").and_then(Value::as_str).unwrap_or("");
        let text = m
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if let Some(mid) = m.pointer("/modelInfo/modelId").and_then(Value::as_str) {
            model = Some(mid.to_string());
        }
        match (ty, say, ask) {
            ("say", "task", _) | ("say", "user_feedback", _) => {
                turns.push(turn(Role::User, ts, text));
            }
            ("say", "api_req_started", _) => {
                let info: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let mut t = turn(Role::Assistant, ts, "");
                t.usage = api_req_usage(&info);
                t.cost_usd = fnum(&info, &["cost"]);
                t.model = model.clone();
                n += 1;
                t.message_id = Some(format!("req-{n}"));
                turns.push(t);
            }
            ("say", "text", _)
            | ("say", "completion_result", _)
            | ("ask", _, "completion_result")
            | ("ask", _, "followup") => {
                if text.trim().is_empty() {
                    continue;
                }
                if ask == "followup" {
                    let q: Value =
                        serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
                    let t = cur_assistant(&mut turns, &ts, &model);
                    let mut c = tool_call("", "ask_followup_question", q);
                    c.status = ToolStatus::Ok;
                    t.tool_calls.push(c);
                    continue;
                }
                let t = cur_assistant(&mut turns, &ts, &model);
                if !t.text.is_empty() {
                    t.text.push('\n');
                }
                t.text.push_str(&text);
            }
            ("ask", _, "tool") | ("say", "tool", _) => {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let raw = s(&v, &["tool"]).unwrap_or_else(|| "tool".into());
                let t = cur_assistant(&mut turns, &ts, &model);
                let mut c = tool_call("", tool_name_of_ui(&raw), v);
                c.name_raw = raw;
                c.status = ToolStatus::Ok;
                t.tool_calls.push(c);
            }
            ("ask", _, "command") | ("say", "command", _) => {
                let t = cur_assistant(&mut turns, &ts, &model);
                let mut c = tool_call(
                    "",
                    "execute_command",
                    serde_json::json!({ "command": text }),
                );
                c.status = ToolStatus::Ok;
                t.tool_calls.push(c);
            }
            ("say", "command_output", _) | ("ask", _, "command_output") => {
                if let Some(c) = turns
                    .iter_mut()
                    .rev()
                    .flat_map(|t| t.tool_calls.iter_mut().rev())
                    .find(|c| c.name_canonical == "Bash")
                {
                    let r = c.result.get_or_insert_with(String::new);
                    r.push_str(&text);
                }
            }
            ("ask", _, "use_mcp_server") | ("say", "use_mcp_server", _) => {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let t = cur_assistant(&mut turns, &ts, &model);
                let mut c = tool_call("", "use_mcp_tool", v);
                c.status = ToolStatus::Ok;
                t.tool_calls.push(c);
            }
            ("say", "mcp_server_response", _) => {
                if let Some(c) = turns
                    .iter_mut()
                    .rev()
                    .flat_map(|t| t.tool_calls.iter_mut().rev())
                    .find(|c| c.name_canonical == "Mcp")
                {
                    c.result = Some(text);
                }
            }
            ("say", "error", _) | ("ask", _, "api_req_failed") => {
                if let Some(c) = turns.last_mut().and_then(|t| t.tool_calls.last_mut()) {
                    c.status = ToolStatus::Error;
                }
            }
            ("say", "subagent_usage", _) => {
                let info: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let mut t = turn(Role::Assistant, ts, "");
                t.usage = api_req_usage(&info);
                t.cost_usd = fnum(&info, &["cost"]);
                t.model = model.clone();
                turns.push(t);
            }
            _ => {}
        }
    }
    turns
}

fn model_from_history_file(task: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(task.join("api_conversation_history.json")).ok()?;
    let i = raw.rfind("<model>")? + "<model>".len();
    let j = raw[i..].find("</model>")? + i;
    let m = raw[i..j].trim();
    (!m.is_empty() && m.len() < 200).then(|| m.to_string())
}

impl ClineSource {
    fn task_meta(&self, label: &str, task: &Path, hist: Option<&Value>) -> SessionMeta {
        let id = file_name(task);
        let ui = task.join("ui_messages.json");
        let src = if ui.is_file() {
            ui
        } else {
            task.join("api_conversation_history.json")
        };
        let mut m = new_meta(self.tool, id, &src);
        m.account = Some(label.to_string());
        let item = hist
            .cloned()
            .or_else(|| read_json(&task.join("history_item.json")));
        if let Some(h) = item {
            m.title = s(&h, &["task"]).map(|t| title_from(&t));
            m.project_path = s(&h, &["cwdOnTaskInitialization", "workspace"]);
            m.ended = h.get("ts").and_then(ts_str);
            m.usage = TokenUsage {
                input: u(&h, &["tokensIn"]),
                output: u(&h, &["tokensOut"]),
                cache_read: u(&h, &["cacheReads"]),
                cache_write: u(&h, &["cacheWrites"]),
                reasoning: 0,
            };
            m.cost_usd = fnum(&h, &["totalCost"]).unwrap_or(0.0);
            if let Some(mid) = s(&h, &["modelId"]) {
                m.models.push(mid);
            }
            m.parent_session = s(&h, &["parentTaskId"]);
        }
        m
    }

    fn load_task(&self, label: &str, ext: &Path, task: &Path) -> Session {
        let hist = history_index(ext);
        let id = file_name(task);
        let mut meta = self.task_meta(label, task, hist.get(&id));
        let default_model = meta
            .models
            .first()
            .cloned()
            .or_else(|| model_from_history_file(task));
        let msgs = read_json(&task.join("ui_messages.json"))
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let mut turns = if msgs.is_empty() {
            // Sem ui_messages: cai para o histórico da API (sem uso).
            let api = read_json(&task.join("api_conversation_history.json"))
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            super::cursor::role_messages_to_turns(&api)
        } else {
            parse_ui_messages(&msgs, default_model.as_deref())
        };
        meta.models.clear();
        finalize(&mut meta, &mut turns);
        if meta.models.is_empty() {
            meta.models.extend(default_model);
        }
        Session { meta, turns }
    }
}

/// Sessão do Cline CLI a partir do `<id>.messages.json` e do manifesto.
pub fn parse_cli_session(
    doc: &Value,
    manifest: Option<&Value>,
    fallback_id: &str,
    source: &Path,
) -> Session {
    let id = s(doc, &["sessionId"])
        .or_else(|| manifest.and_then(|m| s(m, &["session_id", "sessionId"])))
        .unwrap_or_else(|| fallback_id.to_string());
    let mut meta = new_meta("cline", id, source);
    meta.account = Some("cli".into());
    let default_model = manifest.and_then(|m| s(m, &["model"]));
    if let Some(m) = manifest {
        meta.project_path = s(m, &["workspace_root", "cwd"]);
        meta.title = m
            .pointer("/metadata/title")
            .and_then(Value::as_str)
            .map(str::to_string);
        meta.parent_session = s(m, &["parent_session_id"]);
        meta.git_branch = m
            .pointer("/metadata/git/branch")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    let msgs = doc
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| doc.as_array().cloned())
        .unwrap_or_default();
    let mut turns: Vec<Turn> = Vec::new();
    let mut results = Vec::new();
    for m in &msgs {
        let role = role_of(m.get("role").and_then(Value::as_str).unwrap_or(""));
        let (text, calls, res) = anthropic_blocks(m.get("content").unwrap_or(&Value::Null));
        results.extend(res);
        if role == Role::User && text.trim().is_empty() && calls.is_empty() {
            continue;
        }
        let mut t = turn(role, m.get("ts").and_then(ts_str).unwrap_or_default(), text);
        t.tool_calls = calls;
        t.message_id = s(m, &["id"]);
        if role == Role::Assistant {
            t.model = m
                .pointer("/modelInfo/id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| default_model.clone());
            if let Some(mt) = m.get("metrics") {
                let cr = u(mt, &["cacheReadTokens"]);
                let cw = u(mt, &["cacheWriteTokens"]);
                t.usage = TokenUsage {
                    input: u(mt, &["inputTokens"]).saturating_sub(cr + cw),
                    output: u(mt, &["outputTokens"]),
                    cache_read: cr,
                    cache_write: cw,
                    reasoning: 0,
                };
                t.cost_usd = fnum(mt, &["cost"]);
            }
        }
        turns.push(t);
    }
    attach_results(&mut turns, results);
    finalize(&mut meta, &mut turns);
    Session { meta, turns }
}

fn cli_manifest(f: &Path) -> Option<Value> {
    let name = file_name(f);
    let stem = name.strip_suffix(".messages.json")?;
    read_json(&f.with_file_name(format!("{stem}.json")))
}

fn cli_id(f: &Path) -> String {
    file_name(f).trim_end_matches(".messages.json").to_string()
}

impl SessionSource for ClineSource {
    fn tool(&self) -> &'static str {
        self.tool
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.ext_dirs().into_iter().map(|(_, p)| p).collect();
        v.extend(self.cli_roots());
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        let mut hist_cache: HashMap<PathBuf, HashMap<String, Value>> = HashMap::new();
        for (label, ext, task) in self.task_dirs() {
            let hist = hist_cache
                .entry(ext.clone())
                .or_insert_with(|| history_index(&ext));
            let id = file_name(&task);
            let has_totals = hist
                .get(&id)
                .map(|h| h.get("tokensIn").is_some() && h.get("task").is_some())
                .unwrap_or(false);
            if has_totals {
                let mut m = self.task_meta(&label, &task, hist.get(&id));
                if m.started.is_none() {
                    m.started = str_to_ms(&id)
                        .filter(|ms| *ms > 1_000_000_000_000)
                        .map(ms_to_rfc3339);
                }
                out.push(m);
            } else {
                out.push(self.load_task(&label, &ext, &task).meta);
            }
        }
        for f in self.cli_files() {
            let Some(doc) = read_json(&f) else { continue };
            let man = cli_manifest(&f);
            out.push(parse_cli_session(&doc, man.as_ref(), &cli_id(&f), &f).meta);
        }
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        if let Some((label, ext, task)) = self
            .task_dirs()
            .into_iter()
            .find(|(_, _, t)| file_name(t) == id)
        {
            return Some(self.load_task(&label, &ext, &task));
        }
        let f = self.cli_files().into_iter().find(|f| cli_id(f) == id)?;
        let doc = read_json(&f)?;
        let man = cli_manifest(&f);
        Some(parse_cli_session(&doc, man.as_ref(), id, &f))
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        // Só o Cline CLI retoma por id; as tarefas da extensão abrem no painel.
        (self.tool == "cline" && meta.account.as_deref() == Some("cli"))
            .then(|| format!("cline --id {}", shell_quote(&meta.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_messages_fixture() {
        let msgs: Vec<Value> = serde_json::from_str(r#"[
          {"ts":1781220418119,"type":"say","say":"task","text":"Crie um README"},
          {"ts":1781220419000,"type":"say","say":"api_req_started","text":"{\"request\":\"...\",\"tokensIn\":100,\"tokensOut\":50,\"cacheWrites\":5,\"cacheReads\":20,\"cost\":0.12}","modelInfo":{"providerId":"anthropic","modelId":"claude-sonnet-4-5"}},
          {"ts":1781220420000,"type":"say","say":"text","text":"Vou olhar o projeto."},
          {"ts":1781220421000,"type":"ask","ask":"tool","text":"{\"tool\":\"readFile\",\"path\":\"package.json\"}"},
          {"ts":1781220422000,"type":"ask","ask":"command","text":"npm test"},
          {"ts":1781220423000,"type":"say","say":"command_output","text":"ok 3 testes"},
          {"ts":1781220424000,"type":"say","say":"api_req_started","text":"{\"tokensIn\":300,\"tokensOut\":80,\"cacheReads\":100,\"cacheWrites\":0,\"cost\":0.05,\"apiProtocol\":\"openai\"}"},
          {"ts":1781220425000,"type":"say","say":"completion_result","text":"Pronto."},
          {"ts":1781220426000,"type":"say","say":"user_feedback","text":"valeu"}
        ]"#).unwrap();
        let t = parse_ui_messages(&msgs, None);
        assert_eq!(t.len(), 4);
        assert_eq!(t[0].role, Role::User);
        assert_eq!(t[1].usage.input, 100);
        assert_eq!(t[1].cost_usd, Some(0.12));
        assert_eq!(t[1].model.as_deref(), Some("claude-sonnet-4-5"));
        let names: Vec<_> = t[1]
            .tool_calls
            .iter()
            .map(|c| c.name_canonical.as_str())
            .collect();
        assert_eq!(names, ["Read", "Bash"]);
        assert_eq!(t[1].tool_calls[1].result.as_deref(), Some("ok 3 testes"));
        assert_eq!(t[2].usage.input, 200);
        assert_eq!(t[2].text, "Pronto.");
    }

    #[test]
    fn cli_fixture() {
        let man: Value = serde_json::from_str(r#"{"session_id":"cli-1","model":"glm-5.2","workspace_root":"/home/x/p","metadata":{"title":"CLI task"}}"#).unwrap();
        let doc: Value = serde_json::from_str(r#"{"sessionId":"cli-1","messages":[
          {"id":"u1","role":"user","ts":1785320470000,"content":[{"type":"text","text":"leia o README"}]},
          {"id":"a1","role":"assistant","ts":1785320475705,"content":[{"type":"tool_use","id":"t1","name":"read_files","input":{"paths":["README.md"]}}],"modelInfo":{"id":"cline-free/glm-5.2"},"metrics":{"inputTokens":7507,"outputTokens":131,"cacheReadTokens":50,"cacheWriteTokens":0,"cost":0.011}},
          {"id":"r1","role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"README contents"}]}]}
        ]}"#).unwrap();
        let s = parse_cli_session(
            &doc,
            Some(&man),
            "cli-1",
            Path::new("/x/cli-1.messages.json"),
        );
        assert_eq!(s.meta.title.as_deref(), Some("CLI task"));
        assert_eq!(s.meta.project_path.as_deref(), Some("/home/x/p"));
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.turns[1].usage.input, 7457);
        assert_eq!(s.turns[1].tool_calls[0].name_canonical, "Read");
        assert_eq!(
            s.turns[1].tool_calls[0].result.as_deref(),
            Some("README contents")
        );
        assert!((s.meta.cost_usd - 0.011).abs() < 1e-9);
    }
}
