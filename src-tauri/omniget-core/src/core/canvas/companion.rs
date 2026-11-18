use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::term::agent::AgentState;

pub const HISTORY_CAP: usize = 120;
pub const SUMMARY_MAX_CHARS: usize = 280;
pub const SUGGESTION_MAX_CHARS: usize = 200;
pub const SCREEN_MAX_CHARS: usize = 4_000;
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionKind {
    Done,
    Blocked,
    Failed,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelMode {
    #[default]
    Auto,
    Local,
    Cheap,
    Heuristic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompanionSettings {
    pub enabled: bool,
    pub min_work_secs: u64,
    pub model_mode: ModelMode,
    pub ollama_url: String,
    pub ollama_model: Option<String>,
    pub cheap_agent_id: Option<String>,
    pub pet_balloon: bool,
    pub on_done: bool,
    pub on_blocked: bool,
    pub on_failed: bool,
    pub language: Option<String>,
    pub cli_fallback: bool,
    pub cli_agent: Option<String>,
}

impl Default for CompanionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_work_secs: 60,
            model_mode: ModelMode::Auto,
            ollama_url: DEFAULT_OLLAMA_URL.into(),
            ollama_model: None,
            cheap_agent_id: None,
            pet_balloon: true,
            on_done: true,
            on_blocked: true,
            on_failed: true,
            language: None,
            cli_fallback: true,
            cli_agent: None,
        }
    }
}

impl CompanionSettings {
    pub fn sanitized(mut self) -> Self {
        self.min_work_secs = self.min_work_secs.clamp(5, 3_600);
        if self.ollama_url.trim().is_empty() {
            self.ollama_url = DEFAULT_OLLAMA_URL.into();
        }
        self.ollama_model = self
            .ollama_model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        self.cheap_agent_id = self
            .cheap_agent_id
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        self.cli_agent = self
            .cli_agent
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| CLI_AGENTS.contains(&m.as_str()));
        self.language = self
            .language
            .map(|l| l.trim().chars().take(12).collect::<String>())
            .filter(|l| !l.is_empty());
        self
    }

    pub fn wants(&self, kind: CompanionKind) -> bool {
        self.enabled
            && match kind {
                CompanionKind::Done => self.on_done,
                CompanionKind::Blocked => self.on_blocked,
                CompanionKind::Failed => self.on_failed,
                CompanionKind::Manual => true,
            }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionEntry {
    pub id: String,
    pub node_id: String,
    pub workspace_id: String,
    pub floor_id: String,
    pub terminal_id: String,
    pub name: String,
    pub agent: String,
    pub kind: CompanionKind,
    pub summary: String,
    pub suggestions: Vec<String>,
    #[serde(default)]
    pub command: Option<String>,
    pub source: String,
    pub at_ms: i64,
    pub worked_ms: i64,
    pub celebrate: bool,
    #[serde(default)]
    pub dismissed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i18n: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub summary: String,
    pub suggestions: Vec<String>,
    pub command: Option<String>,
    pub i18n: Option<Value>,
}

#[derive(Debug, Clone, Default)]
struct Track {
    work_started_ms: Option<i64>,
    last: Option<AgentState>,
    fired_blocked: bool,
}

pub fn is_working(s: AgentState) -> bool {
    matches!(
        s,
        AgentState::Thinking | AgentState::Working | AgentState::Compacting | AgentState::Looping
    )
}

#[derive(Debug, Default)]
pub struct Tracker {
    nodes: HashMap<String, Track>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(
        &mut self,
        key: &str,
        state: AgentState,
        now_ms: i64,
        min_work_ms: i64,
    ) -> Option<(CompanionKind, i64)> {
        let t = self.nodes.entry(key.to_string()).or_default();
        let prev = t.last;
        t.last = Some(state);
        let worked = |t: &Track| t.work_started_ms.map(|s| (now_ms - s).max(0)).unwrap_or(0);
        match state {
            s if is_working(s) => {
                if t.work_started_ms.is_none() {
                    t.work_started_ms = Some(now_ms);
                }
                t.fired_blocked = false;
                None
            }
            AgentState::WaitingPeer => None,
            AgentState::Blocked => {
                if t.fired_blocked {
                    return None;
                }
                t.fired_blocked = true;
                Some((CompanionKind::Blocked, worked(t)))
            }
            AgentState::Failed => {
                let w = worked(t);
                t.work_started_ms = None;
                t.fired_blocked = false;
                (prev != Some(AgentState::Failed)).then_some((CompanionKind::Failed, w))
            }
            AgentState::Success | AgentState::Idle => {
                let w = worked(t);
                let had_work = t.work_started_ms.take().is_some();
                t.fired_blocked = false;
                (had_work && w >= min_work_ms).then_some((CompanionKind::Done, w))
            }
            _ => None,
        }
    }
}

fn is_chrome(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return true;
    }
    if t.chars().all(|c| {
        matches!(
            c,
            '─' | '━'
                | '│'
                | '┃'
                | '╭'
                | '╮'
                | '╰'
                | '╯'
                | '┌'
                | '┐'
                | '└'
                | '┘'
                | '-'
                | '='
                | '_'
                | '·'
                | ' '
                | '>'
                | '❯'
                | '›'
        )
    }) {
        return true;
    }
    if t.starts_with(['⚠', '⏵', '✻', '✽', '✢', '·']) || t == "❯" || t == ">" {
        return true;
    }
    let lower = t.to_lowercase();
    [
        "? for shortcuts",
        "esc to interrupt",
        "ctrl+c to",
        "shift+tab to",
        "bypass permissions",
        "auto-accept edits",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

fn strip_frame(line: &str) -> String {
    line.trim()
        .trim_start_matches(['│', '┃', '|'])
        .trim_end_matches(['│', '┃', '|'])
        .trim()
        .to_string()
}

pub fn clean_screen(text: &str, max_chars: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        if is_chrome(raw) {
            continue;
        }
        let line = strip_frame(raw);
        if line.is_empty() || out.last() == Some(&line) {
            continue;
        }
        out.push(line);
    }
    let mut total = 0usize;
    let mut kept: Vec<&String> = Vec::new();
    for l in out.iter().rev() {
        let n = l.chars().count() + 1;
        if total + n > max_chars {
            break;
        }
        total += n;
        kept.push(l);
    }
    kept.reverse();
    kept.into_iter().cloned().collect::<Vec<_>>().join("\n")
}

fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn is_shell(agent: &str) -> bool {
    matches!(agent, "shell" | "" | "bash" | "zsh" | "fish" | "pwsh")
}

fn language_name(lang: Option<&str>) -> &'static str {
    match lang
        .unwrap_or("en")
        .split(['-', '_'])
        .next()
        .unwrap_or("en")
    {
        "pt" => "Brazilian Portuguese",
        "es" => "Spanish",
        "fr" => "French",
        "it" => "Italian",
        "ja" => "Japanese",
        "zh" => "Chinese",
        "ru" => "Russian",
        "el" => "Greek",
        "fa" => "Persian",
        _ => "English",
    }
}

pub fn prompt(
    kind: CompanionKind,
    name: &str,
    agent: &str,
    role: Option<&str>,
    worked_ms: i64,
    screen: &str,
    lang: Option<&str>,
) -> (String, String) {
    let focus = if is_shell(agent) {
        "This is a plain shell: the suggestions are shell commands to run next."
    } else {
        "This is a coding agent: the suggestions are short instructions to send it, focused on verification (review the change, run the tests, build, try it by hand) or on unblocking it."
    };
    let system = format!(
        "You are Omni, a small companion that watches AI coding agents on a canvas while the user does something else. \
Read the end of one agent's terminal and reply with ONE JSON object and nothing else: \
{{\"summary\": string, \"suggestions\": [string, string, string], \"command\": string}}. \
summary: one or two short sentences that name the agent ({name}) and say what happened; no markdown. \
suggestions: exactly three next steps, each under 15 words, written as the user would type them to the agent. \
command: the single best next instruction (may repeat one suggestion). {focus} \
Never invent results you cannot see on the screen. Write in {}.",
        language_name(lang)
    );
    let what = match kind {
        CompanionKind::Done => "finished its turn",
        CompanionKind::Blocked => "is waiting for the user (permission or question)",
        CompanionKind::Failed => "failed",
        CompanionKind::Manual => "is being checked by the user",
    };
    let role = role.map(|r| format!(" Role: {r}.")).unwrap_or_default();
    let user = format!(
        "Agent \"{name}\" ({agent}){role} {what} after {} of work.\nEnd of its terminal:\n```\n{screen}\n```",
        human_duration(worked_ms)
    );
    (system, user)
}

pub fn human_duration(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

fn first_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in text[start..].char_indices() {
        if in_str {
            match c {
                _ if esc => esc = false,
                '\\' => esc = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse_reply(text: &str, fallback: &Summary) -> Option<Summary> {
    let v: Value = serde_json::from_str(first_object(text)?).ok()?;
    let summary = clip(v["summary"].as_str()?.trim(), SUMMARY_MAX_CHARS);
    if summary.is_empty() {
        return None;
    }
    let mut suggestions: Vec<String> = v["suggestions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .map(|s| clip(s.trim(), SUGGESTION_MAX_CHARS))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    suggestions.dedup();
    for s in &fallback.suggestions {
        if suggestions.len() >= 3 {
            break;
        }
        if !suggestions.contains(s) {
            suggestions.push(s.clone());
        }
    }
    suggestions.truncate(3);
    let command = v["command"]
        .as_str()
        .map(|c| clip(c.trim(), SUGGESTION_MAX_CHARS))
        .filter(|c| !c.is_empty());
    Some(Summary {
        summary,
        suggestions,
        command,
        i18n: None,
    })
}

fn pt(lang: Option<&str>) -> bool {
    lang.is_some_and(|l| l.starts_with("pt"))
}

const ERROR_MARKERS: &[&str] = &[
    "error",
    "failed",
    "failure",
    "panic",
    "exception",
    "traceback",
    "fatal",
    "denied",
    "not found",
    "no such file",
    "erro",
    "falhou",
    "falha",
    "segmentation fault",
];

pub fn refine_kind(kind: CompanionKind, screen: &str) -> CompanionKind {
    if kind != CompanionKind::Failed {
        return kind;
    }
    let tail: String = screen
        .lines()
        .rev()
        .take(15)
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    if ERROR_MARKERS.iter().any(|m| tail.contains(m)) {
        kind
    } else {
        CompanionKind::Done
    }
}

pub fn last_command(screen: &str) -> Option<String> {
    screen.lines().rev().find_map(|l| {
        let l = l.trim_end();
        for p in ["$ ", "% ", "❯ ", "> "] {
            if let Some(i) = l.rfind(p) {
                let cmd = l[i + p.len()..].trim();
                if !cmd.is_empty() && cmd.len() <= 200 {
                    return Some(cmd.to_string());
                }
            }
        }
        None
    })
}

pub fn heuristic(
    kind: CompanionKind,
    name: &str,
    agent: &str,
    worked_ms: i64,
    screen: &str,
    lang: Option<&str>,
) -> Summary {
    let kind = refine_kind(kind, screen);
    let pt = pt(lang);
    let last = screen
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.chars().filter(|c| c.is_alphanumeric()).count() >= 8)
        .map(|l| clip(l, 140));
    let dur = human_duration(worked_ms);
    let (head, head_key) = match (kind, pt) {
        (CompanionKind::Done, false) => (format!("{name} finished after {dur}."), "done"),
        (CompanionKind::Done, true) => (format!("{name} terminou depois de {dur}."), "done"),
        (CompanionKind::Blocked, false) => (format!("{name} is waiting for you."), "blocked"),
        (CompanionKind::Blocked, true) => (format!("{name} está esperando você."), "blocked"),
        (CompanionKind::Failed, false) => (format!("{name} hit an error."), "failed"),
        (CompanionKind::Failed, true) => (format!("{name} encontrou um erro."), "failed"),
        (CompanionKind::Manual, false) => (format!("Here is where {name} is."), "manual"),
        (CompanionKind::Manual, true) => (format!("Veja onde {name} está."), "manual"),
    };
    let summary = match &last {
        Some(l) => clip(
            &format!(
                "{head} {} “{l}”",
                if pt { "Última linha:" } else { "Last line:" }
            ),
            SUMMARY_MAX_CHARS,
        ),
        None => head,
    };
    let shell = is_shell(agent);
    let last_cmd = shell.then(|| last_command(screen)).flatten();
    let steps: Vec<(String, String, String)> = if shell {
        let mut v: Vec<(String, String, String)> = Vec::new();
        if let Some(c) = &last_cmd {
            v.push(("cmd".into(), c.clone(), c.clone()));
        }
        let extra: &[&str] = if kind == CompanionKind::Failed {
            &["git status", "ls -la", "git diff --stat"]
        } else {
            &["git status", "git diff --stat", "git log --oneline -5"]
        };
        for c in extra {
            if v.len() < 3 && !v.iter().any(|x| x.1 == *c) {
                v.push(("cmd".into(), c.to_string(), c.to_string()));
            }
        }
        v
    } else {
        let t: [(&str, &str, &str); 3] = match kind {
            CompanionKind::Done | CompanionKind::Manual => [
                (
                    "review",
                    "Summarize what you changed and why",
                    "Resuma o que você mudou e por quê",
                ),
                (
                    "tests",
                    "Run the tests and fix any failure",
                    "Rode os testes e corrija o que falhar",
                ),
                (
                    "build",
                    "Build the project and report errors",
                    "Compile o projeto e mostre os erros",
                ),
            ],
            CompanionKind::Blocked => [
                (
                    "need",
                    "Explain what you need from me",
                    "Explique o que você precisa de mim",
                ),
                (
                    "safest",
                    "Go ahead with the safest option",
                    "Siga com a opção mais segura",
                ),
                (
                    "pause",
                    "Stop and summarize the progress so far",
                    "Pare e resuma o progresso até aqui",
                ),
            ],
            CompanionKind::Failed => [
                (
                    "explain",
                    "Explain the error and propose a fix",
                    "Explique o erro e proponha uma correção",
                ),
                (
                    "retry",
                    "Retry the last step",
                    "Tente o último passo de novo",
                ),
                (
                    "output",
                    "Show the full error output",
                    "Mostre a saída completa do erro",
                ),
            ],
        };
        t.iter()
            .map(|(k, en, p)| (k.to_string(), en.to_string(), p.to_string()))
            .collect()
    };
    let i18n = json!({
        "summary": {
            "key": format!("canvas.companion.h.{head_key}{}", if last.is_some() { "_last" } else { "" }),
            "vars": { "name": name, "duration": dur, "last": last.clone().unwrap_or_default() },
        },
        "suggestions": steps
            .iter()
            .map(|(k, en, _)| if k == "cmd" {
                json!({ "text": en })
            } else {
                json!({ "key": format!("canvas.companion.s.{k}"), "text": en })
            })
            .collect::<Vec<_>>(),
    });
    Summary {
        summary,
        suggestions: steps
            .into_iter()
            .map(|(_, en, p)| if pt { p } else { en })
            .collect(),
        command: last_cmd,
        i18n: Some(i18n),
    }
}

pub async fn ollama_models(base: &str) -> Result<Vec<(String, u64)>, String> {
    let client = crate::core::tools::client().map_err(|e| e.to_string())?;
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let v: Value = client
        .get(&url)
        .timeout(Duration::from_millis(1_500))
        .send()
        .await
        .map_err(|e| format!("ollama: {e}"))?
        .json()
        .await
        .map_err(|e| format!("ollama: {e}"))?;
    Ok(v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m["name"]
                .as_str()
                .or_else(|| m["model"].as_str())?
                .to_string();
            let lower = name.to_lowercase();
            let family = m["details"]["family"].as_str().unwrap_or("").to_lowercase();
            if lower.contains("embed") || family.contains("bert") {
                return None;
            }
            Some((name, m["size"].as_u64().unwrap_or(u64::MAX)))
        })
        .collect())
}

pub fn pick_model(models: &[(String, u64)], pinned: Option<&str>) -> Option<String> {
    if let Some(p) = pinned {
        if let Some((n, _)) = models
            .iter()
            .find(|(n, _)| n == p || n.split(':').next() == Some(p))
        {
            return Some(n.clone());
        }
    }
    models
        .iter()
        .min_by_key(|(_, s)| *s)
        .map(|(n, _)| n.clone())
}

pub async fn ollama_generate(
    base: &str,
    pinned: Option<&str>,
    system: &str,
    user: &str,
) -> Result<(String, String), String> {
    let models = ollama_models(base).await?;
    let model =
        pick_model(&models, pinned).ok_or_else(|| "ollama: no model installed".to_string())?;
    let client = crate::core::tools::client().map_err(|e| e.to_string())?;
    let url = format!("{}/api/chat", base.trim_end_matches('/'));
    let mut body = json!({
        "model": model,
        "stream": false,
        "format": "json",
        "think": false,
        "keep_alive": "5m",
        "options": { "temperature": 0.2, "num_predict": 320 },
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });
    let send = |body: &Value| client.post(&url).json(body).timeout(OLLAMA_TIMEOUT).send();
    let mut response = send(&body).await.map_err(|e| format!("ollama: {e}"))?;
    if response.status() == reqwest::StatusCode::BAD_REQUEST {
        // WHY: older servers and models without a thinking mode refuse the `think` field.
        if let Some(map) = body.as_object_mut() {
            map.remove("think");
        }
        response = send(&body).await.map_err(|e| format!("ollama: {e}"))?;
    }
    let v: Value = response
        .error_for_status()
        .map_err(|e| format!("ollama: {e}"))?
        .json()
        .await
        .map_err(|e| format!("ollama: {e}"))?;
    let text = v["message"]["content"].as_str().unwrap_or("").to_string();
    if text.trim().is_empty() {
        return Err("ollama: empty reply".into());
    }
    Ok((model, text))
}

pub const CLI_AGENTS: &[&str] = &["claude", "codex", "gemini"];
const CLI_TIMEOUT: Duration = Duration::from_secs(45);

pub fn cli_label(agent: &str) -> &'static str {
    match agent {
        "claude" => "via Claude (Haiku)",
        "codex" => "via Codex (low effort)",
        "gemini" => "via Gemini (Flash)",
        _ => "via agent CLI",
    }
}

pub fn pick_cli(pinned: Option<&str>, used: &[String]) -> Option<(String, std::path::PathBuf)> {
    use crate::core::clitools::exec::which;
    if let Some(p) = pinned {
        return which(p).map(|b| (p.to_string(), b));
    }
    used.iter()
        .map(String::as_str)
        .chain(CLI_AGENTS.iter().copied())
        .filter(|a| CLI_AGENTS.contains(a))
        .find_map(|a| which(a).map(|b| (a.to_string(), b)))
}

pub fn cli_args(agent: &str, system: &str, out: &std::path::Path, prompt: &str) -> Vec<String> {
    match agent {
        "claude" => vec![
            "-p".into(),
            "--model".into(),
            "haiku".into(),
            "--output-format".into(),
            "json".into(),
            "--tools".into(),
            "".into(),
            "--no-session-persistence".into(),
            "--strict-mcp-config".into(),
            "--system-prompt".into(),
            system.into(),
        ],
        "codex" => vec![
            "exec".into(),
            "--skip-git-repo-check".into(),
            "--ephemeral".into(),
            "-s".into(),
            "read-only".into(),
            "-c".into(),
            "model_reasoning_effort=\"low\"".into(),
            "-o".into(),
            out.to_string_lossy().into_owned(),
            "-".into(),
        ],
        _ => vec![
            "-m".into(),
            "gemini-2.5-flash".into(),
            "-p".into(),
            format!("{system}\n\n{prompt}"),
        ],
    }
}

pub fn cli_text(agent: &str, stdout: &str) -> String {
    if agent == "claude" {
        if let Some(v) = first_object(stdout).and_then(|o| serde_json::from_str::<Value>(o).ok()) {
            if let Some(r) = v["result"].as_str() {
                return r.to_string();
            }
        }
    }
    stdout.trim().to_string()
}

pub async fn cli_generate(
    agent: &str,
    bin: &std::path::Path,
    system: &str,
    user: &str,
) -> Result<(String, String), String> {
    use tokio::io::AsyncWriteExt;
    let dir = std::env::temp_dir().join(format!("omniget-omni-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join("last.txt");
    let prompt = if agent == "codex" {
        format!("{system}\n\n{user}")
    } else {
        user.to_string()
    };
    let path = std::env::join_paths(crate::core::clitools::exec::search_dirs(&[]))
        .ok()
        .unwrap_or_default();
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(cli_args(agent, system, &out, &prompt))
        .current_dir(&dir)
        .env("PATH", path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("{agent}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        if agent != "gemini" {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        }
        drop(stdin);
    }
    let res = tokio::time::timeout(CLI_TIMEOUT, child.wait_with_output()).await;
    let text = match res {
        Err(_) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("{agent}: timeout"));
        }
        Ok(Err(e)) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("{agent}: {e}"));
        }
        Ok(Ok(o)) => {
            let file = std::fs::read_to_string(&out).unwrap_or_default();
            if !file.trim().is_empty() {
                file
            } else if o.status.success() {
                cli_text(agent, &String::from_utf8_lossy(&o.stdout))
            } else {
                let _ = std::fs::remove_dir_all(&dir);
                let err = String::from_utf8_lossy(&o.stderr);
                return Err(format!("{agent}: {}", clip(err.trim(), 300)));
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    if text.trim().is_empty() {
        return Err(format!("{agent}: empty reply"));
    }
    Ok((cli_label(agent).to_string(), text))
}

pub const OMNI_NOTES_TITLE: &str = "Omni Notes";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskIntent {
    Note(String),
    SummarizeNotes,
    Free,
}

pub fn question_language(question: &str) -> Option<&'static str> {
    let low = format!(" {} ", question.to_lowercase());
    const PT: &[&str] = &[
        " anote", " anota", " resuma", " resumo", " minhas", " meus", " você", " voce", " está",
        " estão", " não", " o que ", " quem ", " qual ", " quais ", " fazendo", " lembre",
        " notas", " falta", " agora", " já ", " também", " porque", " por que", " como ",
    ];
    let hits = PT.iter().filter(|w| low.contains(*w)).count();
    let marks = low.chars().filter(|c| matches!(c, 'ã' | 'õ' | 'ç')).count();
    (hits >= 1 || marks >= 1).then_some("pt")
}

pub fn ask_intent(question: &str) -> AskIntent {
    let q = question.trim();
    let low = q.to_lowercase();
    const NOTE: &[&str] = &[
        "anote que",
        "anota que",
        "anote:",
        "anote",
        "anota",
        "lembre que",
        "lembre-se que",
        "note that",
        "take a note that",
        "take a note:",
        "take a note",
        "make a note that",
        "make a note:",
        "write down that",
        "write down",
        "remember that",
        "remember:",
        "remember",
    ];
    for p in NOTE {
        if low.starts_with(p) {
            let rest = q
                .get(p.len()..)
                .unwrap_or("")
                .trim_start_matches([':', ',', ' ', '-'])
                .trim();
            if !rest.is_empty() {
                return AskIntent::Note(rest.to_string());
            }
        }
    }
    let summarize = ["resum", "summar", "sumari", "recap"];
    let notes = ["nota", "note"];
    if summarize.iter().any(|w| low.contains(w)) && notes.iter().any(|w| low.contains(w)) {
        return AskIntent::SummarizeNotes;
    }
    AskIntent::Free
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskTerminal {
    pub node_id: String,
    pub name: String,
    pub agent: String,
    pub floor: String,
    pub state: String,
    pub screen: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskNote {
    pub node_id: Option<String>,
    pub title: String,
    pub text: String,
}

fn is_pt(lang: Option<&str>) -> bool {
    lang.unwrap_or("en").starts_with("pt")
}

fn clip_raw(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn ask_context(
    workspace: &str,
    terminals: &[AskTerminal],
    notes: &[AskNote],
    max_chars: usize,
) -> String {
    let mut out = format!("Workspace: {workspace}\n");
    let per_term = if terminals.is_empty() {
        0
    } else {
        (max_chars * 2 / 3) / terminals.len()
    }
    .max(200);
    for t in terminals {
        out.push_str(&format!(
            "\n## Terminal \"{}\" (id {}, agent {}, floor {}, state {})\n{}\n",
            t.name,
            t.node_id,
            t.agent,
            t.floor,
            t.state,
            clip_raw(&t.screen, per_term)
        ));
    }
    let per_note = if notes.is_empty() {
        0
    } else {
        (max_chars / 3) / notes.len()
    }
    .max(120);
    for n in notes {
        out.push_str(&format!(
            "\n## Note \"{}\"{}\n{}\n",
            n.title,
            n.node_id
                .as_deref()
                .map(|i| format!(" (id {i})"))
                .unwrap_or_default(),
            clip_raw(&n.text, per_note)
        ));
    }
    clip_raw(&out, max_chars)
}

pub fn ask_prompt(question: &str, context: &str, lang: Option<&str>) -> (String, String) {
    let system = format!(
        "You are Omni, the companion of a canvas of coding agents and terminals. \
         Answer the user's question using only the canvas context given (terminal screens, states and notes). \
         Be short and concrete (at most 5 sentences). If the context does not say, answer that you cannot see it. \
         Write in {}. Reply with one JSON object: {{\"answer\": \"...\", \"sources\": [\"ids of the nodes you used\"]}}.",
        language_name(lang)
    );
    let user = format!("Canvas context:\n{context}\n\nQuestion: {question}");
    (system, user)
}

pub fn parse_answer(text: &str, known: &[String]) -> Option<(String, Vec<String>)> {
    let v: Value = first_object(text).and_then(|o| serde_json::from_str(o).ok())?;
    let answer = v["answer"].as_str()?.trim().to_string();
    if answer.is_empty() {
        return None;
    }
    let sources = v["sources"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| known.iter().any(|k| k == s))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some((answer, sources))
}

fn state_word(state: &str, pt: bool) -> &'static str {
    match (state, pt) {
        ("working" | "starting", false) => "working",
        ("working" | "starting", true) => "trabalhando",
        ("needs_you", false) => "needs you",
        ("needs_you", true) => "precisa de você",
        ("done", false) => "finished",
        ("done", true) => "terminou",
        ("stuck", false) => "stuck",
        ("stuck", true) => "travado",
        ("exited", false) => "stopped",
        ("exited", true) => "parado",
        (_, false) => "idle",
        (_, true) => "parado à espera",
    }
}

pub fn mentions(question_lower: &str, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return false;
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    question_lower.match_indices(&name).any(|(i, m)| {
        !word(question_lower[..i].chars().next_back())
            && !word(question_lower[i + m.len()..].chars().next())
    })
}

pub fn status_digest(question: &str, terminals: &[AskTerminal], lang: Option<&str>) -> String {
    let pt = is_pt(lang);
    if terminals.is_empty() {
        return if pt {
            "Não há terminais neste workspace.".into()
        } else {
            "There are no terminals in this workspace.".into()
        };
    }
    let q = question.to_lowercase();
    let mut ts: Vec<&AskTerminal> = terminals.iter().collect();
    ts.sort_by_key(|t| {
        let named = mentions(&q, &t.name);
        let urgent = matches!(t.state.as_str(), "needs_you" | "stuck");
        (!named, !urgent)
    });
    let mut lines = Vec::new();
    for t in ts.iter().take(8) {
        let last = t
            .screen
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| clip_raw(l, 120))
            .unwrap_or_default();
        let st = state_word(&t.state, pt);
        lines.push(if last.is_empty() {
            format!("{}: {st}.", t.name)
        } else {
            format!("{}: {st}. \"{last}\"", t.name)
        });
    }
    lines.join("\n")
}

pub fn notes_digest(notes: &[AskNote], lang: Option<&str>) -> String {
    let pt = is_pt(lang);
    if notes.is_empty() {
        return if pt {
            "Não há notas neste workspace.".into()
        } else {
            "There are no notes in this workspace.".into()
        };
    }
    let mut out = vec![if pt {
        format!("{} notas:", notes.len())
    } else {
        format!("{} notes:", notes.len())
    }];
    for n in notes.iter().take(20) {
        let body: String = n
            .text
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        out.push(if body.is_empty() {
            format!("- {}", n.title)
        } else {
            format!("- {}: {}", n.title, clip_raw(&body, 140))
        });
    }
    out.join("\n")
}

pub fn append_note(current: &str, item: &str) -> String {
    let item = item.trim().trim_end_matches('.');
    let bullet = format!("- {item}");
    if current.trim().is_empty() {
        return format!("# {OMNI_NOTES_TITLE}\n\n{bullet}\n");
    }
    let mut s = current.trim_end().to_string();
    s.push('\n');
    s.push_str(&bullet);
    s.push('\n');
    s
}
