use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{api, IconData};

const MAX_ICONS_PER_SET: usize = 4000;
const MAX_RESPONSES: usize = 400;
const TOUCH_AFTER: i64 = 24 * 3600;

static ICONS_LOCK: Mutex<()> = Mutex::new(());
static RESPONSES_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredIcon {
    #[serde(flatten)]
    icon: IconData,
    #[serde(default)]
    seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    pub fetched_at: i64,
    pub url: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

pub fn icons_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("icons"))
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the path has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn hash_key(key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    hex::encode(h.finalize())
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn open_default() -> Option<Self> {
        icons_dir().map(|d| Self::new(d.join("cache")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn response_path(&self, url: &str) -> PathBuf {
        self.root
            .join("http")
            .join(format!("{}.json", hash_key(url)))
    }

    pub fn icons_path(&self, prefix: &str) -> Option<PathBuf> {
        api::valid_prefix(prefix).then(|| self.root.join("icons").join(format!("{prefix}.json")))
    }

    pub fn read_response(&self, url: &str) -> Option<CacheEntry> {
        let text = std::fs::read_to_string(self.response_path(url)).ok()?;
        let entry: CacheEntry = serde_json::from_str(&text).ok()?;
        (entry.url == url).then_some(entry)
    }

    pub fn write_response(&self, url: &str, body: &str, fetched_at: i64) -> std::io::Result<()> {
        let entry = CacheEntry {
            fetched_at,
            url: url.to_string(),
            body: body.to_string(),
        };
        let bytes = serde_json::to_vec(&entry).map_err(std::io::Error::other)?;
        let _guard = RESPONSES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        write_atomic(&self.response_path(url), &bytes)?;
        self.prune_responses(MAX_RESPONSES);
        Ok(())
    }

    fn prune_responses(&self, keep: usize) {
        let Ok(dir) = std::fs::read_dir(self.root.join("http")) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = dir
            .filter_map(Result::ok)
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                meta.is_file()
                    .then(|| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), e.path()))
            })
            .collect();
        if files.len() <= keep {
            return;
        }
        files.sort_by_key(|f| std::cmp::Reverse(f.0));
        for (_, path) in files.into_iter().skip(keep) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn read_stored(&self, prefix: &str) -> BTreeMap<String, StoredIcon> {
        self.icons_path(prefix)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn read_icons(&self, prefix: &str) -> BTreeMap<String, IconData> {
        self.read_stored(prefix)
            .into_iter()
            .map(|(k, v)| (k, v.icon))
            .collect()
    }

    pub fn merge_icons(&self, prefix: &str, icons: &[IconData], now: i64) -> std::io::Result<()> {
        if icons.is_empty() {
            return Ok(());
        }
        let path = self
            .icons_path(prefix)
            .ok_or_else(|| std::io::Error::other("invalid icon set"))?;
        let _guard = ICONS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.read_stored(prefix);
        for icon in icons {
            map.insert(
                icon.name.clone(),
                StoredIcon {
                    icon: icon.clone(),
                    seen: now,
                },
            );
        }
        evict_oldest(&mut map, MAX_ICONS_PER_SET);
        let bytes = serde_json::to_vec(&map).map_err(std::io::Error::other)?;
        write_atomic(&path, &bytes)
    }

    pub fn touch_icons(&self, prefix: &str, names: &[String], now: i64) -> std::io::Result<()> {
        let Some(path) = self.icons_path(prefix) else {
            return Ok(());
        };
        let _guard = ICONS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.read_stored(prefix);
        let mut changed = false;
        for name in names {
            if let Some(entry) = map.get_mut(name) {
                if now - entry.seen >= TOUCH_AFTER {
                    entry.seen = now;
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&map).map_err(std::io::Error::other)?;
        write_atomic(&path, &bytes)
    }

    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_dir_all(&self.root) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn size_bytes(&self) -> u64 {
        walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum()
    }
}

fn evict_oldest(map: &mut BTreeMap<String, StoredIcon>, cap: usize) {
    if map.len() <= cap {
        return;
    }
    let mut by_age: Vec<(i64, String)> = map.iter().map(|(k, v)| (v.seen, k.clone())).collect();
    by_age.sort();
    let extra = map.len() - cap;
    for (_, k) in by_age.into_iter().take(extra) {
        map.remove(&k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(name: &str) -> IconData {
        IconData {
            name: name.into(),
            body: "<path d=\"M0 0h24\"/>".into(),
            width: 24.0,
            height: 24.0,
            left: 0.0,
            top: 0.0,
            rotate: 0,
            h_flip: false,
            v_flip: false,
        }
    }

    #[test]
    fn response_roundtrip_and_url_check() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("cache"));
        let url = "https://api.iconify.design/collections";
        assert!(cache.read_response(url).is_none());
        cache.write_response(url, "{\"a\":1}", 42).unwrap();
        let entry = cache.read_response(url).unwrap();
        assert_eq!(entry.fetched_at, 42);
        assert_eq!(entry.body, "{\"a\":1}");
        assert!(cache.read_response("https://other").is_none());
        assert!(cache.size_bytes() > 0);
    }

    #[test]
    fn corrupt_response_reads_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let url = "u";
        std::fs::create_dir_all(cache.response_path(url).parent().unwrap()).unwrap();
        std::fs::write(cache.response_path(url), "not json").unwrap();
        assert!(cache.read_response(url).is_none());
    }

    #[test]
    fn merges_icon_bodies_per_set() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.merge_icons("lucide", &[icon("house")], 1).unwrap();
        cache.merge_icons("lucide", &[icon("star")], 2).unwrap();
        let map = cache.read_icons("lucide");
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("house"));
        assert!(cache.read_icons("ph").is_empty());
        assert!(cache.merge_icons("../x", &[icon("a")], 1).is_err());
        assert!(cache.icons_path("../x").is_none());
    }

    #[test]
    fn clear_removes_everything_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("c"));
        cache.write_response("u", "b", 1).unwrap();
        cache.clear().unwrap();
        assert!(cache.read_response("u").is_none());
        cache.clear().unwrap();
        assert_eq!(cache.size_bytes(), 0);
    }

    #[test]
    fn atomic_write_replaces_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a/b.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(std::fs::read_dir(p.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn reads_bodies_written_before_seen_existed() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let path = cache.icons_path("lucide").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old: BTreeMap<String, IconData> = [("house".to_string(), icon("house"))].into();
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(cache.read_icons("lucide")["house"], icon("house"));
    }

    #[test]
    fn evicts_least_recently_seen() {
        let mut map: BTreeMap<String, StoredIcon> = ["a", "b", "c", "d"]
            .iter()
            .zip([40, 10, 30, 20])
            .map(|(n, seen)| {
                (
                    n.to_string(),
                    StoredIcon {
                        icon: icon(n),
                        seen,
                    },
                )
            })
            .collect();
        evict_oldest(&mut map, 2);
        let left: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(left, vec!["a", "c"]);
    }

    #[test]
    fn touch_refreshes_only_stale_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.merge_icons("lucide", &[icon("house")], 100).unwrap();
        cache
            .touch_icons("lucide", &["house".into()], 100 + TOUCH_AFTER)
            .unwrap();
        assert_eq!(cache.read_stored("lucide")["house"].seen, 100 + TOUCH_AFTER);
        cache
            .touch_icons("lucide", &["house".into()], 101 + TOUCH_AFTER)
            .unwrap();
        assert_eq!(cache.read_stored("lucide")["house"].seen, 100 + TOUCH_AFTER);
    }

    #[test]
    fn concurrent_merges_keep_every_icon() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        std::thread::scope(|s| {
            for t in 0..8 {
                let cache = &cache;
                s.spawn(move || {
                    for i in 0..10 {
                        cache
                            .merge_icons("lucide", &[icon(&format!("i{t}-{i}"))], 1)
                            .unwrap();
                    }
                });
            }
        });
        assert_eq!(cache.read_icons("lucide").len(), 80);
    }

    #[test]
    fn prunes_old_responses() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        for i in 0..5 {
            cache.write_response(&format!("u{i}"), "b", 1).unwrap();
        }
        cache.prune_responses(2);
        let n = std::fs::read_dir(dir.path().join("http")).unwrap().count();
        assert_eq!(n, 2);
    }
}
