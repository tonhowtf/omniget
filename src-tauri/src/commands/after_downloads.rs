//! Downloads › Tools › When downloads finish: quit OmniGet, put the computer
//! to sleep or shut it down once the queue is empty.
//!
//! The idea comes from Persepolis ("shut down after download/queue",
//! `persepolis/scripts/shutdown.py` at 5.2.0) and Varia (quit, sleep or shut
//! down with a cancel window, `src/download/listen.py`). Reimplemented here
//! without asking for a password: each system's own sleep and shutdown
//! command runs as the logged-in user. No code was copied.
//!
//! Armed state lives in memory only, like in both apps: a restart disarms it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::core::queue::QueueStatus;
use crate::AppState;

/// Seconds between "the queue is empty" and the action, so the person can
/// cancel (Varia waits 30 s too).
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
    /// None = disarmed.
    pub action: Option<FinishAction>,
    /// Downloads still queued or running (seeding and paused ones do not count).
    pub pending: usize,
    /// True once a download was seen running after arming.
    pub saw_work: bool,
    /// Seconds left before the action, while counting down.
    pub countdown: Option<u64>,
}

struct Armed {
    action: FinishAction,
    saw_work: bool,
    countdown: Option<u64>,
}

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);
/// Bumped on every change, so an old watcher loop stops by itself.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The command that performs `action` on `os` (`std::env::consts::OS`).
/// `Quit` is handled inside the app and has no command.
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

/// One step of the watcher: given what the queue holds now, what happens next.
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
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
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

/// `action`: "quit", "sleep", "shutdown", or null/"none" to disarm.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_while_downloads_run_or_before_any_ran() {
        assert_eq!(next_step(2, true, None), Step::Wait);
        assert_eq!(next_step(0, false, None), Step::Wait);
        // A new download during the countdown puts it back to waiting.
        assert_eq!(next_step(1, true, Some(10)), Step::Wait);
    }

    #[test]
    fn counts_down_then_fires() {
        let mut countdown = None;
        let mut steps = 0;
        loop {
            match next_step(0, true, countdown) {
                Step::Count(s) => countdown = Some(s),
                Step::Fire => break,
                Step::Wait => panic!("should not wait"),
            }
            steps += 1;
            assert!(steps < 100);
        }
        assert_eq!(steps as u64, GRACE_SECONDS / TICK.as_secs());
    }

    #[test]
    fn commands_per_system() {
        assert_eq!(power_command(FinishAction::Quit, "macos"), None);
        assert_eq!(
            power_command(FinishAction::Sleep, "macos").unwrap().0,
            "pmset"
        );
        assert_eq!(
            power_command(FinishAction::Shutdown, "macos").unwrap().0,
            "osascript"
        );
        assert_eq!(
            power_command(FinishAction::Shutdown, "windows").unwrap(),
            ("shutdown", vec!["/s", "/t", "0"])
        );
        assert_eq!(
            power_command(FinishAction::Sleep, "linux").unwrap(),
            ("systemctl", vec!["suspend"])
        );
        assert_eq!(
            power_command(FinishAction::Shutdown, "linux").unwrap(),
            ("systemctl", vec!["poweroff"])
        );
    }

    #[test]
    fn parses_actions() {
        assert_eq!(FinishAction::parse("sleep"), Some(FinishAction::Sleep));
        assert_eq!(FinishAction::parse("reboot"), None);
    }
}
