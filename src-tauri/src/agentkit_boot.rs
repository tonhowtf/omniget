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

pub fn shim_destination() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("bin").join(exe_name()))
}

pub fn ensure_shim() -> Result<Option<PathBuf>, String> {
    let dst = shim_destination().ok_or("AGENTKIT_BOOT: no app data directory")?;
    ensure_shim_from(&candidates(), &dst)
}

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

// WHY: a running shim on Windows keeps the old file locked: the rename fails and the next boot retries.
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

fn ensure_canvas_cli(shim: &Path) {
    let Some(dst) = shim.parent().map(|d| d.join(exe("omniget"))) else {
        return;
    };
    if sha256_file(shim) != sha256_file(&dst) {
        match install(shim, &dst) {
            Ok(()) => tracing::info!("[canvas] omniget helper at {}", dst.display()),
            Err(e) => tracing::warn!("[canvas] omniget helper: {e}"),
        }
    }
}

pub fn spawn_ensure_shim() {
    let _ = std::thread::Builder::new()
        .name("agentkit-shim".into())
        .spawn(|| match ensure_shim() {
            Ok(Some(p)) => {
                tracing::info!("[agentkit] hook shim at {}", p.display());
                ensure_canvas_cli(&p);
                if let Err(e) = crate::canvas::cords::install_plugin() {
                    tracing::warn!("[canvas] plugin: {e}");
                }
            }
            Ok(None) => tracing::warn!(
                "[agentkit] hook shim not found (no `omniget-hook` sidecar in the bundle; in dev: `cargo build -p omniget-cli --bin omniget-hook-shim`): Claude Code turns run without OmniGet's permission hooks"
            ),
            Err(e) => tracing::warn!("[agentkit] {e}"),
        });
}
