//! The app is not running: start it hidden and wait for its bridge (3.2).
//!
//! Every `omniget agent …` command needs the desktop app, which owns the
//! jobs. When its bridge does not answer, the CLI starts the app with
//! `--background` (tray only, no window), waits up to
//! [`BRIDGE_WAIT_S`] seconds for the bridge to take an authenticated
//! request, and goes on. If that cannot happen the error is structured:
//! [`ERR_APP_NOT_RUNNING`], the command that starts the app, and exit code
//! [`EXIT_APP_NOT_RUNNING`]. `OMNIGET_NO_LAUNCH=1` turns the start off.

use std::time::{Duration, Instant};

use omniget_core::core::app_launch::{
    self, AppLaunch, BACKGROUND_FLAG, BRIDGE_WAIT_S, ERR_APP_NOT_RUNNING,
};
use serde_json::json;

use super::agent::{job_context, Bridge};

/// `EX_UNAVAILABLE` of `sysexits.h`: a script can tell "the app is not
/// there" from a failed job.
pub const EXIT_APP_NOT_RUNNING: i32 = 69;

/// The app is not running and was not (or could not be) started.
#[derive(Debug)]
pub struct AppNotRunning {
    pub message: String,
    /// The command that starts the app, when one was found.
    pub launch: Option<String>,
}

impl std::fmt::Display for AppNotRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{ERR_APP_NOT_RUNNING}: {}", self.message)?;
        if let Some(l) = &self.launch {
            write!(f, " (start it with: {l})")?;
        }
        Ok(())
    }
}

impl std::error::Error for AppNotRunning {}

impl AppNotRunning {
    /// The JSON the CLI prints with `--json`.
    pub fn to_json(&self) -> serde_json::Value {
        json!({ "error": { "code": ERR_APP_NOT_RUNNING, "message": self.message, "launch": self.launch } })
    }
}

fn command_line(l: &AppLaunch) -> String {
    std::iter::once(l.program.as_str())
        .chain(l.args.iter().map(String::as_str))
        .chain(std::iter::once(BACKGROUND_FLAG))
        .map(|p| {
            if p.contains(' ') {
                format!("\"{p}\"")
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A bridge that answers an authenticated request, or `None`.
async fn ready() -> Option<Bridge> {
    let bridge = Bridge::open().ok()?;
    bridge.ready().await.then_some(bridge)
}

/// One CLI starts the app; another one started at the same moment waits
/// for it instead of starting a second copy. A lock older than the wait is
/// left over from a CLI that died, and is taken over.
struct LaunchLock(Option<std::path::PathBuf>);

impl LaunchLock {
    fn take(dir: &std::path::Path) -> Self {
        let path = dir.join("app-launch.lock");
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(BRIDGE_WAIT_S + 5));
        if fresh {
            return Self(None);
        }
        let _ = std::fs::remove_file(&path);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Self(Some(path)),
            Err(_) => Self(None),
        }
    }
}

impl Drop for LaunchLock {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The bridge of the running app; starts the app hidden first when nothing
/// answers.
pub async fn connect() -> anyhow::Result<Bridge> {
    if let Some(b) = ready().await {
        return Ok(b);
    }
    // Inside a job the app is running by definition; a job never starts it.
    if job_context().is_some() {
        return Bridge::open();
    }
    let dir = omniget_core::core::paths::app_data_dir();
    let candidates = app_launch::candidates(dir.as_deref());
    let first = candidates.first().cloned();
    if std::env::var("OMNIGET_NO_LAUNCH").is_ok_and(|v| v == "1") {
        return Err(AppNotRunning {
            message: "the OmniGet app is not running (OMNIGET_NO_LAUNCH=1: not starting it)".into(),
            launch: first.as_ref().map(command_line),
        }
        .into());
    }
    let Some(launch) = first else {
        return Err(AppNotRunning {
            message: "the OmniGet app is not running and was not found on this computer; open it once, or set OMNIGET_APP to its executable".into(),
            launch: None,
        }
        .into());
    };
    let t0 = Instant::now();
    let lock = dir
        .as_deref()
        .map(LaunchLock::take)
        .unwrap_or(LaunchLock(None));
    let started = if lock.0.is_some() {
        match app_launch::spawn_detached(&launch) {
            Ok(pid) => Some(pid),
            Err(e) => {
                return Err(AppNotRunning {
                    message: format!(
                        "the OmniGet app is not running and could not be started: {e}"
                    ),
                    launch: Some(command_line(&launch)),
                }
                .into())
            }
        }
    } else {
        // Another CLI is starting it right now.
        None
    };
    while t0.elapsed() < Duration::from_secs(BRIDGE_WAIT_S) {
        tokio::time::sleep(Duration::from_millis(150)).await;
        // A fresh profile gets its CLI token a moment after the bridge
        // answers: wait for it rather than fall back to the extension key
        // (and its deprecation warning), for half the wait at most.
        let token_file = dir
            .as_deref()
            .is_some_and(|d| d.join("llm").join("cli-token").is_file());
        if !token_file
            && std::env::var_os("OMNIGET_TOKEN").is_none()
            && t0.elapsed() < Duration::from_secs(BRIDGE_WAIT_S / 2)
        {
            continue;
        }
        if let Some(b) = ready().await {
            drop(lock);
            let what = match started {
                Some(pid) => format!("started the OmniGet app in the background (pid {pid})"),
                None => "the OmniGet app another command started".to_string(),
            };
            eprintln!(
                "omniget: {what}; its bridge answered in {} ms",
                t0.elapsed().as_millis()
            );
            return Ok(b);
        }
    }
    Err(AppNotRunning {
        message: format!(
            "started the OmniGet app in the background, but its bridge did not answer within {BRIDGE_WAIT_S} s"
        ),
        launch: Some(command_line(&launch)),
    }
    .into())
}
