//! Linux: MangoHud (MIT, github.com/flightlessmango/MangoHud), the
//! system package, never bundled. OmniGet writes a config that belongs to
//! the capture only and points `MANGOHUD_CONFIGFILE` at it, which replaces
//! the whole config search (`src/config.cpp`). The person's own
//! `~/.config/MangoHud/MangoHud.conf` is never read or written (goverlay
//! overwrites it; we do not).
//!
//! MangoHud lives inside the game (Vulkan layer or `LD_PRELOAD` for
//! OpenGL), so the game has to be started with the variables: a game that
//! is already running cannot be measured.
//!
//! Log format (`src/logging.cpp`, `writeFileHeaders`): an optional
//! versioning block, `os,cpu,gpu,ram,kernel,driver,cpuscheduler` and its
//! values, then `fps,frametime,cpu_load,cpu_power,gpu_load,cpu_temp,
//! gpu_temp,...,elapsed` with one line per frame when `log_interval=0`
//! (frametime in ms, elapsed in ns since logging started, ram in GiB).
//! MangoHud's own `_summary.csv` uses nearest-rank percentiles; it is
//! ignored and everything is recomputed by [`super::stats`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::format::{Frame, LaunchCommand, SensorRow};

/// The session's own config. `upload_log(s)` never appears.
pub fn config(output_folder: &Path, max_seconds: u64) -> String {
    format!(
        "# Written by OmniGet for one capture. Your own MangoHud.conf is not used or changed.\n\
         output_folder={}\n\
         log_interval=0\n\
         autostart_log=1\n\
         log_duration={}\n\
         fps\n\
         frametime\n\
         frame_timing\n\
         cpu_stats\n\
         gpu_stats\n",
        output_folder.display(),
        max_seconds
    )
}

/// POSIX single-quote for display. These strings are shown to be copied
/// into a terminal or Steam's launch options; OmniGet never runs them.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+=:@%".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// How to start the game: Vulkan (the implicit layer turns on with
/// `MANGOHUD=1`), OpenGL (the `mangohud` wrapper preloads the shim;
/// `--dlsym` for games that load GL by hand) and Steam launch options.
pub fn launch_commands(config_file: &Path) -> Vec<LaunchCommand> {
    let cfg = sh_quote(&config_file.display().to_string());
    vec![
        LaunchCommand {
            kind: "vulkan".into(),
            command: format!("MANGOHUD=1 MANGOHUD_CONFIGFILE={cfg} <game>"),
        },
        LaunchCommand {
            kind: "opengl".into(),
            command: format!("MANGOHUD_CONFIGFILE={cfg} mangohud --dlsym <game>"),
        },
        LaunchCommand {
            kind: "steam".into(),
            command: format!("MANGOHUD_CONFIGFILE={cfg} mangohud %command%"),
        },
    ]
}

/// The newest MangoHud log in `dir` (not its `_summary.csv`).
pub fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.ends_with(".csv")
                && !n.ends_with("_summary.csv")
                && !matches!(n, "frames.csv" | "sensors.csv")
        })
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Column positions of the frame table.
#[derive(Debug, Clone)]
pub struct Columns {
    n: usize,
    idx: BTreeMap<String, usize>,
}

/// One log line.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub elapsed_ms: f64,
    pub frametime_ms: f64,
    pub sensors: SensorRow,
}

impl Columns {
    pub fn from_header(line: &str) -> Option<Columns> {
        let names: Vec<String> = line
            .trim()
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .collect();
        if names.first().map(String::as_str) != Some("fps")
            || !names.iter().any(|n| n == "frametime")
        {
            return None;
        }
        Some(Columns {
            n: names.len(),
            idx: names.into_iter().enumerate().map(|(i, n)| (n, i)).collect(),
        })
    }

    pub fn row(&self, line: &str) -> Option<Row> {
        let c: Vec<&str> = line.trim().split(',').collect();
        if c.len() < self.n {
            return None;
        }
        let f = |k: &str| -> Option<f64> {
            let v = c.get(*self.idx.get(k)?)?.trim().parse::<f64>().ok()?;
            v.is_finite().then_some(v)
        };
        let frametime = f("frametime").filter(|x| *x > 0.0)?;
        let elapsed_ms = f("elapsed").map(|ns| ns / 1_000_000.0).unwrap_or(0.0);
        // MangoHud writes 0 for a sensor it cannot read.
        let s = |k: &str| f(k).filter(|x| *x > 0.0);
        Some(Row {
            elapsed_ms,
            frametime_ms: frametime,
            sensors: SensorRow {
                t_ms: elapsed_ms,
                cpu_load: f("cpu_load"),
                cpu_temp: s("cpu_temp"),
                gpu_load: f("gpu_load"),
                gpu_temp: s("gpu_temp"),
                power_w: match (s("cpu_power"), s("gpu_power")) {
                    (None, None) => None,
                    (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
                },
                ram_used_mb: s("ram_used").map(|g| super::stats::round(g * 1073.741824, 1)),
            },
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub frames: Vec<Frame>,
    /// MangoHud's own sensor columns averaged per second.
    pub sensors: Vec<SensorRow>,
    /// `os`, `cpu`, `gpu`, `ram`, `kernel`, `driver`, `cpuscheduler`.
    pub system: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

/// Whole log → frames and 1 Hz sensors.
pub fn parse_csv(text: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut lines = text.lines().peekable();
    let mut cols = None;
    while let Some(l) = lines.next() {
        if l.trim_start().starts_with("os,") {
            if let Some(values) = lines.peek() {
                let keys: Vec<&str> = l.split(',').collect();
                let vals: Vec<&str> = values.split(',').collect();
                if vals.len() == keys.len() {
                    out.system = keys
                        .iter()
                        .zip(vals)
                        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                        .collect();
                    lines.next();
                }
            }
            continue;
        }
        if let Some(c) = Columns::from_header(l) {
            cols = Some(c);
            break;
        }
    }
    let Some(cols) = cols else {
        out.warnings.push("unknown_csv_header".into());
        return out;
    };
    let rows: Vec<Row> = lines.filter_map(|l| cols.row(l)).collect();
    if rows.is_empty() {
        return out;
    }
    let t0 = rows[0].elapsed_ms - rows[0].frametime_ms;
    let sampled = rows.len() >= 20 && {
        // log_interval > 0 samples on a clock: the spacing of `elapsed`
        // stops matching the frametimes.
        let span = rows.last().map(|r| r.elapsed_ms).unwrap_or(0.0) - rows[0].elapsed_ms;
        let sum: f64 = rows[1..].iter().map(|r| r.frametime_ms).sum();
        span > 0.0 && (sum / span) < 0.5
    };
    if sampled {
        out.warnings.push("sampled_log".into());
    }
    out.frames = rows
        .iter()
        .map(|r| Frame {
            t_ms: super::stats::round((r.elapsed_ms - t0).max(0.0), 3),
            frametime_ms: r.frametime_ms,
            ..Default::default()
        })
        .collect();
    let mut buckets: BTreeMap<u64, Vec<&SensorRow>> = BTreeMap::new();
    for r in &rows {
        buckets
            .entry(((r.elapsed_ms - t0).max(0.0) / 1000.0) as u64)
            .or_default()
            .push(&r.sensors);
    }
    let mean = |v: &[&SensorRow], f: fn(&SensorRow) -> Option<f64>| {
        let xs: Vec<f64> = v.iter().filter_map(|r| f(r)).collect();
        (!xs.is_empty()).then(|| super::stats::round(xs.iter().sum::<f64>() / xs.len() as f64, 1))
    };
    out.sensors = buckets
        .into_iter()
        .map(|(sec, v)| SensorRow {
            t_ms: sec as f64 * 1000.0,
            cpu_load: mean(&v, |r| r.cpu_load),
            cpu_temp: mean(&v, |r| r.cpu_temp),
            gpu_load: mean(&v, |r| r.gpu_load),
            gpu_temp: mean(&v, |r| r.gpu_temp),
            power_w: mean(&v, |r| r.power_w),
            ram_used_mb: mean(&v, |r| r.ram_used_mb),
        })
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = include_str!("../../../../tests/washmybelly_fixtures/perf/mangohud.csv");
    const LOG_V: &str =
        include_str!("../../../../tests/washmybelly_fixtures/perf/mangohud_versioned.csv");

    #[test]
    fn reads_system_frames_and_sensors() {
        let p = parse_csv(LOG);
        assert_eq!(
            p.system.get("gpu").map(String::as_str),
            Some("AMD Radeon RX 6700 XT")
        );
        assert_eq!(p.frames.len(), 8);
        assert_eq!(p.frames[0].frametime_ms, 6.94);
        // t of the first frame is its own frametime after the start.
        assert!((p.frames[0].t_ms - 6.94).abs() < 1e-6);
        assert!(p.frames.iter().all(|f| f.display_ms.is_none()));
        assert_eq!(p.sensors.len(), 2, "two seconds of log");
        assert_eq!(p.sensors[0].gpu_temp, Some(64.0));
        assert_eq!(p.sensors[0].cpu_temp, None, "0 means unreadable");
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn versioned_log_is_read_the_same_way() {
        let p = parse_csv(LOG_V);
        assert_eq!(p.frames.len(), 3);
        assert_eq!(
            p.system.get("kernel").map(String::as_str),
            Some("6.10.3-arch1-1")
        );
    }

    #[test]
    fn garbage_is_reported() {
        assert_eq!(
            parse_csv("hello\nworld\n").warnings,
            vec!["unknown_csv_header"]
        );
    }

    #[test]
    fn config_is_the_session_only_and_never_uploads() {
        let c = config(Path::new("/home/a/.local/share/omniget/perf/x"), 90);
        assert!(c.contains("output_folder=/home/a/.local/share/omniget/perf/x\n"));
        assert!(
            c.contains("log_interval=0\n")
                && c.contains("log_duration=90\n")
                && c.contains("autostart_log=1\n")
        );
        assert!(!c.contains("upload"));
    }

    #[test]
    fn launch_commands_quote_the_config_path() {
        let cmds = launch_commands(Path::new("/home/a b/it's/MangoHud.conf"));
        assert_eq!(
            cmds[0].command,
            "MANGOHUD=1 MANGOHUD_CONFIGFILE='/home/a b/it'\\''s/MangoHud.conf' <game>"
        );
        assert!(cmds[2].command.ends_with("mangohud %command%"));
        assert_eq!(sh_quote("/plain/path.conf"), "/plain/path.conf");
    }
}
