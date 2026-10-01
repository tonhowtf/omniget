//! Windows: Intel's PresentMon (MIT, github.com/GameTechDev/PresentMon),
//! downloaded on demand like yt-dlp, with the version and SHA-256 fixed
//! here. It reads the frame events Windows already emits (ETW) and never
//! injects into the game, so it is safe with anti-cheats such as Vanguard.
//!
//! Its CSV changes with the version and the flags (v1, v2 and v3 metric
//! names), so it is always read by column name, as CapFrameX does
//! (`PresentMonCaptureService.cs`).
//!
//! Access: PresentMon needs an administrator or a member of the Performance
//! Log Users group (SID `S-1-5-32-559`, `PresentMon/Privilege.cpp`). OmniGet
//! never runs as administrator; the screen offers a one-time setup that
//! joins the group (by SID, since the name changes with the language).

use std::collections::HashMap;

use super::format::Frame;

/// Stable release 2.6.0 (2026-09-21). Hash checked against the GitHub
/// release `digest` and a local download on 2026-09-27.
pub const VERSION: &str = "2.6.0";
pub const ASSET: &str = "PresentMon-2.6.0-x64.exe";
pub const URL: &str =
    "https://github.com/GameTechDev/PresentMon/releases/download/v2.6.0/PresentMon-2.6.0-x64.exe";
pub const SHA256: &str = "b2a706bc6ad475749e3b7e3409263aa1e6906d45bdcf993f6dbc0f660188f1af";
pub const LICENSE_FILE: &str = "PresentMon-LICENSE.txt";
/// `LICENSE.txt` of the PresentMon repository, shipped next to the binary.
pub const LICENSE: &str = "PresentMon is distributed by Intel Corporation under the MIT License\nhttps://github.com/GameTechDev/PresentMon/blob/v2.6.0/LICENSE.txt\n\nCopyright (C) 2017-2024 Intel Corporation\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files (the \"Software\"),\nto deal in the Software without restriction, including without limitation\nthe rights to use, copy, modify, merge, publish, distribute, sublicense,\nand/or sell copies of the Software, and to permit persons to whom the\nSoftware is furnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included\nin all copies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS\nOR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\nFITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL\nTHE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING\nFROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER\nDEALINGS IN THE SOFTWARE.\n";

pub const PERF_LOG_USERS_SID: &str = "S-1-5-32-559";

/// Column names across PresentMon 1.x, 2.x (`--v2_metrics`) and 3.x
/// (default since 2.4), first match wins. Compared case-insensitively.
const T_SECONDS: &[&str] = &["TimeInSeconds", "CPUStartTimeInSeconds"];
const T_MS: &[&str] = &["TimeInMs", "CPUStartTimeInMs"];
const FRAMETIME: &[&str] = &["MsBetweenPresents", "FrameTime", "MsBetweenAppStart"];
const DISPLAY: &[&str] = &["MsBetweenDisplayChange", "DisplayedTime"];
const UNTIL_DISPLAYED: &[&str] = &["MsUntilDisplayed", "DisplayLatency"];
const IN_PRESENT: &[&str] = &["MsInPresentAPI"];
const CPU_BUSY: &[&str] = &["MsCPUBusy", "CPUBusy"];
const GPU_BUSY: &[&str] = &["MsGPUBusy", "GPUBusy", "msGPUActive"];
const LATENCY: &[&str] = &[
    "MsPCLatency",
    "MsAllInputToPhotonLatency",
    "AllInputToPhotonLatency",
];

/// Column positions read from the header line.
#[derive(Debug, Clone)]
pub struct Columns {
    n: usize,
    process_id: Option<usize>,
    swap_chain: Option<usize>,
    application: Option<usize>,
    t_seconds: Option<usize>,
    t_ms: Option<usize>,
    frametime: usize,
    display: Option<usize>,
    until_displayed: Option<usize>,
    in_present: Option<usize>,
    cpu_busy: Option<usize>,
    gpu_busy: Option<usize>,
    dropped: Option<usize>,
    frame_type: Option<usize>,
    latency: Option<usize>,
}

/// One CSV row with the fields that identify where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub pid: Option<u32>,
    pub swap_chain: Option<String>,
    pub application: Option<String>,
    /// Absolute time of the row in ms, when the CSV has one.
    pub time_ms: Option<f64>,
    pub frame: Frame,
}

impl Columns {
    pub fn from_header(line: &str) -> Option<Columns> {
        let names: Vec<String> = line
            .trim()
            .trim_start_matches('\u{feff}')
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .collect();
        let find = |cands: &[&str]| {
            cands
                .iter()
                .find_map(|c| names.iter().position(|n| n == &c.to_ascii_lowercase()))
        };
        Some(Columns {
            n: names.len(),
            process_id: find(&["ProcessID"]),
            swap_chain: find(&["SwapChainAddress"]),
            application: find(&["Application"]),
            t_seconds: find(T_SECONDS),
            t_ms: find(T_MS),
            frametime: find(FRAMETIME)?,
            display: find(DISPLAY),
            until_displayed: find(UNTIL_DISPLAYED),
            in_present: find(IN_PRESENT),
            cpu_busy: find(CPU_BUSY),
            gpu_busy: find(GPU_BUSY),
            dropped: find(&["Dropped"]),
            frame_type: find(&["FrameType"]),
            latency: find(LATENCY),
        })
    }

    pub fn has_display(&self) -> bool {
        self.display.is_some()
    }

    /// Parse one data row. An application name with a comma splits into
    /// extra fields at the front; they are folded back into the first.
    pub fn row(&self, line: &str) -> Option<Row> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return None;
        }
        let mut c: Vec<&str> = line.split(',').collect();
        let extra = c.len().checked_sub(self.n)?;
        let app = if extra > 0 && self.application == Some(0) {
            let name = c[..=extra].join(",");
            c.drain(..extra);
            Some(name)
        } else {
            self.application
                .and_then(|i| c.get(i))
                .map(|s| s.to_string())
        };
        let num = |i: Option<usize>| -> Option<f64> {
            let s = c.get(i?)?.trim();
            s.parse::<f64>().ok().filter(|x| x.is_finite())
        };
        let frametime = num(Some(self.frametime))?;
        let display = num(self.display);
        let dropped = match self.dropped.and_then(|i| c.get(i)) {
            Some(s) => s.trim() == "1",
            // v2/v3: a frame that never reached the screen has "NA".
            None => self.display.is_some() && display.is_none(),
        };
        let time_ms = num(self.t_ms).or_else(|| num(self.t_seconds).map(|s| s * 1000.0));
        Some(Row {
            pid: self
                .process_id
                .and_then(|i| c.get(i))
                .and_then(|s| s.trim().parse().ok()),
            swap_chain: self
                .swap_chain
                .and_then(|i| c.get(i))
                .map(|s| s.trim().to_string()),
            application: app,
            time_ms,
            frame: Frame {
                t_ms: 0.0,
                frametime_ms: frametime,
                display_ms: display.filter(|d| *d > 0.0),
                until_displayed_ms: num(self.until_displayed),
                in_present_ms: num(self.in_present),
                cpu_busy_ms: num(self.cpu_busy),
                gpu_busy_ms: num(self.gpu_busy),
                dropped,
                frame_type: self
                    .frame_type
                    .and_then(|i| c.get(i))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s != "NA"),
                latency_ms: num(self.latency),
            },
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub frames: Vec<Frame>,
    pub application: Option<String>,
    pub warnings: Vec<String>,
    pub has_display: bool,
}

/// Whole CSV → frames of one swap chain. With several processes or swap
/// chains (a launcher next to the game), the one with the most frames of
/// `pid` (or overall) wins. `t_ms` starts at 0.
pub fn parse_csv(text: &str, pid: Option<u32>) -> Parsed {
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    let Some(cols) = lines.next().and_then(Columns::from_header) else {
        return Parsed {
            warnings: vec!["unknown_csv_header".into()],
            ..Default::default()
        };
    };
    let rows: Vec<Row> = lines
        .filter_map(|l| cols.row(l))
        .filter(|r| pid.is_none() || r.pid.is_none() || r.pid == pid)
        .collect();
    let mut by_chain: HashMap<(Option<u32>, Option<String>), usize> = HashMap::new();
    for r in &rows {
        *by_chain.entry((r.pid, r.swap_chain.clone())).or_default() += 1;
    }
    let mut warnings = Vec::new();
    if by_chain.len() > 1 {
        warnings.push("several_swap_chains".to_string());
    }
    let best = by_chain
        .into_iter()
        .max_by_key(|(k, n)| (*n, k.clone()))
        .map(|(k, _)| k);
    let chosen: Vec<&Row> = rows
        .iter()
        .filter(|r| Some((r.pid, r.swap_chain.clone())) == best)
        .collect();
    let application = chosen.first().and_then(|r| r.application.clone());
    let first_t = chosen.iter().find_map(|r| r.time_ms);
    let mut acc = 0.0;
    let frames = chosen
        .iter()
        .map(|r| {
            let t = match (r.time_ms, first_t) {
                (Some(t), Some(t0)) => t - t0,
                _ => acc,
            };
            acc += r.frame.frametime_ms;
            Frame {
                t_ms: super::stats::round(t, 3),
                ..r.frame.clone()
            }
        })
        .collect();
    Parsed {
        frames,
        application,
        warnings,
        has_display: cols.has_display(),
    }
}

/// Command line for one capture. `session` names the ETW session (so a
/// crash leaves a session we stop next time); no console, stop by itself
/// after `max_seconds` or when the game exits.
pub fn args(
    session: &str,
    output: &str,
    pid: Option<u32>,
    process_name: Option<&str>,
    max_seconds: u64,
) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--session_name".into(),
        session.into(),
        "--stop_existing_session".into(),
        "--no_console_stats".into(),
        "--no_track_input".into(),
        "--track_frame_type".into(),
        "--output_file".into(),
        output.into(),
        "--timed".into(),
        max_seconds.to_string(),
        "--terminate_after_timed".into(),
        "--terminate_on_proc_exit".into(),
    ];
    match (pid, process_name) {
        (Some(p), _) => {
            a.push("--process_id".into());
            a.push(p.to_string());
        }
        (None, Some(n)) => {
            a.push("--process_name".into());
            a.push(n.into());
        }
        (None, None) => {}
    }
    a
}

/// The SID in `whoami /user /fo csv` (the second quoted field of the data
/// line; headers are translated, the SID is not).
pub fn user_sid_from_whoami(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.rsplit(',').next())
        .map(|f| f.trim().trim_matches('"').to_string())
        .find(|s| valid_sid(s))
}

/// `S-1-` followed by numbers separated by dashes, nothing else.
pub fn valid_sid(s: &str) -> bool {
    s.len() <= 184
        && s.strip_prefix("S-1-").is_some_and(|rest| {
            !rest.is_empty()
                && rest.split('-').all(|p| {
                    !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit())
                })
        })
}

/// Account name without the `DOMAIN\` part (`net localgroup` wants the bare
/// group name; the user keeps its domain).
pub fn bare_group(account: &str) -> &str {
    account.rsplit('\\').next().unwrap_or(account).trim()
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Stdio};

    use super::*;

    fn bin_dir() -> Result<PathBuf, String> {
        crate::core::dependencies::managed_bin_dir().ok_or_else(|| "no app data dir".to_string())
    }

    /// The pinned binary, when it is there and its hash matches.
    pub fn installed() -> Option<PathBuf> {
        let custom = crate::core::washmybelly::settings().perf_tool_path;
        if !custom.trim().is_empty() {
            let p = PathBuf::from(custom.trim());
            return p.is_file().then_some(p);
        }
        let p = bin_dir().ok()?.join(ASSET);
        let bytes = std::fs::read(&p).ok()?;
        (crate::core::dependencies::integrity::sha256_hex(&bytes) == SHA256).then_some(p)
    }

    /// Download PresentMon once to `<app_data>/bin`, fail-closed on the hash.
    pub async fn ensure() -> Result<PathBuf, String> {
        if let Some(p) = installed() {
            return Ok(p);
        }
        let dir = bin_dir()?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        crate::core::washmybelly::progress("perf_setup", "download", 0, 1, "PresentMon");
        let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(URL)
            .send()
            .await
            .map_err(|e| format!("DOWNLOAD_FAILED: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("DOWNLOAD_FAILED: HTTP {}", resp.status()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("DOWNLOAD_FAILED: {e}"))?;
        crate::core::dependencies::integrity::verify_sha256(&bytes, SHA256, ASSET)
            .map_err(|e| format!("BAD_HASH: {e}"))?;
        let dest = dir.join(ASSET);
        let tmp = dir.join(format!("{ASSET}.part"));
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        let _ = std::fs::write(dir.join(LICENSE_FILE), LICENSE);
        crate::core::washmybelly::progress("perf_setup", "download", 1, 1, "PresentMon");
        Ok(dest)
    }

    pub fn start(exe: &Path, args: &[String]) -> Result<Child, String> {
        crate::core::process::std_command(exe)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("PresentMon: {e}"))
    }

    /// Ask the running PresentMon to end its session cleanly (it flushes
    /// the CSV); the caller kills the child if it does not exit.
    pub fn stop(exe: &Path, session: &str) {
        let _ = crate::core::process::std_command(exe)
            .args(["--terminate_existing_session", "--session_name", session])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    fn output(prog: &str, args: &[&str]) -> Result<String, String> {
        let o = crate::core::process::std_command(prog)
            .args(args)
            .output()
            .map_err(|e| format!("{prog}: {e}"))?;
        if !o.status.success() {
            return Err(format!(
                "{prog}: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    }

    /// SID of the signed-in user (read unelevated, before the prompt: an
    /// over-the-shoulder administrator would be another account).
    pub fn current_user_sid() -> Result<String, String> {
        user_sid_from_whoami(&output("whoami", &["/user", "/fo", "csv"])?)
            .ok_or_else(|| "could not read the user SID".into())
    }

    /// `DOMAIN\name` of a SID. Fixed script; the SID travels in an
    /// environment variable, never in the script text.
    pub fn account_of(sid: &str) -> Result<String, String> {
        if !valid_sid(sid) {
            return Err("invalid SID".into());
        }
        let o = crate::core::process::std_command("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(New-Object System.Security.Principal.SecurityIdentifier($env:WMB_SID)).Translate([System.Security.Principal.NTAccount]).Value",
            ])
            .env("WMB_SID", sid)
            .output()
            .map_err(|e| format!("powershell: {e}"))?;
        let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !o.status.success() || name.is_empty() || name.contains(['\n', '"']) {
            return Err(format!("could not resolve {sid}"));
        }
        Ok(name)
    }
}

/// Run by `omniget-elevate` (as administrator) for the one-time setup:
/// add (or, to undo, remove) the user to Performance Log Users. The group
/// name is resolved from its SID, since it is translated. Needs a sign-out
/// to take effect.
#[cfg(windows)]
pub fn elevated_perf_log_users(user_sid: &str, join: bool) -> Result<String, String> {
    if !valid_sid(user_sid)
        || !user_sid.starts_with("S-1-5-21-") && !user_sid.starts_with("S-1-12-1-")
    {
        return Err("REFUSED: not a user account SID".into());
    }
    let group = account_of(PERF_LOG_USERS_SID)?;
    let user = account_of(user_sid)?;
    let o = crate::core::process::std_command("net")
        .args([
            "localgroup",
            bare_group(&group),
            &user,
            if join { "/add" } else { "/delete" },
        ])
        .output()
        .map_err(|e| format!("net: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    // 1378: already a member; 1377: not a member (undo of an undo).
    if o.status.success() || text.contains("1378") || text.contains("1377") {
        Ok(format!(
            "{} {} {}; sign out to apply",
            user,
            if join { "joined" } else { "left" },
            bare_group(&group)
        ))
    } else {
        Err(format!("net localgroup: {}", text.trim()))
    }
}

#[cfg(not(windows))]
pub fn elevated_perf_log_users(_user_sid: &str, _join: bool) -> Result<String, String> {
    Err("Performance Log Users exists only on Windows".into())
}
