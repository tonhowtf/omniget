fn enhanced_path() -> Option<String> {
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
}

pub fn command<S: AsRef<std::ffi::OsStr>>(program: S) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    if let Some(path) = enhanced_path() {
        cmd.env("PATH", path);
    }
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd
}

pub fn std_command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
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
    cmd
}
