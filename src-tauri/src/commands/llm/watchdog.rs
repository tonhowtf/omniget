use std::time::Duration;

use omniget_core::core::llm::cli_runtime::WatchdogConfig;
use omniget_core::models::settings::LlmSettings;
use tauri::{AppHandle, Manager};

use crate::storage::config;
use crate::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub watchdog: WatchdogConfig,
    pub job_deadline_min: u32,
    pub ask_chat: Duration,
    pub ask_job: Option<Duration>,
    pub notify_delay: Duration,
}

const DAY_S: u32 = 86_400;

pub fn limits_from(s: &LlmSettings) -> Limits {
    let idle = s.turn_idle_timeout_s.clamp(60, DAY_S);
    Limits {
        watchdog: WatchdogConfig {
            idle: Some(Duration::from_secs(idle.into())),
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

pub fn limits(app: &AppHandle) -> Limits {
    limits_from(&config::load_settings(app).llm)
}

pub fn install(app: &AppHandle) {
    install_with(app, &config::load_settings(app).llm);
}

pub fn install_with(app: &AppHandle, settings: &LlmSettings) {
    let l = limits_from(settings);
    let llm = app.state::<AppState>().llm.clone();
    llm.set_watchdog(l.watchdog);
    llm.set_ask_timeout(l.ask_chat);
    if let Some(jobs) = crate::jobs::try_get() {
        jobs.attention.set_delay(l.notify_delay);
    }
}
