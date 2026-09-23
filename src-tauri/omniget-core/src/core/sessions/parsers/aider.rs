//! Aider (estudo 06, Aider §j).
//!
//! Sem banco nem id de sessão: cada projeto tem `.aider.chat.history.md`
//! (Markdown só de acréscimo). Cada sessão começa em
//! `# aider chat started at YYYY-MM-DD HH:MM:SS`; prompt do usuário em linhas
//! `#### …`; saída de ferramenta/avisos em citação `> …` (inclui
//! `> Model: … with … edit format` e o relatório de uso
//! `> Tokens: 12k sent, 1.2k cache write, 8.0k cache hit, 350 received. Cost:
//! $0.02 message, $0.15 session.`); o resto é texto do assistente.
//! Opcional: `--analytics-log x.jsonl` (`AIDER_ANALYTICS_LOG` ou
//! `analytics-log:` no `~/.aider.conf.yml`), eventos
//! `{event:"message_send", properties{main_model, prompt_tokens,
//! completion_tokens, cost, total_cost}, time}`; `launched` abre sessão.
//!
//! Descoberta: `AIDER_CHAT_HISTORY_FILE`, `chat-history-file:` do
//! `~/.aider.conf.yml`, e `.aider.chat.history.md` em pastas de projeto
//! conhecidas (pastas abertas no VS Code/Cursor, projetos do Claude Code) e
//! nas raízes de código comuns (`~/code`, `~/projects`, `~/Documents/…`), até
//! três níveis. Id da sessão = `<hash do arquivo>@<início>`.
//! Aider não está instalado aqui: só fixture.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::util::*;
use crate::core::sessions::model::{Role, Session, SessionMeta, SessionSource, ToolStatus};

pub struct AiderSource;

const TOOL: &str = "aider";
const HISTORY: &str = ".aider.chat.history.md";

fn conf_value(key: &str) -> Option<String> {
    let conf = home_join(&[".aider.conf.yml"])?;
    let txt = std::fs::read_to_string(conf).ok()?;
    for line in txt.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix(key) {
            if let Some(v) = rest.trim_start().strip_prefix(':') {
                let v = v.trim().trim_matches(['"', '\'']);
                if !v.is_empty() {
                    return Some(expand_home(v));
                }
            }
        }
    }
    None
}

fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/"), home()) {
        (Some(rest), Some(h)) => h.join(rest).to_string_lossy().into_owned(),
        _ => p.to_string(),
    }
}

/// Pastas de projeto que outras ferramentas já conhecem.
fn known_project_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for (_, user) in vscode_user_dirs() {
        if let Ok(rd) = std::fs::read_dir(user.join("workspaceStorage")) {
            for e in rd.flatten() {
                if let Some(f) = workspace_folder(&e.path()) {
                    v.push(PathBuf::from(f));
                }
            }
        }
    }
    // ~/.claude/projects/<cwd com / trocado por ->: reconstrução pelo disco.
    if let Some(p) = home_join(&[".claude", "projects"]) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let name = file_name(&e.path());
                if let Some(dir) = super::cursor::resolve_slug(name.trim_start_matches('-')) {
                    v.push(PathBuf::from(dir));
                }
            }
        }
    }
    v
}

fn code_roots() -> Vec<PathBuf> {
    let Some(h) = home() else { return Vec::new() };
    [
        "code",
        "Code",
        "projects",
        "Projects",
        "projetos",
        "src",
        "dev",
        "Developer",
        "workspace",
        "repos",
        "git",
        "GitHub",
        "work",
        "Documents",
    ]
    .iter()
    .map(|d| h.join(d))
    .filter(|p| p.is_dir())
    .collect()
}

/// Todos os históricos encontrados, sem repetição.
pub fn history_files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |p: PathBuf| {
        if p.is_file() {
            let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if seen.insert(key) {
                out.push(p);
            }
        }
    };
    if let Some(p) = env_dir("AIDER_CHAT_HISTORY_FILE") {
        add(p);
    }
    if let Some(p) = conf_value("chat-history-file") {
        add(PathBuf::from(p));
    }
    for d in known_project_dirs() {
        add(d.join(HISTORY));
    }
    if let Some(h) = home() {
        add(h.join(HISTORY));
    }
    for r in code_roots() {
        for f in find_files(&r, 4, |p| file_name(p) == HISTORY) {
            add(f);
        }
    }
    out
}

fn analytics_files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = env_dir("AIDER_ANALYTICS_LOG") {
        v.push(p);
    }
    if let Some(p) = conf_value("analytics-log") {
        v.push(PathBuf::from(p));
    }
    dedup_paths(v).into_iter().filter(|p| p.is_file()).collect()
}

fn file_hash(p: &Path) -> String {
    let mut h = Sha256::new();
    h.update(p.to_string_lossy().as_bytes());
    hex::encode(&h.finalize()[..6])
}

/// `12k` → 12000, `1.2k` → 1200, `1.5M` → 1500000, `350` → 350.
fn parse_tokens(s: &str) -> u64 {
    let s = s.trim().replace(',', "");
    let (num, mult) = if let Some(n) = s.strip_suffix(['k', 'K']) {
        (n.to_string(), 1_000.0)
    } else if let Some(n) = s.strip_suffix(['m', 'M']) {
        (n.to_string(), 1_000_000.0)
    } else {
        (s.clone(), 1.0)
    };
    num.trim()
        .parse::<f64>()
        .map(|f| (f * mult).round() as u64)
        .unwrap_or(0)
}

/// `Tokens: 12k sent, 1.2k cache write, 8.0k cache hit, 350 received. Cost: $0.02 message, $0.15 session.`
fn parse_usage_line(line: &str) -> Option<(crate::core::sessions::model::TokenUsage, Option<f64>)> {
    let rest = line.trim().strip_prefix("Tokens:")?;
    let (tok, cost) = match rest.split_once("Cost:") {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    let mut us = crate::core::sessions::model::TokenUsage::default();
    for part in tok.trim().trim_end_matches('.').split(',') {
        let part = part.trim();
        let (n, label) = part.split_once(' ').unwrap_or((part, ""));
        let n = parse_tokens(n);
        match label.trim() {
            "sent" => us.input = n,
            "received" => us.output = n,
            "cache write" => us.cache_write = n,
            "cache hit" => us.cache_read = n,
            _ => {}
        }
    }
    // O "sent" do aider inclui o cache; separa para não contar duas vezes.
    us.input = us.input.saturating_sub(us.cache_read + us.cache_write);
    let cost = cost.and_then(|c| {
        let c = c.trim();
        let a = c.find('$')? + 1;
        let end = c[a..]
            .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
            .map(|e| a + e)
            .unwrap_or(c.len());
        c[a..end].parse::<f64>().ok()
    });
    Some((us, cost))
}

/// Sessões de um `.aider.chat.history.md`.
pub fn parse_history(txt: &str, path: &Path) -> Vec<Session> {
    let hash = file_hash(path);
    let project = path.parent().map(|p| p.to_string_lossy().into_owned());
    let mut sessions: Vec<Session> = Vec::new();
    let mut model: Option<String> = None;
    let mut in_user = false;
    for line in txt.lines() {
        if let Some(rest) = line.strip_prefix("# aider chat started at ") {
            // Horário local da máquina onde o aider rodou.
            let started_ms =
                chrono::NaiveDateTime::parse_from_str(rest.trim(), "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .and_then(|n| {
                        chrono::TimeZone::from_local_datetime(&chrono::Local, &n).earliest()
                    })
                    .map(|d| d.timestamp_millis());
            let started = started_ms.map(ms_to_rfc3339).unwrap_or_default();
            let stamp: String = rest.trim().chars().filter(|c| c.is_ascii_digit()).collect();
            let mut meta = new_meta(TOOL, format!("{hash}@{stamp}"), path);
            meta.project_path = project.clone();
            meta.started = Some(started).filter(|x| !x.is_empty());
            sessions.push(Session {
                meta,
                turns: Vec::new(),
            });
            model = None;
            in_user = false;
            continue;
        }
        if sessions.is_empty() {
            let mut meta = new_meta(TOOL, format!("{hash}@0"), path);
            meta.project_path = project.clone();
            sessions.push(Session {
                meta,
                turns: Vec::new(),
            });
        }
        let sess = sessions.last_mut().expect("existe");
        let ts = sess.meta.started.clone().unwrap_or_default();
        if let Some(p) = line.strip_prefix("####") {
            let p = p.strip_prefix(' ').unwrap_or(p);
            if in_user {
                if let Some(t) = sess.turns.last_mut() {
                    t.text.push('\n');
                    t.text.push_str(p);
                    continue;
                }
            }
            sess.turns.push(turn(Role::User, ts, p));
            in_user = true;
            continue;
        }
        in_user = false;
        if let Some(q) = line.strip_prefix('>') {
            let q = q.trim();
            if let Some(m) = q
                .strip_prefix("Main model:")
                .or_else(|| q.strip_prefix("Model:"))
            {
                model = m.trim().split_whitespace().next().map(str::to_string);
                continue;
            }
            if let Some((us, message_cost)) = parse_usage_line(q) {
                let need = !matches!(sess.turns.last(), Some(t) if t.role == Role::Assistant);
                if need {
                    sess.turns.push(turn(Role::Assistant, ts, ""));
                }
                let t = sess.turns.last_mut().expect("existe");
                t.usage.add(&us);
                t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + message_cost.unwrap_or(0.0));
                if t.model.is_none() {
                    t.model = model.clone();
                }
                continue;
            }
            // Aplicação de edição vira uma chamada Edit.
            if let Some(f) = q.strip_prefix("Applied edit to ") {
                let need = !matches!(sess.turns.last(), Some(t) if t.role == Role::Assistant);
                if need {
                    sess.turns.push(turn(Role::Assistant, ts, ""));
                }
                let mut c = tool_call("", "edit", serde_json::json!({ "path": f.trim() }));
                c.status = ToolStatus::Ok;
                sess.turns.last_mut().expect("existe").tool_calls.push(c);
            }
            continue;
        }
        if line.trim().is_empty()
            && !matches!(sess.turns.last(), Some(t) if t.role == Role::Assistant)
        {
            continue;
        }
        let need = !matches!(sess.turns.last(), Some(t) if t.role == Role::Assistant);
        if need {
            let mut t = turn(Role::Assistant, ts, "");
            t.model = model.clone();
            sess.turns.push(t);
        }
        let t = sess.turns.last_mut().expect("existe");
        if !t.text.is_empty() {
            t.text.push('\n');
        }
        t.text.push_str(line);
    }
    let mtime = mtime_ms(path);
    let n = sessions.len();
    for (i, s) in sessions.iter_mut().enumerate() {
        for t in s.turns.iter_mut() {
            t.text = t.text.trim().to_string();
            if t.model.is_none() && t.role == Role::Assistant {
                t.model = s.meta.models.first().cloned();
            }
        }
        // Só a última sessão do arquivo está "viva".
        s.meta.mtime_ms = if i + 1 == n {
            mtime
        } else {
            s.meta.started.as_deref().and_then(str_to_ms).unwrap_or(0)
        };
        let mut turns = std::mem::take(&mut s.turns);
        finalize(&mut s.meta, &mut turns);
        s.turns = turns;
    }
    sessions.retain(|s| !s.turns.is_empty());
    sessions
}

/// Sessões do log de analytics (JSONL), cortadas nos eventos `launched`.
pub fn parse_analytics(lines: &[Value], path: &Path) -> Vec<Session> {
    let hash = file_hash(path);
    let mut sessions: Vec<Session> = Vec::new();
    for l in lines {
        let ev = l.get("event").and_then(Value::as_str).unwrap_or("");
        let ts_ms_v = l.get("time").and_then(ts_ms);
        let ts = ts_ms_v.map(ms_to_rfc3339).unwrap_or_default();
        if ev == "launched" || sessions.is_empty() {
            let mut meta = new_meta(
                TOOL,
                format!("{hash}@a{}", ts_ms_v.unwrap_or(0) / 1000),
                path,
            );
            meta.account = Some("analytics".into());
            meta.started = Some(ts.clone()).filter(|x| !x.is_empty());
            sessions.push(Session {
                meta,
                turns: Vec::new(),
            });
            if ev == "launched" {
                continue;
            }
        }
        if ev != "message_send" {
            continue;
        }
        let p = l.get("properties").cloned().unwrap_or(Value::Null);
        let mut t = turn(Role::Assistant, ts, "");
        t.usage.input = u(&p, &["prompt_tokens"]);
        t.usage.output = u(&p, &["completion_tokens"]);
        t.cost_usd = fnum(&p, &["cost"]);
        t.model = s(&p, &["main_model"]);
        sessions.last_mut().expect("existe").turns.push(t);
    }
    for s in sessions.iter_mut() {
        let mut turns = std::mem::take(&mut s.turns);
        finalize(&mut s.meta, &mut turns);
        s.turns = turns;
    }
    sessions.retain(|s| !s.turns.is_empty());
    sessions
}

fn all_sessions() -> Vec<Session> {
    let mut out = Vec::new();
    for f in history_files() {
        if let Ok(txt) = std::fs::read_to_string(&f) {
            out.extend(parse_history(&txt, &f));
        }
    }
    for f in analytics_files() {
        out.extend(parse_analytics(&read_jsonl(&f), &f));
    }
    out
}

impl SessionSource for AiderSource {
    fn tool(&self) -> &'static str {
        TOOL
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = history_files();
        v.extend(analytics_files());
        v
    }

    fn list(&self) -> Vec<SessionMeta> {
        let mut out: Vec<SessionMeta> = all_sessions().into_iter().map(|s| s.meta).collect();
        sort_recent(&mut out);
        out
    }

    fn load(&self, id: &str) -> Option<Session> {
        all_sessions().into_iter().find(|s| s.meta.id == id)
    }

    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        let dir = meta.project_path.as_deref()?;
        Some(format!(
            "cd {} && aider --restore-chat-history",
            shell_quote(dir)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "
# aider chat started at 2026-09-20 10:00:00

> Aider v0.86.0
> Main model: claude-sonnet-4-5 with diff edit format, infinite output

#### adicione testes
#### para o parser

Claro, vou criar `tests/test_parser.py`.

> Applied edit to tests/test_parser.py
> Tokens: 12k sent, 1.2k cache write, 8.0k cache hit, 350 received. Cost: $0.02 message, $0.15 session.

# aider chat started at 2026-09-21 09:00:00

#### oi

Olá!

> Tokens: 900 sent, 30 received. Cost: $0.0010 message, $0.0010 session.
";

    #[test]
    fn history_fixture() {
        let s = parse_history(MD, Path::new("/proj/.aider.chat.history.md"));
        assert_eq!(s.len(), 2);
        let a = &s[0];
        assert_eq!(a.meta.project_path.as_deref(), Some("/proj"));
        assert_eq!(a.turns.len(), 2);
        assert_eq!(a.turns[0].text, "adicione testes\npara o parser");
        assert_eq!(a.turns[1].usage.cache_read, 8000);
        assert_eq!(a.turns[1].usage.cache_write, 1200);
        assert_eq!(a.turns[1].usage.input, 12000 - 9200);
        assert_eq!(a.turns[1].usage.output, 350);
        assert_eq!(a.turns[1].cost_usd, Some(0.02));
        assert_eq!(a.turns[1].model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(a.turns[1].tool_calls[0].name_canonical, "Edit");
        assert!(a.meta.started.is_some());
        assert!(a.meta.id.ends_with("@20260920100000"));
        assert_eq!(s[1].turns[1].usage.input, 900);
    }

    #[test]
    fn analytics_fixture() {
        let l: Vec<Value> = [
            r#"{"event":"launched","properties":{},"user_id":"u","time":1790000000}"#,
            r#"{"event":"message_send","properties":{"main_model":"gpt-5","edit_format":"diff","prompt_tokens":1000,"completion_tokens":100,"total_tokens":1100,"cost":0.01,"total_cost":0.01},"user_id":"u","time":1790000010}"#,
        ]
        .iter()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
        let s = parse_analytics(&l, Path::new("/x/a.jsonl"));
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].meta.usage.input, 1000);
        assert_eq!(s[0].meta.models, vec!["gpt-5".to_string()]);
    }
}
