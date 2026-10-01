//! macOS: the Metal Performance HUD (`MTL_HUD_ENABLED=1`), a system
//! feature, so it also covers Rosetta, CrossOver/Wine and D3DMetal games.
//!
//! Checked on macOS 26.5.2 (25F84) on 2026-09-27 with a small MTKView app:
//!
//! - `MTL_HUD_LOG_ENABLED=1` makes libMTLHud write one line every ~0.5 s,
//!   `metal-HUD: <frame counter>,<a>,<b>,` followed by four numbers per frame
//!   since the previous line (frame interval ms, present delay, logical
//!   frame interval, logical present delay). A stall shows up as its own
//!   long interval (33.33, 83.33, 100 ms at 60 Hz), so this is per frame,
//!   not an average.
//! - The same line goes to the unified log (subsystem `com.apple.metal.hud`,
//!   category `Logging`), but everything after the first three numbers is
//!   `<private>`: another process cannot read the frames of a game that is
//!   already running.
//! - The line is also on the app's stderr, in clear. So frames are captured
//!   only when OmniGet itself starts the game (screen action, never an MCP
//!   tool); a running game gets the HUD instruction and the 1 Hz telemetry.

use super::format::{Frame, LaunchCommand};

/// Frame intervals (ms) in one HUD log line, and the frame counter.
pub fn parse_line(line: &str) -> Option<(u64, Vec<f64>)> {
    let (_, rest) = line.split_once("metal-HUD:")?;
    let rest = rest.trim();
    if rest.contains("<private>") {
        return None;
    }
    let vals: Vec<&str> = rest.split(',').map(str::trim).collect();
    if vals.len() < 3 {
        return None;
    }
    let counter = vals[0].parse::<u64>().ok()?;
    let intervals = vals[3..]
        .chunks(4)
        .filter_map(|c| c.first()?.parse::<f64>().ok())
        .filter(|x| x.is_finite() && *x > 0.0)
        .collect();
    Some((counter, intervals))
}

/// Frames of a whole HUD log (stderr capture), `t_ms` from the first frame.
pub fn parse_log(text: &str) -> Vec<Frame> {
    let mut t = 0.0;
    let mut out = Vec::new();
    for (_, intervals) in text.lines().filter_map(parse_line) {
        for ft in intervals {
            t += ft;
            out.push(Frame {
                t_ms: super::stats::round(t, 3),
                frametime_ms: ft,
                ..Default::default()
            });
        }
    }
    out
}

/// What the person can do by hand to see the HUD on a game OmniGet did
/// not start. Display only.
pub fn launch_commands(executable: Option<&str>) -> Vec<LaunchCommand> {
    let exe = executable
        .map(super::mangohud::sh_quote)
        .unwrap_or_else(|| "<game>".into());
    vec![
        LaunchCommand {
            kind: "metal".into(),
            command: format!("MTL_HUD_ENABLED=1 {exe}"),
        },
        LaunchCommand {
            kind: "metal_open".into(),
            command: "open --env MTL_HUD_ENABLED=1 -a <Game>".into(),
        },
    ]
}

#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Stdio};

    /// The binary inside a `.app` (its `CFBundleExecutable`), or the path
    /// itself when it is already an executable file.
    pub fn executable_of(path: &Path) -> Result<PathBuf, String> {
        if path.extension().and_then(|e| e.to_str()) == Some("app") && path.is_dir() {
            let plist = path.join("Contents/Info.plist");
            let out = crate::core::process::std_command("/usr/bin/plutil")
                .arg("-extract")
                .arg("CFBundleExecutable")
                .arg("raw")
                .arg("-o")
                .arg("-")
                .arg("--")
                .arg(&plist)
                .output()
                .map_err(|e| format!("plutil: {e}"))?;
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !out.status.success() || name.is_empty() || name.contains('/') {
                return Err("NOT_AN_APP: no executable in this app".into());
            }
            let exe = path.join("Contents/MacOS").join(name);
            return exe
                .is_file()
                .then_some(exe)
                .ok_or_else(|| "NOT_AN_APP: the app's executable is missing".into());
        }
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        Err("NOT_FOUND: pick an app or an executable".into())
    }

    /// Start the game with the HUD and its log on stderr (piped to us).
    pub fn launch(exe: &Path) -> Result<Child, String> {
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        crate::core::process::std_command(exe)
            .current_dir(dir)
            .env("MTL_HUD_ENABLED", "1")
            .env("MTL_HUD_LOG_ENABLED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start the game: {e}"))
    }
}
