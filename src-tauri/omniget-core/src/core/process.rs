fn enhanced_path() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let bin_dir = crate::core::paths::app_data_dir()?.join("bin");
            let sep = if cfg!(windows) { ";" } else { ":" };
            let current = std::env::var("PATH").unwrap_or_default();

            #[allow(unused_mut)]
            let mut extra_dirs: Vec<String> = vec![bin_dir.display().to_string()];

            #[cfg(target_os = "macos")]
            {
                extra_dirs.push("/opt/homebrew/bin".into());
                extra_dirs.push("/usr/local/bin".into());
            }

            #[cfg(target_os = "linux")]
            {
                if let Some(home) = dirs::home_dir() {
                    extra_dirs.push(home.join(".local").join("bin").display().to_string());
                }
                extra_dirs.push("/usr/local/bin".into());
            }

            Some(format!("{}{}{}", extra_dirs.join(sep), sep, current))
        })
        .clone()
}

pub fn command<S: AsRef<std::ffi::OsStr>>(program: S) -> tokio::process::Command {
    let pinned = if crate::core::dependencies::worker_mode() {
        program
            .as_ref()
            .to_str()
            .and_then(crate::core::dependencies::worker_tool)
    } else {
        None
    };
    let mut cmd = tokio::process::Command::new(
        pinned
            .as_deref()
            .map(|p| p.as_os_str())
            .unwrap_or(program.as_ref()),
    );
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    if let Some(path) = enhanced_path() {
        cmd.env("PATH", path);
    }
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONLEGACYWINDOWSSTDIO", "0");
    cmd
}

pub fn std_command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    let pinned = if crate::core::dependencies::worker_mode() {
        program
            .as_ref()
            .to_str()
            .and_then(crate::core::dependencies::worker_tool)
    } else {
        None
    };
    let mut cmd = std::process::Command::new(
        pinned
            .as_deref()
            .map(|p| p.as_os_str())
            .unwrap_or(program.as_ref()),
    );
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    if let Some(path) = enhanced_path() {
        cmd.env("PATH", path);
    }
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONLEGACYWINDOWSSTDIO", "0");
    cmd
}

/// `ETXTBSY`: on Linux a file that was just written cannot be executed while
/// any process still holds it open for writing, and a `fork` on another thread
/// keeps that descriptor alive until its own `exec`. The window is a few
/// milliseconds; a launcher script written and started in the same breath
/// (ours, or a test's fake CLI) lands in it now and then.
pub fn is_text_file_busy(err: &std::io::Error) -> bool {
    cfg!(unix) && err.raw_os_error() == Some(26)
}

/// Runs `spawn`, trying again for a moment when the program is "text file busy".
pub fn spawn_retrying_busy<T>(mut spawn: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut tries = 0;
    loop {
        match spawn() {
            Err(e) if is_text_file_busy(&e) && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => return other,
        }
    }
}
