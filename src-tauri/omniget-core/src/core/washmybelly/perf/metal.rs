use super::format::LaunchCommand;

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
