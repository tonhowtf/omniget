use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::format::{Frame, LaunchCommand, SensorRow};

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

#[derive(Debug, Clone)]
pub struct Columns {
    n: usize,
    idx: BTreeMap<String, usize>,
}

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
    pub sensors: Vec<SensorRow>,
    pub system: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

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
