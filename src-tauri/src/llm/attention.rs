use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::time::Instant;

pub const COOLDOWN: Duration = Duration::from_secs(5);

pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiFocus {
    pub focused: bool,
    pub path: String,
    pub job_id: Option<String>,
    pub conversation_id: Option<String>,
}

static REPORTED_FOCUS: Mutex<Option<UiFocus>> = Mutex::new(None);

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionTarget {
    pub job_id: String,
    pub conversation_id: String,
    pub agent: String,
}

pub const CANVAS_PREFIX: &str = "canvas:";

impl AttentionTarget {
    pub fn canvas_node(node_id: &str, name: &str) -> Self {
        Self {
            job_id: format!("{CANVAS_PREFIX}{node_id}"),
            conversation_id: format!("{CANVAS_PREFIX}{node_id}"),
            agent: name.to_string(),
        }
    }

    pub fn canvas_node_id(&self) -> Option<&str> {
        self.job_id.strip_prefix(CANVAS_PREFIX)
    }
}

pub fn suppressed(focus: &UiFocus, target: &AttentionTarget) -> bool {
    if !focus.focused {
        return false;
    }
    if target.canvas_node_id().is_some() {
        return focus.conversation_id.as_deref() == Some(target.conversation_id.as_str());
    }
    if focus.path.trim_end_matches('/') == "/llm/jobs" {
        return true;
    }
    focus.job_id.as_deref() == Some(target.job_id.as_str())
        || focus.conversation_id.as_deref() == Some(target.conversation_id.as_str())
}

#[derive(Debug, Default)]
pub struct AttentionGate {
    last: HashMap<String, Instant>,
}

impl AttentionGate {
    pub fn admit(&mut self, job_id: &str, now: Instant) -> bool {
        if let Some(last) = self.last.get(job_id) {
            if now.saturating_duration_since(*last) < COOLDOWN {
                return false;
            }
        }
        self.last
            .retain(|_, t| now.saturating_duration_since(*t) < COOLDOWN);
        self.last.insert(job_id.to_string(), now);
        true
    }
}

pub trait NotifySink: Send + Sync + 'static {
    fn show(&self, title: &str, body: &str);
    fn focus(&self) -> UiFocus;
}

pub type StillPending = Arc<dyn Fn(&str) -> bool + Send + Sync>;

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
