//! Utilidades comuns dos parsers e das visões de sessão: tempo, nomes
//! canônicos de tool, leitura de linhas (com zstd), aspas de shell e a
//! codificação de diretório de projeto do Claude Code.
//!
//! Tudo aqui é puro ou só lê arquivo; nada escreve.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone, Utc};

/// Diretório home, `None` em ambiente sem home.
pub fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// `$XDG_DATA_HOME` ou `~/.local/share` (as ferramentas Go/TS usam XDG
/// também no macOS).
pub fn xdg_data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => home().map(|h| h.join(".local").join("share")),
    }
}

/// Divide uma variável de ambiente com lista de caminhos (separador do SO e
/// também vírgula, como o `CLAUDE_CONFIG_DIR` aceita).
pub fn env_paths(name: &str) -> Vec<PathBuf> {
    let Some(raw) = std::env::var_os(name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in std::env::split_paths(&raw) {
        let s = part.to_string_lossy().to_string();
        for piece in s.split(',') {
            let t = piece.trim();
            if !t.is_empty() {
                out.push(PathBuf::from(t));
            }
        }
    }
    out
}

/// Caminho canônico para comparar raízes (symlink de conta aponta pro mesmo
/// lugar); cai no próprio caminho se não resolver.
pub fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn file_mtime_ms(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// RFC 3339 com milissegundos, em UTC.
pub fn ms_to_rfc3339(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// Aceita RFC 3339, `YYYY-MM-DD HH:MM:SS` (SQLite, UTC), época em segundos,
/// milissegundos ou microssegundos.
pub fn parse_ts_str(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.timestamp_millis());
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(n) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(n.and_utc().timestamp_millis());
        }
    }
    if let Ok(n) = s.parse::<f64>() {
        return Some(epoch_to_ms(n));
    }
    None
}

/// Número de época em qualquer unidade para ms.
pub fn epoch_to_ms(n: f64) -> i64 {
    let a = n.abs();
    if a >= 1e17 {
        (n / 1e6) as i64 // ns
    } else if a >= 1e14 {
        (n / 1e3) as i64 // µs
    } else if a >= 1e11 {
        n as i64 // ms
    } else {
        (n * 1000.0) as i64 // s
    }
}

pub fn ts_ms_of(v: Option<&serde_json::Value>) -> Option<i64> {
    match v? {
        serde_json::Value::String(s) => parse_ts_str(s),
        serde_json::Value::Number(n) => n.as_f64().map(epoch_to_ms),
        _ => None,
    }
}

/// Timestamp RFC 3339 de uma `Turn` para ms (0 se vazio/ruim).
pub fn turn_ms(ts: &str) -> i64 {
    parse_ts_str(ts).unwrap_or(0)
}

/// Dia local `YYYY-MM-DD` e hora local 0..23 de um instante em ms.
pub fn local_day_hour(ms: i64) -> (String, u32) {
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => (
            d.format("%Y-%m-%d").to_string(),
            d.format("%H").to_string().parse().unwrap_or(0),
        ),
        None => (String::new(), 0),
    }
}

pub fn num(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key)
        .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64)))
        .unwrap_or(0)
}

pub fn str_of<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

/// Corta em `max` caracteres (não bytes), com reticências.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Texto de um valor de resultado arbitrário (string, lista de blocos
/// `{type:text,text}` ou JSON), cortado.
pub fn value_text(v: &serde_json::Value, max: usize) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => {
            let mut parts = Vec::new();
            for it in items {
                match it {
                    serde_json::Value::String(s) => parts.push(s.clone()),
                    serde_json::Value::Object(_) => {
                        if let Some(t) = it.get("text").and_then(|t| t.as_str()) {
                            parts.push(t.to_string());
                        } else if it.get("type").and_then(|t| t.as_str()) == Some("image") {
                            parts.push("[image]".into());
                        } else {
                            parts.push(it.to_string());
                        }
                    }
                    other => parts.push(other.to_string()),
                }
            }
            parts.join("\n")
        }
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    truncate_chars(&s, max)
}

/// Tamanho máximo de resultado de tool guardado numa `Turn`.
pub const RESULT_MAX: usize = 4000;

/// Nome canônico (nomenclatura do Claude) a partir do nome cru de qualquer
/// ferramenta. Uma tabela só: os nomes quase nunca colidem entre ferramentas.
pub fn canonical_tool(raw: &str) -> &'static str {
    let lower = raw.to_ascii_lowercase();
    let n = lower.as_str();
    if n.starts_with("mcp__") || n.starts_with("mcp_") || n.starts_with("mcp:") {
        return "Mcp";
    }
    // Goose: `<extensão>__<tool>`.
    if let Some((ext, tool)) = n.split_once("__") {
        return match ext {
            "developer" => match tool {
                "shell" => "Bash",
                "text_editor" => "Edit",
                "list_windows" | "screen_capture" | "image_processor" => "Other",
                _ => "Other",
            },
            "todo" => "Todo",
            "dynamic_task" | "subagent" | "subrecipe" => "Agent",
            "computercontroller" => "Other",
            "collaboration" => canonical_tool(tool),
            _ => "Mcp",
        };
    }
    match n {
        "bash" | "powershell" | "bashoutput" | "killshell" | "killbash" | "monitor" | "shell"
        | "shell_command" | "exec_command" | "write_stdin" | "local_shell" | "local_shell_call"
        | "exec" | "run_shell_command" | "run_terminal_cmd" | "execute_command" | "job_output"
        | "job_kill" | "run_command" | "terminal" | "command" => "Bash",
        "read" | "notebookread" | "read_file" | "read_many_files" | "view" | "view_image"
        | "read_notebook" | "cat" | "open" => "Read",
        "edit" | "multiedit" | "notebookedit" | "notebook_edit" | "apply_patch" | "replace"
        | "str_replace" | "str_replace_editor" | "edit_file" | "search_replace" | "patch"
        | "text_editor" | "replace_in_file" | "lsp_rename" | "lsp_replace_symbol" => "Edit",
        "write" | "write_file" | "write_to_file" | "create_file" | "download" => "Write",
        "glob" | "list_directory" | "ls" | "list" | "list_files" | "find_files" | "file_search"
        | "find" => "Glob",
        "grep"
        | "grep_search"
        | "search_file_content"
        | "search_files"
        | "codebase_search"
        | "sourcegraph"
        | "ripgrep"
        | "search" => "Grep",
        "webfetch" | "web_fetch" | "fetch" | "agentic_fetch" | "fetch_url" | "read_url" => {
            "WebFetch"
        }
        "websearch" | "web_search" | "google_web_search" | "web_search_call" => "WebSearch",
        "agent" | "task" | "spawn_agent" | "new_task" | "subagent" | "dispatch_agent"
        | "create_sub_session" | "delegate" => "Agent",
        "todowrite"
        | "todoread"
        | "todo_write"
        | "todos"
        | "write_todos"
        | "update_plan"
        | "taskcreate"
        | "taskupdate"
        | "tasklist"
        | "taskget"
        | "task_create"
        | "task_update"
        | "task_list"
        | "update_todo_list"
        | "tracker_create_task"
        | "tracker_update_task"
        | "tracker_list_tasks"
        | "tracker_get_task" => "Todo",
        "skill" | "activate_skill" | "use_skill" => "Skill",
        "askuserquestion"
        | "ask_user"
        | "ask_user_question"
        | "request_user_input"
        | "request_user_input_async"
        | "question"
        | "ask_followup_question" => "AskUser",
        _ => "Other",
    }
}

/// Lê um arquivo de texto inteiro, descomprimindo `.zst`.
pub fn read_maybe_zst(path: &Path) -> Option<Vec<u8>> {
    let is_zst = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("zst") || e.eq_ignore_ascii_case("zstd"))
        .unwrap_or(false);
    let f = std::fs::File::open(path).ok()?;
    if is_zst {
        let mut d = zstd::stream::read::Decoder::new(f).ok()?;
        let mut buf = Vec::new();
        d.read_to_end(&mut buf).ok()?;
        Some(buf)
    } else {
        let mut buf = Vec::new();
        BufReader::new(f).read_to_end(&mut buf).ok()?;
        Some(buf)
    }
}

/// Percorre as linhas completas de um JSONL a partir de `from` (bytes).
/// Devolve o offset logo depois da última linha terminada em `\n`: uma linha
/// que a ferramenta ainda está escrevendo fica para a próxima passada.
/// `.zst` é sempre lido do começo e devolve o tamanho descomprimido.
pub fn for_each_line(path: &Path, from: u64, mut f: impl FnMut(&[u8])) -> Option<u64> {
    let is_zst = path.extension().map(|e| e == "zst").unwrap_or(false);
    if is_zst {
        let buf = read_maybe_zst(path)?;
        let mut consumed = 0u64;
        for line in buf.split_inclusive(|b| *b == b'\n') {
            if line.last() == Some(&b'\n') {
                f(trim_line(line));
                consumed += line.len() as u64;
            } else {
                // Arquivo comprimido está completo: a última linha vale.
                f(trim_line(line));
                consumed += line.len() as u64;
            }
        }
        return Some(consumed);
    }
    let mut file = std::fs::File::open(path).ok()?;
    if from > 0 {
        file.seek(SeekFrom::Start(from)).ok()?;
    }
    let mut r = BufReader::with_capacity(1 << 20, file);
    let mut offset = from;
    let mut line = Vec::with_capacity(8192);
    loop {
        line.clear();
        let n = r.read_until(b'\n', &mut line).ok()?;
        if n == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            break; // parcial
        }
        offset += n as u64;
        f(trim_line(&line));
    }
    Some(offset)
}

fn trim_line(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }
    &line[..end]
}

/// Lê as primeiras `max_lines` linhas (até `max_bytes`).
pub fn head_lines(path: &Path, max_lines: usize, max_bytes: usize) -> Vec<String> {
    let Ok(f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let is_zst = path.extension().map(|e| e == "zst").unwrap_or(false);
    let reader: Box<dyn Read> = if is_zst {
        match zstd::stream::read::Decoder::new(f) {
            Ok(d) => Box::new(d),
            Err(_) => return Vec::new(),
        }
    } else {
        Box::new(f)
    };
    let mut r = BufReader::new(reader.take(max_bytes as u64));
    let mut out = Vec::new();
    let mut buf = String::new();
    while out.len() < max_lines {
        buf.clear();
        match r.read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if buf.ends_with('\n') {
                    out.push(buf.trim_end().to_string());
                } else {
                    break;
                }
            }
        }
    }
    out
}

/// Linhas completas dos últimos `bytes` do arquivo (a primeira, cortada, cai).
pub fn tail_lines(path: &Path, bytes: u64) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = size.saturating_sub(bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

/// Aspas de shell: POSIX com aspas simples; Windows com aspas duplas.
pub fn shell_quote(s: &str) -> String {
    if cfg!(windows) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// `cd <dir> && <cmd>` quando há diretório.
pub fn in_dir(dir: Option<&str>, cmd: String) -> String {
    match dir {
        Some(d) if !d.is_empty() => format!("cd {} && {}", shell_quote(d), cmd),
        _ => cmd,
    }
}

/// Prefixo de variável de ambiente para a conta isolada.
pub fn with_env(var: &str, value: Option<&Path>, cmd: String) -> String {
    match value {
        Some(v) if !v.as_os_str().is_empty() => {
            let q = shell_quote(&v.to_string_lossy());
            if cfg!(windows) {
                format!("set {var}={q} && {cmd}")
            } else {
                format!("{var}={q} {cmd}")
            }
        }
        _ => cmd,
    }
}

/// Nome do diretório de projeto do Claude Code para um cwd: todo caractere
/// não alfanumérico vira `-`; acima de 200 caracteres corta e junta um hash
/// (`sanitizePath` do CLI).
pub fn claude_project_dir_name(cwd: &str) -> String {
    let s: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if s.chars().count() <= 200 {
        return s;
    }
    let head: String = s.chars().take(200).collect();
    format!("{}-{}", head, js_string_hash_base36(cwd))
}

/// Hash de string do JS (`h = (h << 5) - h + c | 0` sobre UTF-16), valor
/// absoluto em base 36.
fn js_string_hash_base36(s: &str) -> String {
    let mut h: i32 = 0;
    for u in s.encode_utf16() {
        h = h.wrapping_shl(5).wrapping_sub(h).wrapping_add(u as i32);
    }
    let mut n = (h as i64).unsigned_abs();
    if n == 0 {
        return "0".into();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while n > 0 {
        out.push(digits[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Busca `needle` (já em minúsculas ASCII) em bytes, ignorando caixa ASCII.
pub fn bytes_contains_ci(hay: &[u8], needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if hay.len() < needle_lower.len() {
        return false;
    }
    let first = needle_lower[0];
    let first_up = first.to_ascii_uppercase();
    let last = hay.len() - needle_lower.len();
    let mut i = 0;
    while i <= last {
        let b = hay[i];
        if b == first || b == first_up {
            let mut ok = true;
            for (j, nb) in needle_lower.iter().enumerate().skip(1) {
                if hay[i + j].to_ascii_lowercase() != *nb {
                    ok = false;
                    break;
                }
            }
            if ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Abre um SQLite de outra ferramenta só para leitura.
pub fn open_ro(path: &Path) -> Option<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    let c = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let _ = c.busy_timeout(std::time::Duration::from_millis(1500));
    Some(c)
}

/// Colunas de uma tabela (vazio se não existe).
pub fn table_columns(c: &rusqlite::Connection, table: &str) -> Vec<String> {
    let Ok(mut st) = c.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "")
    )) else {
        return Vec::new();
    };
    st.query_map([], |r| r.get::<_, String>(1))
        .map(|it| it.filter_map(|x| x.ok()).collect())
        .unwrap_or_default()
}

/// `SELECT * ...` como objetos JSON por nome de coluna: resiste a mudança de
/// esquema entre versões da ferramenta.
pub fn query_json(
    c: &rusqlite::Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let Ok(mut st) = c.prepare(sql) else {
        return Vec::new();
    };
    let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
    let rows = st.query_map(params, |r| {
        let mut m = serde_json::Map::new();
        for (i, n) in names.iter().enumerate() {
            use rusqlite::types::ValueRef;
            let v = match r.get_ref(i)? {
                ValueRef::Null => serde_json::Value::Null,
                ValueRef::Integer(x) => serde_json::Value::from(x),
                ValueRef::Real(x) => serde_json::Value::from(x),
                ValueRef::Text(t) => {
                    serde_json::Value::String(String::from_utf8_lossy(t).to_string())
                }
                ValueRef::Blob(b) => {
                    serde_json::Value::String(String::from_utf8_lossy(b).to_string())
                }
            };
            m.insert(n.clone(), v);
        }
        Ok(m)
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Uma coluna TEXT com JSON dentro vira `Value` (ou o próprio texto).
pub fn json_col(m: &serde_json::Map<String, serde_json::Value>, key: &str) -> serde_json::Value {
    match m.get(key) {
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str(s).unwrap_or_else(|_| serde_json::Value::String(s.clone()))
        }
        Some(v) => v.clone(),
        None => serde_json::Value::Null,
    }
}

pub fn map_str(m: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match m.get(key) {
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

pub fn map_i64(m: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<i64> {
    match m.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

pub fn map_f64(m: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<f64> {
    match m.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

/// Instante (ms) de uma coluna de data em qualquer formato.
pub fn map_ts(m: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<i64> {
    match m.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_f64().map(epoch_to_ms),
        Some(serde_json::Value::String(s)) => parse_ts_str(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_cover_the_group_a_tools() {
        assert_eq!(canonical_tool("Bash"), "Bash");
        assert_eq!(canonical_tool("exec_command"), "Bash");
        assert_eq!(canonical_tool("run_shell_command"), "Bash");
        assert_eq!(canonical_tool("apply_patch"), "Edit");
        assert_eq!(canonical_tool("replace"), "Edit");
        assert_eq!(canonical_tool("view"), "Read");
        assert_eq!(canonical_tool("Task"), "Agent");
        assert_eq!(canonical_tool("spawn_agent"), "Agent");
        assert_eq!(canonical_tool("mcp__github__create_issue"), "Mcp");
        assert_eq!(canonical_tool("developer__shell"), "Bash");
        assert_eq!(canonical_tool("slack__post"), "Mcp");
        assert_eq!(canonical_tool("TaskCreate"), "Todo");
        assert_eq!(canonical_tool("google_web_search"), "WebSearch");
        assert_eq!(canonical_tool("whatever"), "Other");
    }

    #[test]
    fn claude_dir_name_matches_the_cli() {
        assert_eq!(
            claude_project_dir_name("/Users/tonho/Documents/projetos/omniget"),
            "-Users-tonho-Documents-projetos-omniget"
        );
        assert_eq!(claude_project_dir_name("/a/b.c_d"), "-a-b-c-d");
        let long = format!("/{}", "x".repeat(300));
        let enc = claude_project_dir_name(&long);
        assert!(enc.len() > 200 && enc.starts_with("-xxx"));
    }

    #[test]
    fn timestamps_in_every_unit() {
        assert_eq!(
            parse_ts_str("2026-03-04T12:00:00.000Z"),
            Some(1772625600000)
        );
        assert_eq!(parse_ts_str("1772625600"), Some(1772625600000));
        assert_eq!(parse_ts_str("1772625600000"), Some(1772625600000));
        assert_eq!(parse_ts_str("2026-03-04 12:00:00"), Some(1772625600000));
    }

    #[test]
    fn ci_search_on_bytes() {
        assert!(bytes_contains_ci(b"Hello World", b"world"));
        assert!(!bytes_contains_ci(b"Hello", b"world"));
    }
}
