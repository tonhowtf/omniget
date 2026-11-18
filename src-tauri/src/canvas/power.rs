#[cfg(not(windows))]
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use omniget_core::core::term::agent::state::AgentState;
use omniget_core::models::settings::CanvasSettings;
use tauri::AppHandle;
use tokio::sync::Notify;

const TICK: Duration = Duration::from_secs(5);

#[cfg(windows)]
struct Assertion {
    stop: std::sync::mpsc::Sender<()>,
}

#[cfg(windows)]
impl Assertion {
    fn alive(&mut self) -> bool {
        true
    }

    fn release(self) {
        drop(self.stop);
    }
}

#[cfg(not(windows))]
struct Assertion(Child);

#[cfg(not(windows))]
impl Assertion {
    fn alive(&mut self) -> bool {
        !matches!(self.0.try_wait(), Ok(Some(_)))
    }

    fn release(mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Hold {
    assertion: Option<Assertion>,
}

fn hold() -> &'static Mutex<Hold> {
    static H: OnceLock<Mutex<Hold>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Hold { assertion: None }))
}

fn wake() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}

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

fn drop_dead(h: &mut Hold) {
    if h.assertion.as_mut().is_some_and(|a| !a.alive()) {
        h.assertion = None;
    }
}

fn set_hold(on: bool) {
    let mut h = hold().lock().unwrap_or_else(|e| e.into_inner());
    drop_dead(&mut h);
    match (on, h.assertion.is_some()) {
        (true, false) => h.assertion = spawn_assertion(),
        (false, true) => {
            if let Some(a) = h.assertion.take() {
                a.release();
            }
        }
        _ => {}
    }
}

#[cfg(windows)]
fn spawn_assertion() -> Option<Assertion> {
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name("canvas-keep-awake".into())
        .spawn(move || {
            // SAFETY: the call takes plain flags and only changes this thread's execution state.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            let _ = stopped.recv();
            // SAFETY: same as above; it clears what this thread asked for.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        });
    match spawned {
        Ok(_) => Some(Assertion { stop }),
        Err(e) => {
            tracing::warn!("[canvas] sleep prevention unavailable: {e}");
            None
        }
    }
}

#[cfg(not(windows))]
fn spawn_assertion() -> Option<Assertion> {
    let mut cmd = if cfg!(target_os = "macos") {
        let pid = std::process::id().to_string();
        let mut c = Command::new("/usr/bin/caffeinate");
        c.args(["-i", "-w", &pid]);
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
    match cmd.spawn() {
        Ok(c) => Some(Assertion(c)),
        Err(e) => {
            tracing::warn!("[canvas] sleep prevention unavailable: {e}");
            None
        }
    }
}

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

pub fn over_limit(procs: &[(u32, u32, u64)], roots: &[u32], limit_kb: u64) -> Vec<u32> {
    let mut inside: std::collections::HashSet<u32> = roots.iter().copied().collect();
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
