use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    pub url: String,
    pub fetched_at: u64,
    pub body: serde_json::Value,
}

pub fn default_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("components").join("cache"))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn key_for(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    hex::encode(hasher.finalize())
}

fn file_for(dir: &Path, url: &str) -> PathBuf {
    dir.join(format!("{}.json", key_for(url)))
}

pub fn read(dir: &Path, url: &str) -> Option<CacheEntry> {
    let bytes = std::fs::read(file_for(dir, url)).ok()?;
    let entry: CacheEntry = serde_json::from_slice(&bytes).ok()?;
    if entry.url != url {
        return None;
    }
    Some(entry)
}

pub fn write(dir: &Path, entry: &CacheEntry) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let target = file_for(dir, &entry.url);
    let tmp = target.with_extension("json.tmp");
    let bytes = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &target)
}

pub fn is_fresh(entry: &CacheEntry, now: u64, ttl: u64) -> bool {
    now >= entry.fetched_at && now - entry.fetched_at < ttl
}

pub fn clear(dir: &Path) -> std::io::Result<usize> {
    let mut removed = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            std::fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, at: u64) -> CacheEntry {
        CacheEntry {
            url: url.into(),
            fetched_at: at,
            body: serde_json::json!({ "name": "x", "items": [] }),
        }
    }

    #[test]
    fn round_trip_and_key_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let e = entry("https://a.dev/r/registry.json", 100);
        write(dir.path(), &e).unwrap();
        assert_eq!(read(dir.path(), &e.url), Some(e.clone()));
        assert_eq!(key_for(&e.url), key_for("https://a.dev/r/registry.json"));
        assert_ne!(key_for(&e.url), key_for("https://b.dev/r/registry.json"));
        assert!(read(dir.path(), "https://b.dev/r/registry.json").is_none());
    }

    #[test]
    fn freshness_follows_ttl() {
        let e = entry("https://a.dev/r.json", 1_000);
        assert!(is_fresh(&e, 1_000, 60));
        assert!(is_fresh(&e, 1_059, 60));
        assert!(!is_fresh(&e, 1_060, 60));
        assert!(!is_fresh(&e, 999, 60));
    }

    #[test]
    fn corrupt_or_mismatched_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://a.dev/r.json";
        std::fs::write(file_for(dir.path(), url), b"{not json").unwrap();
        assert!(read(dir.path(), url).is_none());
        let other = entry("https://other.dev/r.json", 5);
        std::fs::write(
            file_for(dir.path(), url),
            serde_json::to_vec(&other).unwrap(),
        )
        .unwrap();
        assert!(read(dir.path(), url).is_none());
    }

    #[test]
    fn clear_removes_entries() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &entry("https://a.dev/1.json", 1)).unwrap();
        write(dir.path(), &entry("https://a.dev/2.json", 1)).unwrap();
        assert_eq!(clear(dir.path()).unwrap(), 2);
        assert_eq!(clear(&dir.path().join("missing")).unwrap(), 0);
    }
}
