//! `OMNIGET_WORLD_BENCH=<scenario>` hook: opens `/world?bench=<scenario>` (or the
//! route in `OMNIGET_WORLD_BENCH_ROUTE`, e.g. `/city`), waits for
//! `world_bench_report`, prints one `[world-bench] {json}` line and exits. Same
//! mould as `OMNIGET_SMOKE_EXIT_MS` in `lib.rs`. Owned by `f6-bench`.
//!
//! Why `println!` and not `tracing`: `scripts/smoke-test.mjs` greps `tracing`
//! output because the smoke assertions are about log lines the app already
//! emits. The bench line is *data*, not a log: it has to survive `RUST_LOG`
//! being unset, a subscriber that adds a timestamp prefix, or a future move of
//! the log sink to a file. So the one machine-read line goes to raw stdout with
//! `println!` + an explicit flush, and `scripts/world-bench.mjs` matches
//! `^\[world-bench\] ` on a line of its own. Everything else here is `tracing`.
//!
//! Process CPU and RSS are sampled here, in Rust, instead of in the webview:
//! the page has no way to see the process it lives in, and `sysinfo` is not a
//! dependency of the app (adding one would need a line in the plan's
//! build-vs-adopt table). `/proc` on Linux and `ps` on macOS cost nothing and
//! are only ever read while the bench hook is armed.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Scenario name; also the value the report echoes back.
pub const ENV_SCENARIO: &str = "OMNIGET_WORLD_BENCH";
/// Route the bench opens: `/world` (default) or `/city` (the 3D city and its
/// lite mode, estudo 99).
pub const ENV_ROUTE: &str = "OMNIGET_WORLD_BENCH_ROUTE";
/// Routes the hook may open; anything else falls back to `/world`.
const ROUTES: &[&str] = &["/world", "/city"];
/// Safety net so a wedged webview fails the job instead of hanging it.
pub const ENV_TIMEOUT_MS: &str = "OMNIGET_WORLD_BENCH_TIMEOUT_MS";
/// Filled by the workflow (`apt-cache policy` / `glxinfo`); copied into the JSON.
pub const ENV_WEBKITGTK: &str = "OMNIGET_WORLD_BENCH_WEBKITGTK";
/// Idem, for the Mesa/renderer string.
pub const ENV_MESA: &str = "OMNIGET_WORLD_BENCH_MESA";

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// The page needs a moment to exist before it can be navigated away from.
const NAVIGATE_DELAY_MS: u64 = 500;
/// How often the watcher checks that the webview is still on `/world`.
const NAVIGATE_RETRY_MS: u64 = 2_000;
/// Give up re-navigating after this many checks. Sized to outlast a whole run
/// (scene + IPC), because the page can be pulled off `/world` at any moment.
const NAVIGATE_ATTEMPTS: u32 = 90;

/// RSS of this process right before `/world` is opened, in KiB. `rss_extra_mb`
/// in the report is `rss_now - this`. 0 means "not measured on this platform".
static BASELINE_RSS_KB: AtomicU64 = AtomicU64::new(0);

/// Whether the *page* thinks it is on `/world`, as last reported by the beacon.
///
/// `WebviewWindow::url()` is not enough: it reports the last committed
/// navigation, and a SPA that calls `history.replaceState` (or SvelteKit's
/// `goto`) moves the page without committing anything. That is not theoretical
/// — the app's own clipboard detection does `goto("/")` when it spots a URL in
/// the clipboard (`src/routes/+layout.svelte`), which silently dragged the
/// bench off the route while `url()` still said `/world`. The page has to be
/// asked.
static PAGE_ON_WORLD: AtomicBool = AtomicBool::new(false);

/// Called by the `page-beacon` op with whatever `location.href` the page has.
pub fn note_page_href(href: &str) {
    PAGE_ON_WORLD.store(href_is_route(href, &route()), Ordering::Relaxed);
}

/// Pure: does this href point at the bench route?
fn href_is_route(href: &str, route: &str) -> bool {
    tauri::Url::parse(href)
        .map(|u| u.path() == route)
        .unwrap_or(false)
}

#[cfg(test)]
fn href_is_world(href: &str) -> bool {
    href_is_route(href, "/world")
}

/// `OMNIGET_WORLD_BENCH_ROUTE`, checked against [`ROUTES`].
pub fn route() -> String {
    route_from(std::env::var(ENV_ROUTE).ok().as_deref())
}

fn route_from(raw: Option<&str>) -> String {
    let r = raw.map(str::trim).unwrap_or("");
    let r = if r.starts_with('/') {
        r.to_string()
    } else {
        format!("/{r}")
    };
    if ROUTES.contains(&r.as_str()) {
        r
    } else {
        "/world".into()
    }
}

/// `OMNIGET_WORLD_BENCH` as a scenario name, or `None` when the hook is off.
pub fn scenario() -> Option<String> {
    scenario_from(std::env::var(ENV_SCENARIO).ok().as_deref())
}

/// Pure half of [`scenario`], so the trimming rule is testable.
fn scenario_from(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Pure half of the timeout knob: anything unparseable falls back to the default
/// rather than disabling the safety net.
fn timeout_ms_from(raw: Option<&str>) -> u64 {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
}

/// `<app base><route>?bench=<scenario>` built from the URL the window is on, so
/// this works the same under `tauri://localhost`, `http://localhost:1420` (dev)
/// and the Linux custom protocol.
fn bench_url(current: &tauri::Url, route: &str, scenario: &str) -> tauri::Url {
    let mut url = current.clone();
    url.set_path(route);
    url.set_query(Some(&format!("bench={scenario}")));
    url.set_fragment(None);
    url
}

/// Arms the hook. No-op — and no thread, no allocation — when the env var is
/// absent, which is every normal run of the app.
pub fn maybe_run(app: &tauri::App) {
    let Some(scenario) = scenario() else {
        return;
    };

    BASELINE_RSS_KB.store(process_rss_kb().unwrap_or(0), Ordering::Relaxed);

    let timeout = timeout_ms_from(std::env::var(ENV_TIMEOUT_MS).ok().as_deref());
    let bench_route = route();
    tracing::info!("[world-bench] scenario={scenario} route={bench_route} timeout={timeout}ms");

    let handle = app.handle().clone();
    let scenario_for_nav = scenario.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(NAVIGATE_DELAY_MS));
        // Navigating once is not enough. The window is still loading its own
        // start page when `setup` runs, and that pending load lands *after* the
        // `navigate` and puts the webview back on `/`. So the bench keeps
        // checking and re-navigates whenever it finds itself off `/world`. It
        // costs nothing once the page is where it should be, and without it the
        // job ends in a bare `{"error":"timeout"}` that explains nothing.
        for attempt in 0..NAVIGATE_ATTEMPTS {
            // Re-asserted every round: another window (or the OS) can take the
            // front while the bench is running, and a backgrounded window stops
            // producing frames.
            show_and_focus(&handle);
            match current_path(&handle) {
                Ok(path) if path == bench_route => {}
                Ok(path) => {
                    tracing::info!(
                        "[world-bench] webview is on {path}, navigating (try {attempt})"
                    );
                    if let Err(e) = navigate_to_bench(&handle, &scenario_for_nav) {
                        emit_line(&serde_json::json!({ "scenario": scenario_for_nav, "error": e }));
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    emit_line(&serde_json::json!({ "scenario": scenario_for_nav, "error": e }));
                    std::process::exit(1);
                }
            }
            if let Some(window) = {
                use tauri::Manager;
                handle.get_webview_window("main")
            } {
                let _ = window.eval(BEACON_JS);
            }
            std::thread::sleep(Duration::from_millis(NAVIGATE_RETRY_MS));
        }
    });

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(timeout));
        // `process::exit` and not `handle.exit`: a webview that stopped
        // answering is exactly the case where a graceful exit may not happen,
        // and a hung job is worse than a failed one.
        emit_line(&serde_json::json!({ "scenario": scenario, "error": "timeout" }));
        std::process::exit(1);
    });
}

fn current_path(handle: &tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let window = handle
        .get_webview_window("main")
        .ok_or_else(|| "ERR_WORLD_BENCH_NO_WINDOW".to_string())?;
    window
        .url()
        .map(|u| u.path().to_string())
        .map_err(|e| format!("ERR_WORLD_BENCH_URL: {e}"))
}

/// JS injected on every retry: it reports what the page actually is, back
/// through `world_bench_report` with `op: "page-beacon"`. Without it, a page
/// that failed to boot is indistinguishable from a page that is still
/// measuring — both look like silence followed by a timeout, which is exactly
/// the hole that produced a `{"error":"timeout"}` with no explanation.
const BEACON_JS: &str = r#"(function () {
  try {
    if (!window.__omnigetBenchBeacon) {
      window.__omnigetBenchBeacon = true;
      window.addEventListener('error', function (e) {
        window.__omnigetBenchError = String((e && e.message) || e);
      });
      window.addEventListener('unhandledrejection', function (e) {
        window.__omnigetBenchError = 'unhandled: ' + String((e && e.reason) || e);
      });
    }
    var text = (document.body && document.body.innerText) || '';
    window.__TAURI_INTERNALS__.invoke('world_bench_report', {
      report: {
        op: 'page-beacon',
        href: location.href,
        ready: document.readyState,
        canvas: !!document.querySelector('canvas'),
        text: text.slice(0, 200),
        error: window.__omnigetBenchError || null
      }
    });
  } catch (e) {
    /* a webview that cannot even run this has bigger problems */
  }
})();"#;

/// Brings the bench window to the front and keeps it there.
///
/// Not cosmetics: WKWebView (and WebKitGTK under a compositor) suspends
/// `requestAnimationFrame` for a window that is occluded or unfocused. A bench
/// whose rAF never fires measures nothing and — before this — reported
/// `frames: 0` while exiting 0. The window has to be genuinely on screen for
/// the frame clock to run at all.
fn show_and_focus(handle: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(window) = handle.get_webview_window("main") else {
        return;
    };
    // On macOS a binary started from a shell is not an *active* application:
    // `set_focus` orders the window front but the process never becomes the
    // foreground app, and WKWebView then throttles both rAF and timers to
    // roughly 1 Hz. Asking for the Regular activation policy is what turns the
    // process into something the window server will bring forward.
    #[cfg(target_os = "macos")]
    if let Err(e) = handle.set_activation_policy(tauri::ActivationPolicy::Regular) {
        tracing::warn!("[world-bench] set_activation_policy failed: {e}");
    }
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_always_on_top(true);
    if let Err(e) = window.set_focus() {
        tracing::warn!("[world-bench] set_focus failed: {e}");
    }
}

fn navigate_to_bench(handle: &tauri::AppHandle, scenario: &str) -> Result<(), String> {
    use tauri::Manager;
    let window = handle
        .get_webview_window("main")
        .ok_or_else(|| "ERR_WORLD_BENCH_NO_WINDOW".to_string())?;
    let current = window
        .url()
        .map_err(|e| format!("ERR_WORLD_BENCH_URL: {e}"))?;
    let target = bench_url(&current, &route(), scenario);
    tracing::info!("[world-bench] navigating to {target}");
    window
        .navigate(target)
        .map_err(|e| format!("ERR_WORLD_BENCH_NAVIGATE: {e}"))
}

/// Adds what only the process knows (scenario, RSS delta, webview/Mesa versions)
/// to the object the front-end built. Pure, so the shape is testable.
pub fn decorate(
    mut report: serde_json::Value,
    scenario: Option<&str>,
    rss_extra_mb: Option<f64>,
    webkitgtk: Option<&str>,
    mesa: Option<&str>,
) -> serde_json::Value {
    if !report.is_object() {
        report = serde_json::json!({ "raw": report });
    }
    let map = report.as_object_mut().expect("object above");
    if let Some(s) = scenario {
        map.insert("scenario".into(), serde_json::Value::String(s.to_string()));
    }
    if let Some(mb) = rss_extra_mb {
        map.insert(
            "rss_extra_mb".into(),
            serde_json::json!((mb * 10.0).round() / 10.0),
        );
    }
    if let Some(v) = webkitgtk.filter(|v| !v.trim().is_empty()) {
        map.insert("webkitgtk".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = mesa.filter(|v| !v.trim().is_empty()) {
        map.insert("mesa".into(), serde_json::Value::String(v.into()));
    }
    report
}

/// Prints the report and quits with 0. Called by the `world_bench_report`
/// command once the front-end has finished.
pub fn finish(app: &tauri::AppHandle, report: serde_json::Value) {
    let rss_extra = process_rss_kb().and_then(|now| {
        let base = BASELINE_RSS_KB.load(Ordering::Relaxed);
        if base == 0 || now < base {
            None
        } else {
            Some((now - base) as f64 / 1024.0)
        }
    });
    let decorated = decorate(
        report,
        scenario().as_deref(),
        rss_extra,
        std::env::var(ENV_WEBKITGTK).ok().as_deref(),
        std::env::var(ENV_MESA).ok().as_deref(),
    );
    emit_line(&decorated);
    app.exit(0);
}

fn emit_line(value: &serde_json::Value) {
    use std::io::Write;
    println!("[world-bench] {value}");
    let _ = std::io::stdout().flush();
}

// ---------------------------------------------------------------------------
// Process CPU and RSS
// ---------------------------------------------------------------------------

/// CPU time consumed by this process **and its direct children**, in seconds.
///
/// Children matter on Linux: WebKitGTK runs the page in a `WebKitWebProcess`
/// forked from us, and that is where the WebGL work happens — measuring only
/// `self` would report a fraction of the truth. On macOS the WKWebView content
/// process is an XPC service reparented to `launchd`, so it is *not* visible
/// here; the macOS number is main-process-only and `docs/bench/world-f6.md`
/// says so. The canonical scenario is Linux, where the number is complete.
pub fn process_cpu_seconds() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        let me = std::process::id();
        let mut total = read_proc_cpu_seconds(me)?;
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                    continue;
                };
                if pid == me {
                    continue;
                }
                if read_proc_ppid(pid) == Some(me) {
                    if let Some(secs) = read_proc_cpu_seconds(pid) {
                        total += secs;
                    }
                }
            }
        }
        Some(total)
    }
    #[cfg(target_os = "macos")]
    {
        // `ps -o time=` gives cumulative CPU time as [[dd-]hh:]mm:ss[.ss].
        let out = std::process::Command::new("ps")
            .args(["-o", "time=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        parse_ps_time(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Parses the `[[dd-]hh:]mm:ss[.ss]` of `ps -o time=` into seconds.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_ps_time(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (days, rest) = match raw.split_once('-') {
        Some((d, rest)) => (d.parse::<f64>().ok()?, rest),
        None => (0.0, raw),
    };
    let mut seconds = 0.0;
    for part in rest.split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(days * 86_400.0 + seconds)
}

#[cfg(target_os = "linux")]
fn stat_fields(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The second field is `(comm)` and may itself contain spaces and
    // parentheses, so split after the *last* ')'.
    let close = raw.rfind(')')?;
    let mut fields = vec![raw[..raw.find('(')?].trim().to_string(), String::new()];
    fields.extend(raw[close + 1..].split_whitespace().map(str::to_string));
    Some(fields)
}

#[cfg(target_os = "linux")]
fn read_proc_ppid(pid: u32) -> Option<u32> {
    // fields[0] = pid, fields[1] = comm placeholder, fields[2] = state, [3] = ppid
    stat_fields(pid)?.get(3)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn read_proc_cpu_seconds(pid: u32) -> Option<f64> {
    let fields = stat_fields(pid)?;
    // utime is field 14 and stime field 15 in proc(5), 1-based.
    let utime: f64 = fields.get(13)?.parse().ok()?;
    let stime: f64 = fields.get(14)?.parse().ok()?;
    // `sysconf(_SC_CLK_TCK)` is 100 on every Linux the app ships to; the app has
    // no libc dependency to ask with, and being wrong would only scale a
    // percentage that is compared against itself.
    Some((utime + stime) / 100.0)
}

/// Resident set size of this process in KiB.
pub fn process_rss_kb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                return rest.split_whitespace().next()?.parse().ok();
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Wall-clock + CPU-clock pair used by the IPC burst to report a percentage.
pub struct CpuWindow {
    started: Instant,
    cpu_seconds: Option<f64>,
}

impl CpuWindow {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            cpu_seconds: process_cpu_seconds(),
        }
    }

    /// Percentage of one core consumed since [`CpuWindow::start`].
    pub fn finish(self) -> Option<f64> {
        let before = self.cpu_seconds?;
        let after = process_cpu_seconds()?;
        let wall = self.started.elapsed().as_secs_f64();
        Some(cpu_percent(after - before, wall))
    }
}

/// Pure: CPU seconds over wall seconds as a percentage of one core, rounded to
/// one decimal. 0 wall time means 0, not infinity.
pub fn cpu_percent(cpu_seconds: f64, wall_seconds: f64) -> f64 {
    if wall_seconds <= 0.0 {
        return 0.0;
    }
    ((cpu_seconds / wall_seconds) * 1000.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_ignores_blank_and_trims() {
        assert_eq!(scenario_from(None), None);
        assert_eq!(scenario_from(Some("   ")), None);
        assert_eq!(scenario_from(Some(" llvmpipe ")), Some("llvmpipe".into()));
    }

    #[test]
    fn timeout_falls_back_instead_of_disabling_itself() {
        assert_eq!(timeout_ms_from(None), DEFAULT_TIMEOUT_MS);
        assert_eq!(timeout_ms_from(Some("abc")), DEFAULT_TIMEOUT_MS);
        assert_eq!(timeout_ms_from(Some("0")), DEFAULT_TIMEOUT_MS);
        assert_eq!(timeout_ms_from(Some(" 4500 ")), 4500);
    }

    #[test]
    fn bench_url_keeps_origin_and_replaces_path_and_query() {
        let base = tauri::Url::parse("tauri://localhost/downloads?x=1#frag").unwrap();
        let got = bench_url(&base, "/world", "nocompositing");
        assert_eq!(got.scheme(), "tauri");
        assert_eq!(got.path(), "/world");
        assert_eq!(got.query(), Some("bench=nocompositing"));
        assert_eq!(got.fragment(), None);

        let dev = tauri::Url::parse("http://localhost:1420/").unwrap();
        assert_eq!(
            bench_url(&dev, "/world", "llvmpipe").as_str(),
            "http://localhost:1420/world?bench=llvmpipe"
        );
        assert_eq!(
            bench_url(&dev, "/city", "llvmpipe").as_str(),
            "http://localhost:1420/city?bench=llvmpipe"
        );
    }

    #[test]
    fn route_is_world_unless_a_known_one_is_asked() {
        assert_eq!(route_from(None), "/world");
        assert_eq!(route_from(Some("city")), "/city");
        assert_eq!(route_from(Some(" /city ")), "/city");
        assert_eq!(route_from(Some("/downloads")), "/world");
        assert!(href_is_route("tauri://localhost/city?bench=x", "/city"));
    }

    #[test]
    fn href_is_world_only_for_the_bench_route() {
        assert!(href_is_world("http://localhost:1420/world?bench=native"));
        assert!(href_is_world("tauri://localhost/world"));
        assert!(!href_is_world("http://localhost:1420/"));
        assert!(!href_is_world("http://localhost:1420/downloads"));
        assert!(!href_is_world("nao e uma url"));
    }

    #[test]
    fn decorate_adds_process_side_fields() {
        let got = decorate(
            serde_json::json!({ "backend": "gl2" }),
            Some("llvmpipe"),
            Some(41.26),
            Some("2.50.4"),
            Some("   "),
        );
        assert_eq!(got["backend"], "gl2");
        assert_eq!(got["scenario"], "llvmpipe");
        assert_eq!(got["rss_extra_mb"], serde_json::json!(41.3));
        assert_eq!(got["webkitgtk"], "2.50.4");
        assert!(got.get("mesa").is_none(), "blank mesa must not be written");
    }

    #[test]
    fn decorate_wraps_a_non_object_report() {
        let got = decorate(serde_json::json!("boom"), Some("native"), None, None, None);
        assert_eq!(got["raw"], "boom");
        assert_eq!(got["scenario"], "native");
    }

    #[test]
    fn cpu_percent_is_per_core_and_safe_at_zero() {
        assert_eq!(cpu_percent(1.0, 10.0), 10.0);
        assert_eq!(cpu_percent(2.5, 1.0), 250.0);
        assert_eq!(cpu_percent(1.0, 0.0), 0.0);
    }

    #[test]
    fn ps_time_parses_the_three_shapes() {
        assert_eq!(parse_ps_time(" 0:02.31 "), Some(2.31));
        assert_eq!(parse_ps_time("1:00:00"), Some(3600.0));
        assert_eq!(parse_ps_time("2-01:00:00"), Some(176_400.0));
        assert_eq!(parse_ps_time(""), None);
    }

    #[test]
    fn process_rss_is_readable_on_the_dev_platforms() {
        // Not an assertion about a value, only that the reader works where the
        // bench runs; elsewhere `None` is the documented answer.
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(process_rss_kb().unwrap_or(0) > 0);
        }
    }
}
