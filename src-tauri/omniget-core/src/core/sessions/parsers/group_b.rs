//! Registro das fontes do grupo b (dono: worker s2-sessions) e utilitários
//! comuns aos parsers do grupo: Cursor, Copilot, Cline (e Roo/Kilo legados),
//! Zed, Droid, Kimi, Pi, Amp, Aider, Junie, Auggie, Grok e Kiro.
//!
//! Regras de todos os parsers daqui: só leitura, nunca credencial, arquivo
//! estranho vira vazio (nunca pânico), SQLite aberto em modo somente leitura.

#[path = "aider.rs"]
pub mod aider;
#[path = "amp.rs"]
pub mod amp;
#[path = "auggie.rs"]
pub mod auggie;
#[path = "cline.rs"]
pub mod cline;
#[path = "copilot.rs"]
pub mod copilot;
#[path = "cursor.rs"]
pub mod cursor;
#[path = "droid.rs"]
pub mod droid;
#[path = "grok.rs"]
pub mod grok;
#[path = "junie.rs"]
pub mod junie;
#[path = "kimi.rs"]
pub mod kimi;
#[path = "kiro.rs"]
pub mod kiro;
#[path = "pi.rs"]
pub mod pi;
#[path = "zed.rs"]
pub mod zed;

use crate::core::sessions::model::SessionSource;

pub fn sources() -> Vec<Box<dyn SessionSource>> {
    vec![
        Box::new(cursor::CursorSource),
        Box::new(copilot::CopilotSource),
        Box::new(cline::ClineSource::cline()),
        Box::new(cline::ClineSource::roo()),
        Box::new(cline::ClineSource::kilo_legacy()),
        Box::new(zed::ZedSource),
        Box::new(droid::DroidSource),
        Box::new(kimi::KimiSource),
        Box::new(pi::PiSource),
        Box::new(amp::AmpSource),
        Box::new(aider::AiderSource),
        Box::new(junie::JunieSource),
        Box::new(auggie::AuggieSource),
        Box::new(grok::GrokSource),
        Box::new(kiro::KiroSource),
    ]
}

/// Utilitários compartilhados pelos parsers do grupo b.
pub mod util {
    use std::collections::HashMap;
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};

    use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
    use serde_json::Value;

    use crate::core::sessions::model::{Role, SessionMeta, TokenUsage, ToolCall, ToolStatus, Turn};

    /// Arquivo maior que isto não é lido inteiro (proteção contra lixo).
    pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

    pub fn home() -> Option<PathBuf> {
        dirs::home_dir()
    }

    pub fn home_join(parts: &[&str]) -> Option<PathBuf> {
        let mut p = home()?;
        for part in parts {
            p = p.join(part);
        }
        Some(p)
    }

    /// Variável de ambiente com caminho, ignorando vazia.
    pub fn env_dir(var: &str) -> Option<PathBuf> {
        let v = std::env::var_os(var)?;
        if v.is_empty() {
            return None;
        }
        Some(PathBuf::from(v))
    }

    /// `~/.local/share` no Linux (respeitando `XDG_DATA_HOME`); no macOS e no
    /// Windows as CLIs de Node/Go/Rust costumam usar o mesmo `~/.local/share`
    /// literal, então devolvemos os dois candidatos quando diferem.
    pub fn xdg_data_homes() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(x) = env_dir("XDG_DATA_HOME") {
            v.push(x);
        }
        if let Some(h) = home_join(&[".local", "share"]) {
            v.push(h);
        }
        if let Some(d) = dirs::data_dir() {
            v.push(d);
        }
        dedup_paths(v)
    }

    pub fn xdg_config_homes() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(x) = env_dir("XDG_CONFIG_HOME") {
            v.push(x);
        }
        if let Some(h) = home_join(&[".config"]) {
            v.push(h);
        }
        if let Some(d) = dirs::config_dir() {
            v.push(d);
        }
        dedup_paths(v)
    }

    pub fn dedup_paths(v: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in v {
            if !out.iter().any(|q| q == &p) {
                out.push(p);
            }
        }
        out
    }

    /// Diretórios `User/` dos editores da família VS Code instalados, com o
    /// rótulo do editor (vira `SessionMeta.account`). macOS
    /// `~/Library/Application Support/<App>/User`, Linux `~/.config/<App>/User`,
    /// Windows `%APPDATA%\<App>\User`, mais os servidores remotos
    /// (`~/.vscode-server/data/User` etc.).
    pub fn vscode_user_dirs() -> Vec<(String, PathBuf)> {
        const APPS: &[&str] = &[
            "Code",
            "Code - Insiders",
            "VSCodium",
            "Cursor",
            "Windsurf",
            "Kiro",
            "Trae",
            "Positron",
        ];
        let mut out = Vec::new();
        let mut bases = Vec::new();
        if let Some(c) = dirs::config_dir() {
            bases.push(c);
        }
        if let Some(h) = home_join(&[".config"]) {
            bases.push(h);
        }
        let bases = dedup_paths(bases);
        for base in &bases {
            for app in APPS {
                let p = base.join(app).join("User");
                if p.is_dir() && !out.iter().any(|(_, q): &(String, PathBuf)| q == &p) {
                    out.push((app.to_string(), p));
                }
            }
        }
        for (label, rel) in [
            ("vscode-server", ".vscode-server"),
            ("vscode-server-insiders", ".vscode-server-insiders"),
            ("cursor-server", ".cursor-server"),
        ] {
            if let Some(p) = home_join(&[rel, "data", "User"]) {
                if p.is_dir() {
                    out.push((label.to_string(), p));
                }
            }
        }
        out
    }

    pub fn mtime_ms(p: &Path) -> i64 {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    pub fn ms_to_rfc3339(ms: i64) -> String {
        Utc.timestamp_millis_opt(ms)
            .single()
            .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_default()
    }

    /// Converte número (s, ms, µs, ns) ou string (RFC 3339, "YYYY-MM-DD
    /// HH:MM:SS", numérica) em milissegundos desde a época.
    pub fn ts_ms(v: &Value) -> Option<i64> {
        match v {
            Value::Number(n) => {
                let f = n.as_f64()?;
                num_to_ms(f)
            }
            Value::String(s) => str_to_ms(s),
            _ => None,
        }
    }

    fn num_to_ms(f: f64) -> Option<i64> {
        if !f.is_finite() || f <= 0.0 {
            return None;
        }
        let ms = if f > 1e17 {
            f / 1e6
        } else if f > 1e14 {
            f / 1e3
        } else if f > 1e11 {
            f
        } else {
            f * 1000.0
        };
        Some(ms as i64)
    }

    pub fn str_to_ms(s: &str) -> Option<i64> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Ok(d) = DateTime::parse_from_rfc3339(s) {
            return Some(d.timestamp_millis());
        }
        if let Ok(f) = s.parse::<f64>() {
            return num_to_ms(f);
        }
        for fmt in [
            "%Y-%m-%d %H:%M:%S%.f",
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%dT%H:%M:%S",
        ] {
            if let Ok(n) = NaiveDateTime::parse_from_str(s, fmt) {
                return Some(n.and_utc().timestamp_millis());
            }
        }
        None
    }

    pub fn ts_str(v: &Value) -> Option<String> {
        ts_ms(v).map(ms_to_rfc3339)
    }

    /// Primeiro campo presente (entre `keys`) convertido em timestamp.
    pub fn ts_field(v: &Value, keys: &[&str]) -> Option<i64> {
        keys.iter().find_map(|k| v.get(*k).and_then(ts_ms))
    }

    pub fn read_json(p: &Path) -> Option<Value> {
        let md = fs::metadata(p).ok()?;
        if !md.is_file() || md.len() > MAX_FILE_BYTES {
            return None;
        }
        let bytes = fs::read(p).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Lê JSONL linha a linha; linha quebrada é pulada.
    pub fn read_jsonl(p: &Path) -> Vec<Value> {
        let mut out = Vec::new();
        for_each_jsonl(p, |v| out.push(v));
        out
    }

    pub fn for_each_jsonl(p: &Path, mut f: impl FnMut(Value)) {
        let Ok(md) = fs::metadata(p) else { return };
        if !md.is_file() || md.len() > MAX_FILE_BYTES {
            return;
        }
        let Ok(file) = fs::File::open(p) else { return };
        let reader = BufReader::new(file);
        for line in reader.split(b'\n') {
            let Ok(line) = line else { break };
            let t = trim_bytes(&line);
            if t.is_empty() || t[0] != b'{' {
                continue;
            }
            if let Ok(v) = serde_json::from_slice::<Value>(t) {
                f(v);
            }
        }
    }

    /// Primeira linha JSON de um JSONL (cabeçalho), sem ler o resto.
    pub fn first_jsonl(p: &Path) -> Option<Value> {
        let file = fs::File::open(p).ok()?;
        let reader = BufReader::new(file);
        for line in reader.split(b'\n').take(50) {
            let line = line.ok()?;
            let t = trim_bytes(&line);
            if t.is_empty() {
                continue;
            }
            return serde_json::from_slice(t).ok();
        }
        None
    }

    fn trim_bytes(b: &[u8]) -> &[u8] {
        let mut s = 0;
        let mut e = b.len();
        while s < e && b[s].is_ascii_whitespace() {
            s += 1;
        }
        while e > s && b[e - 1].is_ascii_whitespace() {
            e -= 1;
        }
        &b[s..e]
    }

    /// Abre SQLite só para leitura, sem criar arquivo e sem travar escritor.
    pub fn open_ro(p: &Path) -> Option<rusqlite::Connection> {
        if !p.is_file() {
            return None;
        }
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_URI;
        let conn = rusqlite::Connection::open_with_flags(p, flags).ok()?;
        let _ = conn.busy_timeout(std::time::Duration::from_millis(1500));
        Some(conn)
    }

    pub fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .is_ok()
    }

    pub fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
        let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', ""));
        let Ok(mut st) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let rows = st.query_map([], |r| r.get::<_, String>(1));
        match rows {
            Ok(r) => r.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Valor numérico tolerante (int, float, string numérica), nunca negativo.
    pub fn as_u64(v: &Value) -> Option<u64> {
        match v {
            Value::Number(n) => n
                .as_u64()
                .or_else(|| n.as_i64().map(|i| i.max(0) as u64))
                .or_else(|| {
                    n.as_f64()
                        .filter(|f| f.is_finite())
                        .map(|f| f.max(0.0) as u64)
                }),
            Value::String(s) => s
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .map(|f| f.max(0.0) as u64),
            _ => None,
        }
    }

    pub fn as_f64(v: &Value) -> Option<f64> {
        match v {
            Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
            Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
            _ => None,
        }
    }

    /// Primeiro campo numérico presente entre `keys` (0 se nenhum).
    pub fn u(v: &Value, keys: &[&str]) -> u64 {
        keys.iter()
            .find_map(|k| v.get(*k).and_then(as_u64))
            .unwrap_or(0)
    }

    pub fn fnum(v: &Value, keys: &[&str]) -> Option<f64> {
        keys.iter().find_map(|k| v.get(*k).and_then(as_f64))
    }

    /// Primeira string não vazia entre `keys`.
    pub fn s(v: &Value, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|k| {
            v.get(*k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
        })
    }

    /// Texto de um `content` no formato Anthropic/OpenAI/ACP: string, lista de
    /// blocos `{type:"text", text}` ou objeto com `text`.
    pub fn content_text(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Array(a) => {
                let mut parts = Vec::new();
                for b in a {
                    match b {
                        Value::String(s) => parts.push(s.clone()),
                        Value::Object(_) => {
                            let ty = b.get("type").and_then(Value::as_str).unwrap_or("text");
                            if matches!(ty, "text" | "input_text" | "output_text" | "markdown") {
                                if let Some(t) = b.get("text").and_then(Value::as_str) {
                                    parts.push(t.to_string());
                                } else if let Some(t) = b.get("value").and_then(Value::as_str) {
                                    parts.push(t.to_string());
                                } else if let Some(t) = b.get("content").and_then(Value::as_str) {
                                    parts.push(t.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                parts.join("\n")
            }
            Value::Object(_) => v
                .get("text")
                .or_else(|| v.get("value"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| v.get("content").map(content_text))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Resultado de tool em texto (string, blocos ou JSON cru).
    pub fn result_text(v: &Value) -> String {
        match v {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            Value::Array(_) => {
                let t = content_text(v);
                if t.is_empty() {
                    v.to_string()
                } else {
                    t
                }
            }
            Value::Object(_) => {
                let t = content_text(v);
                if t.is_empty() {
                    v.to_string()
                } else {
                    t
                }
            }
            other => other.to_string(),
        }
    }

    /// Nome canônico (nomenclatura do Claude) a partir do nome cru de qualquer
    /// ferramenta. Tabela do §0C.5 do estudo 06 mais os nomes vistos nos logs
    /// reais (Cursor `read_file_v2`, `run_terminal_command_v2` etc.).
    pub fn canonical_tool(raw: &str) -> String {
        let r = raw.trim();
        let low = r.to_ascii_lowercase();
        if low.starts_with("mcp__")
            || low.starts_with("mcp_")
            || low.starts_with("mcp-")
            || low.starts_with("mcp:")
            || low.starts_with('@')
        {
            return "Mcp".into();
        }
        let mut key: String = low
            .trim_start_matches("copilot_")
            .trim_start_matches("vscode_")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if let Some(stripped) = key.strip_suffix("v2") {
            key = stripped.to_string();
        }
        let c = match key.as_str() {
            "bash" | "shell" | "executecommand" | "runshellcommand" | "execcommand"
            | "runterminalcmd" | "runterminalcommand" | "terminal" | "launchprocess"
            | "execute" | "executebash" | "powershell" | "runcommand" | "runcommands"
            | "shellcommand" | "runinterminal" | "command" | "localshell" | "exec" => "Bash",
            "read" | "readfile" | "view" | "fsread" | "readfiles" | "readmanyfiles"
            | "viewfile" | "readmediafile" | "cat" | "open" | "readmultiplefiles" => "Read",
            "edit"
            | "editfile"
            | "strreplace"
            | "strreplaceeditor"
            | "strreplacebasededittool"
            | "replace"
            | "replaceinfile"
            | "applydiff"
            | "applypatch"
            | "patch"
            | "multiedit"
            | "searchreplace"
            | "searchandreplace"
            | "replacefilecontent"
            | "editor"
            | "replacestring"
            | "multireplacestring"
            | "insertedit"
            | "editfiles"
            | "undoedit"
            | "strreplacebasedtool"
            | "modify" => "Edit",
            "write" | "writefile" | "writetofile" | "create" | "createfile" | "savefile"
            | "fswrite" | "fsappend" | "newfile" => "Write",
            "glob" | "globfilesearch" | "listfiles" | "findpath" | "find" | "filesearch"
            | "listdir" | "listdirectory" | "ls" | "tree" | "findbyname" | "finder"
            | "findfiles" | "list" => "Glob",
            "grep" | "grepsearch" | "searchfiles" | "rg" | "ripgreprawsearch" | "ripgrep"
            | "codebasesearch" | "searchcodebase" | "semanticsearch" | "semanticsearchfull"
            | "codebaseretrieval" | "search" | "findtextinfiles" | "textsearch" => "Grep",
            "webfetch" | "fetch" | "fetchurl" | "readurlcontent" | "readwebpage"
            | "fetchwebcontent" | "fetchweb" | "fetchwebpage" | "urlfetch" => "WebFetch",
            "websearch" | "googlewebsearch" | "searchweb" => "WebSearch",
            "task" | "agent" | "spawnagent" | "newtask" | "subagent" | "invokesubagent"
            | "delegate" | "spawnsubagent" | "runsubagent" | "usesubagents" | "agentswarm"
            | "dispatchagent" => "Agent",
            "todowrite" | "todo" | "updatetodo" | "updatetodolist" | "updateplan"
            | "writetodos" | "todolist" | "todoread" | "managetodolist" | "tasklist"
            | "addtasks" | "updatetasks" => "Todo",
            "skill" | "useskill" | "activateskill" | "skills" => "Skill",
            "askuser"
            | "askuserquestion"
            | "askquestion"
            | "askfollowupquestion"
            | "requestuserinput"
            | "askuserquestions"
            | "askquestions" => "AskUser",
            "usemcptool" | "callmcptool" | "accessmcpresource" | "mcptool" | "readmcpresource"
            | "fetchmcpresource" | "calldynamictool" => "Mcp",
            _ => "Other",
        };
        c.to_string()
    }

    pub fn tool_call(id: impl Into<String>, raw: &str, input: Value) -> ToolCall {
        let canonical = canonical_tool(raw);
        let subagent = if canonical == "Agent" {
            s(
                &input,
                &[
                    "subagent_type",
                    "subagentType",
                    "agent",
                    "agent_name",
                    "agentName",
                    "name",
                ],
            )
        } else {
            None
        };
        ToolCall {
            id: id.into(),
            name_canonical: canonical,
            name_raw: raw.to_string(),
            input,
            result: None,
            status: ToolStatus::Pending,
            ms: None,
            subagent,
        }
    }

    pub fn turn(role: Role, ts: impl Into<String>, text: impl Into<String>) -> Turn {
        Turn {
            role,
            ts: ts.into(),
            text: text.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            cost_usd: None,
            model: None,
            message_id: None,
        }
    }

    pub fn role_of(s: &str) -> Role {
        match s.to_ascii_lowercase().as_str() {
            "user" | "human" | "prompt" => Role::User,
            "system" | "developer" => Role::System,
            "tool" | "toolresult" | "tool_result" | "function" => Role::Tool,
            _ => Role::Assistant,
        }
    }

    pub fn new_meta(tool: &str, id: impl Into<String>, source: &Path) -> SessionMeta {
        SessionMeta {
            tool: tool.to_string(),
            account: None,
            id: id.into(),
            title: None,
            project_path: None,
            git_branch: None,
            started: None,
            ended: None,
            models: Vec::new(),
            parent_session: None,
            turn_count: 0,
            usage: TokenUsage::default(),
            cost_usd: 0.0,
            source: source.to_path_buf(),
            mtime_ms: mtime_ms(source),
        }
    }

    /// Preenche contagem, soma de uso/custo, modelos e início/fim a partir dos
    /// turnos. Não sobrescreve `started`/`ended` já definidos; soma de uso só
    /// substitui a do meta quando os turnos trazem algum número.
    pub fn finalize(meta: &mut SessionMeta, turns: &mut [Turn]) {
        let fallback = meta
            .started
            .clone()
            .unwrap_or_else(|| ms_to_rfc3339(meta.mtime_ms));
        let mut last_ts = fallback.clone();
        for t in turns.iter_mut() {
            if t.ts.is_empty() {
                t.ts = last_ts.clone();
            } else {
                last_ts = t.ts.clone();
            }
        }
        meta.turn_count = turns
            .iter()
            .filter(|t| matches!(t.role, Role::User | Role::Assistant))
            .count() as u32;
        let mut usage = TokenUsage::default();
        let mut cost = 0.0;
        for t in turns.iter() {
            usage.add(&t.usage);
            if let Some(c) = t.cost_usd {
                cost += c;
            }
            if let Some(m) = &t.model {
                if !m.is_empty() && !meta.models.iter().any(|x| x == m) {
                    meta.models.push(m.clone());
                }
            }
        }
        if usage.total() > 0 {
            meta.usage = usage;
        }
        if cost > 0.0 {
            meta.cost_usd = cost;
        }
        if meta.started.is_none() {
            meta.started = turns.iter().map(|t| t.ts.clone()).find(|s| !s.is_empty());
        }
        if meta.ended.is_none() {
            meta.ended = turns
                .iter()
                .rev()
                .map(|t| t.ts.clone())
                .find(|s| !s.is_empty());
        }
        if meta.title.is_none() {
            meta.title = turns
                .iter()
                .find(|t| t.role == Role::User && !t.text.trim().is_empty())
                .map(|t| title_from(&t.text));
        }
    }

    /// Título curto a partir do primeiro prompt.
    pub fn title_from(text: &str) -> String {
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('<'))
            .unwrap_or_else(|| text.trim());
        truncate(line, 120)
    }

    pub fn truncate(s: &str, n: usize) -> String {
        if s.chars().count() <= n {
            return s.to_string();
        }
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }

    /// Aspas de shell portáveis (POSIX); o front mostra o comando para copiar.
    pub fn shell_quote(s: &str) -> String {
        if !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c))
        {
            return s.to_string();
        }
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    /// Blocos de conteúdo no estilo Anthropic (`text`, `thinking`,
    /// `tool_use`, `tool_result`). Devolve texto, chamadas e resultados
    /// `(tool_use_id, texto, is_error)`.
    pub fn anthropic_blocks(
        content: &Value,
    ) -> (String, Vec<ToolCall>, Vec<(String, String, bool)>) {
        let mut text = Vec::new();
        let mut calls = Vec::new();
        let mut results = Vec::new();
        match content {
            Value::String(s) => text.push(s.clone()),
            Value::Array(a) => {
                for b in a {
                    let ty = b.get("type").and_then(Value::as_str).unwrap_or("");
                    match ty {
                        "text" | "input_text" | "output_text" => {
                            if let Some(t) = b.get("text").and_then(Value::as_str) {
                                text.push(t.to_string());
                            }
                        }
                        "tool_use" | "tool-call" | "tool_call" | "toolCall" | "server_tool_use" => {
                            let id = s(b, &["id", "toolCallId", "tool_use_id", "callId"])
                                .unwrap_or_default();
                            let name =
                                s(b, &["name", "toolName"]).unwrap_or_else(|| "unknown".into());
                            let input = b
                                .get("input")
                                .or_else(|| b.get("arguments"))
                                .or_else(|| b.get("args"))
                                .cloned()
                                .unwrap_or(Value::Null);
                            let input = match input {
                                Value::String(ref st) => serde_json::from_str(st).unwrap_or(input),
                                other => other,
                            };
                            calls.push(tool_call(id, &name, input));
                        }
                        "tool_result" | "tool-result" | "toolResult" => {
                            let id = s(
                                b,
                                &["tool_use_id", "toolUseId", "toolUseID", "toolCallId", "id"],
                            )
                            .unwrap_or_default();
                            let body = b
                                .get("content")
                                .or_else(|| b.get("output"))
                                .or_else(|| b.get("result"))
                                .map(result_text)
                                .unwrap_or_default();
                            let err = b
                                .get("is_error")
                                .or_else(|| b.get("isError"))
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            results.push((id, body, err));
                        }
                        _ => {}
                    }
                }
            }
            Value::Object(_) => text.push(content_text(content)),
            _ => {}
        }
        (text.join("\n"), calls, results)
    }

    /// Liga resultados de tool às chamadas já vistas, pelo id.
    pub fn attach_results(turns: &mut [Turn], results: Vec<(String, String, bool)>) {
        if results.is_empty() {
            return;
        }
        let mut idx: HashMap<String, (usize, usize)> = HashMap::new();
        for (ti, t) in turns.iter().enumerate() {
            for (ci, c) in t.tool_calls.iter().enumerate() {
                if !c.id.is_empty() {
                    idx.insert(c.id.clone(), (ti, ci));
                }
            }
        }
        for (id, body, err) in results {
            if let Some((ti, ci)) = idx.get(&id).copied() {
                let c = &mut turns[ti].tool_calls[ci];
                c.result = Some(body);
                c.status = if err {
                    ToolStatus::Error
                } else {
                    ToolStatus::Ok
                };
            }
        }
    }

    /// Arquivos com um nome/sufixo sob uma raiz, até `depth` níveis.
    pub fn find_files(root: &Path, depth: usize, pred: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        if !root.is_dir() {
            return Vec::new();
        }
        walkdir::WalkDir::new(root)
            .max_depth(depth)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && pred(e.path()))
            .map(|e| e.into_path())
            .collect()
    }

    pub fn file_name(p: &Path) -> String {
        p.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }

    pub fn file_stem(p: &Path) -> String {
        p.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }

    pub fn parent_name(p: &Path) -> String {
        p.parent().map(file_name).unwrap_or_default()
    }

    /// Converte `file:///x/y` (ou caminho já cru) em caminho.
    pub fn uri_to_path(s: &str) -> String {
        let s = s.trim();
        let rest = match s.strip_prefix("file://") {
            Some(r) => r,
            None => return s.to_string(),
        };
        let decoded = percent_decode(rest);
        // file:///C:/x no Windows
        let b = decoded.as_bytes();
        if b.len() > 3 && b[0] == b'/' && b[2] == b':' {
            return decoded[1..].to_string();
        }
        decoded
    }

    pub fn percent_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Mapa `workspaceStorage/<hash>` → pasta aberta (lê `workspace.json`).
    pub fn workspace_folder(ws_dir: &Path) -> Option<String> {
        let v = read_json(&ws_dir.join("workspace.json"))?;
        let f = s(&v, &["folder", "workspace"])?;
        Some(uri_to_path(&f))
    }

    /// Ordena por mtime decrescente (mais recente primeiro).
    pub fn sort_recent(v: &mut [crate::core::sessions::model::SessionMeta]) {
        v.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms));
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn canonical_names() {
            assert_eq!(canonical_tool("run_terminal_command_v2"), "Bash");
            assert_eq!(canonical_tool("read_file_v2"), "Read");
            assert_eq!(canonical_tool("StrReplace"), "Edit");
            assert_eq!(canonical_tool("str-replace-editor"), "Edit");
            assert_eq!(canonical_tool("mcp__github__list"), "Mcp");
            assert_eq!(canonical_tool("mcp-Mobbin-search_screens"), "Mcp");
            assert_eq!(canonical_tool("write_to_file"), "Write");
            assert_eq!(canonical_tool("Task"), "Agent");
            assert_eq!(canonical_tool("copilot_findTextInFiles"), "Grep");
            assert_eq!(canonical_tool("whatever"), "Other");
        }

        #[test]
        fn timestamps() {
            assert_eq!(
                ts_ms(&serde_json::json!(1_700_000_000)),
                Some(1_700_000_000_000)
            );
            assert_eq!(
                ts_ms(&serde_json::json!(1_700_000_000_123i64)),
                Some(1_700_000_000_123)
            );
            assert_eq!(str_to_ms("2026-09-22T10:00:00Z"), Some(1_790_071_200_000));
            assert!(str_to_ms("lixo").is_none());
        }

        #[test]
        fn uri() {
            assert_eq!(uri_to_path("file:///Users/a/b%20c"), "/Users/a/b c");
            assert_eq!(uri_to_path("file:///c%3A/x"), "c:/x");
        }
    }
}

#[cfg(test)]
mod real {
    /// Varre as fontes do grupo b nos arquivos reais desta máquina (só
    /// leitura). `cargo test -p omniget-core real_group_b -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_group_b() {
        for src in super::sources() {
            let t0 = std::time::Instant::now();
            let list = src.list();
            let dt = t0.elapsed();
            let tokens: u64 = list.iter().map(|m| m.usage.total()).sum();
            eprintln!(
                "{:<8} sessões={:<5} tokens={:<10} list={:?} raízes={:?}",
                src.tool(),
                list.len(),
                tokens,
                dt,
                src.roots()
            );
            for m in list.iter().take(3) {
                let t1 = std::time::Instant::now();
                let s = src.load(&m.id).expect("load de um id listado");
                let tools: usize = s.turns.iter().map(|t| t.tool_calls.len()).sum();
                eprintln!(
                    "   {} {:?} proj={:?} turnos={} tools={} tokens={} modelos={:?} load={:?} resume={:?}",
                    m.id,
                    m.title.as_deref().map(|t| t.chars().take(40).collect::<String>()),
                    m.project_path,
                    s.turns.len(),
                    tools,
                    s.meta.usage.total(),
                    s.meta.models,
                    t1.elapsed(),
                    src.resume_command(&s.meta)
                );
            }
        }
    }
}
