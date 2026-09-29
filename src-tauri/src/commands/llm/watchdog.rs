//! Turn watchdog, job deadline and permission-question timing, as the
//! settings card sees them (items 1.2 and 1.3 of the `/llm` roadmap).
//!
//! The values live in `LlmSettings`; this layer clamps them into something
//! that cannot hurt (a 0 s idle window would kill every turn on its first
//! silence, a 0 s ask timeout would deny every question) and pushes them into
//! the running pieces: the CLI runtime's watchdog and the broker's default ask
//! timeout. The jobs read the rest (deadline, job ask timeout, notification
//! delay) through [`limits`] when they need it, so a change applies to the
//! next job without a restart.

use std::time::Duration;

use omniget_core::core::llm::cli_runtime::WatchdogConfig;
use omniget_core::models::settings::LlmSettings;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::storage::config;
use crate::AppState;

/// Every timing the watchdog and the asks use, already clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub watchdog: WatchdogConfig,
    pub job_deadline_min: u32,
    pub ask_chat: Duration,
    /// `None` = a job's question waits until the job's own deadline.
    pub ask_job: Option<Duration>,
    pub notify_delay: Duration,
}

const DAY_S: u32 = 86_400;

/// The persisted fields, clamped. The same bounds apply on write
/// ([`llm_watchdog_set`]) and on read, so a hand-edited file cannot slip a
/// zero through either.
pub fn limits_from(s: &LlmSettings) -> Limits {
    let idle = s.turn_idle_timeout_s.clamp(60, DAY_S);
    Limits {
        watchdog: WatchdogConfig {
            idle: Some(Duration::from_secs(idle.into())),
            // Never shorter than the plain window: a tool gets at least as
            // long as a silent model.
            tool_idle: Some(Duration::from_secs(
                s.turn_tool_idle_timeout_s.clamp(idle, DAY_S).into(),
            )),
            start: Some(Duration::from_secs(
                s.turn_start_timeout_s.clamp(10, 3_600).into(),
            )),
        },
        job_deadline_min: s.job_default_deadline_min.clamp(1, 24 * 60),
        ask_chat: Duration::from_secs(s.ask_timeout_chat_s.clamp(30, DAY_S).into()),
        ask_job: s
            .ask_timeout_job_s
            .map(|v| Duration::from_secs(v.clamp(30, DAY_S).into())),
        notify_delay: Duration::from_secs(s.approval_notify_delay_s.min(300).into()),
    }
}

/// The current limits, from the settings store.
pub fn limits(app: &AppHandle) -> Limits {
    limits_from(&config::load_settings(app).llm)
}

/// Pushes the settings into the runtime and the broker. Called once when the
/// LLM stack is wired, and again on every change.
pub fn install(app: &AppHandle) {
    let l = limits(app);
    let llm = app.state::<AppState>().llm.clone();
    llm.set_watchdog(l.watchdog);
    llm.set_ask_timeout(l.ask_chat);
    if let Some(jobs) = crate::jobs::try_get() {
        jobs.attention.set_delay(l.notify_delay);
    }
}

fn view(s: &LlmSettings) -> Value {
    let l = limits_from(s);
    let secs = |d: Option<Duration>| d.map(|d| d.as_secs());
    json!({
        "turn_idle_timeout_s": secs(l.watchdog.idle),
        "turn_tool_idle_timeout_s": secs(l.watchdog.tool_idle),
        "turn_start_timeout_s": secs(l.watchdog.start),
        "job_default_deadline_min": l.job_deadline_min,
        "ask_timeout_chat_s": l.ask_chat.as_secs(),
        "ask_timeout_job_s": l.ask_job.map(|d| d.as_secs()),
        "approval_notify_delay_s": l.notify_delay.as_secs(),
        // Resume by itself after a restart (3.1); off by default.
        "auto_resume": s.auto_resume,
    })
}

#[tauri::command]
pub async fn llm_watchdog_get(app: AppHandle) -> Result<Value, String> {
    Ok(view(&config::load_settings(&app).llm))
}

/// What the card sends: every field optional, so it can save one at a time.
/// `ask_timeout_job_s: 0` means "until the job's deadline" (JSON cannot tell
/// a missing field from `null` through `Option<Option<_>>` without ceremony).
#[derive(Debug, Default, Deserialize)]
pub struct WatchdogInput {
    pub turn_idle_timeout_s: Option<u32>,
    pub turn_tool_idle_timeout_s: Option<u32>,
    pub turn_start_timeout_s: Option<u32>,
    pub job_default_deadline_min: Option<u32>,
    pub ask_timeout_chat_s: Option<u32>,
    pub ask_timeout_job_s: Option<u32>,
    pub approval_notify_delay_s: Option<u32>,
    pub auto_resume: Option<bool>,
}

/// Applies the input over the stored settings, clamped like [`limits_from`].
pub fn merge(mut s: LlmSettings, input: &WatchdogInput) -> LlmSettings {
    if let Some(v) = input.turn_idle_timeout_s {
        s.turn_idle_timeout_s = v.clamp(60, DAY_S);
    }
    if let Some(v) = input.turn_tool_idle_timeout_s {
        s.turn_tool_idle_timeout_s = v.clamp(60, DAY_S);
    }
    if let Some(v) = input.turn_start_timeout_s {
        s.turn_start_timeout_s = v.clamp(10, 3_600);
    }
    if let Some(v) = input.job_default_deadline_min {
        s.job_default_deadline_min = v.clamp(1, 24 * 60);
    }
    if let Some(v) = input.ask_timeout_chat_s {
        s.ask_timeout_chat_s = v.clamp(30, DAY_S);
    }
    if let Some(v) = input.ask_timeout_job_s {
        s.ask_timeout_job_s = (v > 0).then(|| v.clamp(30, DAY_S));
    }
    if let Some(v) = input.approval_notify_delay_s {
        s.approval_notify_delay_s = v.min(300);
    }
    if let Some(v) = input.auto_resume {
        s.auto_resume = v;
    }
    s
}

#[tauri::command]
pub async fn llm_watchdog_set(app: AppHandle, config: WatchdogInput) -> Result<Value, String> {
    let mut current = config::load_settings(&app);
    current.llm = merge(current.llm, &config);
    config::save_settings(&app, &current).map_err(|e| format!("Save: {e}"))?;
    install(&app);
    Ok(view(&current.llm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_documented_ones() {
        let l = limits_from(&LlmSettings::default());
        assert_eq!(l.watchdog, WatchdogConfig::default());
        assert_eq!(l.job_deadline_min, 60);
        assert_eq!(l.ask_chat, Duration::from_secs(1800));
        assert_eq!(l.ask_job, None);
        assert_eq!(l.notify_delay, Duration::from_secs(5));
    }

    #[test]
    fn absurd_values_are_clamped_on_read_and_on_write() {
        let s = LlmSettings {
            turn_idle_timeout_s: 1,
            turn_tool_idle_timeout_s: 5,
            turn_start_timeout_s: 0,
            job_default_deadline_min: 0,
            ask_timeout_chat_s: 0,
            ask_timeout_job_s: Some(2),
            approval_notify_delay_s: 999,
            ..LlmSettings::default()
        };
        let l = limits_from(&s);
        assert_eq!(l.watchdog.idle, Some(Duration::from_secs(60)));
        // The tool window is never under the plain one.
        assert_eq!(l.watchdog.tool_idle, Some(Duration::from_secs(60)));
        assert_eq!(l.watchdog.start, Some(Duration::from_secs(10)));
        assert_eq!(l.job_deadline_min, 1);
        assert_eq!(l.ask_chat, Duration::from_secs(30));
        assert_eq!(l.ask_job, Some(Duration::from_secs(30)));
        assert_eq!(l.notify_delay, Duration::from_secs(300));

        let merged = merge(
            LlmSettings::default(),
            &WatchdogInput {
                turn_idle_timeout_s: Some(0),
                ask_timeout_job_s: Some(0),
                approval_notify_delay_s: Some(10_000),
                ..WatchdogInput::default()
            },
        );
        assert_eq!(merged.turn_idle_timeout_s, 60);
        assert_eq!(
            merged.ask_timeout_job_s, None,
            "0 = until the job's deadline"
        );
        assert_eq!(merged.approval_notify_delay_s, 300);
        // Untouched fields keep their values.
        assert_eq!(merged.turn_start_timeout_s, 90);
    }
}
