use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResumeState {
    pub completed: HashSet<i32>,
    #[serde(default)]
    pub last_updated: i64,
}

pub fn compute_fingerprint(chat_id: i64, message_ids: &[i32]) -> String {
    let mut sorted: Vec<i32> = message_ids.to_vec();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(chat_id.to_le_bytes());
    for id in &sorted {
        hasher.update(id.to_le_bytes());
    }
    let hash = hasher.finalize();
    let hex: String = hash.iter().take(8).map(|b| format!("{:02x}", b)).collect();
    hex
}

pub fn resume_file_path(plugin_data_dir: &Path, fingerprint: &str) -> PathBuf {
    plugin_data_dir
        .join("resume")
        .join(format!("{}.json", fingerprint))
}

pub async fn load_resume_state(plugin_data_dir: &Path, fingerprint: &str) -> ResumeState {
    let path = resume_file_path(plugin_data_dir, fingerprint);
    match tokio::fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice::<ResumeState>(&bytes) {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("[tg-resume] failed to parse {}: {}", path.display(), e);
                ResumeState::default()
            }
        },
        Err(_) => ResumeState::default(),
    }
}

pub async fn save_resume_state(
    plugin_data_dir: &Path,
    fingerprint: &str,
    state: &ResumeState,
) -> anyhow::Result<()> {
    let path = resume_file_path(plugin_data_dir, fingerprint);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state)?;
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

pub async fn mark_completed(
    plugin_data_dir: &Path,
    fingerprint: &str,
    message_id: i32,
) -> anyhow::Result<()> {
    let mut state = load_resume_state(plugin_data_dir, fingerprint).await;
    state.completed.insert(message_id);
    state.last_updated = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    save_resume_state(plugin_data_dir, fingerprint, &state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic() {
        let a = compute_fingerprint(123, &[1, 2, 3]);
        let b = compute_fingerprint(123, &[1, 2, 3]);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_ignores_message_id_order() {
        let a = compute_fingerprint(123, &[3, 1, 2]);
        let b = compute_fingerprint(123, &[1, 2, 3]);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_changes_when_chat_id_differs() {
        let a = compute_fingerprint(100, &[1, 2, 3]);
        let b = compute_fingerprint(200, &[1, 2, 3]);
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_changes_when_message_set_differs() {
        let a = compute_fingerprint(123, &[1, 2, 3]);
        let b = compute_fingerprint(123, &[1, 2, 4]);
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_is_16_hex_chars() {
        let a = compute_fingerprint(123, &[1, 2, 3]);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn load_returns_default_when_file_missing() {
        let dir = make_temp_dir("missing");
        let state = load_resume_state(&dir, "nonexistent_fp").await;
        assert!(state.completed.is_empty());
        cleanup(&dir);
    }

    #[tokio::test]
    async fn save_then_load_round_trip() {
        let dir = make_temp_dir("roundtrip");
        let mut s = ResumeState::default();
        s.completed.insert(42);
        s.completed.insert(100);
        s.last_updated = 1234567890;
        save_resume_state(&dir, "test_fp", &s).await.unwrap();
        let loaded = load_resume_state(&dir, "test_fp").await;
        assert_eq!(loaded.completed.len(), 2);
        assert!(loaded.completed.contains(&42));
        assert!(loaded.completed.contains(&100));
        assert_eq!(loaded.last_updated, 1234567890);
        cleanup(&dir);
    }

    #[tokio::test]
    async fn mark_completed_accumulates() {
        let dir = make_temp_dir("accumulate");
        let fp = "accumulate_fp";
        mark_completed(&dir, fp, 1).await.unwrap();
        mark_completed(&dir, fp, 2).await.unwrap();
        mark_completed(&dir, fp, 3).await.unwrap();
        let loaded = load_resume_state(&dir, fp).await;
        assert_eq!(loaded.completed.len(), 3);
        cleanup(&dir);
    }

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("tg-resume-test-{}-{}", label, nanos));
        let _ = std::fs::create_dir_all(&p);
        p
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
