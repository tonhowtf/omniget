use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use super::cdp::{Cdp, Event};
use super::harness::{self, AttachTarget};
use super::{data_dir, emit, sniffer};

#[derive(Debug, Clone, Serialize, Default)]
pub struct Tab {
    pub id: String,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub private: bool,
    #[serde(skip)]
    pub session: Option<String>,
    #[serde(skip)]
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub tab: String,
    pub width: u32,
    pub height: u32,
    pub dpr: f64,
}

pub struct Browser {
    pub cdp: Arc<Cdp>,
    pub exe: String,
    pub pid: Option<u32>,
    pub headless: bool,
    pub attached: bool,
    tabs: Mutex<HashMap<String, Tab>>,
    order: Mutex<Vec<String>>,
    ua_override: Mutex<Option<Value>>,
    view: Mutex<Option<View>>,
    private_contexts: Mutex<std::collections::HashSet<String>>,
    child_sessions: Mutex<HashMap<String, String>>,
    ready: Mutex<std::collections::HashSet<String>>,
}

fn slot() -> &'static Mutex<Option<Arc<Browser>>> {
    static S: OnceLock<Mutex<Option<Arc<Browser>>>> = OnceLock::new();
    S.get_or_init(Mutex::default)
}

fn start_lock() -> &'static tokio::sync::Mutex<()> {
    static L: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    L.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub fn current() -> Option<Arc<Browser>> {
    let b = slot().lock().ok()?.clone()?;
    (!b.cdp.is_closed()).then_some(b)
}

pub fn require() -> Result<Arc<Browser>, String> {
    current().ok_or_else(|| "the Global Downloader browser is not open".to_string())
}

fn profile_dir() -> PathBuf {
    let p = data_dir().join("browser-profile");
    let _ = std::fs::create_dir_all(&p);
    p
}

fn config() -> Value {
    std::fs::read_to_string(data_dir().join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn set_browser_path(path: Option<String>) -> Result<(), String> {
    set_config(
        "browser_path",
        path.map(Value::String).unwrap_or(Value::Null),
    )
}

pub fn set_config(key: &str, value: Value) -> Result<(), String> {
    let mut c = config();
    c[key] = value;
    std::fs::write(data_dir().join("config.json"), c.to_string()).map_err(|e| e.to_string())
}

pub fn mode() -> String {
    config()["mode"].as_str().unwrap_or("embedded").to_string()
}

pub fn block_ads() -> bool {
    config()["block_ads"].as_bool().unwrap_or(true)
}

pub fn autoplay() -> bool {
    config()["autoplay"].as_bool().unwrap_or(true)
}

pub fn downloads_dir() -> PathBuf {
    let d = config()["downloads_dir"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(std::env::temp_dir)
                .join("OmniGet")
                .join("Browser")
        });
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn search_url(q: &str) -> String {
    let engine = config()["search"]
        .as_str()
        .unwrap_or("duckduckgo")
        .to_string();
    let q = urlencoding::encode(q);
    match engine.as_str() {
        "google" => format!("https://www.google.com/search?q={q}"),
        "bing" => format!("https://www.bing.com/search?q={q}"),
        "brave" => format!("https://search.brave.com/search?q={q}"),
        _ => format!("https://duckduckgo.com/?q={q}"),
    }
}

pub fn address_to_url(input: &str) -> String {
    let t = input.trim();
    if t.contains("://") || t.starts_with("about:") || t.starts_with("chrome:") {
        return t.to_string();
    }
    let looks_host = !t.contains(' ')
        && (t.contains('.') || t.starts_with("localhost"))
        && url::Url::parse(&format!("https://{t}")).is_ok();
    if looks_host {
        format!("https://{t}")
    } else {
        search_url(t)
    }
}

fn runtime_file() -> PathBuf {
    profile_dir().join("omniget-runtime.json")
}

pub fn installed_browsers() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut push = |name: &str, p: PathBuf| {
        if p.is_file() && !out.iter().any(|(_, q)| *q == p) {
            out.push((name.to_string(), p));
        }
    };
    if cfg!(target_os = "macos") {
        let home = dirs::home_dir().unwrap_or_default();
        for base in [PathBuf::from("/Applications"), home.join("Applications")] {
            push(
                "Google Chrome",
                base.join("Google Chrome.app/Contents/MacOS/Google Chrome"),
            );
            push(
                "Chromium",
                base.join("Chromium.app/Contents/MacOS/Chromium"),
            );
            push(
                "Microsoft Edge",
                base.join("Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
            );
            push(
                "Brave",
                base.join("Brave Browser.app/Contents/MacOS/Brave Browser"),
            );
        }
    } else if cfg!(windows) {
        let env = |k: &str| std::env::var(k).ok().map(PathBuf::from);
        for base in [
            env("ProgramFiles"),
            env("ProgramFiles(x86)"),
            env("LOCALAPPDATA"),
        ]
        .into_iter()
        .flatten()
        {
            push(
                "Google Chrome",
                base.join("Google\\Chrome\\Application\\chrome.exe"),
            );
            push(
                "Microsoft Edge",
                base.join("Microsoft\\Edge\\Application\\msedge.exe"),
            );
            push(
                "Brave",
                base.join("BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
            );
            push("Chromium", base.join("Chromium\\Application\\chrome.exe"));
        }
    } else {
        for (name, bin) in [
            ("Google Chrome", "google-chrome"),
            ("Google Chrome", "google-chrome-stable"),
            ("Chromium", "chromium"),
            ("Chromium", "chromium-browser"),
            ("Microsoft Edge", "microsoft-edge"),
            ("Brave", "brave-browser"),
        ] {
            if let Ok(p) = which_in_path(bin) {
                push(name, p);
            }
        }
    }
    if let Some(p) = cft_exe().filter(|p| p.is_file()) {
        push("Chrome for Testing", p);
    }
    out
}

fn which_in_path(bin: &str) -> Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
        .ok_or(())
}

fn cft_dir() -> PathBuf {
    data_dir().join("chrome")
}

fn cft_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", _) => Some("mac-x64"),
        ("windows", "x86") => Some("win32"),
        ("windows", _) => Some("win64"),
        ("linux", "x86_64") => Some("linux64"),
        _ => None,
    }
}

fn cft_exe() -> Option<PathBuf> {
    let plat = cft_platform()?;
    let root = cft_dir().join(format!("chrome-{plat}"));
    Some(match std::env::consts::OS {
        "macos" => {
            root.join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
        }
        "windows" => root.join("chrome.exe"),
        _ => root.join("chrome"),
    })
}

pub async fn download_chrome_for_testing() -> Result<PathBuf, String> {
    let plat = cft_platform().ok_or("Chrome for Testing has no build for this system")?;
    emit(json!({ "kind": "browser", "state": "downloading" }));
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| e.to_string())?;
    let index: Value = client
        .get("https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let url = index["channels"]["Stable"]["downloads"]["chrome"]
        .as_array()
        .and_then(|a| a.iter().find(|d| d["platform"] == plat))
        .and_then(|d| d["url"].as_str())
        .ok_or("Chrome for Testing index has no download for this system")?
        .to_string();
    let bytes = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let dir = cft_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join("chrome.zip");
    std::fs::write(&zip, &bytes).map_err(|e| e.to_string())?;
    let status = if cfg!(windows) {
        std::process::Command::new("tar")
            .args(["-xf"])
            .arg(&zip)
            .arg("-C")
            .arg(&dir)
            .status()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("ditto")
            .args(["-x", "-k"])
            .arg(&zip)
            .arg(&dir)
            .status()
    } else {
        std::process::Command::new("unzip")
            .args(["-q", "-o"])
            .arg(&zip)
            .arg("-d")
            .arg(&dir)
            .status()
    }
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip);
    if !status.success() {
        return Err("could not unpack Chrome for Testing".into());
    }
    let exe = cft_exe().ok_or("unknown Chrome for Testing layout")?;
    if !exe.is_file() {
        return Err("Chrome for Testing unpacked without its executable".into());
    }
    #[cfg(target_os = "macos")]
    {
        let app = exe.ancestors().nth(3).map(Path::to_path_buf);
        if let Some(app) = app {
            let _ = std::process::Command::new("xattr")
                .args(["-dr", "com.apple.quarantine"])
                .arg(app)
                .status();
        }
    }
    Ok(exe)
}

async fn pick_exe(allow_download: bool) -> Result<PathBuf, String> {
    if let Some(p) = config()["browser_path"].as_str() {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
    }
    if let Some((_, p)) = installed_browsers().into_iter().next() {
        return Ok(p);
    }
    if allow_download {
        return download_chrome_for_testing().await;
    }
    Err("no Chromium browser found (Chrome, Edge, Brave or Chromium)".into())
}

fn read_active_port(profile: &Path) -> Option<String> {
    let text = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let mut lines = text.lines();
    let port = lines.next()?.trim().to_string();
    let path = lines.next()?.trim().to_string();
    Some(format!("ws://127.0.0.1:{port}{path}"))
}

pub async fn start(url: Option<String>, allow_download: bool) -> Result<Arc<Browser>, String> {
    let _guard = start_lock().lock().await;
    if let Some(b) = current() {
        if let Some(u) = url {
            b.open_tab(&u).await?;
        }
        return Ok(b);
    }
    let profile = profile_dir();

    if let Some(ws) = read_active_port(&profile) {
        let rt: Value = std::fs::read_to_string(runtime_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let headless = rt["headless"].as_bool().unwrap_or(false);
        if let Ok(b) = attach(&ws, "reconnected".into(), None, headless, false).await {
            if let Some(u) = url {
                b.open_tab(&u).await?;
            }
            return Ok(b);
        }
        let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
    }

    let exe = pick_exe(allow_download).await?;
    let headless = mode() != "window";
    emit(json!({ "kind": "browser", "state": "starting" }));
    let mut cmd = tokio::process::Command::new(&exe);
    if headless {
        cmd.arg("--headless=new");
        cmd.arg("--autoplay-policy=no-user-gesture-required");
        if let Some(ua) = real_user_agent(&exe).await {
            cmd.arg(format!("--user-agent={ua}"));
        }
    } else {
        cmd.arg("--disable-backgrounding-occluded-windows")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-background-timer-throttling");
    }
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-port=0")
        .arg("--remote-allow-origins=*")
        .arg("--disable-blink-features=AutomationControlled")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-features=Translate,ChromeWhatsNewUI")
        .arg("--window-size=1280,860")
        .arg("about:blank")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    let pid = child.id();

    let mut ws = None;
    for _ in 0..100 {
        if let Some(w) = read_active_port(&profile) {
            ws = Some(w);
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("the browser exited at start ({status})"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let ws = ws.ok_or("the browser did not open its debugging port")?;
    let _ = std::fs::write(runtime_file(), json!({ "headless": headless }).to_string());
    let b = attach(&ws, exe.display().to_string(), pid, headless, false).await?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    if let Some(u) = url {
        match b.tabs().into_iter().find(|t| t.url == "about:blank") {
            Some(t) => b.navigate(&t.id, &u).await?,
            None => {
                b.open_tab(&u).await?;
            }
        }
    }
    Ok(b)
}

async fn attach(
    ws: &str,
    exe: String,
    pid: Option<u32>,
    headless: bool,
    attached: bool,
) -> Result<Arc<Browser>, String> {
    let (cdp, events) = Cdp::connect(ws).await?;
    adopt(cdp, events, exe, pid, headless, attached).await
}

async fn adopt(
    cdp: Arc<Cdp>,
    events: tokio::sync::mpsc::UnboundedReceiver<Event>,
    exe: String,
    pid: Option<u32>,
    headless: bool,
    attached: bool,
) -> Result<Arc<Browser>, String> {
    let b = Arc::new(Browser {
        cdp: cdp.clone(),
        exe,
        pid,
        headless,
        attached,
        tabs: Mutex::default(),
        order: Mutex::default(),
        ua_override: Mutex::default(),
        view: Mutex::default(),
        private_contexts: Mutex::default(),
        child_sessions: Mutex::default(),
        ready: Mutex::default(),
    });
    if headless {
        if let Ok(v) = cdp.call("Browser.getVersion", json!({}), None).await {
            *b.ua_override.lock().unwrap_or_else(|e| e.into_inner()) = Some(ua_override_for(&v));
        }
    }
    if !attached {
        let _ = cdp
            .call(
                "Browser.setDownloadBehavior",
                json!({ "behavior": "allow", "downloadPath": downloads_dir(), "eventsEnabled": true }),
                None,
            )
            .await;
    }
    if let Ok(mut s) = slot().lock() {
        *s = Some(b.clone());
    }
    tokio::spawn(event_loop(b.clone(), events));
    cdp.call(
        "Target.setDiscoverTargets",
        json!({ "discover": true }),
        None,
    )
    .await?;
    let targets = cdp.call("Target.getTargets", json!({}), None).await?;
    for t in targets["targetInfos"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        b.on_target(&t);
    }
    emit(json!({ "kind": "browser", "state": "open" }));
    b.emit_tabs();
    Ok(b)
}

async fn discover(base: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get(format!("{base}/json/version"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    harness::parse_version(&body)
        .ok_or_else(|| "the answer has no webSocketDebuggerUrl".to_string())
}

async fn running_chrome_ws(base: &str) -> Option<String> {
    let port = url::Url::parse(base).ok()?.port()?;
    let dirs = tokio::task::spawn_blocking(super::profiles::sources)
        .await
        .ok()?;
    dirs.iter()
        .filter_map(|s| read_active_port(&s.data_dir))
        .find(|ws| ws.starts_with(&format!("ws://127.0.0.1:{port}/")))
}

pub async fn attach_external(target: AttachTarget) -> Result<Arc<Browser>, String> {
    let _guard = start_lock().lock().await;
    let (ws, label) = match target {
        AttachTarget::Ws(ws) => (ws.clone(), ws),
        AttachTarget::Http(base) => match discover(&base).await {
            Ok(ws) => (ws, base),
            Err(detail) => match running_chrome_ws(&base).await {
                Some(ws) => (ws, base),
                None => return Err(harness::not_debuggable(&base, &detail).to_string()),
            },
        },
    };
    if current().is_some() {
        ensure_no_running_job(false)?;
    }
    let (cdp, events) = Cdp::connect(&ws)
        .await
        .map_err(|e| harness::connect_failed(&label, &e).to_string())?;
    stop().await?;
    adopt(cdp, events, format!("attached {label}"), None, false, true).await
}

pub async fn stop() -> Result<(), String> {
    if let Some(b) = current() {
        if b.attached {
            b.cdp.close();
        } else {
            let _ = b.cdp.call("Browser.close", json!({}), None).await;
        }
    }
    if let Ok(mut s) = slot().lock() {
        *s = None;
    }
    emit(json!({ "kind": "browser", "state": "closed" }));
    Ok(())
}

async fn real_user_agent(exe: &Path) -> Option<String> {
    let cache = data_dir().join("ua-cache.json");
    let stamp = std::fs::metadata(exe)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let key = format!("{}@{stamp}", exe.display());
    let mut c: Value = std::fs::read_to_string(&cache)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(ua) = c[&key].as_str() {
        return Some(ua.to_string());
    }
    let tmp = std::env::temp_dir().join(format!("omniget-ua-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let mut child = tokio::process::Command::new(exe)
        .arg("--headless=new")
        .arg(format!("--user-data-dir={}", tmp.display()))
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("about:blank")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut ua = None;
    for _ in 0..50 {
        if let Some(ws) = read_active_port(&tmp) {
            if let Ok((cdp, _rx)) = Cdp::connect(&ws).await {
                if let Ok(v) = cdp.call("Browser.getVersion", json!({}), None).await {
                    ua = v["userAgent"]
                        .as_str()
                        .map(|u| u.replace("HeadlessChrome", "Chrome"));
                }
                let _ = cdp.call("Browser.close", json!({}), None).await;
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&tmp);
    let ua = ua?;
    c[&key] = json!(ua);
    let _ = std::fs::write(&cache, c.to_string());
    Some(ua)
}

fn ua_override_for(version: &Value) -> Value {
    let ua = version["userAgent"]
        .as_str()
        .unwrap_or_default()
        .replace("HeadlessChrome", "Chrome");
    let product = version["product"].as_str().unwrap_or_default();
    let full = product.rsplit('/').next().unwrap_or("").to_string();
    let major = full.split('.').next().unwrap_or("").to_string();
    let (platform, arch) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("macOS", "arm"),
        ("macos", _) => ("macOS", "x86"),
        ("windows", _) => ("Windows", "x86"),
        _ => ("Linux", "x86"),
    };
    let brand = if product.contains("Edg") {
        "Microsoft Edge"
    } else {
        "Google Chrome"
    };
    json!({
        "userAgent": ua,
        "userAgentMetadata": {
            "brands": [
                { "brand": brand, "version": major },
                { "brand": "Chromium", "version": major },
                { "brand": "Not.A/Brand", "version": "99" }
            ],
            "fullVersionList": [
                { "brand": brand, "version": full },
                { "brand": "Chromium", "version": full },
                { "brand": "Not.A/Brand", "version": "99.0.0.0" }
            ],
            "fullVersion": full,
            "platform": platform,
            "platformVersion": "",
            "architecture": arch,
            "model": "",
            "mobile": false,
            "bitness": "64",
            "wow64": false
        }
    })
}

pub const BROWSER_BUSY: &str = "BROWSER_BUSY";

fn busy_error(running: usize, force: bool) -> Result<(), String> {
    if running > 0 && !force {
        return Err(format!(
            "{BROWSER_BUSY}: {running} download job(s) running. Restarting or stopping the browser now can make their remaining items fail, so wait for them or cancel them first."
        ));
    }
    Ok(())
}

pub fn ensure_no_running_job(force: bool) -> Result<(), String> {
    busy_error(super::jobs::running_count(), force)
}

pub async fn switch_mode(new_mode: &str) -> Result<Arc<Browser>, String> {
    set_config("mode", json!(new_mode))?;
    let urls: Vec<String> = current()
        .map(|b| {
            b.tabs()
                .into_iter()
                .filter(|t| {
                    !t.private && !t.url.starts_with("about:") && !t.url.starts_with("chrome")
                })
                .map(|t| t.url)
                .collect()
        })
        .unwrap_or_default();
    stop().await?;
    for _ in 0..50 {
        if read_active_port(&profile_dir()).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::fs::remove_file(profile_dir().join("DevToolsActivePort"));
    let mut it = urls.into_iter();
    let b = start(it.next(), true).await?;
    for u in it {
        let _ = b.open_tab(&u).await;
    }
    Ok(b)
}

pub fn status() -> Value {
    match current() {
        Some(b) => json!({
            "open": true,
            "exe": b.exe,
            "pid": b.pid,
            "mode": if b.headless { "embedded" } else { "window" },
            "attached": b.attached,
            "tabs": b.tabs(),
            "view": b.view.lock().ok().and_then(|v| v.clone()),
            "network": sniffer::summary(),
            "block_ads": block_ads(),
            "autoplay": autoplay(),
            "downloads_dir": downloads_dir(),
        }),
        None => json!({
            "open": false,
            "installed": installed_browsers()
                .into_iter()
                .map(|(n, p)| json!({ "name": n, "path": p }))
                .collect::<Vec<_>>(),
            "configured": config()["browser_path"].clone(),
            "mode": mode(),
            "block_ads": block_ads(),
            "autoplay": autoplay(),
        }),
    }
}

async fn event_loop(b: Arc<Browser>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Event>) {
    while let Some(ev) = rx.recv().await {
        b.on_event(ev);
    }
    let mut replaced = false;
    if let Ok(mut s) = slot().lock() {
        if s.as_ref().is_some_and(|x| Arc::ptr_eq(x, &b)) {
            *s = None;
        } else {
            replaced = s.is_some();
        }
    }
    if !replaced {
        emit(json!({ "kind": "browser", "state": "closed" }));
    }
}

impl Browser {
    fn on_target(self: &Arc<Self>, info: &Value) {
        if info["type"] != "page" {
            return;
        }
        let Some(id) = info["targetId"].as_str() else {
            return;
        };
        let url = info["url"].as_str().unwrap_or_default().to_string();
        let title = info["title"].as_str().unwrap_or_default().to_string();
        let is_new = {
            let mut tabs = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
            let new = !tabs.contains_key(id);
            let tab = tabs.entry(id.to_string()).or_insert_with(|| Tab {
                id: id.to_string(),
                ..Default::default()
            });
            tab.url = url;
            tab.title = title;
            let ctx = info["browserContextId"].as_str().map(String::from);
            tab.private = ctx.as_ref().is_some_and(|c| {
                self.private_contexts
                    .lock()
                    .map(|p| p.contains(c))
                    .unwrap_or(false)
            });
            tab.context = ctx;
            new
        };
        if is_new {
            if let Ok(mut o) = self.order.lock() {
                o.push(id.to_string());
            }
            let me = self.clone();
            let id = id.to_string();
            tokio::spawn(async move {
                let _ = me
                    .cdp
                    .call(
                        "Target.attachToTarget",
                        json!({ "targetId": id, "flatten": true }),
                        None,
                    )
                    .await;
            });
        }
        self.emit_tabs();
    }

    fn on_event(self: &Arc<Self>, ev: Event) {
        let p = &ev.params;
        match ev.method.as_str() {
            "Target.targetCreated" | "Target.targetInfoChanged" => self.on_target(&p["targetInfo"]),
            "Target.targetDestroyed" => {
                if let Some(id) = p["targetId"].as_str() {
                    self.tabs
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(id);
                    if let Ok(mut o) = self.order.lock() {
                        o.retain(|x| x != id);
                    }
                    self.emit_tabs();
                }
            }
            "Target.attachedToTarget" => {
                let (Some(sid), Some(tid)) = (
                    p["sessionId"].as_str(),
                    p["targetInfo"]["targetId"].as_str(),
                ) else {
                    return;
                };
                if p["targetInfo"]["type"] != "page" {
                    let parent = ev
                        .session
                        .as_deref()
                        .and_then(|ps| {
                            self.tab_by_session(ps)
                                .map(|t| t.id)
                                .or_else(|| self.child_sessions.lock().ok()?.get(ps).cloned())
                        })
                        .unwrap_or_default();
                    if let Ok(mut c) = self.child_sessions.lock() {
                        c.insert(sid.to_string(), parent);
                    }
                    let me = self.clone();
                    let sid = sid.to_string();
                    tokio::spawn(async move { me.setup_child(&sid).await });
                    return;
                }
                if let Some(tab) = self
                    .tabs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_mut(tid)
                {
                    tab.session = Some(sid.to_string());
                }
                let me = self.clone();
                let sid = sid.to_string();
                tokio::spawn(async move { me.setup_session(&sid).await });
            }
            "Target.detachedFromTarget" => {
                if let Some(sid) = p["sessionId"].as_str() {
                    if let Ok(mut r) = self.ready.lock() {
                        r.remove(sid);
                    }
                    for tab in self
                        .tabs
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .values_mut()
                    {
                        if tab.session.as_deref() == Some(sid) {
                            tab.session = None;
                        }
                    }
                }
            }
            "Network.requestWillBeSent" => {
                if let Some(s) = &ev.session {
                    sniffer::on_request(s, p);
                }
            }
            "Network.requestWillBeSentExtraInfo" => {
                if let Some(s) = &ev.session {
                    sniffer::on_request_extra(s, p);
                }
            }
            "Page.screencastFrame" => {
                let Some(s) = &ev.session else { return };
                let cdp = self.cdp.clone();
                let (sid, ack) = (s.clone(), p["sessionId"].clone());
                tokio::spawn(async move {
                    let _ = cdp
                        .call(
                            "Page.screencastFrameAck",
                            json!({ "sessionId": ack }),
                            Some(&sid),
                        )
                        .await;
                });
                let tab = self.tab_by_session(s).map(|t| t.id).unwrap_or_default();
                emit(json!({
                    "kind": "frame",
                    "tab": tab,
                    "data": p["data"],
                    "width": p["metadata"]["deviceWidth"],
                    "height": p["metadata"]["deviceHeight"],
                }));
            }
            "Page.javascriptDialogOpening" => {
                let tab = ev
                    .session
                    .as_deref()
                    .and_then(|s| self.tab_by_session(s))
                    .map(|t| t.id);
                emit(json!({
                    "kind": "dialog",
                    "tab": tab,
                    "type": p["type"],
                    "message": p["message"],
                    "default": p["defaultPrompt"],
                    "url": p["url"],
                }));
            }
            "Page.fileChooserOpened" => {
                let tab = ev
                    .session
                    .as_deref()
                    .and_then(|s| self.tab_by_session(s))
                    .map(|t| t.id);
                emit(json!({
                    "kind": "file_chooser",
                    "tab": tab,
                    "mode": p["mode"],
                    "backend_node_id": p["backendNodeId"],
                }));
            }
            "Page.frameStartedLoading" | "Page.frameStoppedLoading" => {
                let Some(s) = &ev.session else { return };
                let loading = ev.method == "Page.frameStartedLoading";
                let fid = p["frameId"].as_str().unwrap_or_default();
                let changed = {
                    let mut tabs = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
                    match tabs
                        .values_mut()
                        .find(|t| t.session.as_deref() == Some(s) && t.id == fid)
                    {
                        Some(t) if t.loading != loading => {
                            t.loading = loading;
                            true
                        }
                        _ => false,
                    }
                };
                if changed {
                    self.emit_tabs();
                }
            }
            "Browser.downloadWillBegin" => {
                emit(json!({
                    "kind": "browser_download",
                    "guid": p["guid"],
                    "url": p["url"],
                    "filename": p["suggestedFilename"],
                    "state": "started",
                    "dir": downloads_dir(),
                }));
            }
            "Browser.downloadProgress" => {
                emit(json!({
                    "kind": "browser_download",
                    "guid": p["guid"],
                    "received": p["receivedBytes"],
                    "total": p["totalBytes"],
                    "state": p["state"],
                }));
            }
            "Network.responseReceived" => {
                let Some(s) = &ev.session else { return };
                let tab = self.tab_by_session(s).unwrap_or_default();
                if let Some(item) = sniffer::on_response(s, &tab.id, (&tab.url, &tab.title), p) {
                    emit(json!({ "kind": "sniffed", "item": item }));
                    if item.kind == "hls" {
                        tokio::spawn(inspect_playlist(item));
                    }
                }
            }
            _ => {}
        }
    }

    async fn setup_child(&self, sid: &str) {
        let s = Some(sid);
        let ua = self.ua_override.lock().ok().and_then(|u| u.clone());
        if let Some(ua) = ua {
            let _ = self.cdp.call("Network.setUserAgentOverride", ua, s).await;
        }
        let _ = self
            .cdp
            .call("Network.enable", json!({ "maxPostDataSize": 65536 }), s)
            .await;
        if block_ads() && !self.attached {
            let _ = self
                .cdp
                .call(
                    "Network.setBlockedURLs",
                    json!({ "urls": super::blocklist::patterns() }),
                    s,
                )
                .await;
        }
        let _ = self
            .cdp
            .call(
                "Target.setAutoAttach",
                json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
                s,
            )
            .await;
        let _ = self
            .cdp
            .call("Runtime.runIfWaitingForDebugger", json!({}), s)
            .await;
    }

    async fn setup_session(&self, sid: &str) {
        let s = Some(sid);
        let _ = self
            .cdp
            .call(
                "Target.setAutoAttach",
                json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
                s,
            )
            .await;
        self.setup_session_rest(sid).await
    }

    async fn setup_session_rest(&self, sid: &str) {
        let s = Some(sid);
        let _ = self
            .cdp
            .call("Network.enable", json!({ "maxPostDataSize": 65536 }), s)
            .await;
        let ua = self.ua_override.lock().ok().and_then(|u| u.clone());
        if let Some(ua) = ua {
            let _ = self.cdp.call("Network.setUserAgentOverride", ua, s).await;
        }
        if block_ads() && !self.attached {
            let _ = self
                .cdp
                .call(
                    "Network.setBlockedURLs",
                    json!({ "urls": super::blocklist::patterns() }),
                    s,
                )
                .await;
        }
        let _ = self.cdp.call("Page.enable", json!({}), s).await;
        if self.headless {
            let _ = self
                .cdp
                .call(
                    "Page.setInterceptFileChooserDialog",
                    json!({ "enabled": true }),
                    s,
                )
                .await;
        }
        if let Ok(mut r) = self.ready.lock() {
            r.insert(sid.to_string());
        }
        let _ = self
            .cdp
            .call("Runtime.runIfWaitingForDebugger", json!({}), s)
            .await;
    }

    pub async fn view(
        &self,
        tab_id: &str,
        width: u32,
        height: u32,
        dpr: f64,
    ) -> Result<(), String> {
        let (width, height) = (width.clamp(200, 4000), height.clamp(150, 4000));
        let dpr = if dpr.is_finite() && dpr > 0.0 {
            dpr.min(3.0)
        } else {
            1.0
        };
        let prev = self.view.lock().ok().and_then(|v| v.clone());
        if let Some(p) = &prev {
            if p.tab != tab_id {
                if let Some(s) = self.tab(&p.tab).and_then(|t| t.session) {
                    let _ = self
                        .cdp
                        .call("Page.stopScreencast", json!({}), Some(&s))
                        .await;
                }
            }
        }
        let s = self.session_for(tab_id).await?;
        let _ = self
            .cdp
            .call("Target.activateTarget", json!({ "targetId": tab_id }), None)
            .await;
        self.cdp
            .call(
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": width, "height": height, "deviceScaleFactor": dpr, "mobile": false }),
                Some(&s),
            )
            .await?;
        let _ = self
            .cdp
            .call("Page.stopScreencast", json!({}), Some(&s))
            .await;
        self.cdp
            .call(
                "Page.startScreencast",
                json!({
                    "format": "jpeg",
                    "quality": 80,
                    "maxWidth": (width as f64 * dpr).round() as u32,
                    "maxHeight": (height as f64 * dpr).round() as u32,
                    "everyNthFrame": 1
                }),
                Some(&s),
            )
            .await?;
        if let Ok(mut v) = self.view.lock() {
            *v = Some(View {
                tab: tab_id.to_string(),
                width,
                height,
                dpr,
            });
        }
        Ok(())
    }

    pub async fn unview(&self) {
        let prev = self.view.lock().ok().and_then(|mut v| v.take());
        if let Some(p) = prev {
            if let Some(s) = self.tab(&p.tab).and_then(|t| t.session) {
                let _ = self
                    .cdp
                    .call("Page.stopScreencast", json!({}), Some(&s))
                    .await;
                let _ = self
                    .cdp
                    .call("Emulation.clearDeviceMetricsOverride", json!({}), Some(&s))
                    .await;
            }
        }
    }

    pub async fn mouse(&self, tab_id: &str, params: Value) -> Result<(), String> {
        let s = self.session_for(tab_id).await?;
        self.cdp
            .call_timeout(
                "Input.dispatchMouseEvent",
                params,
                Some(&s),
                Duration::from_secs(5),
            )
            .await
            .map(|_| ())
    }

    pub async fn key(&self, tab_id: &str, params: Value) -> Result<(), String> {
        let s = self.session_for(tab_id).await?;
        self.cdp
            .call_timeout(
                "Input.dispatchKeyEvent",
                params,
                Some(&s),
                Duration::from_secs(5),
            )
            .await
            .map(|_| ())
    }

    pub async fn insert_text(&self, tab_id: &str, text: &str) -> Result<(), String> {
        let s = self.session_for(tab_id).await?;
        self.cdp
            .call("Input.insertText", json!({ "text": text }), Some(&s))
            .await
            .map(|_| ())
    }

    pub async fn press(&self, tab_id: &str, key: &str, modifiers: u32) -> Result<(), String> {
        let (code, vk, text) = key_info(key);
        let text = if modifiers & (2 | 4) != 0 {
            String::new()
        } else {
            text
        };
        let down = json!({
            "type": if text.is_empty() { "rawKeyDown" } else { "keyDown" },
            // WHY: no nativeVirtualKeyCode: macOS reads it as a native key code, and a Windows code there hangs the renderer.
            // No nativeVirtualKeyCode: macOS reads it as a native key code,
            // and a Windows code there hangs the renderer on real pages.
            "key": key, "code": code, "windowsVirtualKeyCode": vk,
            "text": text, "unmodifiedText": text, "modifiers": modifiers,
        });
        self.key(tab_id, down).await?;
        self.key(
            tab_id,
            json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers }),
        )
        .await
    }

    pub async fn history(&self, tab_id: &str, action: &str) -> Result<(), String> {
        let s = self.session_for(tab_id).await?;
        match action {
            "reload" => self
                .cdp
                .call("Page.reload", json!({}), Some(&s))
                .await
                .map(|_| ()),
            "stop" => self
                .cdp
                .call("Page.stopLoading", json!({}), Some(&s))
                .await
                .map(|_| ()),
            "back" | "forward" => {
                let h = self
                    .cdp
                    .call("Page.getNavigationHistory", json!({}), Some(&s))
                    .await?;
                let cur = h["currentIndex"].as_i64().unwrap_or(0);
                let idx = if action == "back" { cur - 1 } else { cur + 1 };
                let Some(entry) = h["entries"]
                    .as_array()
                    .and_then(|e| e.get(idx.max(0) as usize))
                else {
                    return Ok(());
                };
                if idx < 0 {
                    return Ok(());
                }
                self.cdp
                    .call(
                        "Page.navigateToHistoryEntry",
                        json!({ "entryId": entry["id"] }),
                        Some(&s),
                    )
                    .await
                    .map(|_| ())
            }
            other => Err(format!("unknown history action {other}")),
        }
    }

    pub async fn screenshot(&self, tab_id: &str) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let s = self.session_for(tab_id).await?;
        let r = self
            .cdp
            .call_timeout(
                "Page.captureScreenshot",
                json!({ "format": "png" }),
                Some(&s),
                Duration::from_secs(20),
            )
            .await?;
        base64::engine::general_purpose::STANDARD
            .decode(r["data"].as_str().unwrap_or_default())
            .map_err(|e| e.to_string())
    }

    pub async fn open_private_tab(&self, url: &str) -> Result<String, String> {
        let c = self
            .cdp
            .call(
                "Target.createBrowserContext",
                json!({ "disposeOnDetach": false }),
                None,
            )
            .await?;
        let ctx = c["browserContextId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if let Ok(mut p) = self.private_contexts.lock() {
            p.insert(ctx.clone());
        }
        let r = self
            .cdp
            .call(
                "Target.createTarget",
                json!({ "url": url, "browserContextId": ctx }),
                None,
            )
            .await?;
        Ok(r["targetId"].as_str().unwrap_or_default().to_string())
    }

    pub async fn show_window(&self, tab_id: &str) -> Result<(), String> {
        let w = self
            .cdp
            .call(
                "Browser.getWindowForTarget",
                json!({ "targetId": tab_id }),
                None,
            )
            .await?;
        let id = w["windowId"].clone();
        let _ = self
            .cdp
            .call(
                "Browser.setWindowBounds",
                json!({ "windowId": id, "bounds": { "windowState": "normal" } }),
                None,
            )
            .await;
        let _ = self
            .cdp
            .call(
                "Browser.setWindowBounds",
                json!({ "windowId": id, "bounds": { "left": 40, "top": 40 } }),
                None,
            )
            .await;
        self.activate_tab(tab_id).await
    }

    fn tab_by_session(&self, sid: &str) -> Option<Tab> {
        let direct = self
            .tabs
            .lock()
            .ok()?
            .values()
            .find(|t| t.session.as_deref() == Some(sid))
            .cloned();
        direct.or_else(|| {
            let parent = self.child_sessions.lock().ok()?.get(sid).cloned()?;
            self.tab(&parent)
        })
    }

    pub fn tab(&self, id: &str) -> Option<Tab> {
        self.tabs.lock().ok()?.get(id).cloned()
    }

    pub fn tabs(&self) -> Vec<Tab> {
        let tabs = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
        let order = self.order.lock().unwrap_or_else(|e| e.into_inner());
        order
            .iter()
            .filter_map(|id| tabs.get(id).cloned())
            .collect()
    }

    fn emit_tabs(&self) {
        emit(json!({ "kind": "tabs", "tabs": self.tabs() }));
    }

    pub async fn open_tab(&self, url: &str) -> Result<String, String> {
        let blank = url.is_empty() || url == "about:blank";
        let r = self
            .cdp
            .call("Target.createTarget", json!({ "url": "about:blank" }), None)
            .await?;
        let id = r["targetId"].as_str().unwrap_or_default().to_string();
        let _ = self
            .cdp
            .call("Target.activateTarget", json!({ "targetId": id }), None)
            .await;
        if !blank {
            for _ in 0..50 {
                let ready = self
                    .tab(&id)
                    .and_then(|t| t.session)
                    .is_some_and(|s| self.ready.lock().is_ok_and(|r| r.contains(&s)));
                if ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(60)).await;
            }
            self.navigate(&id, url).await?;
        }
        Ok(id)
    }

    pub async fn close_tab(&self, id: &str) -> Result<(), String> {
        let ctx = self.tab(id).filter(|t| t.private).and_then(|t| t.context);
        let r = self
            .cdp
            .call("Target.closeTarget", json!({ "targetId": id }), None)
            .await
            .map(|_| ());
        if let Some(c) = ctx {
            let still_used = self
                .tabs()
                .iter()
                .any(|t| t.id != id && t.context.as_deref() == Some(&c));
            if !still_used {
                let _ = self
                    .cdp
                    .call(
                        "Target.disposeBrowserContext",
                        json!({ "browserContextId": c }),
                        None,
                    )
                    .await;
                if let Ok(mut p) = self.private_contexts.lock() {
                    p.remove(&c);
                }
            }
        }
        r
    }

    pub async fn activate_tab(&self, id: &str) -> Result<(), String> {
        self.cdp
            .call("Target.activateTarget", json!({ "targetId": id }), None)
            .await
            .map(|_| ())
    }

    pub async fn session_for(&self, tab_id: &str) -> Result<String, String> {
        for _ in 0..30 {
            if let Some(s) = self.tab(tab_id).and_then(|t| t.session) {
                return Ok(s);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(format!("tab {tab_id} is not attached"))
    }

    pub async fn navigate(&self, tab_id: &str, url: &str) -> Result<(), String> {
        let s = self.session_for(tab_id).await?;
        self.cdp
            .call("Page.navigate", json!({ "url": url }), Some(&s))
            .await
            .map(|_| ())
    }

    pub async fn eval(
        &self,
        tab_id: &str,
        expression: &str,
        timeout: Duration,
    ) -> Result<Value, String> {
        let s = self.session_for(tab_id).await?;
        let r = self
            .cdp
            .call_timeout(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                    "userGesture": true,
                }),
                Some(&s),
                timeout,
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex["exception"]["description"]
                .as_str()
                .or_else(|| ex["text"].as_str())
                .unwrap_or("script error");
            return Err(msg.to_string());
        }
        Ok(r["result"]["value"].clone())
    }

    pub async fn cookies_for(&self, host: &str) -> Result<Vec<Value>, String> {
        let r = self.cdp.call("Storage.getCookies", json!({}), None).await?;
        Ok(r["cookies"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| {
                let d = c["domain"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches('.');
                host == d || host.ends_with(&format!(".{d}"))
            })
            .collect())
    }

    pub async fn user_agent(&self) -> Option<String> {
        let r = self
            .cdp
            .call("Browser.getVersion", json!({}), None)
            .await
            .ok()?;
        r["userAgent"].as_str().map(String::from)
    }

    pub fn tab_for_origin(&self, url: &str) -> Option<Tab> {
        let target = url::Url::parse(url).ok()?.origin();
        self.tabs().into_iter().find(|t| {
            url::Url::parse(&t.url)
                .map(|u| u.origin() == target)
                .unwrap_or(false)
        })
    }

    pub async fn response_body(&self, session: &str, request_id: &str) -> Result<String, String> {
        let r = self
            .cdp
            .call(
                "Network.getResponseBody",
                json!({ "requestId": request_id }),
                Some(session),
            )
            .await?;
        let body = r["body"].as_str().unwrap_or_default();
        if r["base64Encoded"].as_bool().unwrap_or(false) {
            Ok(format!("<base64, {} chars>", body.len()))
        } else {
            Ok(body.to_string())
        }
    }
}

pub async fn warm_up(url: &str, reload: bool) {
    let Some(b) = current() else { return };
    let Ok(origin) = url::Url::parse(url).map(|u| u.origin().ascii_serialization()) else {
        return;
    };
    let tab_id = match b.tab_for_origin(url) {
        Some(t) => {
            if !reload {
                return;
            }
            if let Ok(s) = b.session_for(&t.id).await {
                let _ = b.cdp.call("Page.reload", json!({}), Some(&s)).await;
            }
            t.id
        }
        None => match b
            .cdp
            .call(
                "Target.createTarget",
                json!({ "url": format!("{origin}/"), "background": true }),
                None,
            )
            .await
        {
            Ok(r) => r["targetId"].as_str().unwrap_or_default().to_string(),
            Err(_) => return,
        },
    };
    tokio::time::sleep(Duration::from_secs(2)).await;
    for _ in 0..28 {
        let title = b.tab(&tab_id).map(|t| t.title).unwrap_or_default();
        let ready = b
            .eval(&tab_id, "document.readyState", Duration::from_secs(5))
            .await
            .ok()
            .and_then(|v| v.as_str().map(|s| s == "complete"))
            .unwrap_or(false);
        if ready && !title.to_ascii_lowercase().starts_with("just a moment") {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub async fn fetch(
    url: &str,
    method: &str,
    headers: &serde_json::Map<String, Value>,
    body: Option<&str>,
) -> Result<Value, String> {
    let b = current();
    if let Some(tab) = b.as_ref().and_then(|b| b.tab_for_origin(url)) {
        let init = json!({
            "method": method,
            "headers": headers,
            "body": body,
            "credentials": "include",
        });
        let expr = format!(
            "(async()=>{{const i={init};if(i.body===null)delete i.body;const r=await fetch({url},i);\
             const h={{}};r.headers.forEach((v,k)=>h[k]=v);\
             return {{status:r.status,url:r.url,headers:h,body:await r.text()}}}})()",
            init = init,
            url = Value::String(url.to_string()),
        );
        if let Some(b) = &b {
            return b.eval(&tab.id, &expr, Duration::from_secs(90)).await;
        }
    }

    let host = super::host_of(url).ok_or("bad url")?;
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(90));
    let mut cookie = String::new();
    if let Some(b) = &b {
        if let Some(ua) = b.user_agent().await {
            builder = builder.user_agent(ua.replace("HeadlessChrome", "Chrome"));
        }
        for c in b.cookies_for(&host).await.unwrap_or_default() {
            if !cookie.is_empty() {
                cookie.push_str("; ");
            }
            cookie.push_str(&format!(
                "{}={}",
                c["name"].as_str().unwrap_or_default(),
                c["value"].as_str().unwrap_or_default()
            ));
        }
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
    let mut req = client.request(m, url);
    for (k, v) in headers {
        if let Some(v) = v.as_str() {
            req = req.header(k.as_str(), v);
        }
    }
    if !cookie.is_empty() && !headers.keys().any(|k| k.eq_ignore_ascii_case("cookie")) {
        req = req.header("Cookie", cookie);
    }
    if let Some(body) = body {
        req = req.body(body.to_string());
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let final_url = resp.url().to_string();
    let hdrs: serde_json::Map<String, Value> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                Value::String(v.to_str().unwrap_or("").to_string()),
            )
        })
        .collect();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    Ok(json!({ "status": status, "url": final_url, "headers": hdrs, "body": text }))
}

pub fn key_info(key: &str) -> (String, i64, String) {
    let named: &[(&str, &str, i64)] = &[
        ("Enter", "Enter", 13),
        ("Tab", "Tab", 9),
        ("Backspace", "Backspace", 8),
        ("Escape", "Escape", 27),
        ("Delete", "Delete", 46),
        ("Space", "Space", 32),
        ("ArrowLeft", "ArrowLeft", 37),
        ("ArrowUp", "ArrowUp", 38),
        ("ArrowRight", "ArrowRight", 39),
        ("ArrowDown", "ArrowDown", 40),
        ("Home", "Home", 36),
        ("End", "End", 35),
        ("PageUp", "PageUp", 33),
        ("PageDown", "PageDown", 34),
        ("Insert", "Insert", 45),
        ("F1", "F1", 112),
        ("F2", "F2", 113),
        ("F3", "F3", 114),
        ("F4", "F4", 115),
        ("F5", "F5", 116),
        ("F6", "F6", 117),
        ("F7", "F7", 118),
        ("F8", "F8", 119),
        ("F9", "F9", 120),
        ("F10", "F10", 121),
        ("F11", "F11", 122),
        ("F12", "F12", 123),
        ("Shift", "ShiftLeft", 16),
        ("Control", "ControlLeft", 17),
        ("Alt", "AltLeft", 18),
        ("Meta", "MetaLeft", 91),
    ];
    if let Some((_, code, vk)) = named.iter().find(|(k, _, _)| *k == key) {
        let text = match key {
            "Enter" => "\r",
            "Space" => " ",
            _ => "",
        };
        return (code.to_string(), *vk, text.to_string());
    }
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        let up = c.to_ascii_uppercase();
        let (code, vk) = if up.is_ascii_alphabetic() {
            (format!("Key{up}"), up as i64)
        } else if c.is_ascii_digit() {
            (format!("Digit{c}"), c as i64)
        } else if c == ' ' {
            ("Space".into(), 32)
        } else {
            (String::new(), 0)
        };
        return (code, vk, c.to_string());
    }
    (String::new(), 0, String::new())
}

async fn inspect_playlist(item: sniffer::Sniffed) {
    let Ok(client) = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return;
    };
    let mut req = client.get(&item.url);
    for (k, v) in &item.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let Ok(resp) = req.send().await else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(body) = resp.bytes().await else {
        return;
    };
    if body.len() > 4 * 1024 * 1024 {
        return;
    }
    let removed = sniffer::absorb_playlist(item.id, &body);
    if !removed.is_empty() {
        emit(json!({ "kind": "sniffed_removed", "ids": removed }));
        return;
    }
    let media = match m3u8_rs::parse_playlist_res(&body) {
        Ok(m3u8_rs::Playlist::MediaPlaylist(p)) => Some(p),
        Ok(m3u8_rs::Playlist::MasterPlaylist(m)) => {
            let Some(v) = m.variants.iter().find(|v| !v.is_i_frame) else {
                return;
            };
            let Ok(u) = url::Url::parse(&item.url).and_then(|b| b.join(&v.uri)) else {
                return;
            };
            let mut req = client.get(u.as_str());
            for (k, val) in &item.headers {
                req = req.header(k.as_str(), val.as_str());
            }
            match req.send().await {
                Ok(r) if r.status().is_success() => match r.bytes().await {
                    Ok(b) => match m3u8_rs::parse_playlist_res(&b) {
                        Ok(m3u8_rs::Playlist::MediaPlaylist(p)) => Some(p),
                        _ => None,
                    },
                    Err(_) => None,
                },
                _ => None,
            }
        }
        Err(_) => None,
    };
    if media.is_some_and(|p| crate::core::hls_downloader::is_live(&p)) {
        sniffer::mark_live(item.id);
        emit(json!({ "kind": "sniffed_live", "id": item.id }));
    }
}

#[cfg(test)]
mod busy_tests {
    use super::{busy_error, BROWSER_BUSY};

    #[test]
    fn stopping_the_browser_asks_first_while_a_job_runs() {
        assert!(busy_error(1, false).unwrap_err().starts_with(BROWSER_BUSY));
        assert!(busy_error(1, true).is_ok());
        assert!(busy_error(0, false).is_ok());
    }
}
