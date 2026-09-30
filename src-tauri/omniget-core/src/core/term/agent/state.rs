//! Agent state of one terminal.
//!
//! Sources, strongest first: process exit, hooks (the agent tells us), OSC
//! sequences and the window title (the agent hints on the PTY), heuristics
//! (foreground process, output quiescence, screen patterns).
//!
//! A proposal from a source at least as strong as the one that set the
//! current state always applies. A weaker one applies only when the stronger
//! source has been silent for its hold window (10 s for hooks, 4 s for
//! OSC/title), and never replaces `blocked`, `waiting_peer`, `failed` or
//! `success` (those need the same or a stronger source, the user answering in
//! the terminal, or focus for `success`).

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
    /// Waiting for another agent (an `ask` through a cord).
    WaitingPeer,
    /// Waiting for the human: permission, question, elicitation.
    Blocked,
    /// Turn finished and not seen yet (focus turns it into `idle`).
    Success,
    Failed,
    Compacting,
    Looping,
    #[default]
    Unknown,
}

impl AgentState {
    /// Wire code (`AGENT` section).
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
    /// Epoch ms of the last state change.
    pub since_ms: i64,
    pub message: Option<String>,
    /// OSC 9;4 progress: 0-100, 254 indeterminate; `None` without a bar.
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
    /// From a program label ("claude", "codex", "zsh").
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

/// Output this close to a keystroke is treated as echo, not activity.
const ECHO_WINDOW: Duration = Duration::from_millis(200);
/// Output sustained this long counts as "working" for the heuristics.
const BURST_MIN: Duration = Duration::from_millis(300);
/// Silence after which a heuristic "working" becomes "idle".
const QUIET_IDLE: Duration = Duration::from_millis(2500);
/// A gap in output this long ends a burst.
const BURST_GAP: Duration = Duration::from_millis(700);
/// Same tool call this many times in a row = looping.
const LOOP_REPEATS: u32 = 4;

pub struct AgentTracker {
    kind: AgentKind,
    signal: AgentSignal,
    /// When each source last spoke (index = `SignalSource::code`).
    spoke: [Option<Instant>; 6],
    seq: u64,
    last_output: Option<Instant>,
    burst_start: Option<Instant>,
    last_input: Option<Instant>,
    loop_sig: Option<(u64, u32)>,
    changed_at: Instant,
    /// Last line of the screen question that made the state `blocked`.
    blocked_line: Option<String>,
    /// That question, once the user answered it, and when.
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

    /// Bumped on every visible change (state, message or progress).
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Whether a proposal from `source` could apply right now (lets the
    /// session skip reading the screen for heuristics that would be ignored).
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

    /// Applies a proposal under the precedence rules. Returns whether the
    /// visible signal changed.
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
            // A new state starts without the previous state's message.
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

    /// The user focused the node: an unseen `success` becomes `idle`.
    pub fn acknowledge(&mut self, now: Instant) -> bool {
        if self.signal.state == AgentState::Success {
            let src = self.signal.source;
            return self.force(AgentState::Idle, src, None, now);
        }
        false
    }

    /// Keystrokes from the user. A submit (Enter) answers a prompt: a sticky
    /// `blocked`/`success`/`failed` gives way to `thinking` right away.
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
            // A generic program (a custom agent CLI) that never said anything
            // about its state: a submitted line starts its turn.
            // A shell program's "[Y/n]" answered: it goes on with its work.
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

    /// Whether the periodic heuristics still have something to decide (a
    /// busy state to end, or recent output).
    pub fn wants_tick(&self, now: Instant) -> bool {
        self.signal.source != SignalSource::Exit
            && (self.signal.state.busy()
                || self
                    .last_output
                    .is_some_and(|t| now.duration_since(t) < QUIET_IDLE * 2))
    }

    /// Output from the program (bytes read), for the quiescence heuristic.
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

    /// A hook event in Claude's vocabulary (the shim normalises Codex,
    /// Gemini and Cline to it), plus Codex `notify` and OpenCode plugin
    /// event names, and OmniGet's own `omniget.*` events.
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
        // The streak spans a turn (Pre/Post pairs); a new turn resets it.
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

    /// The program ended.
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

    /// Heuristics, called periodically by the session. `screen` is the tail
    /// of the visible screen (last lines), read only when
    /// `would_accept(Heuristic)` holds.
    pub fn tick(&mut self, now: Instant, fg: &Foreground, screen: Option<&str>) -> bool {
        if fg.running {
            let k = AgentKind::from_label(&fg.label);
            if k.is_agent() {
                self.kind = k;
            }
        }
        // A program other than the shell (and not an agent CLI) in the
        // foreground is busy even when it prints nothing (`sleep 60`, a
        // build waiting on the network).
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
                // A "[Y/n]" still on screen after the user answered it is not a
                // new question: blocked needs output after the last keystroke.
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
            // A generic program that went quiet after a turn has finished it
            // (the attention dot, like the Maestri's "stopped producing
            // output"); a shell just went back to its prompt.
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

/// Screen patterns of the agent CLIs (fallback only; they change between
/// versions).
fn screen_state(kind: AgentKind, screen: &str) -> Option<AgentState> {
    let lower = screen.to_lowercase();
    let last = lower
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let blocked_generic = asks_yes_no(last);
    // Trust dialogs and permission/choice menus of Claude, Codex and Gemini
    // (a cursor on "1." with a second option) mean the human has to answer,
    // whatever the agent kind detected so far.
    if choice_prompt(&lower) {
        return Some(AgentState::Blocked);
    }
    // The account ran out: the agent sits at its prompt but cannot answer.
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

/// The line ends in a yes/no question still waiting for its answer
/// ("Continue? [Y/n]", "Overwrite (y/n)?"). Once the user typed an answer
/// after it ("[Y/n] y"), it no longer does.
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

/// A selection menu waiting for the user: a cursor glyph on an option "1."
/// and a second option "2.", plus a question, trust or confirm hint. Plain
/// numbered lists in the agent's answers have no cursor and are ignored.
fn choice_prompt(lower: &str) -> bool {
    // Unnumbered menus ("❯ No, exit / Yes, I trust this folder"): a cursor
    // line plus the confirm footer those dialogs print.
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
        // Cursor on another option ("❯ 2. No") after the user moved it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn hook_beats_osc_and_heuristics_inside_the_hold() {
        let now = t0();
        let mut a = AgentTracker::new(AgentKind::Claude);
        assert!(a.on_hook("UserPromptSubmit", &json!({}), now));
        assert_eq!(a.signal().state, AgentState::Thinking);
        // Title spinner (weaker) is ignored inside the hook's hold.
        assert!(!a.on_title("◐ Claude", now + Duration::from_secs(1)));
        assert_eq!(a.signal().source, SignalSource::Hook);
        // After the hold a non-sticky state gives way.
        assert!(a.on_title("◐ Claude", now + Duration::from_secs(11)));
        assert_eq!(a.signal().state, AgentState::Working);
        assert_eq!(a.signal().source, SignalSource::Title);
    }

    #[test]
    fn sticky_blocked_survives_weaker_sources_until_the_user_answers() {
        let now = t0();
        let mut a = AgentTracker::new(AgentKind::Claude);
        a.on_hook(
            "Notification",
            &json!({"notification_type": "permission_prompt", "message": "Claude needs your permission"}),
            now,
        );
        assert_eq!(a.signal().state, AgentState::Blocked);
        let later = now + Duration::from_secs(60);
        assert!(!a.on_title("✳ Claude", later));
        assert!(!a.tick(later, &Foreground::default(), Some("esc to interrupt")));
        assert_eq!(a.signal().state, AgentState::Blocked);
        // The user answers in the terminal.
        assert!(a.on_user_input(true, later));
        assert_eq!(a.signal().state, AgentState::Thinking);
        // A hook always applies.
        assert!(a.on_hook("Stop", &json!({"last_assistant_message": "done"}), later));
        assert_eq!(a.signal().state, AgentState::Success);
        assert_eq!(a.signal().message.as_deref(), Some("done"));
        assert!(a.acknowledge(later));
        assert_eq!(a.signal().state, AgentState::Idle);
    }

    #[test]
    fn a_silent_foreground_program_keeps_a_shell_busy() {
        let now = t0();
        let mut a = AgentTracker::new(AgentKind::Shell);
        assert!(a.on_osc(
            &OscEvent::Prompt {
                mark: 'C',
                exit: None
            },
            now
        ));
        assert_eq!(a.signal().state, AgentState::Working);
        let sleep = Foreground {
            running: true,
            label: "sleep".into(),
            pid: Some(42),
        };
        // Ten seconds of silence: still working while `sleep` runs.
        let later = now + Duration::from_secs(10);
        a.tick(later, &sleep, Some("$ sleep 120"));
        assert_eq!(a.signal().state, AgentState::Working);
        // Back at the prompt: idle.
        a.tick(
            later + Duration::from_secs(1),
            &Foreground::default(),
            Some("$"),
        );
        assert_eq!(a.signal().state, AgentState::Idle);
    }

    #[test]
    fn an_answered_yes_no_prompt_does_not_block_again() {
        let now = t0();
        let mut a = AgentTracker::new(AgentKind::Shell);
        a.on_output(now);
        a.tick(now, &Foreground::default(), Some("Continue? [Y/n]"));
        assert_eq!(a.signal().state, AgentState::Blocked);
        let later = now + Duration::from_secs(1);
        assert!(a.on_user_input(true, later));
        assert_ne!(a.signal().state, AgentState::Blocked);
        // The question is still the last line on screen.
        let after = later + Duration::from_secs(5);
        a.tick(after, &Foreground::default(), Some("Continue? [Y/n]"));
        assert_ne!(a.signal().state, AgentState::Blocked);
        assert!(!asks_yes_no("Continue? [Y/n] y"));
        assert!(asks_yes_no("Overwrite file (y/n)? "));
    }

    #[test]
    fn a_generic_program_turn_ends_with_attention() {
        let now = t0();
        let mut a = AgentTracker::new(AgentKind::Unknown);
        assert!(a.on_user_input(true, now));
        assert_eq!(a.signal().state, AgentState::Thinking);
        a.on_output(now + Duration::from_millis(500));
        a.tick(
            now + Duration::from_secs(4),
            &Foreground::default(),
            Some("stub> "),
        );
        assert_eq!(a.signal().state, AgentState::Success);
    }

    #[test]
    fn osc_beats_heuristics_and_exit_beats_all() {
        let now = t0();
        let mut a = AgentTracker::new(AgentKind::Codex);
        assert!(a.on_osc(
            &OscEvent::Progress {
                state: 3,
                value: None
            },
            now
        ));
        assert_eq!(a.signal().state, AgentState::Working);
        assert_eq!(a.signal().progress, Some(254));
        // Quiet screen heuristic says idle, but OSC holds for 4 s.
        assert!(!a.tick(now + Duration::from_secs(3), &Foreground::default(), None));
        assert!(a.on_osc(
            &OscEvent::Notify {
                title: None,
                body: "Approval requested: rm -rf".into()
            },
            now + Duration::from_secs(3)
        ));
        assert_eq!(a.signal().state, AgentState::Blocked);
        assert!(a.on_exit(Some(1), None, now + Duration::from_secs(4)));
        assert_eq!(a.signal().state, AgentState::Failed);
        assert_eq!(a.signal().source, SignalSource::Exit);
        assert!(!a.on_hook("UserPromptSubmit", &json!({}), now + Duration::from_secs(5)));
    }

    #[test]
    fn repeated_tool_calls_become_looping() {
        let now = t0();
        let mut a = AgentTracker::new(AgentKind::Claude);
        let p = json!({"tool_name": "Bash", "tool_input": {"command": "pnpm test"}});
        for _ in 0..3 {
            a.on_hook("PreToolUse", &p, now);
            a.on_hook("PostToolUse", &json!({}), now);
        }
        assert_ne!(a.signal().state, AgentState::Looping);
        a.on_hook("PreToolUse", &p, now);
        assert_eq!(a.signal().state, AgentState::Looping);
        // A different call breaks the streak.
        a.on_hook("PreToolUse", &json!({"tool_name": "Read"}), now);
        assert_eq!(a.signal().state, AgentState::Working);
    }

    #[test]
    fn trust_and_choice_prompts_are_blocked() {
        let claude_trust = "│ do you trust the files in this folder?\n│\n│ ❯ 1. yes, proceed\n│   2. no, exit\n╰───╯\n   enter to confirm · esc to exit";
        assert_eq!(
            screen_state(AgentKind::Unknown, claude_trust),
            Some(AgentState::Blocked)
        );
        let claude_new = "quick safety check: is this a project you created or one you trust?\n ❯ 1. Yes, I trust this folder\n   2. No, exit\n enter to confirm · esc to cancel".to_lowercase();
        assert_eq!(
            screen_state(AgentKind::Claude, &claude_new),
            Some(AgentState::Blocked)
        );
        let codex = "› 1. yes, allow codex to work in this folder without asking for approval\n  2. no, ask me to approve edits and commands\n  press enter to continue";
        assert_eq!(
            screen_state(AgentKind::Codex, codex),
            Some(AgentState::Blocked)
        );
        let gemini = "do you trust this folder?\n ● 1. trust folder\n   2. trust parent folder\n   3. don't trust (esc)";
        assert_eq!(
            screen_state(AgentKind::Gemini, gemini),
            Some(AgentState::Blocked)
        );
        let moved =
            "do you want to proceed?\n   1. yes\n ❯ 2. yes, and don't ask again\n   3. no (esc)";
        assert_eq!(
            screen_state(AgentKind::Claude, moved),
            Some(AgentState::Blocked)
        );
        let claude_v2 = " claude code'll be able to read, edit, and execute files here.\n\n ❯ no, exit\n   yes, i trust this folder\n\n enter to confirm · esc to cancel";
        assert_eq!(
            screen_state(AgentKind::Claude, claude_v2),
            Some(AgentState::Blocked)
        );
        let input_box = "❯ fix the tests\n⏺ working on it\n  esc to interrupt";
        assert_eq!(
            screen_state(AgentKind::Claude, input_box),
            Some(AgentState::Working)
        );
        let limit = "■ you've hit your usage limit. visit https://chatgpt.com/codex/settings/usage\n\n› ask codex to do anything\n  ? for shortcuts";
        assert_eq!(
            screen_state(AgentKind::Codex, limit),
            Some(AgentState::Failed)
        );
        let answer = "here is the plan:\n1. do this\n2. do that\nesc to interrupt";
        assert_eq!(
            screen_state(AgentKind::Claude, answer),
            Some(AgentState::Working)
        );
    }
}
