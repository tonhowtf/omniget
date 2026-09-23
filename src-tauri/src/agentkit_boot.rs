//! Boot step of the Central's agentkit: puts the hook shim (`omniget-hook-shim`,
//! a binary of `omniget-cli`) at a stable place, `<app_data>/bin/`, and pins the
//! converters to that path, so the hook commands written into other tools'
//! config files survive app updates, moves of the bundle and dev/release
//! switches.
//!
//! Source lookup: next to the running executable (release bundle, where Tauri
//! puts `externalBin` sidecars without their target triple, and `cargo run`
//! in dev, where every workspace binary lands in `target/<profile>/`), then the
//! workspace `target/debug` and `target/release` folders (dev builds started
//! from another folder). The copy happens only when the destination is missing
//! or its hash differs. Runs once, off the main thread.

use std::path::{Path, PathBuf};

use omniget_core::core::agentkit::convert::hook_common::{set_shim_path, SHIM_NAME};

fn exe_name() -> String {
    if cfg!(windows) {
        format!("{SHIM_NAME}.exe")
    } else {
        SHIM_NAME.to_string()
    }
}

fn sha256_file(p: &Path) -> Option<String> {
    use sha2::Digest;
    let bytes = std::fs::read(p).ok()?;
    Some(hex::encode(sha2::Sha256::digest(&bytes)))
}

/// Where a built shim may be, in order.
fn candidates() -> Vec<PathBuf> {
    let name = exe_name();
    let mut out = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        out.push(dir.join(&name));
        // macOS bundle: Contents/MacOS/<app> — resources live in Contents/Resources
        if let Some(contents) = dir.parent() {
            out.push(contents.join("Resources").join(&name));
        }
    }
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    for profile in ["debug", "release"] {
        out.push(target.join(profile).join(&name));
    }
    out
}

/// The destination: `<app_data>/bin/omniget-hook-shim[.exe]`.
pub fn shim_destination() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("bin").join(exe_name()))
}

/// Copies the shim to `<app_data>/bin/` when missing or different and pins the
/// converters to it. Returns the pinned path (or `None` when no shim exists
/// anywhere: hook commands then fall back to the core's own lookup).
pub fn ensure_shim() -> Result<Option<PathBuf>, String> {
    let dst = shim_destination().ok_or("AGENTKIT_BOOT: no app data directory")?;
    let src = candidates().into_iter().find(|p| p.is_file() && *p != dst);
    if let Some(src) = &src {
        let want = sha256_file(src);
        let have = sha256_file(&dst);
        if want.is_some() && want != have {
            install(src, &dst)?;
            tracing::info!(
                "[agentkit] hook shim copied from {} to {}",
                src.display(),
                dst.display()
            );
        }
    }
    if dst.is_file() {
        set_shim_path(Some(dst.clone()));
        Ok(Some(dst))
    } else {
        Ok(None)
    }
}

/// Atomic copy with the executable bit (a running shim on Windows keeps the
/// old file: the rename fails and the next boot tries again).
fn install(src: &Path, dst: &Path) -> Result<(), String> {
    let dir = dst.parent().ok_or("AGENTKIT_BOOT: bad destination")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("AGENTKIT_BOOT: {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.{}",
        exe_name(),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    std::fs::copy(src, &tmp).map_err(|e| format!("AGENTKIT_BOOT: copy: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    if let Err(e) = std::fs::rename(&tmp, dst) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("AGENTKIT_BOOT: replace {}: {e}", dst.display()));
    }
    Ok(())
}

/// Setup hook: runs [`ensure_shim`] on its own thread and logs the outcome.
pub fn spawn_ensure_shim() {
    let _ = std::thread::Builder::new()
        .name("agentkit-shim".into())
        .spawn(|| match ensure_shim() {
            Ok(Some(p)) => tracing::info!("[agentkit] hook shim at {}", p.display()),
            Ok(None) => tracing::warn!(
                "[agentkit] hook shim not found (build it with `cargo build -p omniget-cli --bin omniget-hook-shim`)"
            ),
            Err(e) => tracing::warn!("[agentkit] {e}"),
        });
}
