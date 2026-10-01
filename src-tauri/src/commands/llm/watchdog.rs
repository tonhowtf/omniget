use std::time::Duration;

use omniget_core::core::llm::cli_runtime::WatchdogConfig;
use omniget_core::models::settings::LlmSettings;
use serde::Deserialize;
use serde_json::{json, Value};
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
        "auto_resume": s.auto_resume,
    })
}

#[tauri::command]
pub async fn llm_watchdog_get(app: AppHandle) -> Result<Value, String> {
    Ok(view(&config::load_settings(&app).llm))
}

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
    let current = config::update_settings(&app, |current| {
        current.llm = merge(current.llm.clone(), &config);
    })
    .map_err(|e| format!("Save: {e}"))?;
    crate::storage::settings_effects::settle(&[
        crate::storage::settings_effects::EffectId::LlmWatchdog,
    ]);
    Ok(view(&current.llm))
}
