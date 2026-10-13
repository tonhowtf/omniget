use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::AppHandle;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

const RING_LINES: usize = 3_000;
const SUPPORT_EMAIL: &str = "tonhowtf@gmail.com";
const ISSUES_URL: &str = "https://getomniget.com/contact";

static RING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

pub fn init_tracing() {
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(RingLayer)
        .with(tracing_subscriber::filter::LevelFilter::INFO)
        .try_init();
}

struct RingLayer;

#[derive(Default)]
struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.insert_str(0, &format!("{value:?}"));
        } else {
            self.0.push_str(&format!(" {}={value:?}", field.name()));
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0.insert_str(0, value);
        } else {
            self.0.push_str(&format!(" {}={value}", field.name()));
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut f = Fields::default();
        event.record(&mut f);
        let m = event.metadata();
        let line = format!(
            "{} {:>5} {}: {}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            m.level(),
            m.target(),
            f.0
        );
        if let Ok(mut r) = RING.lock() {
            if r.len() >= RING_LINES {
                r.pop_front();
            }
            r.push_back(line);
        }
    }
}

pub fn redact(text: &str, home: Option<&str>, user: Option<&str>) -> String {
    use std::sync::OnceLock;
    static RES: OnceLock<Vec<(regex::Regex, &'static str)>> = OnceLock::new();
    let res = RES.get_or_init(|| {
        [
            (r"(?i)\b(oct|ogt|sk|pk|rk|ghp|gho|ghs|ghu|github_pat|xox[abpr]|AKIA|AIza|hf|glpat)[-_][A-Za-z0-9_\-]{6,}", "[REDACTED]"),
            (r#"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=\-]{6,}"#, "$1 [REDACTED]"),
            (r#"(?i)((?:token|secret|password|passwd|api[_-]?key|authorization|cookie|session)[A-Za-z_]*["']?\s*[:=]\s*["']?)[^\s"',;&]+"#, "${1}[REDACTED]"),
            (r"[A-Za-z0-9_\-+/]{40,}={0,2}", "[REDACTED]"),
            (r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}", "[email]"),
        ]
        .into_iter()
        .filter_map(|(p, r)| regex::Regex::new(p).ok().map(|re| (re, r)))
        .collect()
    });
    let mut out = text.to_string();
    if let Some(h) = home.filter(|h| h.len() > 1) {
        out = out.replace(h, "~");
    }
    for (re, rep) in res {
        out = re.replace_all(&out, *rep).into_owned();
    }
    if let Some(u) = user.filter(|u| u.len() > 2) {
        out = out.replace(u, "[user]");
    }
    out
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .map(|o| format!("macOS {}", String::from_utf8_lossy(&o.stdout).trim()))
            .unwrap_or_else(|| "macOS".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::consts::OS.to_string()
    }
}

fn canvas_stats() -> Value {
    let s = omniget_core::core::canvas::store::shared();
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let mut floors = 0u64;
    let wss = s.workspaces();
    for w in &wss {
        for f in &w.workspace.floors {
            floors += 1;
            let _ = s.read(&w.workspace.id, &f.id, |d| {
                for n in &d.nodes {
                    let k = serde_json::to_value(&n.kind)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_else(|| "unknown".into());
                    *kinds.entry(k).or_default() += 1;
                }
            });
        }
    }
    let mut agents: BTreeMap<String, u64> = BTreeMap::new();
    let mut alive = 0u64;
    let terms = crate::pty::term().list();
    for t in &terms {
        if t.alive {
            alive += 1;
        }
        *agents
            .entry(format!("{:?}", t.agent_kind).to_lowercase())
            .or_default() += 1;
    }
    let p = omniget_core::core::canvas::envs::providers(&s);
    json!({
        "workspaces": wss.len(),
        "floors": floors,
        "nodes": kinds,
        "terminals": { "open": terms.len(), "alive": alive, "agents": agents },
        "environments": {
            "tmux": p.tmux, "ssh": p.ssh, "docker": p.docker, "sandbox": p.sandbox,
            "custom": p.custom, "wsl": p.wsl, "runtimes": p.runtimes.len(),
        },
    })
}

fn crash_reports() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join("Library/Logs/DiagnosticReports"));
    }
    dirs.push(PathBuf::from("/Library/Logs/DiagnosticReports"));
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86_400);
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.starts_with("omniget") {
                continue;
            }
            if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                if m >= cutoff {
                    found.push((m, e.path()));
                }
            }
        }
    }
    found.sort_by_key(|b| std::cmp::Reverse(b.0));
    found.into_iter().take(5).map(|(_, p)| p).collect()
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn build(app: &AppHandle) -> Result<Value, String> {
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned());
    let user = std::env::var("USER").ok();
    let red = |t: &str| redact(t, home.as_deref(), user.as_deref());
    let settings = crate::storage::config::load_settings(app);
    let version = env!("CARGO_PKG_VERSION");
    let os = os_version();
    let about = json!({
        "app": "OmniGet",
        "version": version,
        "os": os,
        "arch": std::env::consts::ARCH,
        "language": settings.appearance.language,
        "createdAt": chrono::Utc::now().to_rfc3339(),
        "canvas": canvas_stats(),
        "remote": {
            "running": crate::local_bridge_remote::hub().status().running,
            "privateOnly": crate::local_bridge_remote::hub().config().private_only,
            "devices": crate::local_bridge_remote::hub().status().devices,
        },
        "notes": "Metadata, recent app log lines and crash reports only. No code, notes, prompts or terminal output; tokens, e-mails and the home folder are masked.",
    });
    let log: Vec<String> = RING
        .lock()
        .map(|r| r.iter().map(|l| red(l)).collect())
        .unwrap_or_default();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let dir = dirs::download_dir()
        .filter(|d| d.is_dir())
        .or_else(omniget_core::core::paths::app_data_dir)
        .ok_or("no folder to write the report to")?;
    let path = dir.join(format!("omniget-report-{stamp}.zip"));
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut files = vec!["about.json".to_string(), "log.txt".to_string()];
    let put =
        |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, body: &[u8]| -> Result<(), String> {
            zip.start_file(name, opts).map_err(|e| e.to_string())?;
            zip.write_all(body).map_err(|e| e.to_string())
        };
    put(
        &mut zip,
        "about.json",
        red(&serde_json::to_string_pretty(&about).unwrap_or_default()).as_bytes(),
    )?;
    put(&mut zip, "log.txt", log.join("\n").as_bytes())?;
    for c in crash_reports() {
        let Ok(text) = std::fs::read_to_string(&c) else {
            continue;
        };
        let name = format!(
            "crashes/{}",
            c.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        put(&mut zip, &name, red(&text).as_bytes())?;
        files.push(name);
    }
    zip.finish().map_err(|e| e.to_string())?;
    let shown = red(&path.to_string_lossy());
    let subject = format!("OmniGet {version} problem report");
    let body = format!(
        "What happened:\n\nWhat I expected:\n\nSteps:\n1.\n\n---\nOmniGet {version} on {os} ({}).\nPlease attach the report file: {}\n",
        std::env::consts::ARCH,
        path.to_string_lossy()
    );
    Ok(json!({
        "path": path.to_string_lossy(),
        "shownPath": shown,
        "files": files,
        "logLines": log.len(),
        "mailto": format!("mailto:{SUPPORT_EMAIL}?subject={}&body={}", url_encode(&subject), url_encode(&body)),
        "issueUrl": ISSUES_URL,
    }))
}

#[tauri::command]
pub async fn canvas_report_problem(app: AppHandle, reveal: Option<bool>) -> Result<Value, String> {
    let r = tauri::async_runtime::spawn_blocking(move || build(&app))
        .await
        .map_err(|e| e.to_string())??;
    if reveal == Some(true) {
        if let Some(p) = r["path"].as_str() {
            reveal_in_folder(Path::new(p));
        }
    }
    Ok(r)
}

fn reveal_in_folder(p: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", p.display()))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(d) = p.parent() {
        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
    }
}
