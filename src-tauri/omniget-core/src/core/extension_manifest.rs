use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

pub const MANIFEST_TTL_SECS: u64 = 600;

pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

pub fn manifest_dir() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("extension-manifests")
}

pub fn manifest_file_name(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    format!("{}.m3u8", hex::encode(hasher.finalize()))
}

pub fn store_manifest(url: &str, text: &str) -> anyhow::Result<()> {
    store_manifest_in(&manifest_dir(), url, text, SystemTime::now())
}

pub fn load_manifest_for_url(url: &str) -> Option<String> {
    load_manifest_in(&manifest_dir(), url, SystemTime::now())
}

fn is_expired(modified: SystemTime, now: SystemTime) -> bool {
    match now.duration_since(modified) {
        Ok(age) => age.as_secs() > MANIFEST_TTL_SECS,
        Err(_) => false,
    }
}

fn store_manifest_in(dir: &Path, url: &str, text: &str, now: SystemTime) -> anyhow::Result<()> {
    if text.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!(
            "Extension playlist is {} bytes, over the {} byte limit",
            text.len(),
            MAX_MANIFEST_BYTES
        );
    }

    fs::create_dir_all(dir)?;
    prune_expired_in(dir, now);

    let path = dir.join(manifest_file_name(url));
    fs::write(&path, text)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn load_manifest_in(dir: &Path, url: &str, now: SystemTime) -> Option<String> {
    let path = dir.join(manifest_file_name(url));

    let meta = fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }

    if meta.len() > MAX_MANIFEST_BYTES as u64 {
        tracing::warn!(
            "extension playlist for a queued URL is {} bytes, over the {} byte limit; ignoring it",
            meta.len(),
            MAX_MANIFEST_BYTES
        );
        let _ = fs::remove_file(&path);
        return None;
    }

    let modified = meta.modified().ok()?;
    if is_expired(modified, now) {
        let _ = fs::remove_file(&path);
        return None;
    }

    fs::read_to_string(&path).ok()
}

fn prune_expired_in(dir: &Path, now: SystemTime) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("m3u8") {
            continue;
        }
        let expired = entry
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .map(|modified| is_expired(modified, now))
            .unwrap_or(false);
        if expired {
            let _ = fs::remove_file(&path);
        }
    }
}
