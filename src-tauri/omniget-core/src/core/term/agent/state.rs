use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::term::osc::OscEvent;
use crate::core::term::session::foreground::Foreground;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Thinking,
    Working,
    WaitingPeer,
    Blocked,
    Success,
    Failed,
    Compacting,
    Looping,
    #[default]
    Unknown,
}

impl AgentState {
    pub fn code(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Thinking => 1,
            Self::Working => 2,
            Self::WaitingPeer => 3,
            Self::Blocked => 4,
            Self::Success => 5,
            Self::Failed => 6,
            Self::Compacting => 7,
            Self::Looping => 8,
            Self::Unknown => 9,
        }
    }

    fn sticky(self) -> bool {
        matches!(
            self,
            Self::Blocked | Self::WaitingPeer | Self::Failed | Self::Success
        )
    }

    fn busy(self) -> bool {
        matches!(
            self,
            Self::Thinking | Self::Working | Self::Compacting | Self::Looping
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalSource {
    #[default]
    None,
    Hook,
    Osc,
    Title,
    Heuristic,
    Exit,
}

impl SignalSource {
    pub fn code(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Hook => 1,
            Self::Osc => 2,
            Self::Title => 3,
            Self::Heuristic => 4,
            Self::Exit => 5,
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Heuristic => 1,
            Self::Osc | Self::Title => 2,
            Self::Hook => 3,
            Self::Exit => 4,
        }
    }

    fn hold(self) -> Duration {
        match self {
            Self::Hook => Duration::from_secs(10),
            Self::Osc | Self::Title => Duration::from_secs(4),
            _ => Duration::ZERO,
        }
    }

    fn index(self) -> usize {
        self.code() as usize
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSignal {
    pub state: AgentState,
    pub source: SignalSource,
    pub since_ms: i64,
    pub message: Option<String>,
    pub progress: Option<u8>,
}

impl Default for AgentSignal {
    fn default() -> Self {
        Self {
            state: AgentState::Unknown,
            source: SignalSource::None,
            since_ms: now_ms(),
            message: None,
            progress: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Opencode,
    Ollama,
    Shell,
    Native,
    #[default]
    Unknown,
}

impl AgentKind {
    pub fn from_label(label: &str) -> Self {
        let l = label.to_ascii_lowercase();
        match l.as_str() {
            "claude" => Self::Claude,
            "codex" | "codex-aarch64-apple-darwin" | "codex-x86_64-unknown-linux-musl" => {
                Self::Codex
            }
            "gemini" => Self::Gemini,
            "opencode" => Self::Opencode,
            "ollama" => Self::Ollama,
            "zsh" | "bash" | "fish" | "sh" | "dash" | "ksh" | "nu" | "pwsh" | "powershell"
            | "cmd" => Self::Shell,
            _ if l.starts_with("codex") => Self::Codex,
            _ => Self::Unknown,
        }
    }

    pub fn is_agent(self) -> bool {
        matches!(
            self,
            Self::Claude
                | Self::Codex
                | Self::Gemini
                | Self::Opencode
                | Self::Ollama
                | Self::Native
        )
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const ECHO_WINDOW: Duration = Duration::from_millis(200);
const BURST_MIN: Duration = Duration::from_millis(300);
const QUIET_IDLE: Duration = Duration::from_millis(2500);
const BURST_GAP: Duration = Duration::from_millis(700);
const LOOP_REPEATS: u32 = 4;

pub struct AgentTracker {
    kind: AgentKind,
    signal: AgentSignal,
    spoke: [Option<Instant>; 6],
    seq: u64,
    last_output: Option<Instant>,
    burst_start: Option<Instant>,
    last_input: Option<Instant>,
    loop_sig: Option<(u64, u32)>,
    changed_at: Instant,
    blocked_line: Option<String>,
    answered: Option<(String, Instant)>,
}

impl AgentTracker {
    pub fn new(kind: AgentKind) -> Self {
        Self {
            kind,
            signal: AgentSignal::default(),
            spoke: [None; 6],
            seq: 0,
            last_output: None,
            burst_start: None,
            last_input: None,
            loop_sig: None,
            changed_at: Instant::now(),
            blocked_line: None,
            answered: None,
        }
    }

    pub fn kind(&self) -> AgentKind {
        self.kind
    }

    pub fn set_kind(&mut self, kind: AgentKind) {
        if kind != AgentKind::Unknown {
            self.kind = kind;
        }
    }

    pub fn signal(&self) -> &AgentSignal {
        &self.signal
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn would_accept(&self, source: SignalSource, now: Instant) -> bool {
        let cur = self.signal.source;
        if cur == SignalSource::Exit {
            return source == SignalSource::Exit;
        }
        if source.rank() >= cur.rank() {
            return true;
        }
        if self.signal.state.sticky() {
            return false;
        }
        match self.spoke[cur.index()] {
            Some(t) => now.duration_since(t) >= cur.hold(),
            None => true,
        }
    }

    pub fn propose(
        &mut self,
        state: AgentState,
        source: SignalSource,
        message: Option<String>,
        now: Instant,
    ) -> bool {
        if !self.would_accept(source, now) {
            return false;
        }
        self.spoke[source.index()] = Some(now);
        self.force(state, source, message, now)
    }

    fn force(
        &mut self,
        state: AgentState,
        source: SignalSource,
        message: Option<String>,
        now: Instant,
    ) -> bool {
        let s = &mut self.signal;
        let state_changed = s.state != state;
        let changed =
            state_changed || s.source != source || (message.is_some() && s.message != message);
        if !changed {
            return false;
        }
        if state_changed {
            s.since_ms = now_ms();
            self.changed_at = now;
            s.message = None;
        }
        s.state = state;
        s.source = source;
        if message.is_some() {
            s.message = message;
        }
        self.seq += 1;
        true
    }

    fn set_progress(&mut self, p: Option<u8>) -> bool {
        if self.signal.progress == p {
            return false;
        }
        self.signal.progress = p;
        self.seq += 1;
        true
    }

    pub fn acknowledge(&mut self, now: Instant) -> bool {
        if self.signal.state == AgentState::Success {
            let src = self.signal.source;
            return self.force(AgentState::Idle, src, None, now);
        }
        false
    }

    pub fn on_user_input(&mut self, submit: bool, now: Instant) -> bool {
        self.last_input = Some(now);
        if !submit || self.signal.source == SignalSource::Exit {
            return false;
        }
        if self.signal.state == AgentState::Blocked {
            if let Some(line) = self.blocked_line.take() {
                self.answered = Some((line, now));
            }
        }
        match self.signal.state {
            AgentState::Blocked | AgentState::Success | AgentState::Failed | AgentState::Idle
                if self.kind.is_agent() || self.kind == AgentKind::Unknown =>
            {
                self.spoke[SignalSource::Heuristic.index()] = Some(now);
                self.force(AgentState::Thinking, SignalSource::Heuristic, None, now)
            }
            AgentState::Blocked => {
                self.spoke[SignalSource::Heuristic.index()] = Some(now);
                self.force(AgentState::Working, SignalSource::Heuristic, None, now)
            }
            AgentState::Unknown if self.kind == AgentKind::Unknown => {
                self.spoke[SignalSource::Heuristic.index()] = Some(now);
                self.force(AgentState::Thinking, SignalSource::Heuristic, None, now)
            }
            _ => false,
        }
    }

    pub fn wants_tick(&self, now: Instant) -> bool {
        self.signal.source != SignalSource::Exit
            && (self.signal.state.busy()
                || self
                    .last_output
                    .is_some_and(|t| now.duration_since(t) < QUIET_IDLE * 2))
    }

    pub fn on_output(&mut self, now: Instant) {
        if self
            .last_input
            .is_some_and(|t| now.duration_since(t) < ECHO_WINDOW)
        {
            return;
        }
        match self.last_output {
            Some(t) if now.duration_since(t) < BURST_GAP => {}
            _ => self.burst_start = Some(now),
        }
        self.last_output = Some(now);
    }

    pub fn on_title(&mut self, title: &str, now: Instant) -> bool {
        let Some(first) = title.trim_start().chars().next() else {
            return false;
        };
        let spinner = ('\u{2800}'..='\u{28FF}').contains(&first)
            || matches!(first, '◐' | '◑' | '◒' | '◓' | '⏳' | '⠂' | '⠐');
        if spinner {
            return self.propose(AgentState::Working, SignalSource::Title, None, now);
        }
        if first == '✳' && (self.signal.state.busy() || self.signal.state == AgentState::Unknown)
        {
            return self.propose(AgentState::Idle, SignalSource::Title, None, now);
        }
        false
    }

    fn turn_just_ended(&self, now: Instant) -> bool {
        self.signal.state.busy()
            || (self.signal.state == AgentState::Idle
                && now.duration_since(self.changed_at) < Duration::from_secs(3))
    }

    pub fn on_osc(&mut self, ev: &OscEvent, now: Instant) -> bool {
        match ev {
            OscEvent::Title(t) => self.on_title(t, now),
            OscEvent::Progress { state, value } => {
                let mut changed = false;
                match state {
                    0 => {
                        changed |= self.set_progress(None);
                        if self.signal.state.busy() {
                            changed |=
                                self.propose(AgentState::Success, SignalSource::Osc, None, now);
                        }
                    }
                    1 => {
                        changed |= self.set_progress(Some(value.unwrap_or(0)));
                        changed |= self.propose(AgentState::Working, SignalSource::Osc, None, now);
                    }
                    2 => {
                        changed |= self.set_progress(None);
                        changed |= self.propose(AgentState::Failed, SignalSource::Osc, None, now);
                    }
                    3 => {
                        changed |= self.set_progress(Some(254));
                        changed |= self.propose(AgentState::Working, SignalSource::Osc, None, now);
                    }
                    _ => {
                        changed |= self.set_progress(value.or(Some(254)));
                    }
                }
                changed
            }
            OscEvent::Notify { title, body } => {
                let text = match title {
                    Some(t) if !body.is_empty() => format!("{t}: {body}"),
                    Some(t) => t.clone(),
                    None => body.clone(),
                };
                let lower = text.to_lowercase();
                let asks = [
                    "permission",
                    "approv",
                    "allow",
                    "confirm",
                    "needs your",
                    "requires your",
                ]
                .iter()
                .any(|k| lower.contains(k));
                let msg = Some(text.chars().take(500).collect::<String>());
                if asks {
                    self.propose(AgentState::Blocked, SignalSource::Osc, msg, now)
                } else if lower.contains("waiting for your input")
                    || lower.contains("waiting for input")
                {
                    if self.signal.state == AgentState::Success {
                        false
                    } else {
                        self.propose(AgentState::Idle, SignalSource::Osc, msg, now)
                    }
                } else {
                    self.propose(AgentState::Success, SignalSource::Osc, msg, now)
                }
            }
            OscEvent::Prompt { mark, exit } => match mark {
                'A' => {
                    if self.signal.state.busy() || self.signal.state == AgentState::Unknown {
                        self.propose(AgentState::Idle, SignalSource::Osc, None, now)
                    } else {
                        false
                    }
                }
                'C' => self.propose(AgentState::Working, SignalSource::Osc, None, now),
                'D' => match exit {
                    Some(c) if *c != 0 => self.propose(
                        AgentState::Failed,
                        SignalSource::Osc,
                        Some(format!("exit {c}")),
                        now,
                    ),
                    _ if self.signal.state.busy() => {
                        self.propose(AgentState::Success, SignalSource::Osc, None, now)
                    }
                    _ => false,
                },
                _ => false,
            },
            OscEvent::Bell => {
                if self.turn_just_ended(now) {
                    self.propose(AgentState::Success, SignalSource::Osc, None, now)
                } else {
                    false
                }
            }
            OscEvent::Cwd(_) | OscEvent::UserVar { .. } => false,
        }
    }

    pub fn on_hook(&mut self, event: &str, payload: &Value, now: Instant) -> bool {
        let text = |k: &str| {
            payload
                .get(k)
                .and_then(Value::as_str)
                .map(|s| s.chars().take(500).collect::<String>())
                .filter(|s| !s.is_empty())
        };
        let (state, message) = match event {
            "UserPromptSubmit" | "BeforeAgent" => (AgentState::Thinking, None),
            "PreToolUse" | "BeforeTool" | "tool.execute.before" => {
                let tool = text("tool_name");
                if self.track_loop(payload) {
                    (AgentState::Looping, tool)
                } else {
                    (AgentState::Working, tool)
                }
            }
            "PostToolUse" | "PostToolBatch" | "PostToolUseFailure" | "AfterTool"
            | "tool.execute.after" | "SubagentStop" | "permission.replied" => {
                (AgentState::Thinking, None)
            }
            "SubagentStart" => (AgentState::Working, text("agent_type")),
            "PermissionRequest" | "approval-requested" | "permission.asked" | "question.asked" => (
                AgentState::Blocked,
                text("tool_name").or_else(|| text("message")),
            ),
            "Notification" => {
                let kind = text("notification_type").unwrap_or_default();
                let msg = text("message");
                match kind.as_str() {
                    "permission_prompt"
                    | "elicitation_dialog"
                    | "elicitation_url_dialog"
                    | "agent_needs_input" => (AgentState::Blocked, msg),
                    "idle_prompt" => {
                        if self.signal.state == AgentState::Success {
                            return false;
                        }
                        (AgentState::Idle, msg)
                    }
                    "agent_completed" => (AgentState::Success, msg),
                    _ => {
                        let lower = msg.clone().unwrap_or_default().to_lowercase();
                        if lower.contains("permission") || lower.contains("approv") {
                            (AgentState::Blocked, msg)
                        } else {
                            return false;
                        }
                    }
                }
            }
            "Stop" | "AfterAgent" | "agent-turn-complete" | "session.idle" => (
                AgentState::Success,
                text("last_assistant_message").or_else(|| text("last-assistant-message")),
            ),
            "StopFailure" | "session.error" => (
                AgentState::Failed,
                text("error_type")
                    .or_else(|| text("reason"))
                    .or_else(|| text("error")),
            ),
            "SessionStart" | "SessionEnd" => (AgentState::Idle, None),
            "PreCompact" | "PreCompress" => (AgentState::Compacting, None),
            "PostCompact" => (AgentState::Thinking, None),
            "session.status" => match payload.get("status").and_then(Value::as_str) {
                Some("busy") => (AgentState::Working, None),
                Some("idle") => (AgentState::Idle, None),
                _ => return false,
            },
            "omniget.waiting_peer" => (AgentState::WaitingPeer, text("message")),
            "omniget.peer_done" => (AgentState::Thinking, None),
            _ => return false,
        };
        if !matches!(
            state,
            AgentState::Working | AgentState::Looping | AgentState::Thinking
        ) || event == "UserPromptSubmit"
        {
            self.loop_sig = None;
        }
        self.propose(state, SignalSource::Hook, message, now)
    }

    fn track_loop(&mut self, payload: &Value) -> bool {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        payload
            .get("tool_name")
            .map(|v| v.to_string())
            .unwrap_or_default()
            .hash(&mut h);
        payload
            .get("tool_input")
            .map(|v| v.to_string())
            .unwrap_or_default()
            .hash(&mut h);
        let sig = h.finish();
        let count = match self.loop_sig {
            Some((s, n)) if s == sig => n + 1,
            _ => 1,
        };
        self.loop_sig = Some((sig, count));
        count >= LOOP_REPEATS
    }

    pub fn on_exit(&mut self, code: Option<i32>, signal: Option<&str>, now: Instant) -> bool {
        let (state, msg) = match (code, signal) {
            (Some(0), None) => (
                if self.kind == AgentKind::Shell {
                    AgentState::Idle
                } else {
                    AgentState::Success
                },
                Some("exited".to_string()),
            ),
            (Some(c), _) => (AgentState::Failed, Some(format!("exit code {c}"))),
            (None, Some(s)) => (AgentState::Failed, Some(format!("signal {s}"))),
            (None, None) => (AgentState::Failed, Some("exited".to_string())),
        };
        self.set_progress(None);
        self.spoke[SignalSource::Exit.index()] = Some(now);
        self.force(state, SignalSource::Exit, msg, now)
    }

    pub fn tick(&mut self, now: Instant, fg: &Foreground, screen: Option<&str>) -> bool {
        if fg.running {
            let k = AgentKind::from_label(&fg.label);
            if k.is_agent() {
                self.kind = k;
            }
        }
        let fg_busy = fg.running
            && self.kind == AgentKind::Shell
            && !AgentKind::from_label(&fg.label).is_agent();
        if fg_busy && !self.signal.state.busy() && !self.signal.state.sticky() {
            if self.would_accept(SignalSource::Heuristic, now) {
                return self.propose(AgentState::Working, SignalSource::Heuristic, None, now);
            }
            return false;
        }
        if !self.would_accept(SignalSource::Heuristic, now) {
            return false;
        }
        if let Some(screen) = screen {
            if let Some(state) = screen_state(self.kind, screen) {
                let last = last_line(screen);
                let answered = state == AgentState::Blocked
                    && self.signal.state != AgentState::Blocked
                    && self.answered.as_ref().is_some_and(|(line, at)| {
                        *line == last && self.last_output.is_none_or(|o| o <= *at)
                    });
                if !answered {
                    if state == AgentState::Blocked {
                        self.blocked_line = Some(last);
                    }
                    return self.propose(state, SignalSource::Heuristic, None, now);
                }
            }
        }
        let active = match (self.last_output, self.burst_start) {
            (Some(last), Some(start)) => {
                now.duration_since(last) < BURST_GAP && last.duration_since(start) >= BURST_MIN
            }
            _ => false,
        };
        if active && (fg.running || self.kind.is_agent() || self.kind == AgentKind::Unknown) {
            return self.propose(AgentState::Working, SignalSource::Heuristic, None, now);
        }
        let quiet = self
            .last_output
            .map(|t| now.duration_since(t) >= QUIET_IDLE)
            .unwrap_or(true);
        if quiet && self.signal.state.busy() && !fg_busy {
            let end = if self.kind == AgentKind::Unknown {
                AgentState::Success
            } else {
                AgentState::Idle
            };
            return self.propose(end, SignalSource::Heuristic, None, now);
        }
        if !fg.running
            && matches!(self.kind, AgentKind::Shell | AgentKind::Unknown)
            && self.signal.state == AgentState::Unknown
            && quiet
            && self.last_output.is_some()
        {
            return self.propose(AgentState::Idle, SignalSource::Heuristic, None, now);
        }
        if !fg.running && self.kind == AgentKind::Shell && self.signal.state == AgentState::Unknown
        {
            return self.propose(AgentState::Idle, SignalSource::Heuristic, None, now);
        }
        false
    }
}

fn screen_state(kind: AgentKind, screen: &str) -> Option<AgentState> {
    let lower = screen.to_lowercase();
    let last = lower
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let blocked_generic = asks_yes_no(last);
    if choice_prompt(&lower) {
        return Some(AgentState::Blocked);
    }
    if lower.contains("hit your usage limit") || lower.contains("usage limit reached") {
        return Some(AgentState::Failed);
    }
    match kind {
        AgentKind::Claude => {
            if lower.contains("do you want to")
                && (lower.contains("1. yes") || lower.contains("❯ 1."))
            {
                Some(AgentState::Blocked)
            } else if lower.contains("esc to interrupt") {
                Some(AgentState::Working)
            } else {
                None
            }
        }
        AgentKind::Codex => {
            if lower.contains("would you like to run")
                || lower.contains("allow command")
                || lower.contains("would you like to make")
            {
                Some(AgentState::Blocked)
            } else if lower.contains("esc to interrupt") {
                Some(AgentState::Working)
            } else {
                None
            }
        }
        AgentKind::Gemini => {
            if lower.contains("allow execution") || lower.contains("apply this change") {
                Some(AgentState::Blocked)
            } else if lower.contains("esc to cancel") {
                Some(AgentState::Working)
            } else {
                None
            }
        }
        _ if blocked_generic => Some(AgentState::Blocked),
        _ => None,
    }
}

fn last_line(screen: &str) -> String {
    screen
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim_end()
        .to_string()
}

fn asks_yes_no(line: &str) -> bool {
    let l = line
        .trim_end()
        .trim_end_matches([':', '?', '>', ' '])
        .trim_end();
    [
        "[y/n]", "(y/n)", "[yes/no]", "(yes/no)", "[y/n/a]", "(y/n/a)",
    ]
    .iter()
    .any(|p| l.ends_with(p))
}

fn choice_prompt(lower: &str) -> bool {
    const FOOTERS: [&str; 7] = [
        "enter to confirm",
        "enter to continue",
        "enter continue",
        "press enter",
        "esc to cancel",
        "esc to exit",
        "esc quit",
    ];
    let cursor_line = lower.lines().any(|line| {
        let l = line.trim_matches(|c: char| c.is_whitespace() || c == '│' || c == '|');
        let rest = l.trim_start_matches(['❯', '›', '●', '▶']).trim_start();
        rest.len() < l.len() && !rest.is_empty()
    });
    if cursor_line && FOOTERS.iter().any(|f| lower.contains(f)) {
        return true;
    }
    let mut cursor_on_first = false;
    let mut second = false;
    for line in lower.lines() {
        let l = line.trim_matches(|c: char| c.is_whitespace() || c == '│' || c == '|');
        let rest = l
            .trim_start_matches(['❯', '›', '>', '●', '▶', '→'])
            .trim_start();
        if rest.len() < l.len() && rest.starts_with("1.") {
            cursor_on_first = true;
        }
        if l.starts_with("2.") || rest.starts_with("2.") {
            second = true;
        }
    }
    if !(cursor_on_first && second) {
        let moved = lower.lines().any(|line| {
            let l = line.trim_matches(|c: char| c.is_whitespace() || c == '│' || c == '|');
            let rest = l.trim_start_matches(['❯', '›', '●', '▶']).trim_start();
            rest.len() < l.len()
                && rest.len() > 2
                && rest.as_bytes()[0].is_ascii_digit()
                && rest.as_bytes()[1] == b'.'
        });
        if !(moved && lower.contains("1.")) {
            return false;
        }
    }
    const HINTS: [&str; 14] = [
        "trust",
        "do you want",
        "would you like",
        "proceed",
        "allow",
        "approve",
        "enter to confirm",
        "enter to continue",
        "press enter",
        "yes, ",
        "1. yes",
        "no, exit",
        "esc to cancel",
        "(esc)",
    ];
    HINTS.iter().any(|h| lower.contains(h))
}
