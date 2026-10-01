//! Canvas watchers that act on the machine (Maestri: sleep prevention and
//! the per-terminal memory limit).
//!
//! Sleep prevention follows real agent activity, not live processes: with
//! `while_working` an assertion is held while any terminal's fused state is
//! working, thinking, compacting or waiting on a peer. The assertion is a
//! child process that holds it for as long as it lives (`caffeinate` on
//! macOS, `systemd-inhibit` on Linux, a PowerShell `SetThreadExecutionState`
//! loop on Windows), so releasing it is killing the child, and a crash of the
//! app releases it too.
//!
//! Memory limit: every tick, each terminal's descendant processes (never the
//! terminal's own process, the shell) above the limit are killed.

use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use omniget_core::core::term::agent::state::AgentState;
use omniget_core::models::settings::CanvasSettings;
use tauri::AppHandle;
use tokio::sync::Notify;

const TICK: Duration = Duration::from_secs(5);

struct Hold {
    child: Option<Child>,
}

fn hold() -> &'static Mutex<Hold> {
    static H: OnceLock<Mutex<Hold>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Hold { child: None }))
}

fn wake() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}

/// Starts the watcher once (idempotent). Called at boot and by the canvas
/// terminal host, so it runs whenever terminals exist.
pub fn start(app: &AppHandle) {
    static ON: OnceLock<()> = OnceLock::new();
    ON.get_or_init(|| {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let canvas = crate::storage::config::load_settings(&app).canvas;
                let app2 = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || tick(&app2, &canvas)).await;
                tokio::select! {
                    _ = tokio::time::sleep(TICK) => {}
                    _ = wake().notified() => {}
                }
            }
        });
    });
}

/// Re-evaluates now (after a settings change).
pub fn refresh(app: &AppHandle) {
    start(app);
    wake().notify_one();
}

fn working(s: AgentState) -> bool {
    matches!(
        s,
        AgentState::Working
            | AgentState::Thinking
            | AgentState::WaitingPeer
            | AgentState::Compacting
    )
}

/// Whether the assertion should be held for this mode and activity.
pub fn wants_hold(mode: &str, any_working: bool) -> bool {
    match mode {
        "always" => true,
        "never" => false,
        _ => any_working,
    }
}

fn tick(app: &AppHandle, canvas: &CanvasSettings) {
    let term = super::term::host(app);
    let infos = term.list();
    let any_working = infos.iter().any(|i| i.alive && working(i.agent.state));
    set_hold(wants_hold(&canvas.power.prevent_sleep, any_working));
    if canvas.terminal.memory_limit_mb > 0 {
        let roots: Vec<u32> = infos
            .iter()
            .filter(|i| i.alive)
            .filter_map(|i| i.pid)
            .collect();
        enforce_memory(&roots, canvas.terminal.memory_limit_mb as u64 * 1024);
    }
}

/// Whether the assertion is held right now (for status tools).
pub fn holding() -> bool {
    let mut h = hold().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = h.child.as_mut() {
        if matches!(c.try_wait(), Ok(Some(_))) {
            h.child = None;
        }
    }
    h.child.is_some()
}

fn set_hold(on: bool) {
    let mut h = hold().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = h.child.as_mut() {
        // The helper died (killed by someone): take it again on the next hold.
        if matches!(c.try_wait(), Ok(Some(_))) {
            h.child = None;
        }
    }
    match (on, h.child.is_some()) {
        (true, false) => h.child = spawn_assertion(),
        (false, true) => {
            if let Some(mut c) = h.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        _ => {}
    }
}

fn spawn_assertion() -> Option<Child> {
    let pid = std::process::id().to_string();
    let mut cmd = if cfg!(target_os = "macos") {
        // -i: no idle sleep; -w: also ends if the app dies.
        let mut c = Command::new("/usr/bin/caffeinate");
        c.args(["-i", "-w", &pid]);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$t=Add-Type -Name P -Namespace OmniGet -PassThru -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; \
             $t::SetThreadExecutionState(0x80000001) | Out-Null; while ($true) { Start-Sleep -Seconds 60 }",
        ]);
        c
    } else {
        let mut c = Command::new("systemd-inhibit");
        c.args([
            "--what=idle:sleep",
            "--who=OmniGet",
            "--why=Agents are working",
            "--mode=block",
            "sleep",
            "infinity",
        ]);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match cmd.spawn() {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!("[canvas] sleep prevention unavailable: {e}");
            None
        }
    }
}

/// `(pid, ppid, rss_kb)` of every process (Unix `ps`).
#[cfg(unix)]
fn processes() -> Vec<(u32, u32, u64)> {
    let Ok(out) = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,rss="])
        .output()
    else {
        return Vec::new();
    };
    parse_ps(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(unix))]
fn processes() -> Vec<(u32, u32, u64)> {
    Vec::new()
}

pub fn parse_ps(text: &str) -> Vec<(u32, u32, u64)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((
                it.next()?.parse().ok()?,
                it.next()?.parse().ok()?,
                it.next()?.parse().ok()?,
            ))
        })
        .collect()
}

/// Descendants of `roots` (not the roots) whose RSS is above `limit_kb`.
pub fn over_limit(procs: &[(u32, u32, u64)], roots: &[u32], limit_kb: u64) -> Vec<u32> {
    let mut inside: std::collections::HashSet<u32> = roots.iter().copied().collect();
    // Grow the tree until stable (ps order is not parent-first).
    loop {
        let before = inside.len();
        for &(pid, ppid, _) in procs {
            if inside.contains(&ppid) {
                inside.insert(pid);
            }
        }
        if inside.len() == before {
            break;
        }
    }
    procs
        .iter()
        .filter(|(pid, _, rss)| !roots.contains(pid) && inside.contains(pid) && *rss > limit_kb)
        .map(|(pid, _, _)| *pid)
        .collect()
}

fn enforce_memory(roots: &[u32], limit_kb: u64) {
    if roots.is_empty() {
        return;
    }
    for pid in over_limit(&processes(), roots, limit_kb) {
        tracing::warn!("[canvas] terminal child {pid} above the memory limit, killing it");
        #[cfg(unix)]
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}
