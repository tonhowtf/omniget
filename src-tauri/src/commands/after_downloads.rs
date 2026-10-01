use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::core::queue::QueueStatus;
use crate::AppState;

pub const GRACE_SECONDS: u64 = 30;
const TICK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FinishAction {
    Quit,
    Sleep,
    Shutdown,
}

impl FinishAction {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "quit" => Some(Self::Quit),
            "sleep" => Some(Self::Sleep),
            "shutdown" => Some(Self::Shutdown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AfterDownloadsState {
    pub action: Option<FinishAction>,
    pub pending: usize,
    pub saw_work: bool,
    pub countdown: Option<u64>,
}

struct Armed {
    action: FinishAction,
    saw_work: bool,
    countdown: Option<u64>,
}

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn power_command(action: FinishAction, os: &str) -> Option<(&'static str, Vec<&'static str>)> {
    match (action, os) {
        (FinishAction::Quit, _) => None,
        (FinishAction::Sleep, "macos") => Some(("pmset", vec!["sleepnow"])),
        (FinishAction::Shutdown, "macos") => Some((
            "osascript",
            vec!["-e", "tell application \"System Events\" to shut down"],
        )),
        (FinishAction::Sleep, "windows") => Some((
            "rundll32.exe",
            vec!["powrprof.dll,SetSuspendState", "0,1,0"],
        )),
        (FinishAction::Shutdown, "windows") => Some(("shutdown", vec!["/s", "/t", "0"])),
        (FinishAction::Sleep, _) => Some(("systemctl", vec!["suspend"])),
        (FinishAction::Shutdown, _) => Some(("systemctl", vec!["poweroff"])),
    }
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Wait,
    Count(u64),
    Fire,
}

pub fn next_step(pending: usize, saw_work: bool, countdown: Option<u64>) -> Step {
    if pending > 0 || !saw_work {
        return Step::Wait;
    }
    match countdown {
        None => Step::Count(GRACE_SECONDS),
        Some(s) if s <= TICK.as_secs() => Step::Fire,
        Some(s) => Step::Count(s - TICK.as_secs()),
    }
}

async fn pending_downloads(app: &AppHandle) -> usize {
    let queue = app.state::<AppState>().download_queue.clone();
    let q = queue.lock().await;
    q.items
        .iter()
        .filter(|i| matches!(i.status, QueueStatus::Queued | QueueStatus::Active))
        .count()
}

fn snapshot(pending: usize) -> AfterDownloadsState {
    let armed = ARMED.lock().unwrap_or_else(|e| e.into_inner());
    AfterDownloadsState {
        action: armed.as_ref().map(|a| a.action),
        pending,
        saw_work: armed.as_ref().is_some_and(|a| a.saw_work),
        countdown: armed.as_ref().and_then(|a| a.countdown),
    }
}

fn perform(app: &AppHandle, action: FinishAction) {
    tracing::info!("[after-downloads] queue empty, running {:?}", action);
    if action == FinishAction::Quit {
        app.exit(0);
        return;
    }
    if let Some((program, args)) = power_command(action, std::env::consts::OS) {
        let mut cmd = std::process::Command::new(program);
        cmd.args(&args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        if let Err(e) = cmd.spawn() {
            tracing::warn!("[after-downloads] {program} failed: {e}");
        }
    }
}

fn watch(app: AppHandle, generation: u64) {
    tauri::async_runtime::spawn(async move {
        loop {
            if GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            let pending = pending_downloads(&app).await;
            let fire = {
                let mut armed = ARMED.lock().unwrap_or_else(|e| e.into_inner());
                let Some(a) = armed.as_mut() else { return };
                if pending > 0 {
                    a.saw_work = true;
                }
                match next_step(pending, a.saw_work, a.countdown) {
                    Step::Wait => {
                        a.countdown = None;
                        None
                    }
                    Step::Count(s) => {
                        a.countdown = Some(s);
                        None
                    }
                    Step::Fire => {
                        let action = a.action;
                        *armed = None;
                        Some(action)
                    }
                }
            };
            if let Some(action) = fire {
                GENERATION.fetch_add(1, Ordering::SeqCst);
                perform(&app, action);
                return;
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[tauri::command]
pub async fn after_downloads_get(app: AppHandle) -> Result<AfterDownloadsState, String> {
    let pending = pending_downloads(&app).await;
    Ok(snapshot(pending))
}

#[tauri::command]
pub async fn after_downloads_set(
    app: AppHandle,
    action: Option<String>,
) -> Result<AfterDownloadsState, String> {
    let parsed = match action.as_deref() {
        None | Some("") | Some("none") => None,
        Some(v) => Some(FinishAction::parse(v).ok_or("AFTER_DOWNLOADS_UNKNOWN_ACTION")?),
    };
    let pending = pending_downloads(&app).await;
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let mut armed = ARMED.lock().unwrap_or_else(|e| e.into_inner());
        *armed = parsed.map(|action| Armed {
            action,
            saw_work: pending > 0,
            countdown: None,
        });
    }
    if parsed.is_some() {
        watch(app, generation);
    }
    Ok(snapshot(pending))
}
