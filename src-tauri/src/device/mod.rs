pub mod android;
pub mod idb;
pub mod ios;
pub mod tools;

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncBufReadExt;

use android::LogLine;

pub const EVENT_FRAME: &str = "canvas://device-frame";
pub const EVENT_STATE: &str = "canvas://device-state";

const TICK: Duration = Duration::from_millis(333);
const SETTLE_FIRST: Duration = Duration::from_millis(300);
const SETTLE_MAX: Duration = Duration::from_secs(2);
const LOG_CAP: usize = 4000;
const TREE_LIMIT: usize = 300;
const NO_TOUCH: &str = "CAPABILITY_UNAVAILABLE: iOS touch, keys and the element tree need idb_companion (brew install facebook/fb/idb-companion); without it this node can show the screen, install, launch, open URLs (deep links) and read logs";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Platform {
    Ios,
    Android,
}

impl Platform {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ios" | "iphone" | "ipad" => Ok(Self::Ios),
            "android" => Ok(Self::Android),
            other => Err(format!(
                "INVALID_ARGUMENT: platform '{other}' (ios or android)"
            )),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub seq: u64,
    hash: u64,
}

impl Frame {
    fn json(&self, scale: f64) -> Value {
        json!({
            "path": self.path.to_string_lossy(),
            "w": self.w,
            "h": self.h,
            "seq": self.seq,
            "input": {"w": (f64::from(self.w) / scale).round(), "h": (f64::from(self.h) / scale).round()},
        })
    }
}

pub struct Session {
    pub node_id: String,
    pub platform: Platform,
    pub device: String,
    pub name: String,
    booted: bool,
    dir: PathBuf,
    scale: f64,
    app: AppHandle,
    seq: AtomicU64,
    watching: AtomicBool,
    closed: AtomicBool,
    busy: tokio::sync::Mutex<()>,
    last: Mutex<Option<Frame>>,
    logs: Mutex<VecDeque<LogLine>>,
    log_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    companion: tokio::sync::Mutex<Option<idb::Companion>>,
    emulator: Mutex<Option<tokio::process::Child>>,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn sessions() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Session>>> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn get(node_id: &str) -> Option<Arc<Session>> {
    sessions().get(node_id).cloned()
}

pub fn need(node_id: &str) -> Result<Arc<Session>, String> {
    get(node_id).ok_or_else(|| {
        format!(
            "DEVICE_NOT_OPEN: node {node_id} has no device attached; call canvas_device_open first"
        )
    })
}

pub fn frames_dir(node_id: &str) -> Result<PathBuf, String> {
    let safe: String = node_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe.is_empty() {
        return Err("INVALID_ARGUMENT: nodeId".into());
    }
    omniget_core::core::paths::app_data_dir()
        .map(|d| d.join("canvas").join("devices").join(safe))
        .ok_or_else(|| "DEVICE_FRAME: no app data folder".into())
}

fn hash_of(b: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

fn xcrun() -> Result<PathBuf, String> {
    tools::xcrun().ok_or_else(|| {
        "CAPABILITY_UNAVAILABLE: the iOS Simulator needs macOS with Xcode (xcrun simctl)".into()
    })
}

fn adb() -> Result<PathBuf, String> {
    tools::adb().ok_or_else(|| {
        "CAPABILITY_UNAVAILABLE: adb not found (install the Android SDK platform-tools or set ANDROID_HOME)".into()
    })
}

async fn simctl_json(args: &[&str]) -> Result<Value, String> {
    let x = xcrun()?;
    let mut full = vec!["simctl"];
    full.extend_from_slice(args);
    let out = tools::text(&x, &full, 60).await?;
    serde_json::from_str(&out).map_err(|e| format!("DEVICE_COMMAND: simctl json: {e}"))
}

async fn simulators() -> Result<Vec<ios::Sim>, String> {
    Ok(ios::parse_list(
        &simctl_json(&["list", "devices", "available", "-j"]).await?,
    ))
}

async fn adb_devices() -> Result<Vec<android::Attached>, String> {
    let out = tools::text(&adb()?, &["devices", "-l"], 20).await?;
    Ok(android::parse_devices(&out))
}

async fn avd_of(serial: &str) -> Option<String> {
    if !serial.starts_with("emulator-") {
        return None;
    }
    let out = tools::text(&adb().ok()?, &["-s", serial, "emu", "avd", "name"], 10)
        .await
        .ok()?;
    out.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && *l != "OK")
        .map(str::to_string)
}

async fn avds() -> Vec<String> {
    let Some(em) = tools::emulator() else {
        return Vec::new();
    };
    tools::text(&em, &["-list-avds"], 20)
        .await
        .map(|o| {
            o.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.contains(' ') && !l.starts_with("INFO"))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub async fn list() -> Value {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    match simulators().await {
        Ok(sims) => {
            for s in sims {
                out.push(json!({
                    "platform": "ios", "id": s.udid, "name": s.name, "os": s.os,
                    "state": if s.state == "Booted" { "booted" } else { "shutdown" }, "kind": "simulator",
                }));
            }
        }
        Err(e) => notes.push(e),
    }
    let mut running_avds = Vec::new();
    match adb_devices().await {
        Ok(devs) => {
            for d in devs {
                let avd = avd_of(&d.serial).await;
                if let Some(a) = &avd {
                    running_avds.push(a.clone());
                }
                out.push(json!({
                    "platform": "android", "id": d.serial,
                    "name": avd.clone().unwrap_or_else(|| d.model.clone()),
                    "state": if d.state == "device" { "booted" } else { d.state.as_str() },
                    "kind": if avd.is_some() { "emulator" } else { "device" }, "avd": avd,
                }));
            }
        }
        Err(e) => notes.push(e),
    }
    for a in avds().await {
        if !running_avds.contains(&a) {
            out.push(json!({"platform": "android", "id": a, "name": a, "state": "shutdown", "kind": "avd"}));
        }
    }
    let attached: Vec<Value> = sessions()
        .values()
        .map(|s| json!({"nodeId": s.node_id, "platform": s.platform.as_str(), "id": s.device}))
        .collect();
    json!({
        "devices": out,
        "attached": attached,
        "touchIos": tools::idb_companion().is_some(),
        "notes": notes,
    })
}

fn capabilities(p: Platform) -> Value {
    match p {
        Platform::Android => json!({
            "frames": true, "tap": true, "swipe": true, "text": true, "key": true, "home": true,
            "install": true, "launch": true, "openurl": true, "terminate": true, "logs": true, "tree": true,
        }),
        Platform::Ios => {
            let touch = tools::idb_companion().is_some();
            json!({
                "frames": true, "tap": touch, "swipe": touch, "text": touch, "key": touch, "home": touch,
                "install": true, "launch": true, "openurl": true, "terminate": true, "logs": true, "tree": touch,
            })
        }
    }
}

impl Session {
    pub fn info(&self) -> Value {
        let frame = self
            .last
            .lock()
            .ok()
            .and_then(|l| l.as_ref().map(|f| f.json(self.scale)));
        json!({
            "nodeId": self.node_id,
            "platform": self.platform.as_str(),
            "device": {"id": self.device, "name": self.name},
            "bootedByOmniget": self.booted,
            "scale": self.scale,
            "coordinates": if self.platform == Platform::Ios { "points" } else { "pixels" },
            "capabilities": capabilities(self.platform),
            "watching": self.watching.load(Ordering::Relaxed),
            "frame": frame,
        })
    }

    fn emit_state(&self) {
        let _ = self.app.emit(EVENT_STATE, self.info());
    }

    async fn grab(&self) -> Result<(Vec<u8>, u64, u32, u32), String> {
        match self.platform {
            Platform::Ios => {
                let tmp = self.dir.join("grab.jpg");
                let x = xcrun()?;
                let p = tmp.to_string_lossy().into_owned();
                tools::run(
                    &x,
                    &[
                        "simctl",
                        "io",
                        &self.device,
                        "screenshot",
                        "--type=jpeg",
                        &p,
                    ],
                    20,
                )
                .await?;
                let bytes = tokio::fs::read(&tmp)
                    .await
                    .map_err(|e| format!("DEVICE_FRAME: {e}"))?;
                let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
                    .with_guessed_format()
                    .map_err(|e| format!("DEVICE_FRAME: {e}"))?
                    .into_dimensions()
                    .map_err(|e| format!("DEVICE_FRAME: {e}"))?;
                let hash = hash_of(&bytes);
                Ok((bytes, hash, w, h))
            }
            Platform::Android => {
                let raw =
                    tools::run(&adb()?, &["-s", &self.device, "exec-out", "screencap"], 20).await?;
                let hash = hash_of(&raw);
                let last = self.last.lock().ok().and_then(|l| l.clone());
                if let Some(f) = last.filter(|f| f.hash == hash) {
                    return Ok((Vec::new(), hash, f.w, f.h));
                }
                tauri::async_runtime::spawn_blocking(move || {
                    let r = android::parse_raw_header(&raw)?;
                    let px = &raw[r.offset..];
                    let mut rgb = Vec::with_capacity((r.w * r.h * 3) as usize);
                    for p in px.chunks_exact(4) {
                        rgb.extend_from_slice(&p[..3]);
                    }
                    let mut out = Vec::new();
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 72)
                        .encode(&rgb, r.w, r.h, image::ExtendedColorType::Rgb8)
                        .map_err(|e| format!("DEVICE_FRAME: jpeg: {e}"))?;
                    Ok::<_, String>((out, hash, r.w, r.h))
                })
                .await
                .map_err(|e| format!("DEVICE_FRAME: {e}"))?
            }
        }
    }

    async fn capture_locked(&self) -> Result<(Frame, bool), String> {
        let (bytes, hash, w, h) = self.grab().await?;
        if let Some(f) = self
            .last
            .lock()
            .ok()
            .and_then(|l| l.clone())
            .filter(|f| f.hash == hash)
        {
            return Ok((f, false));
        }
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let path = self.dir.join(if seq % 2 == 1 { "a.jpg" } else { "b.jpg" });
        let tmp = self.dir.join("next.jpg");
        tokio::fs::write(&tmp, &bytes)
            .await
            .map_err(|e| format!("DEVICE_FRAME: {e}"))?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| format!("DEVICE_FRAME: {e}"))?;
        let f = Frame {
            path,
            w,
            h,
            seq,
            hash,
        };
        if let Ok(mut l) = self.last.lock() {
            *l = Some(f.clone());
        }
        let _ = self.app.emit(
            EVENT_FRAME,
            json!({"nodeId": self.node_id, "seq": seq, "file": f.path.to_string_lossy(), "w": w, "h": h, "scale": self.scale}),
        );
        Ok((f, true))
    }

    pub async fn capture(&self) -> Result<Frame, String> {
        let _g = self.busy.lock().await;
        self.capture_locked().await.map(|(f, _)| f)
    }

    pub async fn settled(&self) -> Result<Frame, String> {
        let start = Instant::now();
        tokio::time::sleep(SETTLE_FIRST).await;
        let _g = self.busy.lock().await;
        let (mut f, _) = self.capture_locked().await?;
        while start.elapsed() < SETTLE_MAX {
            let (next, changed) = self.capture_locked().await?;
            f = next;
            if !changed {
                break;
            }
        }
        Ok(f)
    }

    pub fn copy_out(&self, f: &Frame, name: &str) -> Result<Value, String> {
        let to = self.dir.join(name);
        std::fs::copy(&f.path, &to).map_err(|e| format!("DEVICE_FRAME: {e}"))?;
        let mut v = f.json(self.scale);
        v["path"] = json!(to.to_string_lossy());
        Ok(v)
    }

    async fn idb_sock(&self) -> Result<PathBuf, String> {
        let bin = tools::idb_companion().ok_or(NO_TOUCH)?;
        let mut c = self.companion.lock().await;
        if let Some(c) = c.as_ref() {
            if idb::alive(&c.sock).await {
                return Ok(c.sock.clone());
            }
        }
        *c = None;
        let started = idb::start(&bin, &self.device, &self.node_id).await?;
        let sock = started.sock.clone();
        *c = Some(started);
        Ok(sock)
    }

    async fn hid(&self, events: &[idb::Hid]) -> Result<(), String> {
        let sock = self.idb_sock().await?;
        idb::hid(&sock, events).await
    }

    async fn adb_shell(&self, cmd: &str, secs: u64) -> Result<String, String> {
        tools::text(&adb()?, &["-s", &self.device, "shell", cmd], secs).await
    }

    async fn simctl(&self, args: &[&str], secs: u64) -> Result<String, String> {
        let x = xcrun()?;
        let mut full = vec!["simctl"];
        full.extend_from_slice(args);
        tools::text(&x, &full, secs).await
    }

    pub async fn tap(&self, x: f64, y: f64) -> Result<(), String> {
        match self.platform {
            Platform::Android => self
                .adb_shell(&format!("input tap {} {}", x.round(), y.round()), 20)
                .await
                .map(drop),
            Platform::Ios => self.hid(&idb::tap(x, y)).await,
        }
    }

    pub async fn swipe(&self, x: f64, y: f64, x2: f64, y2: f64, ms: u64) -> Result<(), String> {
        let ms = ms.clamp(50, 10_000);
        match self.platform {
            Platform::Android => self
                .adb_shell(
                    &format!(
                        "input swipe {} {} {} {} {ms}",
                        x.round(),
                        y.round(),
                        x2.round(),
                        y2.round()
                    ),
                    30,
                )
                .await
                .map(drop),
            Platform::Ios => {
                self.hid(&[idb::Hid::Swipe {
                    x1: x,
                    y1: y,
                    x2,
                    y2,
                    secs: ms as f64 / 1000.0,
                }])
                .await
            }
        }
    }

    pub async fn text(&self, s: &str) -> Result<(), String> {
        match self.platform {
            Platform::Android => self
                .adb_shell(&format!("input text {}", android::input_text(s)?), 60)
                .await
                .map(drop),
            Platform::Ios => self.hid(&idb::text(s)?).await,
        }
    }

    pub async fn key(&self, k: &str) -> Result<(), String> {
        match self.platform {
            Platform::Android => self
                .adb_shell(&format!("input keyevent {}", android::keycode(k)), 20)
                .await
                .map(drop),
            Platform::Ios => match k.trim().to_ascii_lowercase().as_str() {
                "home" => self.hid(&idb::button(idb::BUTTON_HOME)).await,
                "lock" | "power" => self.hid(&idb::button(idb::BUTTON_LOCK)).await,
                other => {
                    let code = idb::named_key(other).ok_or_else(|| {
                        format!("INVALID_ARGUMENT: key '{k}' (enter, backspace, tab, escape, space, arrows, home, lock or a HID keycode)")
                    })?;
                    self.hid(&[
                        idb::Hid::Key { code, up: false },
                        idb::Hid::Key { code, up: true },
                    ])
                    .await
                }
            },
        }
    }

    pub async fn home(&self) -> Result<(), String> {
        self.key("home").await
    }

    pub async fn install(&self, path: &str) -> Result<(), String> {
        let p = Path::new(path);
        if !p.exists() {
            return Err(format!("INVALID_ARGUMENT: {path} does not exist"));
        }
        match self.platform {
            Platform::Android => {
                tools::text(&adb()?, &["-s", &self.device, "install", "-r", path], 600)
                    .await
                    .map(drop)
            }
            Platform::Ios => self
                .simctl(&["install", &self.device, path], 600)
                .await
                .map(drop),
        }
    }

    pub async fn launch(&self, id: &str) -> Result<(), String> {
        match self.platform {
            Platform::Android => self
                .adb_shell(
                    &format!(
                        "monkey -p {} -c android.intent.category.LAUNCHER 1",
                        android::shell_quote(id)
                    ),
                    30,
                )
                .await
                .map(drop),
            Platform::Ios => {
                self.simctl(&["launch", &self.device, id], 60).await?;
                if let Some(exe) = self.ios_executable(id).await {
                    self.start_logs(Some(exe));
                }
                Ok(())
            }
        }
    }

    async fn ios_executable(&self, bundle: &str) -> Option<String> {
        let dir = self
            .simctl(&["get_app_container", &self.device, bundle, "app"], 20)
            .await
            .ok()?;
        let plist = PathBuf::from(dir.trim()).join("Info.plist");
        let out = tools::text(
            Path::new("/usr/bin/plutil"),
            &[
                "-extract",
                "CFBundleExecutable",
                "raw",
                "-o",
                "-",
                &plist.to_string_lossy(),
            ],
            10,
        )
        .await
        .ok()?;
        Some(out.trim().to_string()).filter(|s| !s.is_empty())
    }

    pub async fn openurl(&self, url: &str) -> Result<(), String> {
        match self.platform {
            Platform::Android => self
                .adb_shell(
                    &format!(
                        "am start -a android.intent.action.VIEW -d {}",
                        android::shell_quote(url)
                    ),
                    30,
                )
                .await
                .map(drop),
            Platform::Ios => self
                .simctl(&["openurl", &self.device, url], 30)
                .await
                .map(drop),
        }
    }

    pub async fn terminate(&self, id: &str) -> Result<(), String> {
        match self.platform {
            Platform::Android => self
                .adb_shell(&format!("am force-stop {}", android::shell_quote(id)), 20)
                .await
                .map(drop),
            Platform::Ios => self
                .simctl(&["terminate", &self.device, id], 30)
                .await
                .map(drop),
        }
    }

    pub async fn tree(&self) -> Result<Vec<Value>, String> {
        match self.platform {
            Platform::Android => {
                let remote = "/sdcard/omniget-ui.xml";
                self.adb_shell(&format!("uiautomator dump {remote}"), 60)
                    .await?;
                let xml = tools::text(
                    &adb()?,
                    &["-s", &self.device, "exec-out", "cat", remote],
                    30,
                )
                .await?;
                Ok(android::parse_ui(&xml, TREE_LIMIT))
            }
            Platform::Ios => {
                let sock = self.idb_sock().await?;
                Ok(idb::trim_tree(
                    &idb::accessibility(&sock).await?,
                    TREE_LIMIT,
                ))
            }
        }
    }

    fn push_log(&self, l: LogLine) {
        if let Ok(mut q) = self.logs.lock() {
            if q.len() >= LOG_CAP {
                q.pop_front();
            }
            q.push_back(l);
        }
    }

    fn start_logs(&self, ios_process: Option<String>) {
        let Some(me) = get(&self.node_id) else { return };
        if let Ok(mut t) = self.log_task.lock() {
            if let Some(old) = t.take() {
                old.abort();
            }
            *t = Some(tauri::async_runtime::spawn(log_pump(me, ios_process)));
        }
    }

    pub fn logs(
        &self,
        limit: usize,
        level: Option<&str>,
        app: Option<&str>,
        pid: Option<u32>,
        clear: bool,
    ) -> Vec<Value> {
        let min = level.map(level_rank).unwrap_or(0);
        let app = app.map(str::to_ascii_lowercase);
        let mut q = match self.logs.lock() {
            Ok(q) => q,
            Err(e) => e.into_inner(),
        };
        let picked: Vec<Value> = q
            .iter()
            .filter(|l| level_rank(l.level) >= min)
            .filter(|l| match (&app, pid) {
                (_, Some(p)) if l.pid == Some(p) => true,
                (Some(a), _) => {
                    l.source.to_ascii_lowercase().contains(a)
                        || l.message.to_ascii_lowercase().contains(a)
                }
                (None, None) => true,
                (None, Some(_)) => false,
            })
            .rev()
            .take(limit)
            .map(|l| json!({"time": l.time, "level": l.level, "source": l.source, "pid": l.pid, "message": l.message}))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if clear {
            q.clear();
        }
        picked
    }

    pub async fn pid_of(&self, package: &str) -> Option<u32> {
        if self.platform != Platform::Android {
            return None;
        }
        self.adb_shell(&format!("pidof {}", android::shell_quote(package)), 10)
            .await
            .ok()?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    pub fn watch(&self, on: bool) {
        self.watching.store(on, Ordering::Relaxed);
    }
}

pub fn level_rank(l: &str) -> u8 {
    match l.to_ascii_lowercase().as_str() {
        "verbose" | "v" => 0,
        "debug" | "d" => 1,
        "info" | "i" | "default" | "log" => 2,
        "warning" | "warn" | "w" => 3,
        "error" | "e" => 4,
        "fault" | "fatal" | "f" => 5,
        _ => 0,
    }
}

async fn log_pump(s: Arc<Session>, ios_process: Option<String>) {
    let spawned = match s.platform {
        Platform::Android => adb().map(|b| {
            let mut c = omniget_core::core::process::command(b);
            c.args(["-s", &s.device, "logcat", "-v", "threadtime", "-T", "1"]);
            c
        }),
        Platform::Ios => xcrun().map(|b| {
            let predicate = match &ios_process {
                Some(p) => format!("process == \"{}\"", p.replace('"', "")),
                None => "messageType == error OR messageType == fault".to_string(),
            };
            let mut c = omniget_core::core::process::command(b);
            c.args([
                "simctl",
                "spawn",
                &s.device,
                "log",
                "stream",
                "--style",
                "compact",
                "--level",
                if ios_process.is_some() {
                    "debug"
                } else {
                    "info"
                },
                "--predicate",
                &predicate,
            ]);
            c
        }),
    };
    let Ok(mut cmd) = spawned else { return };
    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let Ok(mut child) = child else { return };
    let Some(out) = child.stdout.take() else {
        return;
    };
    let mut lines = tokio::io::BufReader::new(out).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if s.closed.load(Ordering::Relaxed) {
            break;
        }
        let parsed = match s.platform {
            Platform::Android => android::parse_logcat(&line),
            Platform::Ios => ios::parse_log(&line),
        };
        if let Some(l) = parsed {
            s.push_log(l);
        }
    }
    let _ = child.start_kill();
}

async fn frame_loop(s: Arc<Session>) {
    let mut tick: u64 = 0;
    loop {
        tokio::time::sleep(TICK).await;
        if s.closed.load(Ordering::Relaxed) {
            return;
        }
        tick += 1;
        if tick.is_multiple_of(15) && !node_exists(&s.node_id) {
            let _ = close(&s.node_id, true).await;
            return;
        }
        if !s.watching.load(Ordering::Relaxed) {
            continue;
        }
        let Ok(_g) = s.busy.try_lock() else { continue };
        if let Err(e) = s.capture_locked().await {
            let _ = s
                .app
                .emit(EVENT_STATE, json!({"nodeId": s.node_id, "error": e}));
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

fn node_exists(node_id: &str) -> bool {
    crate::canvas::doc::host()
        .store()
        .locate_node(node_id)
        .is_some()
}

async fn ios_scale(sim: &ios::Sim) -> f64 {
    let fallback = 3.0;
    let Ok(types) = simctl_json(&["list", "devicetypes", "-j"]).await else {
        return fallback;
    };
    let Some(bundle) = ios::bundle_path(&types, &sim.type_id) else {
        return fallback;
    };
    let plist = PathBuf::from(bundle).join("Contents/Resources/profile.plist");
    let out = tools::text(
        Path::new("/usr/bin/plutil"),
        &[
            "-extract",
            "mainScreenScale",
            "raw",
            "-o",
            "-",
            &plist.to_string_lossy(),
        ],
        10,
    )
    .await;
    out.ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| *v > 0.0)
        .unwrap_or(fallback)
}

async fn open_ios(want: Option<&str>) -> Result<(String, String, bool, f64), String> {
    let sims = simulators().await?;
    let sim = ios::pick(&sims, want)
        .ok_or_else(|| match want {
            Some(w) => format!("DEVICE_NOT_FOUND: no simulator '{w}' (see canvas_device_list)"),
            None => "DEVICE_NOT_FOUND: no iOS simulator installed (Xcode > Settings > Components)"
                .into(),
        })?
        .clone();
    let mut booted = false;
    if sim.state != "Booted" {
        let x = xcrun()?;
        if let Err(e) = tools::run(&x, &["simctl", "boot", &sim.udid], 120).await {
            if !e.contains("current state: Booted") {
                return Err(e);
            }
        } else {
            booted = true;
        }
        tools::run(&x, &["simctl", "bootstatus", &sim.udid, "-b"], 300).await?;
    }
    let scale = ios_scale(&sim).await;
    Ok((
        sim.udid,
        format!("{} ({})", sim.name, sim.os),
        booted,
        scale,
    ))
}

fn free_port(taken: &[String]) -> u16 {
    (5554..=5682)
        .step_by(2)
        .find(|p| !taken.iter().any(|s| s == &format!("emulator-{p}")))
        .unwrap_or(5554)
}

async fn wait_android_boot(serial: &str, child: &mut tokio::process::Child) -> Result<(), String> {
    let a = adb()?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(300) {
        if let Ok(Some(st)) = child.try_wait() {
            return Err(format!("DEVICE_BOOT: the emulator exited ({st})"));
        }
        if let Ok(v) = tools::text(
            &a,
            &["-s", serial, "shell", "getprop", "sys.boot_completed"],
            10,
        )
        .await
        {
            if v.trim() == "1" {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err("DEVICE_TIMEOUT: the emulator did not finish booting in 5 min".into())
}

async fn open_android(
    want: Option<&str>,
) -> Result<(String, String, bool, Option<tokio::process::Child>), String> {
    let devs = adb_devices().await?;
    let want = want.map(str::trim).filter(|w| !w.is_empty());
    for d in devs.iter().filter(|d| d.state == "device") {
        let avd = avd_of(&d.serial).await;
        let hit = match want {
            None => true,
            Some(w) => d.serial == w || avd.as_deref() == Some(w),
        };
        if hit {
            let name = avd.unwrap_or_else(|| d.model.clone());
            return Ok((d.serial.clone(), name, false, None));
        }
    }
    let list = avds().await;
    let avd = match want {
        Some(w) => list.iter().find(|a| a.as_str() == w).cloned().ok_or_else(|| {
            format!("DEVICE_NOT_FOUND: no running device or AVD named '{w}' (see canvas_device_list)")
        })?,
        None => list.first().cloned().ok_or(
            "DEVICE_NOT_FOUND: no Android device attached and no AVD created (Android Studio > Device Manager)",
        )?,
    };
    let em = tools::emulator().ok_or(
        "CAPABILITY_UNAVAILABLE: the Android emulator binary was not found (set ANDROID_HOME)",
    )?;
    let taken: Vec<String> = devs.iter().map(|d| d.serial.clone()).collect();
    let port = free_port(&taken);
    let serial = format!("emulator-{port}");
    let mut child = omniget_core::core::process::command(&em)
        .args([
            "-avd",
            &avd,
            "-port",
            &port.to_string(),
            "-no-window",
            "-no-audio",
            "-no-boot-anim",
            "-no-snapshot-save",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| format!("DEVICE_BOOT: {e}"))?;
    if let Err(e) = wait_android_boot(&serial, &mut child).await {
        let _ = tools::run(&adb()?, &["-s", &serial, "emu", "kill"], 15).await;
        let _ = child.start_kill();
        return Err(e);
    }
    Ok((serial, avd, true, Some(child)))
}

pub async fn open(
    app: &AppHandle,
    node_id: &str,
    platform: Platform,
    want: Option<&str>,
) -> Result<Arc<Session>, String> {
    if let Some(s) = get(node_id) {
        let same = s.platform == platform
            && want.is_none_or(|w| w == s.device || w == s.name || s.name.starts_with(w));
        if same {
            return Ok(s);
        }
        close(node_id, true).await?;
    }
    let dir = frames_dir(node_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("DEVICE_FRAME: {e}"))?;
    let (device, name, booted, scale, emulator) = match platform {
        Platform::Ios => {
            let (d, n, b, sc) = open_ios(want).await?;
            (d, n, b, sc, None)
        }
        Platform::Android => {
            let (d, n, b, c) = open_android(want).await?;
            (d, n, b, 1.0, c)
        }
    };
    let s = Arc::new(Session {
        node_id: node_id.to_string(),
        platform,
        device,
        name,
        booted,
        dir,
        scale,
        app: app.clone(),
        seq: AtomicU64::new(0),
        watching: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        busy: tokio::sync::Mutex::new(()),
        last: Mutex::new(None),
        logs: Mutex::new(VecDeque::new()),
        log_task: Mutex::new(None),
        companion: tokio::sync::Mutex::new(None),
        emulator: Mutex::new(emulator),
    });
    sessions().insert(node_id.to_string(), s.clone());
    s.start_logs(None);
    tauri::async_runtime::spawn(frame_loop(s.clone()));
    let _ = s.capture().await;
    s.emit_state();
    Ok(s)
}

fn device_in_use(device: &str, except: &str) -> bool {
    sessions()
        .values()
        .any(|s| s.device == device && s.node_id != except)
}

pub async fn close(node_id: &str, remove_frames: bool) -> Result<Value, String> {
    let Some(s) = sessions().remove(node_id) else {
        if remove_frames {
            if let Ok(d) = frames_dir(node_id) {
                let _ = std::fs::remove_dir_all(d);
            }
        }
        return Ok(json!({"nodeId": node_id, "closed": false}));
    };
    s.closed.store(true, Ordering::Relaxed);
    if let Ok(mut t) = s.log_task.lock() {
        if let Some(t) = t.take() {
            t.abort();
        }
    }
    *s.companion.lock().await = None;
    let mut shut = false;
    if s.booted && !device_in_use(&s.device, node_id) {
        shut = true;
        match s.platform {
            Platform::Ios => {
                if let Ok(x) = xcrun() {
                    let _ = tools::run(&x, &["simctl", "shutdown", &s.device], 60).await;
                }
            }
            Platform::Android => {
                if let Ok(a) = adb() {
                    let _ = tools::run(&a, &["-s", &s.device, "emu", "kill"], 30).await;
                }
                let child = s.emulator.lock().ok().and_then(|mut c| c.take());
                if let Some(mut c) = child {
                    let _ = tokio::time::timeout(Duration::from_secs(20), c.wait()).await;
                    let _ = c.start_kill();
                }
            }
        }
    }
    if remove_frames {
        let _ = std::fs::remove_dir_all(&s.dir);
    }
    let _ = s
        .app
        .emit(EVENT_STATE, json!({"nodeId": node_id, "closed": true}));
    Ok(json!({"nodeId": node_id, "closed": true, "device": s.device, "shutDown": shut}))
}

pub fn shutdown_all() {
    let all: Vec<Arc<Session>> = sessions().drain().map(|(_, s)| s).collect();
    for s in all {
        s.closed.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&s.dir);
        if !s.booted {
            continue;
        }
        let r = match s.platform {
            Platform::Ios => tools::xcrun().map(|x| {
                omniget_core::core::process::std_command(x)
                    .args(["simctl", "shutdown", &s.device])
                    .output()
            }),
            Platform::Android => tools::adb().map(|a| {
                omniget_core::core::process::std_command(a)
                    .args(["-s", &s.device, "emu", "kill"])
                    .output()
            }),
        };
        let _ = r;
    }
}

pub async fn act(s: &Session, action: &str, a: &Value) -> Result<(), String> {
    let num = |k: &str| a.get(k).and_then(Value::as_f64);
    let st = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_string);
    let req = |k: &str| st(k).ok_or_else(|| format!("MISSING_ARGUMENT: {k}"));
    let xy = || match (num("x"), num("y")) {
        (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => Ok((x, y)),
        _ => Err("MISSING_ARGUMENT: x and y (device points, not negative)".to_string()),
    };
    match action {
        "tap" => {
            let (x, y) = xy()?;
            s.tap(x, y).await
        }
        "swipe" => {
            let (x, y) = xy()?;
            let (x2, y2) = match (num("x2"), num("y2")) {
                (Some(a), Some(b)) => (a, b),
                _ => return Err("MISSING_ARGUMENT: x2 and y2".into()),
            };
            let ms = num("durationMs").unwrap_or(300.0).max(0.0) as u64;
            s.swipe(x, y, x2, y2, ms).await
        }
        "text" => s.text(&req("text")?).await,
        "key" => s.key(&req("key")?).await,
        "home" => s.home().await,
        "install" => s.install(&req("path")?).await,
        "launch" => s.launch(&req("bundleId")?).await,
        "openurl" => s.openurl(&req("url")?).await,
        "terminate" => s.terminate(&req("bundleId")?).await,
        other => Err(format!(
            "INVALID_ARGUMENT: action '{other}' (tap, swipe, text, key, home, install, launch, openurl, terminate)"
        )),
    }
}

pub fn remember(s: &Session) {
    let store = crate::canvas::doc::host().store().clone();
    let Some((ws, floor)) = store.locate_node(&s.node_id) else {
        return;
    };
    let id = if s.device.starts_with("emulator-") {
        &s.name
    } else {
        &s.device
    };
    let mut patch = serde_json::Map::new();
    patch.insert(
        "data".into(),
        json!({"portalKind": s.platform.as_str(), "deviceId": id, "deviceName": s.name}),
    );
    let op = omniget_core::core::canvas::ops::Op::UpdateNode {
        id: s.node_id.clone(),
        patch,
    };
    let _ = store.apply(&ws, &floor, vec![op], "device", None);
}
