//! Downloads › Tools › Profile sync: save a profile or gallery link once,
//! download all of it the first time, and afterwards only what is new.
//!
//! The idea comes from SCrawler (AAndyProgram/SCrawler), which keeps a data
//! file per user with every post it already saved and stops reading a profile
//! at the first known post. Reimplemented on top of gallery-dl: one
//! `--download-archive` per profile, and `--abort` after a run of known files
//! once the profile has been synced. No code was copied.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

/// On later syncs, stop reading the profile after this many files in a row
/// that are already in its archive (the newest posts come first).
const ABORT_AFTER_KNOWN: u32 = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncedProfile {
    pub url: String,
    pub added_at: i64,
    #[serde(default)]
    pub last_sync_at: Option<i64>,
    /// Files saved by the last sync.
    #[serde(default)]
    pub last_new: usize,
    /// Files saved by every sync together.
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub profile: SyncedProfile,
    pub new_files: usize,
    pub skipped: usize,
    pub first_sync: bool,
}

fn store_dir() -> Result<PathBuf, String> {
    let dir = omniget_core::core::paths::app_data_dir()
        .ok_or("No app data folder")?
        .join("profile-sync");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn load(dir: &Path) -> Vec<SyncedProfile> {
    std::fs::read_to_string(dir.join("profiles.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(dir: &Path, list: &[SyncedProfile]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("profiles.json"), json).map_err(|e| e.to_string())
}

/// The same profile written in different ways is one entry.
pub fn normalize_url(url: &str) -> String {
    let u = url.trim();
    let u = u.split('#').next().unwrap_or(u);
    let u = u.split('?').next().unwrap_or(u);
    u.trim_end_matches('/').replacen("://www.", "://", 1)
}

/// Archive file of a profile: a stable name from its URL.
pub fn archive_name(url: &str) -> String {
    // FNV-1a, enough to give each profile its own file name.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in normalize_url(url).bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}.sqlite3")
}

pub fn sync_args(url: &str, output_dir: &str, archive: &Path, first: bool) -> Vec<String> {
    let mut a = vec![
        "--no-colors".to_string(),
        "-d".to_string(),
        output_dir.to_string(),
        "--download-archive".to_string(),
        archive.to_string_lossy().into_owned(),
    ];
    if !first {
        a.push("--abort".to_string());
        a.push(ABORT_AFTER_KNOWN.to_string());
    }
    a.push("--".to_string());
    a.push(url.to_string());
    a
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn profile_sync_list() -> Result<Vec<SyncedProfile>, String> {
    Ok(load(&store_dir()?))
}

#[tauri::command]
pub fn profile_sync_add(url: String) -> Result<Vec<SyncedProfile>, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let dir = store_dir()?;
    let mut list = load(&dir);
    let key = normalize_url(&url);
    if !list.iter().any(|p| normalize_url(&p.url) == key) {
        list.push(SyncedProfile {
            url,
            added_at: now(),
            last_sync_at: None,
            last_new: 0,
            total: 0,
            folder: None,
        });
        save(&dir, &list)?;
    }
    Ok(list)
}

/// Removes the profile from the list and forgets what it saved; the files stay.
#[tauri::command]
pub fn profile_sync_remove(url: String) -> Result<Vec<SyncedProfile>, String> {
    let dir = store_dir()?;
    let key = normalize_url(&url);
    let mut list = load(&dir);
    list.retain(|p| normalize_url(&p.url) != key);
    let _ = std::fs::remove_file(dir.join(archive_name(&url)));
    save(&dir, &list)?;
    Ok(list)
}

#[tauri::command]
pub async fn profile_sync_run(url: String, output_dir: String) -> Result<SyncResult, String> {
    if output_dir.trim().is_empty() {
        return Err("Choose a download folder first".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    let dir = store_dir()?;
    let archive = dir.join(archive_name(&url));
    let first = !archive.exists();
    let bin = omniget_core::core::dependencies::ensure_gallerydl()
        .await
        .ok_or_else(|| "gallery-dl is not available on this computer".to_string())?;

    let mut cmd = omniget_core::core::process::command(&bin);
    cmd.args(sync_args(&url, &output_dir, &archive, first));
    if let Some(cookies) = omniget_core::core::paths::app_data_dir()
        .map(|d| d.join("chrome-extension-cookies.txt"))
        .filter(|f| std::fs::metadata(f).map(|m| m.len() > 0).unwrap_or(false))
    {
        cmd.arg("--cookies").arg(cookies);
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start gallery-dl: {e}"))?;
    let mut child = output;
    let stdout = child.stdout.take().ok_or("No output from gallery-dl")?;
    let stderr = child.stderr.take().ok_or("No output from gallery-dl")?;
    let err_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            buf.push_str(&l);
            buf.push('\n');
        }
        buf
    });
    let (mut saved, mut skipped, mut folder) = (0usize, 0usize, None::<String>);
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        // gallery-dl prints each saved path, and `# <path>` for a file it skipped.
        if line.starts_with('#') {
            skipped += 1;
        } else {
            saved += 1;
            if folder.is_none() {
                folder = Path::new(line)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned());
            }
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let stderr_text = err_task.await.unwrap_or_default();
    // Exit code 1 with nothing saved and nothing known is a real failure; after
    // `--abort` gallery-dl also exits early, which is the point.
    if !status.success() && saved == 0 && skipped == 0 {
        let detail = stderr_text
            .lines()
            .rev()
            .find(|l| l.contains("[error]"))
            .map(|l| l.split("[error]").last().unwrap_or(l).trim().to_string())
            .unwrap_or_else(|| "gallery-dl could not read this profile".to_string());
        return Err(detail);
    }

    let mut list = load(&dir);
    let key = normalize_url(&url);
    if !list.iter().any(|p| normalize_url(&p.url) == key) {
        list.push(SyncedProfile {
            url: url.clone(),
            added_at: now(),
            last_sync_at: None,
            last_new: 0,
            total: 0,
            folder: None,
        });
    }
    let entry = list
        .iter_mut()
        .find(|p| normalize_url(&p.url) == key)
        .ok_or("Profile not found")?;
    entry.last_sync_at = Some(now());
    entry.last_new = saved;
    entry.total += saved;
    if folder.is_some() {
        entry.folder = folder;
    }
    let profile = entry.clone();
    save(&dir, &list)?;
    Ok(SyncResult {
        profile,
        new_files: saved,
        skipped,
        first_sync: first,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_profile_one_archive() {
        assert_eq!(
            normalize_url("https://www.flickr.com/photos/blender/?x=1#top"),
            "https://flickr.com/photos/blender"
        );
        assert_eq!(
            archive_name("https://www.flickr.com/photos/blender/"),
            archive_name("https://flickr.com/photos/blender")
        );
        assert_ne!(
            archive_name("https://flickr.com/photos/a"),
            archive_name("https://flickr.com/photos/b")
        );
    }

    #[test]
    fn later_syncs_stop_at_known_files() {
        let arch = Path::new("/tmp/x.sqlite3");
        let first = sync_args("https://h/u", "/out", arch, true);
        assert!(first.contains(&"--download-archive".to_string()));
        assert!(!first.contains(&"--abort".to_string()));
        let later = sync_args("https://h/u", "/out", arch, false);
        let i = later.iter().position(|a| a == "--abort").unwrap();
        assert_eq!(later[i + 1], "20");
        assert_eq!(later.last().unwrap(), "https://h/u");
    }
}
