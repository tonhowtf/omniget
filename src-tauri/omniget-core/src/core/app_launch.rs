//! How the command line finds and starts the desktop app (3.2, S17 phase 1).
//!
//! `omniget agent …` talks to the app through its local bridge. When the app
//! is not running, the CLI starts it hidden (`--background`: tray only, no
//! window) and waits for the bridge, instead of dying with "open OmniGet
//! once". The CLI ships apart from the app (a tarball, Homebrew, winget), so
//! it cannot assume a path: the app itself writes where it lives
//! ([`BREADCRUMB`] in its data folder) every time it starts, and the CLI
//! reads it back. Installed paths are only a fallback for an app that never
//! ran on this profile.
//!
//! Flatpak: the app writes `flatpak run <id>` as its launcher, but its data
//! folder lives inside the sandbox (`~/.var/app/<id>/data`), which the host
//! CLI does not read unless `OMNIGET_DATA_DIR` points there; and starting at
//! login needs the Background portal. Neither is verified; the settings
//! screen says so.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `<app data>/app-launch.json`: written by the app at every start.
pub const BREADCRUMB: &str = "app-launch.json";

/// The flag that starts the app with no window (tray only).
pub const BACKGROUND_FLAG: &str = "--background";

/// The flag the login item carries (`tauri-plugin-autostart`): a start at
/// login is hidden when the person asked for the tray or for agents in the
/// background, and shows the window otherwise.
pub const AUTOSTART_FLAG: &str = "--autostart";

/// The code of the CLI's error when the app is not running and could not be
/// started in time.
pub const ERR_APP_NOT_RUNNING: &str = "ERR_APP_NOT_RUNNING";

/// How long the CLI waits for the bridge of an app it started.
pub const BRIDGE_WAIT_S: u64 = 15;

/// Where the app lives and how to start it again, as the app wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLaunch {
    /// The executable (or `flatpak`).
    pub program: String,
    /// Arguments before ours (`["run", "wtf.tonho.omniget"]` for Flatpak).
    #[serde(default)]
    pub args: Vec<String>,
    /// The process that wrote it (the running app, while it runs).
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub flatpak: bool,
    /// Whether that start was hidden (`--background` or a hidden login).
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub written_ms: u64,
}

/// Whether a start with these arguments shows no window: `--background`
/// always; a start at login (`--autostart`) when the person chose the tray
/// or agents in the background. `start_minimized` hides any start, as before.
pub fn starts_hidden<S: AsRef<str>>(
    args: &[S],
    start_minimized: bool,
    run_in_background: bool,
) -> bool {
    let has = |flag: &str| args.iter().any(|a| a.as_ref() == flag);
    has(BACKGROUND_FLAG) || start_minimized || (has(AUTOSTART_FLAG) && run_in_background)
}

/// How this process would be started again: an AppImage by its image (the
/// mounted path changes every run), a Flatpak through `flatpak run`, anything
/// else by its own executable.
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

/// Writes the breadcrumb into `dir` (the app's data folder), atomically.
pub fn record_in(dir: &Path, launch: &AppLaunch) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(BREADCRUMB);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(launch).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

/// The breadcrumb in `dir`, if any.
pub fn read_in(dir: &Path) -> Option<AppLaunch> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(BREADCRUMB)).ok()?).ok()
}

/// Every way to start the app the CLI may try, best first: `OMNIGET_APP`
/// (an executable or a macOS `.app`), the breadcrumb, an `omniget` next to
/// the CLI, then the places the installers put it. Only paths that exist
/// (a Flatpak launcher is taken as written).
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
        // The CLI may itself be installed as `omniget`: never start ourselves.
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

/// Starts the app with `--background`, detached from the caller: its own
/// session on Unix (`setsid`, so closing the terminal does not take it
/// down), `DETACHED_PROCESS` on Windows, no inherited stdio. The pattern is
/// herdr's autospawn (`src/platform/mod.rs`, Apache-2.0), rewritten.
/// Returns the pid. The environment is inherited, `OMNIGET_DATA_DIR`
/// included, so the app opens the same profile the CLI reads.
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

/// A macOS bundle resolves to its executable; anything else must be a file.
fn executable_of(p: &Path) -> Option<PathBuf> {
    if p.extension().is_some_and(|e| e == "app") {
        let bin = p.join("Contents").join("MacOS").join("omniget");
        return bin.is_file().then_some(bin);
    }
    p.is_file().then(|| p.to_path_buf())
}

/// The desktop app is far bigger than the CLI (the webview, the UI): a file
/// named `omniget` under 40 MB next to the CLI is a CLI build, not the app.
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
    fn background_hides_and_a_login_start_hides_only_when_asked() {
        assert!(starts_hidden(&["omniget", "--background"], false, false));
        assert!(!starts_hidden(&["omniget"], false, false));
        assert!(!starts_hidden(&["omniget", "--autostart"], false, false));
        assert!(starts_hidden(&["omniget", "--autostart"], false, true));
        assert!(starts_hidden(&["omniget", "--autostart"], true, false));
        // "Start in tray only" keeps hiding every start, as before 3.2.
        assert!(starts_hidden(&["omniget"], true, false));
        // A person opening the app by hand with agents in the background
        // still gets the window.
        assert!(!starts_hidden(&["omniget"], false, true));
    }

    #[test]
    fn the_breadcrumb_round_trips_and_leads_the_candidates() {
        let dir = std::env::temp_dir().join(format!("omniget-launch-{}", uuid::Uuid::new_v4()));
        let fake = dir.join("omniget-app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&fake, b"#!/bin/sh\n").unwrap();
        let launch = AppLaunch {
            program: fake.to_string_lossy().into_owned(),
            args: vec![],
            pid: 42,
            version: "0.10.1".into(),
            flatpak: false,
            background: true,
            written_ms: 1,
        };
        record_in(&dir, &launch).unwrap();
        assert_eq!(read_in(&dir).as_ref(), Some(&launch));
        let c = candidates(Some(&dir));
        let first = c
            .iter()
            .find(|l| l.program == launch.program)
            .expect("the breadcrumb is a candidate");
        assert_eq!(first.pid, 0, "the old pid never travels");
        // A breadcrumb pointing at a file that is gone is skipped.
        std::fs::remove_file(&fake).unwrap();
        assert!(!candidates(Some(&dir))
            .iter()
            .any(|l| l.program == launch.program));
        // Flatpak's launcher is not a file on the host and is kept.
        let flat = AppLaunch {
            program: "flatpak".into(),
            args: vec!["run".into(), "wtf.tonho.omniget".into()],
            flatpak: true,
            ..launch
        };
        record_in(&dir, &flat).unwrap();
        assert!(candidates(Some(&dir))
            .iter()
            .any(|l| l.flatpak && l.args == flat.args));
        let _ = std::fs::remove_dir_all(dir);
    }
}
