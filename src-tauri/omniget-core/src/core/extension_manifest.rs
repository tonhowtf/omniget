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

/// Full path a given URL maps to inside [`manifest_dir`].
pub fn manifest_path_for_url(url: &str) -> PathBuf {
    manifest_dir().join(manifest_file_name(url))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omniget-manifest-{}-{}-{}-{}",
            tag,
            std::process::id(),
            nanos,
            seq
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Backdate a file so TTL logic can be exercised without sleeping.
    fn backdate(path: &Path, secs: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        let when = SystemTime::now() - Duration::from_secs(secs);
        file.set_modified(when).unwrap();
    }

    // `manifest_dir()` is deliberately not exercised here: it goes through
    // `paths::app_data_dir()`, which can migrate a legacy data directory as a
    // side effect. The path is `manifest_dir().join(manifest_file_name(url))`,
    // so pinning the file name pins the path.

    #[test]
    fn file_name_is_stable_for_the_same_url() {
        let url = "https://cdn.example.com/live/master.m3u8?token=abc";
        assert_eq!(manifest_file_name(url), manifest_file_name(url));
        let base = Path::new("/some/data/dir");
        assert_eq!(
            base.join(manifest_file_name(url)),
            base.join(manifest_file_name(url))
        );
    }

    #[test]
    fn file_name_differs_between_urls() {
        let a = manifest_file_name("https://cdn.example.com/a.m3u8");
        let b = manifest_file_name("https://cdn.example.com/b.m3u8");
        assert_ne!(a, b);
        // Even a single query-string character apart must not collide.
        assert_ne!(
            manifest_file_name("https://cdn.example.com/a.m3u8?t=1"),
            manifest_file_name("https://cdn.example.com/a.m3u8?t=2")
        );
    }

    #[test]
    fn file_name_is_sha256_hex_with_m3u8_extension() {
        let name = manifest_file_name("https://cdn.example.com/a.m3u8");
        assert!(name.ends_with(".m3u8"));
        let stem = name.trim_end_matches(".m3u8");
        assert_eq!(stem.len(), 64);
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
        // Known-answer check so a hashing swap can never go unnoticed.
        assert_eq!(
            manifest_file_name(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855.m3u8"
        );
    }

    #[test]
    fn store_then_load_roundtrips_the_text() {
        let dir = temp_dir("roundtrip");
        let url = "https://cdn.example.com/live/master.m3u8";
        let text = "#EXTM3U\n#EXTINF:4.0,\nseg0.ts\n";

        store_manifest_in(&dir, url, text, SystemTime::now()).unwrap();
        assert_eq!(
            load_manifest_in(&dir, url, SystemTime::now()).as_deref(),
            Some(text)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_of_an_unknown_url_is_none() {
        let dir = temp_dir("unknown");
        store_manifest_in(
            &dir,
            "https://a.example/x.m3u8",
            "#EXTM3U\n",
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(
            load_manifest_in(&dir, "https://b.example/y.m3u8", SystemTime::now()),
            None
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_manifest_loads_as_none_and_is_deleted() {
        let dir = temp_dir("expired");
        let url = "https://cdn.example.com/live/master.m3u8";
        store_manifest_in(&dir, url, "#EXTM3U\n", SystemTime::now()).unwrap();

        let path = dir.join(manifest_file_name(url));
        backdate(&path, MANIFEST_TTL_SECS + 30);

        assert_eq!(load_manifest_in(&dir, url, SystemTime::now()), None);
        assert!(!path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_just_inside_the_ttl_still_loads() {
        let dir = temp_dir("fresh");
        let url = "https://cdn.example.com/live/master.m3u8";
        store_manifest_in(&dir, url, "#EXTM3U\n", SystemTime::now()).unwrap();

        backdate(&dir.join(manifest_file_name(url)), MANIFEST_TTL_SECS - 60);

        assert_eq!(
            load_manifest_in(&dir, url, SystemTime::now()).as_deref(),
            Some("#EXTM3U\n")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_text_is_refused_on_write() {
        let dir = temp_dir("oversize-write");
        let url = "https://cdn.example.com/live/master.m3u8";
        let text = "a".repeat(MAX_MANIFEST_BYTES + 1);

        let err = store_manifest_in(&dir, url, &text, SystemTime::now()).unwrap_err();
        assert!(err.to_string().contains("over the"));
        assert!(!dir.join(manifest_file_name(url)).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_exactly_at_the_limit_is_accepted() {
        let dir = temp_dir("limit-write");
        let url = "https://cdn.example.com/live/master.m3u8";
        let text = "a".repeat(MAX_MANIFEST_BYTES);

        store_manifest_in(&dir, url, &text, SystemTime::now()).unwrap();
        assert_eq!(
            load_manifest_in(&dir, url, SystemTime::now()).map(|t| t.len()),
            Some(MAX_MANIFEST_BYTES)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_file_on_disk_is_refused_on_read() {
        let dir = temp_dir("oversize-read");
        let url = "https://cdn.example.com/live/master.m3u8";
        let path = dir.join(manifest_file_name(url));
        // Written straight to disk to bypass the write-side guard.
        fs::write(&path, "a".repeat(MAX_MANIFEST_BYTES + 1)).unwrap();

        assert_eq!(load_manifest_in(&dir, url, SystemTime::now()), None);
        assert!(!path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn storing_prunes_expired_neighbours_but_keeps_fresh_ones() {
        let dir = temp_dir("prune");
        let stale_url = "https://cdn.example.com/stale.m3u8";
        let fresh_url = "https://cdn.example.com/fresh.m3u8";

        store_manifest_in(&dir, stale_url, "#EXTM3U\n", SystemTime::now()).unwrap();
        store_manifest_in(&dir, fresh_url, "#EXTM3U\n", SystemTime::now()).unwrap();
        backdate(
            &dir.join(manifest_file_name(stale_url)),
            MANIFEST_TTL_SECS + 30,
        );

        store_manifest_in(
            &dir,
            "https://cdn.example.com/new.m3u8",
            "#EXTM3U\n",
            SystemTime::now(),
        )
        .unwrap();

        assert!(!dir.join(manifest_file_name(stale_url)).exists());
        assert!(dir.join(manifest_file_name(fresh_url)).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_leaves_unrelated_files_alone() {
        let dir = temp_dir("prune-foreign");
        let foreign = dir.join("notes.txt");
        fs::write(&foreign, "keep me").unwrap();
        backdate(&foreign, MANIFEST_TTL_SECS + 30);

        prune_expired_in(&dir, SystemTime::now());
        assert!(foreign.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_expired_handles_clock_skew_as_fresh() {
        let now = SystemTime::now();
        let future = now + Duration::from_secs(3600);
        assert!(!is_expired(future, now));
        assert!(!is_expired(now, now));
        assert!(is_expired(
            now - Duration::from_secs(MANIFEST_TTL_SECS + 1),
            now
        ));
    }
}
