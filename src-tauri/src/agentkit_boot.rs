//! Boot step of the Central's agentkit: puts the hook shim (`omniget-hook-shim`,
//! a binary of `omniget-cli`) at a stable place, `<app_data>/bin/`, and pins the
//! converters to that path, so the hook commands written into other tools'
//! config files survive app updates, moves of the bundle and dev/release
//! switches.
//!
//! Source lookup: next to the running executable (release bundle, where Tauri
//! puts the `omniget-hook` `externalBin` sidecar without its target triple,
//! and `cargo run` in dev, where every workspace binary lands in
//! `target/<profile>/`), then the workspace `target/debug` and
//! `target/release` folders (dev builds started from another folder). Only a
//! runnable file counts (`usable_binary`): the empty sidecar placeholder of a
//! dev/CI build is never installed, and an installed copy that is not
//! runnable is not pinned. The copy happens only when the destination is
//! missing or its hash differs. Runs once, off the main thread.

use std::path::{Path, PathBuf};

use omniget_core::core::agentkit::convert::hook_common::{
    set_shim_path, usable_binary, SHIM_NAME, SHIM_SIDECAR,
};

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn exe_name() -> String {
    exe(SHIM_NAME)
}

fn sha256_file(p: &Path) -> Option<String> {
    use sha2::Digest;
    let bytes = std::fs::read(p).ok()?;
    Some(hex::encode(sha2::Sha256::digest(&bytes)))
}

/// Where a built shim may be, in order: the bundled sidecar first.
fn candidates() -> Vec<PathBuf> {
    let name = exe_name();
    let sidecar = exe(SHIM_SIDECAR);
    let mut out = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        out.push(dir.join(&sidecar));
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
    ensure_shim_from(&candidates(), &dst)
}

/// [`ensure_shim`] with the sources and the destination given (tests).
fn ensure_shim_from(candidates: &[PathBuf], dst: &Path) -> Result<Option<PathBuf>, String> {
    let dst = dst.to_path_buf();
    let src = candidates
        .iter()
        .find(|p| usable_binary(p) && **p != dst)
        .cloned();
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
    if usable_binary(&dst) {
        set_shim_path(Some(dst.clone()));
        Ok(Some(dst))
    } else {
        // An empty or broken copy (an older build installed the placeholder)
        // goes away: a hook command must never point at it.
        if dst.exists() {
            let _ = std::fs::remove_file(&dst);
            tracing::warn!(
                "[agentkit] removed an unusable hook shim at {}",
                dst.display()
            );
        }
        set_shim_path(None);
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
                "[agentkit] hook shim not found (no `omniget-hook` sidecar in the bundle; in dev: `cargo build -p omniget-cli --bin omniget-hook-shim`): Claude Code turns run without OmniGet's permission hooks"
            ),
            Err(e) => tracing::warn!("[agentkit] {e}"),
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "omniget-shim-boot-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_empty_placeholder_is_never_installed_and_a_real_shim_is() {
        let d = tmp("boot");
        let placeholder = d.join("omniget-hook");
        std::fs::write(&placeholder, b"").unwrap();
        let dst = d.join("bin").join(exe_name());
        // Only the placeholder: nothing installed, nothing pinned.
        assert_eq!(
            ensure_shim_from(std::slice::from_ref(&placeholder), &dst),
            Ok(None)
        );
        assert!(!dst.exists());
        // A real executable (this test binary) behind the placeholder wins.
        let real = std::env::current_exe().unwrap();
        assert_eq!(
            ensure_shim_from(&[placeholder.clone(), real.clone()], &dst),
            Ok(Some(dst.clone()))
        );
        assert_eq!(sha256_file(&dst), sha256_file(&real));
        let _ = std::fs::remove_dir_all(&d);
        set_shim_path(None);
    }

    #[test]
    fn an_installed_empty_copy_is_removed_not_pinned() {
        let d = tmp("stale");
        let dst = d.join("bin").join(exe_name());
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::write(&dst, b"").unwrap();
        let placeholder = d.join("omniget-hook");
        std::fs::write(&placeholder, b"").unwrap();
        assert_eq!(ensure_shim_from(&[placeholder], &dst), Ok(None));
        assert!(!dst.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
