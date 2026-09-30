use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tokio::sync::Mutex;

pub const DEFAULT_MAX_FILES: usize = 30;
pub const DEFAULT_MAX_BYTES: u64 = 80 * 1024 * 1024;

pub struct PreviewDiskCache {
    dir: PathBuf,
    max_files: usize,
    max_bytes: u64,
    write_mutex: Mutex<()>,
}

impl PreviewDiskCache {
    pub fn new(dir: PathBuf, max_files: usize, max_bytes: u64) -> Self {
        Self {
            dir,
            max_files,
            max_bytes,
            write_mutex: Mutex::new(()),
        }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{}.bin", safe))
    }

    pub async fn ensure_dir(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.path_for(key);
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let _ = bump_mtime(&path).await;
                Some(bytes)
            }
            Err(_) => None,
        }
    }

    pub async fn put(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        if let Err(e) = self.ensure_dir().await {
            tracing::warn!("[preview-cache] ensure_dir failed: {}", e);
            return Err(e);
        }
        let _g = self.write_mutex.lock().await;
        let path = self.path_for(key);
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        if let Err(e) = self.prune_locked().await {
            tracing::warn!("[preview-cache] prune failed: {}", e);
        }
        Ok(())
    }

    async fn prune_locked(&self) -> std::io::Result<()> {
        let mut entries: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let mut rd = match tokio::fs::read_dir(&self.dir).await {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_file() {
                continue;
            }
            let size = meta.len();
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((path, size, mtime));
        }
        entries.sort_by_key(|a| a.2);
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        let mut count = entries.len();
        while count > self.max_files || total > self.max_bytes {
            if entries.is_empty() {
                break;
            }
            let (path, sz, _) = entries.remove(0);
            if let Err(e) = tokio::fs::remove_file(&path).await {
                tracing::warn!("[preview-cache] failed to remove {}: {}", path.display(), e);
            } else {
                total = total.saturating_sub(sz);
                count -= 1;
            }
        }
        Ok(())
    }

    pub async fn prune(&self) -> std::io::Result<()> {
        let _g = self.write_mutex.lock().await;
        self.prune_locked().await
    }
}

async fn bump_mtime(path: &Path) -> std::io::Result<()> {
    let now = filetime::FileTime::now();
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || filetime::set_file_mtime(&p, now))
        .await
        .map_err(|_| std::io::Error::other("join error"))??;
    Ok(())
}
