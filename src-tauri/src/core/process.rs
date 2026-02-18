use std::path::PathBuf;

fn managed_bin_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("omniget").join("bin"))
}

fn enhanced_path() -> Option<String> {
    let bin_dir = managed_bin_dir()?;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let current = std::env::var("PATH").unwrap_or_default();
    Some(format!("{}{}{}", bin_dir.display(), sep, current))
}

pub fn command<S: AsRef<std::ffi::OsStr>>(program: S) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    if let Some(path) = enhanced_path() {
        cmd.env("PATH", path);
    }
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
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd
}
