use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProfile {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_redacted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AccountsManifest {
    #[serde(default)]
    pub profiles: Vec<AccountProfile>,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn manifest_path(plugin_data_dir: &Path) -> PathBuf {
    plugin_data_dir.join("accounts.json")
}

fn profile_dir(plugin_data_dir: &Path, id: &str) -> PathBuf {
    plugin_data_dir.join("accounts").join(id)
}

fn profile_session_path(plugin_data_dir: &Path, id: &str) -> PathBuf {
    profile_dir(plugin_data_dir, id).join("telegram.session")
}

static FILE_LOCK: Mutex<()> = Mutex::new(());

pub fn load_manifest(plugin_data_dir: &Path) -> AccountsManifest {
    let _g = FILE_LOCK.lock().ok();
    let path = manifest_path(plugin_data_dir);
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_manifest(plugin_data_dir: &Path, m: &AccountsManifest) -> anyhow::Result<()> {
    let path = manifest_path(plugin_data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(m)?)?;
    Ok(())
}

fn redact_phone(phone: &str) -> String {
    let trimmed: String = phone
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    if trimmed.len() <= 4 {
        return "***".to_string();
    }
    let suffix: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("***{}", suffix)
}

fn live_session_path() -> anyhow::Result<PathBuf> {
    super::auth::session_file_path()
}

pub fn list_profiles(plugin_data_dir: &Path) -> Vec<AccountProfile> {
    let m = load_manifest(plugin_data_dir);
    let mut list = m.profiles;
    list.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
    list
}

pub fn save_current_as_profile(
    plugin_data_dir: &Path,
    label: &str,
    phone: Option<&str>,
    user_id: Option<i64>,
) -> anyhow::Result<AccountProfile> {
    let _g = FILE_LOCK.lock().ok();
    let live = live_session_path()?;
    if !live.exists() {
        anyhow::bail!("No active session to save");
    }
    let id = format!("acct-{}", now_unix());
    let dir = profile_dir(plugin_data_dir, &id);
    std::fs::create_dir_all(&dir)?;

    let dest_session = profile_session_path(plugin_data_dir, &id);
    std::fs::copy(&live, &dest_session)?;

    let profile = AccountProfile {
        id: id.clone(),
        label: if label.trim().is_empty() {
            phone
                .map(redact_phone)
                .unwrap_or_else(|| "Unnamed".to_string())
        } else {
            label.trim().to_string()
        },
        phone_redacted: phone.map(redact_phone),
        user_id,
        created_at: now_unix(),
        updated_at: now_unix(),
    };

    let mut m = load_manifest(plugin_data_dir);
    m.profiles.push(profile.clone());
    save_manifest(plugin_data_dir, &m)?;
    Ok(profile)
}

pub fn restore_profile(plugin_data_dir: &Path, id: &str) -> anyhow::Result<()> {
    let _g = FILE_LOCK.lock().ok();
    let src = profile_session_path(plugin_data_dir, id);
    if !src.exists() {
        anyhow::bail!("Profile '{}' not found", id);
    }
    let live = live_session_path()?;
    if let Some(parent) = live.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if live.exists() {
        let bk_dir = plugin_data_dir.join("backups");
        std::fs::create_dir_all(&bk_dir)?;
        let bk_name = format!("session-pre-restore-{}.bak", now_unix());
        let _ = std::fs::copy(&live, bk_dir.join(bk_name));
    }
    std::fs::copy(&src, &live)?;

    let mut m = load_manifest(plugin_data_dir);
    if let Some(p) = m.profiles.iter_mut().find(|p| p.id == id) {
        p.updated_at = now_unix();
    }
    save_manifest(plugin_data_dir, &m)?;
    Ok(())
}

pub fn remove_profile(plugin_data_dir: &Path, id: &str) -> anyhow::Result<()> {
    let _g = FILE_LOCK.lock().ok();
    let dir = profile_dir(plugin_data_dir, id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let mut m = load_manifest(plugin_data_dir);
    m.profiles.retain(|p| p.id != id);
    save_manifest(plugin_data_dir, &m)?;
    Ok(())
}

pub fn rename_profile(
    plugin_data_dir: &Path,
    id: &str,
    label: &str,
) -> anyhow::Result<AccountProfile> {
    let _g = FILE_LOCK.lock().ok();
    let mut m = load_manifest(plugin_data_dir);
    let p = m
        .profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| anyhow::anyhow!("Profile '{}' not found", id))?;
    p.label = label.trim().to_string();
    p.updated_at = now_unix();
    let cloned = p.clone();
    save_manifest(plugin_data_dir, &m)?;
    Ok(cloned)
}

pub fn backup_current_session(plugin_data_dir: &Path) -> anyhow::Result<PathBuf> {
    let _g = FILE_LOCK.lock().ok();
    let live = live_session_path()?;
    if !live.exists() {
        anyhow::bail!("No active session to back up");
    }
    let bk_dir = plugin_data_dir.join("backups");
    std::fs::create_dir_all(&bk_dir)?;
    let ts = chrono_like_timestamp();
    let dest = bk_dir.join(format!("session-{}.bak", ts));
    std::fs::copy(&live, &dest)?;
    Ok(dest)
}

fn chrono_like_timestamp() -> String {
    let secs = now_unix();
    let days = secs / 86_400;
    let (y, m, d) = super::bandwidth::unix_days_to_ymd_pub(days);
    let rem = secs % 86_400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, m, d, hh, mm, ss)
}

pub fn list_backups(plugin_data_dir: &Path) -> Vec<(String, i64)> {
    let bk_dir = plugin_data_dir.join("backups");
    let mut out = Vec::new();
    if let Ok(it) = std::fs::read_dir(&bk_dir) {
        for entry in it.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let mtime = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                out.push((name, mtime));
            }
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.1));
    out
}
