//! Disk-backed storage for HLS playlist *text* captured by the browser
//! extension.
//!
//! The extension's deep search can recover playlists that never travel the
//! network as a fetchable document: the page builds the manifest in JavaScript
//! and hands it to the player through a `blob:` URL. A `blob:` URL only exists
//! inside the tab that minted it, so the native side cannot download it — the
//! extension therefore ships the playlist *body* along with the enqueue
//! request.
//!
//! The plumbing follows the same shape as the extension cookie jar
//! (`cookie_parser::load_extension_cookies_for_url`): the app side writes an
//! artefact into `app_data_dir()` and `omniget-core` picks it up on its own
//! when the download actually starts. That keeps the playlist text out of
//! `DownloadOptions` and out of the already very wide `queue::enqueue`
//! signature.
//!
//! Files live in `app_data_dir()/extension-manifests/<sha256(url)>.m3u8` and
//! are pruned on every write.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

/// How long a captured playlist stays usable, in seconds.
///
/// The metadata store next door uses 60 s, which is fine for data read the
/// instant the URL is queued. A playlist, however, is only read when the
/// download actually starts, and that can sit behind other items in the
/// queue for a while — hence the far more generous 10 minutes.
pub const MANIFEST_TTL_SECS: u64 = 600;

/// Largest playlist text accepted, both when writing and when reading back.
/// Real manifests are a few hundred kilobytes at worst; anything past this is
/// either a bug or an attempt to fill the user's disk.
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

/// Directory holding every captured playlist.
pub fn manifest_dir() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("extension-manifests")
}

/// File name a given URL maps to: `<sha256hex(url)>.m3u8`.
///
/// Hashing keeps the name filesystem-safe and fixed-length no matter how long
/// or exotic the source URL is.
pub fn manifest_file_name(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    format!("{}.m3u8", hex::encode(hasher.finalize()))
}

/// Persist the playlist text captured for `url`, pruning expired files in the
/// same pass. Returns an error when the text is over [`MAX_MANIFEST_BYTES`] or
/// the write itself fails; callers treat that as non-fatal.
pub fn store_manifest(url: &str, text: &str) -> anyhow::Result<()> {
    store_manifest_in(&manifest_dir(), url, text, SystemTime::now())
}

/// Playlist text captured for `url`, when one exists and is still fresh.
/// Never panics: every failure path collapses into `None`.
pub fn load_manifest_for_url(url: &str) -> Option<String> {
    load_manifest_in(&manifest_dir(), url, SystemTime::now())
}

/// A file stamped in the future (clock skew, restored backup) counts as
/// fresh rather than as an error.
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

/// Delete every stored playlist past its TTL. Best effort: unreadable
/// entries and failed deletions are skipped silently.
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
