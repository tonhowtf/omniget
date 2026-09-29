//! One capture at a time: start the backend, sample the machine once a
//! second into `sensors.csv`, follow the backend's log for a live FPS, stop
//! by itself at the deadline (or when the game exits), then convert
//! everything to `omniget-frames/1` and write the summary.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::format::{
    self, Frame, LaunchCommand, Machine, SensorRow, Session, SourceTool, Summary, Target,
};
use crate::core::washmybelly::store::Ctx;

pub fn perf_dir(ctx: &Ctx) -> PathBuf {
    ctx.dir.join("perf")
}

/// Ids are ours (`<ms>-<hex>`); anything else is refused before it becomes
/// a path.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn session_dir(ctx: &Ctx, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("NOT_FOUND: unknown session".into());
    }
    Ok(perf_dir(ctx).join(id))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// What a listing shows per session (and what the League tab reads).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    pub session_id: String,
    pub label: String,
    pub method: String,
    pub status: String,
    pub target: Target,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub frames: usize,
    pub fps_avg: Option<f64>,
    pub low_1: Option<f64>,
    pub low_01: Option<f64>,
    pub stutters: usize,
    pub freezes: usize,
    pub reason: Option<String>,
}

pub fn load_session(dir: &Path) -> Result<Session, String> {
    format::read_json(&dir.join("session.json"))
}

pub fn load_summary(dir: &Path) -> Option<Summary> {
    format::read_json(&dir.join("summary.json")).ok()
}

pub fn list_summary(s: &Session, sum: Option<&Summary>) -> SessionSummary {
    SessionSummary {
        session_id: s.id.clone(),
        label: s.label.clone(),
        method: s.method.clone(),
        status: s.status.clone(),
        target: s.target.clone(),
        started_ms: s.started_ms,
        ended_ms: s.ended_ms,
        frames: s.frames,
        fps_avg: sum.and_then(|x| x.stats.fps_avg),
        low_1: sum.and_then(|x| x.stats.low_1),
        low_01: sum.and_then(|x| x.stats.low_01),
        stutters: sum.map(|x| x.stats.stutters).unwrap_or(0),
        freezes: sum.map(|x| x.stats.freezes).unwrap_or(0),
        reason: s.reason.clone(),
    }
}

/// Every session in `ctx`, newest first.
pub fn list(ctx: &Ctx) -> Vec<SessionSummary> {
    let Ok(rd) = std::fs::read_dir(perf_dir(ctx)) else {
        return vec![];
    };
    let mut out: Vec<SessionSummary> = rd
        .flatten()
        .filter(|e| e.path().is_dir() && e.file_name().to_str().is_some_and(valid_id))
        .filter_map(|e| {
            let s = load_session(&e.path()).ok()?;
            let sum = load_summary(&e.path());
            Some(list_summary(&s, sum.as_ref()))
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.started_ms));
    out
}

/// Sessions that overlap `[start_ms, end_ms]` (a League match, for one).
pub fn between(ctx: &Ctx, start_ms: u64, end_ms: u64) -> Vec<SessionSummary> {
    list(ctx)
        .into_iter()
        .filter(|s| {
            let end = s.ended_ms.unwrap_or(u64::MAX);
            s.started_ms <= end_ms && end >= start_ms
        })
        .collect()
}

pub fn read_frames(dir: &Path) -> Vec<Frame> {
    std::fs::read_to_string(dir.join("frames.csv"))
        .map(|t| format::parse_frames_csv(&t))
        .unwrap_or_default()
}

pub fn read_sensors(dir: &Path) -> Vec<SensorRow> {
    std::fs::read_to_string(dir.join("sensors.csv"))
        .map(|t| format::parse_sensors_csv(&t))
        .unwrap_or_default()
}

/// The summary on disk, or computed now from `frames.csv` and cached.
pub fn summary_of(ctx: &Ctx, id: &str) -> Result<Summary, String> {
    let dir = session_dir(ctx, id)?;
    let s = load_session(&dir).map_err(|_| "NOT_FOUND: unknown session".to_string())?;
    if let Some(sum) = load_summary(&dir) {
        return Ok(sum);
    }
    if s.status == "recording" || s.status == "waiting_launch" {
        return Err("RECORDING: the capture is still running; stop it first".into());
    }
    let sum = format::summarize(&s, &read_frames(&dir), &read_sensors(&dir));
    let _ = format::write_json(&dir.join("summary.json"), &sum);
    Ok(sum)
}

// ── The running capture ──────────────────────────────────────────────────

/// Which log the sampler follows for the live FPS.
enum LiveSource {
    None,
    #[cfg_attr(not(windows), allow(dead_code))]
    PresentMon(PathBuf, Option<super::presentmon::Columns>),
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    MangoHud(PathBuf, Option<super::mangohud::Columns>),
    /// Frames pushed by the stderr reader (Metal HUD).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Pushed,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Live {
    pub frames: usize,
    pub fps_now: Option<f64>,
    /// Last frametimes, newest last, for the live sparkline.
    pub recent: VecDeque<f64>,
    #[serde(skip)]
    window: VecDeque<(Instant, f64)>,
    pub game_exited: bool,
}

impl Live {
    pub fn push(&mut self, ft: f64) {
        let now = Instant::now();
        self.frames += 1;
        self.recent.push_back(ft);
        while self.recent.len() > 240 {
            self.recent.pop_front();
        }
        self.window.push_back((now, ft));
        while self
            .window
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(1))
        {
            self.window.pop_front();
        }
        let sum: f64 = self.window.iter().map(|(_, f)| f).sum();
        self.fps_now =
            (sum > 0.0).then(|| super::stats::round(self.window.len() as f64 * 1000.0 / sum, 1));
    }
}

struct Active {
    id: String,
    dir: PathBuf,
    started: Instant,
    stop: Arc<AtomicBool>,
    live: Arc<Mutex<Live>>,
    sampler: Option<std::thread::JoinHandle<()>>,
    backend: Backend,
}

enum Backend {
    Telemetry,
    #[cfg_attr(not(windows), allow(dead_code))]
    PresentMon {
        exe: PathBuf,
        session_name: String,
        child: std::process::Child,
        csv: PathBuf,
        pid: Option<u32>,
    },
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    MangoHud,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Metal {
        frames: Arc<Mutex<Vec<Frame>>>,
        recording: Arc<AtomicBool>,
    },
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

pub fn active_id() -> Option<String> {
    ACTIVE.lock().ok()?.as_ref().map(|a| a.id.clone())
}

/// Live state for the screen.
pub fn status() -> serde_json::Value {
    let Ok(g) = ACTIVE.lock() else {
        return serde_json::json!({ "active": false });
    };
    let Some(a) = g.as_ref() else {
        return serde_json::json!({ "active": false });
    };
    let live = a.live.lock().map(|l| l.clone()).unwrap_or_default();
    let session = load_session(&a.dir).ok();
    serde_json::json!({
        "active": true,
        "session_id": a.id,
        "status": session.as_ref().map(|s| s.status.clone()),
        "method": session.as_ref().map(|s| s.method.clone()),
        "label": session.as_ref().map(|s| s.label.clone()),
        "launch": session.as_ref().map(|s| s.launch.clone()).unwrap_or_default(),
        "elapsed_s": a.started.elapsed().as_secs(),
        "max_seconds": session.as_ref().map(|s| s.max_seconds),
        "frames": live.frames,
        "fps_now": live.fps_now,
        "recent": live.recent,
    })
}

/// A new session folder with its `session.json`.
pub struct NewSession {
    pub label: String,
    pub method: &'static str,
    pub frame_mode: &'static str,
    pub status: &'static str,
    pub target: Target,
    pub machine: Machine,
    pub source: SourceTool,
    pub max_seconds: u64,
    pub launch: Vec<LaunchCommand>,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
}

pub fn new_id(started_ms: u64) -> String {
    format!(
        "{}-{}",
        started_ms,
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    )
}

pub fn create(
    ctx: &Ctx,
    id: &str,
    started_ms: u64,
    n: NewSession,
) -> Result<(PathBuf, Session), String> {
    let dir = session_dir(ctx, id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let s = Session {
        format: format::FORMAT.into(),
        id: id.into(),
        label: n.label,
        status: n.status.into(),
        method: n.method.into(),
        frame_mode: n.frame_mode.into(),
        target: n.target,
        machine: n.machine,
        source: n.source,
        started_ms,
        ended_ms: None,
        max_seconds: n.max_seconds,
        frames: 0,
        warnings: n.warnings,
        reason: n.reason,
        launch: n.launch,
    };
    format::write_json(&dir.join("session.json"), &s)?;
    std::fs::write(
        dir.join("sensors.csv"),
        format!("{}\n", format::SENSORS_HEADER),
    )
    .map_err(|e| e.to_string())?;
    Ok((dir, s))
}

/// Register the running capture and start the 1 Hz sampler.
#[allow(private_interfaces)]
fn begin(
    id: String,
    dir: PathBuf,
    max_seconds: u64,
    backend: Backend,
    source: LiveSource,
    pid: Option<u32>,
    live: Arc<Mutex<Live>>,
) -> Result<(), String> {
    let mut g = ACTIVE.lock().map_err(|_| "perf lock")?;
    if let Some(a) = g.as_ref() {
        return Err(format!("BUSY: capture {} is already running", a.id));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let deadline = started + Duration::from_secs(max_seconds);
    let sampler = {
        let (stop, live, dir) = (stop.clone(), live.clone(), dir.clone());
        std::thread::Builder::new()
            .name("wmb-perf".into())
            .spawn(move || sample_loop(dir, started, deadline, stop, live, source, pid))
            .map_err(|e| e.to_string())?
    };
    *g = Some(Active {
        id,
        dir,
        started,
        stop,
        live,
        sampler: Some(sampler),
        backend,
    });
    Ok(())
}

fn pid_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    let p = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[p]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(p).is_some()
}

/// Read what was appended to `path` since the last call, whole lines only.
#[derive(Default)]
struct Tail {
    pos: u64,
    partial: String,
}

impl Tail {
    fn lines(&mut self, path: &Path) -> Vec<String> {
        let Ok(mut f) = std::fs::File::open(path) else {
            return vec![];
        };
        if f.seek(SeekFrom::Start(self.pos)).is_err() {
            return vec![];
        }
        let mut buf = Vec::new();
        let _ = f.take(4 << 20).read_to_end(&mut buf);
        self.pos += buf.len() as u64;
        self.partial.push_str(&String::from_utf8_lossy(&buf));
        let mut out: Vec<String> = self.partial.split('\n').map(str::to_string).collect();
        self.partial = out.pop().unwrap_or_default();
        out
    }
}

fn sample_loop(
    dir: PathBuf,
    started: Instant,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    live: Arc<Mutex<Live>>,
    mut source: LiveSource,
    pid: Option<u32>,
) {
    let mut tail = Tail::default();
    let mut tick = 0u64;
    let mut rows = String::new();
    while !stop.load(Ordering::Relaxed) {
        let t_ms = started.elapsed().as_millis() as f64;
        if let Ok(v) = crate::core::washmybelly::monitor::live_snapshot() {
            rows.push_str(&format::sensor_line(&format::sensor_from_snapshot(
                &v, t_ms,
            )));
            rows.push('\n');
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .append(true)
                .open(dir.join("sensors.csv"))
            {
                use std::io::Write;
                if f.write_all(rows.as_bytes()).is_ok() {
                    rows.clear();
                }
            }
        }
        follow(&mut source, &mut tail, &dir, &live);
        let exited = live.lock().map(|l| l.game_exited).unwrap_or(false)
            || (tick % 3 == 2 && pid.is_some_and(|p| !pid_alive(p)));
        if Instant::now() >= deadline || exited {
            let why = if exited {
                "target_exited"
            } else {
                "time_limit"
            };
            let _ = std::thread::Builder::new()
                .name("wmb-perf-stop".into())
                .spawn(move || {
                    let _ = finish(why);
                });
            return;
        }
        tick += 1;
        // Sleep in small steps so a stop is quick.
        let until = started + Duration::from_secs(tick);
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    follow(&mut source, &mut tail, &dir, &live);
}

fn follow(source: &mut LiveSource, tail: &mut Tail, dir: &Path, live: &Arc<Mutex<Live>>) {
    let _ = dir;
    match source {
        LiveSource::None | LiveSource::Pushed => {}
        LiveSource::PresentMon(path, cols) => {
            for l in tail.lines(path) {
                match cols {
                    None => *cols = super::presentmon::Columns::from_header(&l),
                    Some(c) => {
                        if let (Some(r), Ok(mut lv)) = (c.row(&l), live.lock()) {
                            lv.push(r.frame.frametime_ms);
                        }
                    }
                }
            }
        }
        LiveSource::MangoHud(path, cols) => {
            if path.as_os_str().is_empty() {
                match super::mangohud::newest_log(dir) {
                    Some(p) => *path = p,
                    None => return,
                }
            }
            for l in tail.lines(path) {
                match cols {
                    None => *cols = super::mangohud::Columns::from_header(&l),
                    Some(c) => {
                        if let (Some(r), Ok(mut lv)) = (c.row(&l), live.lock()) {
                            lv.push(r.frametime_ms);
                        }
                    }
                }
            }
        }
    }
}

/// Stop the running capture (by the person, the agent, the deadline or the
/// game exiting), convert the frames and write the summary.
pub fn finish(why: &str) -> Result<Session, String> {
    let mut a = {
        let mut g = ACTIVE.lock().map_err(|_| "perf lock")?;
        g.take().ok_or("NOT_RUNNING: no capture is running")?
    };
    a.stop.store(true, Ordering::Relaxed);
    if let Some(h) = a.sampler.take() {
        if h.thread().id() != std::thread::current().id() {
            let _ = h.join();
        }
    }
    let mut s = load_session(&a.dir)?;
    let mut sensors = read_sensors(&a.dir);
    let mut frames: Vec<Frame> = Vec::new();
    match a.backend {
        Backend::Telemetry => {}
        #[cfg(windows)]
        Backend::PresentMon {
            exe,
            session_name,
            mut child,
            csv,
            pid,
        } => {
            super::presentmon::stop(&exe, &session_name);
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(5) {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let mut err = String::new();
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            match std::fs::read_to_string(&csv) {
                Ok(text) => {
                    let p = super::presentmon::parse_csv(&text, pid);
                    frames = p.frames;
                    s.warnings.extend(p.warnings);
                    if s.target.name.is_none() {
                        s.target.name = p.application;
                    }
                    if !p.has_display {
                        s.warnings.push("display_times_unavailable".into());
                    }
                }
                Err(_) => {
                    s.reason = Some(
                        if err.to_ascii_lowercase().contains("access") {
                            "access_denied"
                        } else {
                            "no_frames_seen"
                        }
                        .into(),
                    );
                }
            }
        }
        #[cfg(not(windows))]
        Backend::PresentMon { .. } => {}
        Backend::MangoHud => match super::mangohud::newest_log(&a.dir) {
            Some(path) => {
                let p =
                    super::mangohud::parse_csv(&std::fs::read_to_string(&path).unwrap_or_default());
                frames = p.frames;
                s.warnings.extend(p.warnings);
                if sensors.is_empty() {
                    sensors = p.sensors;
                    let _ =
                        std::fs::write(a.dir.join("sensors.csv"), format::sensors_csv(&sensors));
                }
                if s.machine.gpu.is_empty() {
                    s.machine.gpu = p.system.get("gpu").cloned().unwrap_or_default();
                }
                s.source.version = p.system.get("driver").cloned().unwrap_or_default();
            }
            None => s.reason = Some("game_not_started_with_command".into()),
        },
        Backend::Metal {
            frames: f,
            recording,
        } => {
            recording.store(false, Ordering::Relaxed);
            frames = f.lock().map(|v| v.clone()).unwrap_or_default();
            if frames.is_empty() {
                s.reason = Some("no_frames_seen".into());
            }
        }
    }
    if frames.is_empty() && s.reason.is_none() && s.frame_mode != "none" {
        s.reason = Some("no_frames_seen".into());
    }
    std::fs::write(a.dir.join("frames.csv"), format::frames_csv(&frames))
        .map_err(|e| e.to_string())?;
    s.frames = frames.len();
    s.ended_ms = Some(now_ms());
    s.status = if frames.is_empty() && s.frame_mode != "none" {
        "failed".into()
    } else {
        "done".into()
    };
    if why == "target_exited" {
        s.warnings.push("target_exited".into());
    }
    s.warnings.dedup();
    format::write_json(&a.dir.join("session.json"), &s)?;
    let sum = format::summarize(&s, &frames, &sensors);
    format::write_json(&a.dir.join("summary.json"), &sum)?;
    crate::core::washmybelly::progress("perf_capture", &s.status, 1, 1, s.id.clone());
    Ok(s)
}

// ── Starting a capture, per backend ──────────────────────────────────────

pub struct StartArgs {
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub max_seconds: u64,
    pub label: String,
    pub machine: Machine,
}

fn target(a: &StartArgs) -> Target {
    Target {
        name: a.process_name.clone(),
        pid: a.pid,
    }
}

/// Only sensors (macOS for a running game, or when nothing else exists).
pub fn start_telemetry(ctx: &Ctx, a: StartArgs, reason: &str) -> Result<Session, String> {
    let started_ms = now_ms();
    let id = new_id(started_ms);
    let (dir, s) = create(
        ctx,
        &id,
        started_ms,
        NewSession {
            label: a.label.clone(),
            method: "telemetry",
            frame_mode: "none",
            status: "recording",
            target: target(&a),
            machine: a.machine.clone(),
            source: SourceTool {
                tool: "omniget-monitor".into(),
                ..Default::default()
            },
            max_seconds: a.max_seconds,
            launch: if cfg!(target_os = "macos") {
                super::metal::launch_commands(None)
            } else {
                vec![]
            },
            reason: Some(reason.into()),
            warnings: vec![],
        },
    )?;
    let live = Arc::new(Mutex::new(Live::default()));
    begin(
        id,
        dir,
        a.max_seconds,
        Backend::Telemetry,
        LiveSource::None,
        a.pid,
        live,
    )?;
    Ok(s)
}

#[cfg(windows)]
pub fn start_presentmon(ctx: &Ctx, a: StartArgs, exe: PathBuf) -> Result<Session, String> {
    let started_ms = now_ms();
    let id = new_id(started_ms);
    let session_name = format!("OmniGet-{}", &id[id.len() - 6..]);
    let dir = session_dir(ctx, &id)?;
    let csv = dir.join("presentmon.csv");
    // A few seconds of slack so our own deadline ends it cleanly first.
    let args = super::presentmon::args(
        &session_name,
        &csv.display().to_string(),
        a.pid,
        a.process_name.as_deref(),
        a.max_seconds + 5,
    );
    let (dir, s) = create(
        ctx,
        &id,
        started_ms,
        NewSession {
            label: a.label.clone(),
            method: "presentmon",
            frame_mode: "per_frame",
            status: "recording",
            target: target(&a),
            machine: a.machine.clone(),
            source: SourceTool {
                tool: "PresentMon".into(),
                version: super::presentmon::VERSION.into(),
                args: args.clone(),
            },
            max_seconds: a.max_seconds,
            launch: vec![],
            reason: None,
            warnings: vec![],
        },
    )?;
    let child = super::presentmon::start(&exe, &args)?;
    let live = Arc::new(Mutex::new(Live::default()));
    let backend = Backend::PresentMon {
        exe,
        session_name,
        child,
        csv: csv.clone(),
        pid: a.pid,
    };
    if let Err(e) = begin(
        id,
        dir.clone(),
        a.max_seconds,
        backend,
        LiveSource::PresentMon(csv, None),
        a.pid,
        live,
    ) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(s)
}

/// Linux: write the session's MangoHud.conf and wait for the game to be
/// started with the variables.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn start_mangohud(ctx: &Ctx, a: StartArgs) -> Result<Session, String> {
    let started_ms = now_ms();
    let id = new_id(started_ms);
    let dir = session_dir(ctx, &id)?;
    let conf = dir.join("MangoHud.conf");
    let (dir, s) = create(
        ctx,
        &id,
        started_ms,
        NewSession {
            label: a.label.clone(),
            method: "mangohud",
            frame_mode: "per_frame",
            status: "waiting_launch",
            target: target(&a),
            machine: a.machine.clone(),
            source: SourceTool {
                tool: "MangoHud".into(),
                version: String::new(),
                args: vec![],
            },
            max_seconds: a.max_seconds,
            launch: super::mangohud::launch_commands(&conf),
            reason: None,
            warnings: vec![],
        },
    )?;
    std::fs::write(&conf, super::mangohud::config(&dir, a.max_seconds))
        .map_err(|e| e.to_string())?;
    let live = Arc::new(Mutex::new(Live::default()));
    // Pid is not watched: the game is not running yet.
    begin(
        id,
        dir,
        a.max_seconds + 60,
        Backend::MangoHud,
        LiveSource::MangoHud(PathBuf::new(), None),
        None,
        live,
    )?;
    Ok(s)
}

/// macOS: start the game ourselves with the Metal HUD and read its frames
/// from stderr. The reader keeps draining stderr after the capture so the
/// game never gets a broken pipe.
#[cfg(target_os = "macos")]
pub fn start_metal(ctx: &Ctx, mut a: StartArgs, exe: &Path) -> Result<Session, String> {
    use std::io::BufRead;
    let mut child = super::metal::launch(exe)?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    a.pid = Some(child.id());
    if a.process_name.is_none() {
        a.process_name = exe.file_name().map(|n| n.to_string_lossy().to_string());
    }
    let started_ms = now_ms();
    let id = new_id(started_ms);
    let (dir, s) = create(
        ctx,
        &id,
        started_ms,
        NewSession {
            label: a.label.clone(),
            method: "metal_hud",
            frame_mode: "per_frame",
            status: "recording",
            target: target(&a),
            machine: a.machine.clone(),
            source: SourceTool {
                tool: "Metal Performance HUD".into(),
                version: String::new(),
                args: vec!["MTL_HUD_ENABLED=1".into(), "MTL_HUD_LOG_ENABLED=1".into()],
            },
            max_seconds: a.max_seconds,
            launch: vec![],
            reason: None,
            warnings: vec![],
        },
    )?;
    let frames = Arc::new(Mutex::new(Vec::<Frame>::new()));
    let recording = Arc::new(AtomicBool::new(true));
    let live = Arc::new(Mutex::new(Live::default()));
    {
        let (frames, recording, live) = (frames.clone(), recording.clone(), live.clone());
        std::thread::Builder::new()
            .name("wmb-perf-metal".into())
            .spawn(move || {
                let mut t = 0.0;
                for line in std::io::BufReader::new(stderr).lines() {
                    let Ok(line) = line else { break };
                    if !recording.load(Ordering::Relaxed) {
                        continue;
                    }
                    if let Some((_, ivs)) = super::metal::parse_line(&line) {
                        let mut f = frames.lock().unwrap_or_else(|p| p.into_inner());
                        let mut lv = live.lock().unwrap_or_else(|p| p.into_inner());
                        for ft in ivs {
                            t += ft;
                            f.push(Frame {
                                t_ms: super::stats::round(t, 3),
                                frametime_ms: ft,
                                ..Default::default()
                            });
                            lv.push(ft);
                        }
                    }
                }
                if let Ok(mut lv) = live.lock() {
                    lv.game_exited = true;
                }
                let _ = child.wait();
            })
            .map_err(|e| e.to_string())?;
    }
    begin(
        id,
        dir,
        a.max_seconds,
        Backend::Metal { frames, recording },
        LiveSource::Pushed,
        None,
        live,
    )?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_checked_before_becoming_paths() {
        assert!(valid_id("1727430000000-a1b2c3"));
        assert!(!valid_id("../x"));
        assert!(!valid_id(""));
        assert!(!valid_id("a/b"));
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(tmp.path());
        assert!(session_dir(&ctx, "..").is_err());
    }

    fn seed(ctx: &Ctx, id: &str, started: u64, ended: u64, ft: f64) {
        let (dir, mut s) = create(
            ctx,
            id,
            started,
            NewSession {
                label: id.into(),
                method: "presentmon",
                frame_mode: "per_frame",
                status: "recording",
                target: Target {
                    name: Some("League of Legends.exe".into()),
                    pid: Some(1),
                },
                machine: Machine::default(),
                source: SourceTool::default(),
                max_seconds: 60,
                launch: vec![],
                reason: None,
                warnings: vec![],
            },
        )
        .unwrap();
        let frames: Vec<Frame> = (0..600)
            .map(|i| Frame {
                t_ms: i as f64 * ft,
                frametime_ms: ft,
                ..Default::default()
            })
            .collect();
        std::fs::write(dir.join("frames.csv"), format::frames_csv(&frames)).unwrap();
        s.status = "done".into();
        s.ended_ms = Some(ended);
        s.frames = frames.len();
        format::write_json(&dir.join("session.json"), &s).unwrap();
    }

    #[test]
    fn list_between_and_lazy_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(tmp.path());
        seed(&ctx, "1000-aaaaaa", 1_000, 61_000, 10.0);
        seed(&ctx, "200000-bbbbbb", 200_000, 260_000, 20.0);
        let all = list(&ctx);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].session_id, "200000-bbbbbb", "newest first");
        assert_eq!(all[0].fps_avg, None, "no summary yet");
        let sum = summary_of(&ctx, "200000-bbbbbb").unwrap();
        assert_eq!(sum.stats.fps_avg, Some(50.0));
        assert!(tmp.path().join("perf/200000-bbbbbb/summary.json").is_file());
        assert_eq!(list(&ctx)[0].fps_avg, Some(50.0));
        // A match from 30 s to 90 s overlaps only the first capture.
        let m = between(&ctx, 30_000, 90_000);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].session_id, "1000-aaaaaa");
        assert!(summary_of(&ctx, "nope").is_err());
    }

    #[test]
    fn telemetry_capture_runs_and_stops() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(tmp.path());
        // Other tests of this module never start a capture, so the global
        // slot is free here.
        let s = start_telemetry(
            &ctx,
            StartArgs {
                pid: None,
                process_name: None,
                max_seconds: 30,
                label: "t".into(),
                machine: Machine::default(),
            },
            "test",
        )
        .unwrap();
        assert_eq!(active_id().as_deref(), Some(s.id.as_str()));
        assert!(start_telemetry(
            &ctx,
            StartArgs {
                pid: None,
                process_name: None,
                max_seconds: 30,
                label: "t".into(),
                machine: Machine::default()
            },
            "test"
        )
        .unwrap_err()
        .starts_with("BUSY"));
        // The first sample is taken right away; wait for it, not a second.
        let id_dir = tmp.path().join("perf").join(&s.id);
        let t0 = Instant::now();
        while read_sensors(&id_dir).is_empty() && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let done = finish("stopped").unwrap();
        assert_eq!(done.status, "done");
        assert_eq!(done.frame_mode, "none");
        assert!(active_id().is_none());
        let dir = tmp.path().join("perf").join(&s.id);
        assert!(dir.join("summary.json").is_file() && dir.join("frames.csv").is_file());
        assert!(!read_sensors(&dir).is_empty(), "sampled at least once");
        assert!(finish("again").unwrap_err().starts_with("NOT_RUNNING"));
    }

    /// Opens a real window: set `WMB_METAL_TEST_BIN` to a Metal app that
    /// renders for a few seconds (the scratch MTKView app of the 2026-09-27
    /// check) and run with `--ignored`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "starts a Metal app; needs WMB_METAL_TEST_BIN"]
    fn metal_hud_capture_of_a_launched_app() {
        let bin = std::env::var("WMB_METAL_TEST_BIN").expect("WMB_METAL_TEST_BIN");
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(tmp.path());
        let s = start_metal(
            &ctx,
            StartArgs {
                pid: None,
                process_name: None,
                max_seconds: 30,
                label: "metal".into(),
                machine: Machine::default(),
            },
            Path::new(&bin),
        )
        .unwrap();
        // The app quits by itself; the capture stops when it exits.
        let t = Instant::now();
        while active_id().is_some() && t.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(200));
        }
        let dir = tmp.path().join("perf").join(&s.id);
        let done = load_session(&dir).unwrap();
        assert_eq!(done.status, "done", "{done:?}");
        assert!(done.frames > 30, "{} frames", done.frames);
        let sum = load_summary(&dir).unwrap();
        assert!(sum.stats.fps_avg.unwrap() > 5.0);
        eprintln!(
            "metal: {} frames, avg {:?} fps, 1% low {:?}, stutters {}",
            done.frames, sum.stats.fps_avg, sum.stats.low_1, sum.stats.stutters
        );
    }

    #[test]
    fn tail_reads_whole_lines_only() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("x.csv");
        std::fs::write(&p, "a,b\n1,2\n3,").unwrap();
        let mut t = Tail::default();
        assert_eq!(t.lines(&p), vec!["a,b", "1,2"]);
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"4\n")
            .unwrap();
        assert_eq!(t.lines(&p), vec!["3,4"]);
    }

    #[test]
    fn live_fps_is_over_the_last_second() {
        let mut l = Live::default();
        for _ in 0..10 {
            l.push(10.0);
        }
        assert_eq!(l.fps_now, Some(100.0));
        assert_eq!(l.frames, 10);
    }
}
