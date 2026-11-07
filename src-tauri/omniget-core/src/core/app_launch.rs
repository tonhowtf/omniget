use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const BREADCRUMB: &str = "app-launch.json";

pub const BACKGROUND_FLAG: &str = "--background";

pub const AUTOSTART_FLAG: &str = "--autostart";

pub const ERR_APP_NOT_RUNNING: &str = "ERR_APP_NOT_RUNNING";

pub const BRIDGE_WAIT_S: u64 = 15;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLaunch {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub flatpak: bool,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub written_ms: u64,
}

pub fn starts_hidden<S: AsRef<str>>(args: &[S], start_minimized: bool) -> bool {
    let has = |flag: &str| args.iter().any(|a| a.as_ref() == flag);
    has(BACKGROUND_FLAG) || (has(AUTOSTART_FLAG) && start_minimized)
}

pub fn current(background: bool) -> Option<AppLaunch> {
    let (program, args, flatpak) = if let Ok(id) = std::env::var("FLATPAK_ID") {
        ("flatpak".to_string(), vec!["run".to_string(), id], true)
    } else if let Some(image) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        (image.to_string_lossy().into_owned(), Vec::new(), false)
    } else {
        let exe = std::env::current_exe().ok()?;
        (exe.to_string_lossy().into_owned(), Vec::new(), false)
    };
    Some(AppLaunch {
        program,
        args,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        flatpak,
        background,
        written_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    })
}

pub fn record_in(dir: &Path, launch: &AppLaunch) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(BREADCRUMB);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(launch).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

pub fn read_in(dir: &Path) -> Option<AppLaunch> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(BREADCRUMB)).ok()?).ok()
}

pub fn candidates(data_dir: Option<&Path>) -> Vec<AppLaunch> {
    let mut out: Vec<AppLaunch> = Vec::new();
    let exe = |p: PathBuf| AppLaunch {
        program: p.to_string_lossy().into_owned(),
        args: Vec::new(),
        pid: 0,
        version: String::new(),
        flatpak: false,
        background: false,
        written_ms: 0,
    };
    if let Some(p) = std::env::var_os("OMNIGET_APP").map(PathBuf::from) {
        if let Some(bin) = executable_of(&p) {
            out.push(exe(bin));
        }
    }
    if let Some(b) = data_dir.and_then(read_in) {
        if b.flatpak || Path::new(&b.program).is_file() {
            out.push(AppLaunch { pid: 0, ..b });
        }
    }
    let me = std::env::current_exe().ok();
    if let Some(dir) = me.as_deref().and_then(Path::parent) {
        let sibling = dir.join(if cfg!(windows) {
            "omniget.exe"
        } else {
            "omniget"
        });
        let is_me = me
            .as_deref()
            .zip(sibling.canonicalize().ok())
            .is_some_and(|(m, s)| m.canonicalize().ok().as_deref() == Some(s.as_path()));
        if sibling.is_file() && !is_me && looks_like_the_app(&sibling) {
            out.push(exe(sibling));
        }
    }
    for p in installed_paths() {
        if p.is_file() {
            out.push(exe(p));
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|l| seen.insert((l.program.clone(), l.args.clone())));
    out
}

/// herdr's autospawn (`src/platform/mod.rs`, Apache-2.0), rewritten.
pub fn spawn_detached(launch: &AppLaunch) -> std::io::Result<u32> {
    let mut cmd = std::process::Command::new(&launch.program);
    cmd.args(&launch.args)
        .arg(BACKGROUND_FLAG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe; nothing else runs between
        // fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let child = cmd.spawn()?;
    Ok(child.id())
}

fn executable_of(p: &Path) -> Option<PathBuf> {
    if p.extension().is_some_and(|e| e == "app") {
        let bin = p.join("Contents").join("MacOS").join("omniget");
        return bin.is_file().then_some(bin);
    }
    p.is_file().then(|| p.to_path_buf())
}

fn looks_like_the_app(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.len() > 40 * 1024 * 1024)
}

fn installed_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(target_os = "macos")]
    {
        v.push(PathBuf::from(
            "/Applications/omniget.app/Contents/MacOS/omniget",
        ));
        if let Some(home) = dirs::home_dir() {
            v.push(home.join("Applications/omniget.app/Contents/MacOS/omniget"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        v.push(PathBuf::from("/usr/bin/omniget"));
        v.push(PathBuf::from("/usr/local/bin/omniget"));
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(local).join("omniget").join("omniget.exe"));
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                v.push(PathBuf::from(pf).join("omniget").join("omniget.exe"));
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_login_launch_may_stay_in_the_tray() {
        assert!(!starts_hidden(&["omniget"], true));
        assert!(starts_hidden(&["omniget", AUTOSTART_FLAG], true));
        assert!(!starts_hidden(&["omniget", AUTOSTART_FLAG], false));
        assert!(starts_hidden(&["omniget", BACKGROUND_FLAG], false));
    }
}
