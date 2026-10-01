use std::path::{Path, PathBuf};
use std::time::Duration;

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn on_path(name: &str) -> Option<PathBuf> {
    let paths = omniget_core::core::process::enhanced_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let cwd = std::env::temp_dir();
    which::which_in(name, Some(paths), cwd).ok()
}

pub fn sdk_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join("Library/Android/sdk"));
        out.push(home.join("Android/Sdk"));
    }
    if let Some(local) = dirs::data_local_dir() {
        out.push(local.join("Android/Sdk"));
    }
    out
}

fn in_sdk(sub: &str, name: &str) -> Option<PathBuf> {
    sdk_roots()
        .into_iter()
        .map(|r| r.join(sub).join(exe(name)))
        .find(|p| p.is_file())
        .or_else(|| on_path(name))
}

pub fn adb() -> Option<PathBuf> {
    in_sdk("platform-tools", "adb")
}

pub fn emulator() -> Option<PathBuf> {
    in_sdk("emulator", "emulator")
}

pub fn xcrun() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let p = PathBuf::from("/usr/bin/xcrun");
    if p.is_file() {
        Some(p)
    } else {
        on_path("xcrun")
    }
}

pub fn idb_companion() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    [
        "/opt/homebrew/bin/idb_companion",
        "/usr/local/bin/idb_companion",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
    .or_else(|| on_path("idb_companion"))
}

pub async fn run(bin: &Path, args: &[&str], secs: u64) -> Result<Vec<u8>, String> {
    let child = omniget_core::core::process::command(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("DEVICE_COMMAND: {}: {e}", bin.display()))?;
    let out = tokio::time::timeout(Duration::from_secs(secs), child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "DEVICE_TIMEOUT: {} {} took over {secs} s",
                name_of(bin),
                args.first().copied().unwrap_or_default()
            )
        })?
        .map_err(|e| format!("DEVICE_COMMAND: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = if err.trim().is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err.trim().to_string()
        };
        return Err(format!(
            "DEVICE_COMMAND: {} {} failed: {}",
            name_of(bin),
            args.iter().take(3).copied().collect::<Vec<_>>().join(" "),
            msg.chars().take(600).collect::<String>()
        ));
    }
    Ok(out.stdout)
}

pub async fn text(bin: &Path, args: &[&str], secs: u64) -> Result<String, String> {
    run(bin, args, secs)
        .await
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn name_of(bin: &Path) -> String {
    bin.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
