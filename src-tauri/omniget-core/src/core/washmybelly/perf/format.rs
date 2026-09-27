//! `omniget-frames/1`: one folder per capture in `<data>/washmybelly/perf/<id>/`
//! with `session.json`, `frames.csv`, `sensors.csv` and `summary.json`.
//! Every backend (PresentMon, MangoHud, Metal HUD) is converted to these
//! files, and the summary is always computed by [`super::stats`].

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::stats::{self, Event, Stats};

pub const FORMAT: &str = "omniget-frames/1";
pub const FRAMES_HEADER: &str =
    "t_ms,frametime_ms,display_ms,until_displayed_ms,in_present_ms,cpu_busy_ms,gpu_busy_ms,dropped,frame_type,latency_ms";
pub const SENSORS_HEADER: &str = "t_ms,cpu_load,cpu_temp,gpu_load,gpu_temp,power_w,ram_used_mb";

/// One presented frame. `frametime_ms` is present to present; `display_ms`
/// is the time between two frames reaching the screen (PresentMon only).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub t_ms: f64,
    pub frametime_ms: f64,
    pub display_ms: Option<f64>,
    pub until_displayed_ms: Option<f64>,
    pub in_present_ms: Option<f64>,
    pub cpu_busy_ms: Option<f64>,
    pub gpu_busy_ms: Option<f64>,
    pub dropped: bool,
    pub frame_type: Option<String>,
    pub latency_ms: Option<f64>,
}

/// One 1 Hz sample of the machine during the capture.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorRow {
    pub t_ms: f64,
    pub cpu_load: Option<f64>,
    pub cpu_temp: Option<f64>,
    pub gpu_load: Option<f64>,
    pub gpu_temp: Option<f64>,
    pub power_w: Option<f64>,
    pub ram_used_mb: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub name: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Machine {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub cpu: String,
    pub gpu: String,
    pub ram_gb: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceTool {
    pub tool: String,
    pub version: String,
    pub args: Vec<String>,
}

/// How to start the game so the capture sees it (Linux MangoHud, Mac
/// Metal HUD). Display text: OmniGet never runs these strings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LaunchCommand {
    /// `vulkan`, `opengl`, `steam`, `metal`.
    pub kind: String,
    pub command: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub format: String,
    pub id: String,
    pub label: String,
    /// `recording`, `waiting_launch`, `done`, `failed`.
    pub status: String,
    /// `presentmon`, `mangohud`, `metal_hud`, `telemetry`.
    pub method: String,
    /// `per_frame` or `none` (telemetry only).
    pub frame_mode: String,
    pub target: Target,
    pub machine: Machine,
    pub source: SourceTool,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub max_seconds: u64,
    pub frames: usize,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Why there are no frames (a code the screen translates).
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub launch: Vec<LaunchCommand>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SensorSummary {
    pub samples: usize,
    pub cpu_load_avg: Option<f64>,
    pub gpu_load_avg: Option<f64>,
    pub cpu_temp_max: Option<f64>,
    pub gpu_temp_max: Option<f64>,
    pub power_avg_w: Option<f64>,
}

/// What the sensors said around the bad frames.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Correlation {
    /// Codes: `gpu_bound`, `cpu_busy`, `hot`, `stutters_when_hot`,
    /// `stutters_when_gpu_full`, `no_sensors`, `no_frames`.
    pub findings: Vec<String>,
    pub stutters_checked: usize,
    pub stutters_hot: usize,
    pub stutters_gpu_full: usize,
    pub stutters_cpu_full: usize,
    /// Share of the capture with the GPU at 95% or more.
    pub gpu_full_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    pub format: String,
    pub session_id: String,
    pub label: String,
    pub method: String,
    /// `present` (present to present) or `display` (arrival on screen,
    /// used when frame generation is on).
    pub source: String,
    #[serde(flatten)]
    pub stats: Stats,
    pub stutter_events: Vec<Event>,
    pub freeze_events: Vec<Event>,
    /// The other source, when the capture has both.
    pub display_stats: Option<Stats>,
    pub sensors: SensorSummary,
    pub correlation: Correlation,
    /// Half-to-half spread of each metric: the noise floor for a compare.
    pub noise: BTreeMap<String, Option<f64>>,
    pub warnings: Vec<String>,
    pub reason: Option<String>,
    pub formulas: String,
}

pub const FORMULAS: &str = "CapFrameX (MIT): average FPS = n*1000/sum(ft); percentiles R-8; 1% low average = 1000/mean(ft >= Q(ft,0.99)); 1% low integral = slowest frames adding up to 1% of time; stutter = ft > 2.5 x moving average (window round(sqrt(mean ft)*10)); freeze = ft >= 100 ms";

fn num(v: Option<f64>) -> String {
    match v {
        Some(x) if x.is_finite() => {
            let s = format!("{:.4}", x);
            let s = s.trim_end_matches('0').trim_end_matches('.');
            if s.is_empty() || s == "-" {
                "0".into()
            } else {
                s.to_string()
            }
        }
        _ => String::new(),
    }
}

fn clean_text(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, ',' | '\n' | '\r' | '"')).collect()
}

pub fn frames_csv(frames: &[Frame]) -> String {
    let mut out = String::with_capacity(frames.len() * 48 + 128);
    out.push_str(FRAMES_HEADER);
    out.push('\n');
    for f in frames {
        let cols = [
            num(Some(f.t_ms)),
            num(Some(f.frametime_ms)),
            num(f.display_ms),
            num(f.until_displayed_ms),
            num(f.in_present_ms),
            num(f.cpu_busy_ms),
            num(f.gpu_busy_ms),
            if f.dropped { "1".into() } else { "0".into() },
            f.frame_type.as_deref().map(clean_text).unwrap_or_default(),
            num(f.latency_ms),
        ];
        out.push_str(&cols.join(","));
        out.push('\n');
    }
    out
}

fn opt(s: Option<&str>) -> Option<f64> {
    s.and_then(|x| x.trim().parse::<f64>().ok()).filter(|x| x.is_finite())
}

pub fn parse_frames_csv(text: &str) -> Vec<Frame> {
    let mut lines = text.lines();
    let Some(head) = lines.next() else { return vec![] };
    let idx: BTreeMap<&str, usize> = head.split(',').enumerate().map(|(i, h)| (h.trim(), i)).collect();
    lines
        .filter_map(|l| {
            let c: Vec<&str> = l.split(',').collect();
            let get = |k: &str| idx.get(k).and_then(|&i| c.get(i).copied());
            Some(Frame {
                t_ms: opt(get("t_ms"))?,
                frametime_ms: opt(get("frametime_ms"))?,
                display_ms: opt(get("display_ms")),
                until_displayed_ms: opt(get("until_displayed_ms")),
                in_present_ms: opt(get("in_present_ms")),
                cpu_busy_ms: opt(get("cpu_busy_ms")),
                gpu_busy_ms: opt(get("gpu_busy_ms")),
                dropped: get("dropped").map(str::trim) == Some("1"),
                frame_type: get("frame_type").map(str::trim).filter(|s| !s.is_empty()).map(String::from),
                latency_ms: opt(get("latency_ms")),
            })
        })
        .collect()
}

pub fn sensor_line(r: &SensorRow) -> String {
    [
        num(Some(r.t_ms)),
        num(r.cpu_load),
        num(r.cpu_temp),
        num(r.gpu_load),
        num(r.gpu_temp),
        num(r.power_w),
        num(r.ram_used_mb),
    ]
    .join(",")
}

pub fn sensors_csv(rows: &[SensorRow]) -> String {
    let mut out = String::from(SENSORS_HEADER);
    out.push('\n');
    for r in rows {
        out.push_str(&sensor_line(r));
        out.push('\n');
    }
    out
}

pub fn parse_sensors_csv(text: &str) -> Vec<SensorRow> {
    let mut lines = text.lines();
    let Some(head) = lines.next() else { return vec![] };
    let idx: BTreeMap<&str, usize> = head.split(',').enumerate().map(|(i, h)| (h.trim(), i)).collect();
    lines
        .filter_map(|l| {
            let c: Vec<&str> = l.split(',').collect();
            let get = |k: &str| idx.get(k).and_then(|&i| c.get(i).copied());
            Some(SensorRow {
                t_ms: opt(get("t_ms"))?,
                cpu_load: opt(get("cpu_load")),
                cpu_temp: opt(get("cpu_temp")),
                gpu_load: opt(get("gpu_load")),
                gpu_temp: opt(get("gpu_temp")),
                power_w: opt(get("power_w")),
                ram_used_mb: opt(get("ram_used_mb")),
            })
        })
        .collect()
}

/// A sensor row from one `monitor::live_snapshot` tick, `t_ms` after the
/// start of the capture.
pub fn sensor_from_snapshot(v: &serde_json::Value, t_ms: f64) -> SensorRow {
    let f = |p: &str| v.pointer(p).and_then(serde_json::Value::as_f64);
    SensorRow {
        t_ms,
        cpu_load: f("/cpu/total"),
        cpu_temp: f("/temp_c"),
        gpu_load: f("/gpu_percent"),
        gpu_temp: f("/gpu_temp_c"),
        power_w: f("/power_w"),
        ram_used_mb: f("/memory/used").map(|b| stats::round(b / 1_000_000.0, 1)),
    }
}

/// A frame the app rendered itself (as opposed to one generated by DLSS-FG,
/// FSR-FG or the driver).
fn is_app_frame(t: &Option<String>) -> bool {
    match t.as_deref() {
        None => true,
        Some(s) => {
            let s = s.to_ascii_lowercase();
            s.is_empty() || s == "application" || s == "app" || s == "notset" || s == "na"
        }
    }
}

/// The sequence the summary uses: arrival on screen when frame generation
/// is detected and display times exist (present times alternate between
/// real and generated frames and fake thousands of stutters), else present
/// to present. Returns (source, frametimes, times) plus the other sequence
/// when both exist.
#[allow(clippy::type_complexity)]
pub fn pick_source(frames: &[Frame]) -> ((&'static str, Vec<f64>, Vec<f64>), Option<(Vec<f64>, Vec<f64>)>, bool) {
    let present: (Vec<f64>, Vec<f64>) = (
        frames.iter().map(|f| f.frametime_ms).collect(),
        frames.iter().map(|f| f.t_ms).collect(),
    );
    let shown: Vec<&Frame> = frames
        .iter()
        .filter(|f| !f.dropped && f.display_ms.is_some_and(|d| d > 0.0))
        .collect();
    let display = (shown.len() * 2 >= frames.len() && !shown.is_empty()).then(|| {
        (
            shown.iter().filter_map(|f| f.display_ms).collect::<Vec<f64>>(),
            shown.iter().map(|f| f.t_ms).collect::<Vec<f64>>(),
        )
    });
    let frame_gen = frames.iter().any(|f| !is_app_frame(&f.frame_type));
    match display {
        Some(d) if frame_gen => (("display", d.0, d.1), Some(present), true),
        Some(d) => (("present", present.0, present.1), Some(d), false),
        None => (("present", present.0, present.1), None, frame_gen),
    }
}

fn avg(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (s, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| stats::round(s / n as f64, 1))
}

fn max(v: impl Iterator<Item = f64>) -> Option<f64> {
    v.fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
}

pub fn sensor_summary(rows: &[SensorRow]) -> SensorSummary {
    SensorSummary {
        samples: rows.len(),
        cpu_load_avg: avg(rows.iter().filter_map(|r| r.cpu_load)),
        gpu_load_avg: avg(rows.iter().filter_map(|r| r.gpu_load)),
        cpu_temp_max: max(rows.iter().filter_map(|r| r.cpu_temp)),
        gpu_temp_max: max(rows.iter().filter_map(|r| r.gpu_temp)),
        power_avg_w: avg(rows.iter().filter_map(|r| r.power_w)),
    }
}

pub const HOT_CPU_C: f64 = 90.0;
pub const HOT_GPU_C: f64 = 85.0;
pub const FULL_GPU: f64 = 95.0;
pub const FULL_CPU: f64 = 90.0;

/// Sensor state at each stutter (the sample within one second), and what
/// stands out over the whole capture.
pub fn correlate(stutters: &[Event], rows: &[SensorRow], frames: usize) -> Correlation {
    let mut c = Correlation::default();
    if frames == 0 {
        c.findings.push("no_frames".into());
    }
    if rows.is_empty() {
        c.findings.push("no_sensors".into());
        return c;
    }
    let near = |t: f64| {
        rows.iter()
            .filter(|r| (r.t_ms - t).abs() <= 1000.0)
            .min_by(|a, b| (a.t_ms - t).abs().total_cmp(&(b.t_ms - t).abs()))
    };
    for e in stutters {
        let Some(r) = near(e.t_ms) else { continue };
        c.stutters_checked += 1;
        if r.cpu_temp.is_some_and(|x| x >= HOT_CPU_C) || r.gpu_temp.is_some_and(|x| x >= HOT_GPU_C) {
            c.stutters_hot += 1;
        }
        if r.gpu_load.is_some_and(|x| x >= FULL_GPU) {
            c.stutters_gpu_full += 1;
        }
        if r.cpu_load.is_some_and(|x| x >= FULL_CPU) {
            c.stutters_cpu_full += 1;
        }
    }
    let gpu: Vec<f64> = rows.iter().filter_map(|r| r.gpu_load).collect();
    if !gpu.is_empty() {
        let full = gpu.iter().filter(|x| **x >= FULL_GPU).count();
        c.gpu_full_pct = Some(stats::round(100.0 * full as f64 / gpu.len() as f64, 1));
    }
    if c.gpu_full_pct.is_some_and(|p| p >= 60.0) {
        c.findings.push("gpu_bound".into());
    }
    let cpu = avg(rows.iter().filter_map(|r| r.cpu_load));
    if cpu.is_some_and(|x| x >= 85.0) {
        c.findings.push("cpu_busy".into());
    }
    let hot = rows
        .iter()
        .any(|r| r.cpu_temp.is_some_and(|x| x >= HOT_CPU_C + 5.0) || r.gpu_temp.is_some_and(|x| x >= HOT_GPU_C + 5.0));
    if hot {
        c.findings.push("hot".into());
    }
    if c.stutters_checked >= 3 && c.stutters_hot * 2 >= c.stutters_checked {
        c.findings.push("stutters_when_hot".into());
    }
    if c.stutters_checked >= 3 && c.stutters_gpu_full * 2 >= c.stutters_checked {
        c.findings.push("stutters_when_gpu_full".into());
    }
    c
}

/// The whole summary of a session from its frames and sensor rows.
pub fn summarize(session: &Session, frames: &[Frame], rows: &[SensorRow]) -> Summary {
    let ((source, ft, t), other, frame_gen) = pick_source(frames);
    let (st, stutters, freezes) = stats::compute(&ft, &t);
    let display_stats = other.map(|(o, ot)| stats::compute(&o, &ot).0);
    let mut warnings = session.warnings.clone();
    if frame_gen && !warnings.iter().any(|w| w == "frame_generation") {
        warnings.push("frame_generation".into());
    }
    if frame_gen && source == "present" {
        warnings.push("frame_generation_without_display_times".into());
    }
    let correlation = correlate(&stutters, rows, st.frames);
    let noise = stats::half_spread(&ft).into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Summary {
        format: FORMAT.into(),
        session_id: session.id.clone(),
        label: session.label.clone(),
        method: session.method.clone(),
        source: source.into(),
        stutter_events: stutters.into_iter().take(500).collect(),
        freeze_events: freezes.into_iter().take(500).collect(),
        stats: st,
        display_stats,
        sensors: sensor_summary(rows),
        correlation,
        noise,
        warnings,
        reason: session.reason.clone(),
        formulas: FORMULAS.into(),
    }
}

pub fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t: f64, ft: f64) -> Frame {
        Frame { t_ms: t, frametime_ms: ft, ..Default::default() }
    }

    #[test]
    fn frames_round_trip() {
        let frames = vec![
            Frame {
                t_ms: 0.0,
                frametime_ms: 16.6667,
                display_ms: Some(16.5),
                gpu_busy_ms: Some(9.25),
                frame_type: Some("Application".into()),
                ..Default::default()
            },
            Frame { t_ms: 16.6667, frametime_ms: 33.0, dropped: true, ..Default::default() },
        ];
        let text = frames_csv(&frames);
        assert!(text.starts_with(FRAMES_HEADER));
        assert_eq!(parse_frames_csv(&text), frames);
    }

    #[test]
    fn sensors_round_trip_and_snapshot() {
        let v = serde_json::json!({ "cpu": { "total": 41.5 }, "temp_c": 72.0, "gpu_percent": 97.0, "gpu_temp_c": null, "memory": { "used": 8_000_000_000u64 } });
        let r = sensor_from_snapshot(&v, 1000.0);
        assert_eq!(r.cpu_load, Some(41.5));
        assert_eq!(r.gpu_temp, None);
        assert_eq!(r.ram_used_mb, Some(8000.0));
        assert_eq!(parse_sensors_csv(&sensors_csv(std::slice::from_ref(&r))), vec![r]);
    }

    #[test]
    fn frame_generation_switches_to_display_times() {
        // Present times alternate 2/14 ms (real + generated); display is a steady 8.
        let frames: Vec<Frame> = (0..200)
            .map(|i| Frame {
                t_ms: i as f64 * 8.0,
                frametime_ms: if i % 2 == 0 { 2.0 } else { 14.0 },
                display_ms: Some(8.0),
                frame_type: Some(if i % 2 == 0 { "Application" } else { "AMD_AFMF" }.into()),
                ..Default::default()
            })
            .collect();
        let ((src, ft, _), other, fg) = pick_source(&frames);
        assert_eq!(src, "display");
        assert!(fg);
        assert!(ft.iter().all(|x| *x == 8.0));
        assert!(other.is_some());
        let plain: Vec<Frame> = (0..10).map(|i| frame(i as f64, 16.0)).collect();
        assert_eq!(pick_source(&plain).0 .0, "present");
    }

    #[test]
    fn correlation_finds_hot_stutters() {
        let stutters: Vec<Event> = [1000.0, 2000.0, 3000.0].iter().map(|t| Event { t_ms: *t, frametime_ms: 60.0 }).collect();
        let rows: Vec<SensorRow> = (0..5)
            .map(|i| SensorRow {
                t_ms: i as f64 * 1000.0,
                cpu_temp: Some(96.0),
                gpu_load: Some(99.0),
                cpu_load: Some(40.0),
                ..Default::default()
            })
            .collect();
        let c = correlate(&stutters, &rows, 100);
        assert_eq!(c.stutters_hot, 3);
        assert!(c.findings.contains(&"stutters_when_hot".to_string()));
        assert!(c.findings.contains(&"gpu_bound".to_string()));
        assert!(c.findings.contains(&"hot".to_string()));
        assert!(!c.findings.contains(&"cpu_busy".to_string()));
        assert_eq!(correlate(&stutters, &[], 100).findings, vec!["no_sensors".to_string()]);
    }

    #[test]
    fn summary_serialises_the_tool_fields_at_top_level() {
        let s = Session { id: "x".into(), method: "presentmon".into(), ..Default::default() };
        let frames: Vec<Frame> = (0..300).map(|i| frame(i as f64 * 10.0, 10.0)).collect();
        let sum = summarize(&s, &frames, &[]);
        let v = serde_json::to_value(&sum).unwrap();
        assert_eq!(v["session_id"], "x");
        assert_eq!(v["fps_avg"], 100.0);
        assert_eq!(v["low_1"], 100.0);
        assert_eq!(v["low_01"], 100.0);
        assert_eq!(v["stutters"], 0);
    }
}
