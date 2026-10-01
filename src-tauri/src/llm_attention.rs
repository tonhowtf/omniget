//! "Needs you" and "finished" notifications of the agents (item 1.3).
//!
//! The old behaviour: a question to the user surfaced only inside the jobs
//! page, polled every 2 s while that page was open, and died as a silent
//! "no" after 120 s. Now every question reaches the OS as a notification —
//! but only when it would help:
//!
//! * **delay:** it goes out `approval_notify_delay_s` (5 s) after the ask, and
//!   only if the ask is **still pending at delivery**: a question answered in
//!   2 s never pops anything (79#P3, herdr re-checks the state on delivery);
//! * **focus:** nothing when the window is focused on that job already (its
//!   row open, its conversation on screen, or the jobs page, which shows every
//!   question inline) — orca's "window focused on that worktree" (78#P6);
//! * **cooldown:** at most one notification per job every 5 s, whatever raised
//!   it (78#P6).
//!
//! The same gate carries "expired" (the reason is said, never a silent "no")
//! and "finished/failed" of background jobs. The clock is tokio's, so the
//! tests run the whole timing in paused virtual time; the OS side is a
//! [`NotifySink`] (the Tauri notification plugin in the app).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::time::Instant;

/// One notification per job at most this often.
pub const COOLDOWN: Duration = Duration::from_secs(5);

/// Replaces every `{{name}}` in a pushed translation.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// What the user is looking at, as the front reports it (`llm_ui_focus`) plus
/// whether the main window has the focus right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiFocus {
    pub focused: bool,
    pub path: String,
    /// The job whose row is open.
    pub job_id: Option<String>,
    /// The conversation on screen in the chat.
    pub conversation_id: Option<String>,
}

/// What the front last reported (`llm_ui_focus`); `focused` is filled by the
/// sink from the real window at delivery.
static REPORTED_FOCUS: Mutex<Option<UiFocus>> = Mutex::new(None);

/// Records the route (and the open job / conversation) the front shows.
pub fn report_focus(path: String, job_id: Option<String>, conversation_id: Option<String>) {
    *REPORTED_FOCUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(UiFocus {
        focused: false,
        path,
        job_id: job_id.filter(|j| !j.is_empty()),
        conversation_id: conversation_id.filter(|c| !c.is_empty()),
    });
}

pub fn reported_focus() -> UiFocus {
    REPORTED_FOCUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// The last texts the front pushed, kept here so they survive until the
/// jobs open (the push can come first).
static PUSHED_STRINGS: Mutex<Option<NotifyStrings>> = Mutex::new(None);

pub fn push_strings(strings: NotifyStrings) {
    *PUSHED_STRINGS.lock().unwrap_or_else(|e| e.into_inner()) = Some(strings);
}

pub fn pushed_strings() -> NotifyStrings {
    PUSHED_STRINGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// The job a notification is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionTarget {
    pub job_id: String,
    pub conversation_id: String,
    /// Shown in the title: the agent's display name.
    pub agent: String,
}

/// Prefix of the ids of a canvas node target (C2): `job_id` and
/// `conversation_id` are both `canvas:<node id>`, so the cooldown is per node
/// and the canvas front reports a focused node as the conversation
/// `canvas:<node id>` (`llm_ui_focus`).
pub const CANVAS_PREFIX: &str = "canvas:";

impl AttentionTarget {
    /// A node of the canvas (terminal or native agent), named as on the canvas.
    pub fn canvas_node(node_id: &str, name: &str) -> Self {
        Self {
            job_id: format!("{CANVAS_PREFIX}{node_id}"),
            conversation_id: format!("{CANVAS_PREFIX}{node_id}"),
            agent: name.to_string(),
        }
    }

    /// The canvas node this target is about, if it is one.
    pub fn canvas_node_id(&self) -> Option<&str> {
        self.job_id.strip_prefix(CANVAS_PREFIX)
    }
}

/// True when the focused window already shows this job.
pub fn suppressed(focus: &UiFocus, target: &AttentionTarget) -> bool {
    if !focus.focused {
        return false;
    }
    // A canvas node: only when that node is the focused one on the canvas.
    if target.canvas_node_id().is_some() {
        return focus.conversation_id.as_deref() == Some(target.conversation_id.as_str());
    }
    // The jobs page lists every job with its question inline.
    if focus.path.trim_end_matches('/') == "/llm/jobs" {
        return true;
    }
    focus.job_id.as_deref() == Some(target.job_id.as_str())
        || focus.conversation_id.as_deref() == Some(target.conversation_id.as_str())
}

/// The per-job cooldown.
#[derive(Debug, Default)]
pub struct AttentionGate {
    last: HashMap<String, Instant>,
}

impl AttentionGate {
    /// Lets one notification of `job_id` through, unless one went out less
    /// than [`COOLDOWN`] ago.
    pub fn admit(&mut self, job_id: &str, now: Instant) -> bool {
        if let Some(last) = self.last.get(job_id) {
            if now.saturating_duration_since(*last) < COOLDOWN {
                return false;
            }
        }
        // Old entries are useless once past the cooldown: keep the map small.
        self.last
            .retain(|_, t| now.saturating_duration_since(*t) < COOLDOWN);
        self.last.insert(job_id.to_string(), now);
        true
    }
}

/// Where a notification goes, and what the user is looking at.
pub trait NotifySink: Send + Sync + 'static {
    fn show(&self, title: &str, body: &str);
    fn focus(&self) -> UiFocus;
}

/// "Is this question (by tool call id) still waiting?", asked at delivery.
pub type StillPending = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The texts, pushed by the front in the interface language
/// (`sync_llm_notify_strings`); the English set is only the fallback until
/// the first push. Placeholders: `{{agent}}`, `{{tool}}`, `{{preview}}`,
/// `{{minutes}}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotifyStrings {
    pub needs_you_title: String,
    pub needs_you_body: String,
    pub expired_title: String,
    pub expired_body: String,
    pub finished_title: String,
    pub failed_title: String,
}

impl Default for NotifyStrings {
    fn default() -> Self {
        Self {
            needs_you_title: "{{agent}} needs you".into(),
            needs_you_body: "Allow {{tool}}? {{preview}}".into(),
            expired_title: "{{agent}}: a question went unanswered".into(),
            expired_body: "Nobody answered {{tool}} in {{minutes}} min, so it did not run.".into(),
            finished_title: "{{agent}} finished".into(),
            failed_title: "{{agent}} failed".into(),
        }
    }
}

/// The notification logic of the agents, one per app.
pub struct Attention {
    sink: Arc<dyn NotifySink>,
    gate: Mutex<AttentionGate>,
    strings: RwLock<NotifyStrings>,
    delay: RwLock<Duration>,
}

impl Attention {
    pub fn new(sink: Arc<dyn NotifySink>) -> Arc<Self> {
        Arc::new(Self {
            sink,
            gate: Mutex::new(AttentionGate::default()),
            strings: RwLock::new(NotifyStrings::default()),
            delay: RwLock::new(Duration::from_secs(5)),
        })
    }

    /// `approval_notify_delay_s`.
    pub fn set_delay(&self, delay: Duration) {
        *self.delay.write().unwrap_or_else(|e| e.into_inner()) = delay;
    }

    pub fn set_strings(&self, strings: NotifyStrings) {
        *self.strings.write().unwrap_or_else(|e| e.into_inner()) = strings;
    }

    fn strings(&self) -> NotifyStrings {
        self.strings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// A question was raised: after the delay, if it is still unanswered and
    /// nobody is looking at the job, one notification. The handle resolves to
    /// whether it went out. Must run inside a tokio runtime.
    pub fn ask_opened(
        self: &Arc<Self>,
        target: AttentionTarget,
        tool_call_id: String,
        tool: String,
        preview: String,
        still_pending: StillPending,
    ) -> tokio::task::JoinHandle<bool> {
        let this = self.clone();
        let delay = *self.delay.read().unwrap_or_else(|e| e.into_inner());
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // Re-checked at delivery: answered, expired or cancelled in the
            // meantime means there is nothing to ask the user any more.
            if !still_pending(&tool_call_id) {
                return false;
            }
            let s = this.strings();
            let preview: String = preview.trim().chars().take(160).collect();
            let values = [
                ("agent", target.agent.as_str()),
                ("tool", tool.as_str()),
                ("preview", preview.as_str()),
            ];
            this.deliver(
                &target,
                fill(&s.needs_you_title, &values),
                fill(&s.needs_you_body, &values),
            )
        })
    }

    /// A question expired: said at once, with the tool and how long it
    /// waited, so the "no" is never silent.
    pub fn ask_expired(&self, target: &AttentionTarget, tool: &str, waited: Duration) -> bool {
        let s = self.strings();
        let minutes = ((waited.as_secs() + 30) / 60).max(1).to_string();
        let values = [
            ("agent", target.agent.as_str()),
            ("tool", tool),
            ("minutes", minutes.as_str()),
        ];
        self.deliver(
            target,
            fill(&s.expired_title, &values),
            fill(&s.expired_body, &values),
        )
    }

    /// A canvas node needs the human (a permission, a question, a delivery
    /// that did not take): after the delay, if `still` holds and the node is
    /// not the focused one, one notification. Must run inside a tokio runtime.
    pub fn node_needs_you(
        self: &Arc<Self>,
        target: AttentionTarget,
        what: String,
        still: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> tokio::task::JoinHandle<bool> {
        let this = self.clone();
        let delay = *self.delay.read().unwrap_or_else(|e| e.into_inner());
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if !still() {
                return false;
            }
            let s = this.strings();
            let title = fill(&s.needs_you_title, &[("agent", target.agent.as_str())]);
            let body: String = what.trim().chars().take(180).collect();
            this.deliver(&target, title, body)
        })
    }

    /// A background job (or Loop) ended: `detail` is the first line of its
    /// result or its error.
    pub fn job_ended(&self, target: &AttentionTarget, ok: bool, detail: &str) -> bool {
        let s = self.strings();
        let title = if ok {
            &s.finished_title
        } else {
            &s.failed_title
        };
        let body: String = detail.trim().chars().take(180).collect();
        self.deliver(
            target,
            fill(title, &[("agent", target.agent.as_str())]),
            body,
        )
    }

    fn deliver(&self, target: &AttentionTarget, title: String, body: String) -> bool {
        if suppressed(&self.sink.focus(), target) {
            return false;
        }
        if !self
            .gate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admit(&target.job_id, Instant::now())
        {
            return false;
        }
        self.sink.show(&format!("OmniGet · {title}"), &body);
        true
    }
}
