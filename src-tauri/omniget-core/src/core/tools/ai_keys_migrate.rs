use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::KeyEntry;
use crate::core::ai::AiConfig;
use crate::core::secrets::{self, AI_KEYS};

pub fn key_account(kind: &str, id: &str) -> String {
    format!("{}:{}", kind, id)
}

pub fn token_account(kind: &str, id: &str) -> String {
    format!("{}:{}#access_token", kind, id)
}

pub const APP_OPENAI_ACCOUNT: &str = "app:openai_key";
pub const APP_ANTHROPIC_ACCOUNT: &str = "app:anthropic_key";

pub fn accounts_of(entry: &KeyEntry) -> [(String, String); 2] {
    [
        (
            key_account(&entry.kind, &entry.id),
            entry.key.trim().to_string(),
        ),
        (
            token_account(&entry.kind, &entry.id),
            entry.access_token.trim().to_string(),
        ),
    ]
}

pub fn entry_has_plaintext(entry: &KeyEntry) -> bool {
    !entry.key.trim().is_empty() || !entry.access_token.trim().is_empty()
}

pub fn take_secrets(entries: &mut [KeyEntry]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in entries.iter_mut() {
        for (account, value) in accounts_of(entry) {
            out.push((account, value));
        }
        entry.key.clear();
        entry.access_token.clear();
    }
    out
}

pub fn take_config_secrets(cfg: &mut AiConfig) -> Vec<(String, String)> {
    let out = vec![
        (
            APP_OPENAI_ACCOUNT.to_string(),
            cfg.openai_key.trim().to_string(),
        ),
        (
            APP_ANTHROPIC_ACCOUNT.to_string(),
            cfg.anthropic_key.trim().to_string(),
        ),
    ];
    cfg.openai_key.clear();
    cfg.anthropic_key.clear();
    out
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub moved: usize,
    pub backup: Option<PathBuf>,
}

impl Report {
    pub fn ran(&self) -> bool {
        self.backup.is_some()
    }
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(".bak");
    PathBuf::from(name)
}

fn write_backup(path: &Path) -> Result<PathBuf, String> {
    let target = backup_path(path);
    let bytes = std::fs::read(path)
        .map_err(|e| format!("ai-keys: could not read {}: {}", path.display(), e))?;
    secrets::write_private(&target, &bytes)?;
    Ok(target)
}

pub fn migrate_vault_file(path: &Path) -> Report {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Report::default();
    };
    let Ok(mut entries) = serde_json::from_str::<Vec<KeyEntry>>(&text) else {
        return Report::default();
    };
    if !entries.iter().any(entry_has_plaintext) {
        return Report::default();
    }
    let pairs = take_secrets(&mut entries);
    let mut moved = 0usize;
    for (account, value) in pairs {
        if value.is_empty() {
            continue;
        }
        match secrets::set(AI_KEYS, &account, &value) {
            Ok(()) => moved += 1,
            Err(e) => {
                tracing::warn!("[ai-keys] could not move {} into the store: {}", account, e);
                return Report::default();
            }
        }
    }
    let backup = match write_backup(path) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("[ai-keys] backup failed, keeping the file as it is: {}", e);
            return Report::default();
        }
    };
    let Ok(json) = serde_json::to_string_pretty(&entries) else {
        return Report::default();
    };
    if let Err(e) = secrets::write_private(path, json.as_bytes()) {
        tracing::warn!("[ai-keys] could not rewrite the vault: {}", e);
        return Report::default();
    }
    tracing::info!(
        "[ai-keys] moved {} key(s) out of {} into the secret store (backup at {})",
        moved,
        path.display(),
        backup.display()
    );
    Report {
        moved,
        backup: Some(backup),
    }
}

pub fn migrate_config_file(path: &Path, cfg: &mut AiConfig) -> Report {
    if cfg.openai_key.trim().is_empty() && cfg.anthropic_key.trim().is_empty() {
        return Report::default();
    }
    let pairs = take_config_secrets(cfg);
    let mut moved = 0usize;
    for (account, value) in pairs {
        if value.is_empty() {
            continue;
        }
        match secrets::set(AI_KEYS, &account, &value) {
            Ok(()) => moved += 1,
            Err(e) => {
                tracing::warn!("[ai] could not move {} into the store: {}", account, e);
                return Report::default();
            }
        }
    }
    let backup = match write_backup(path) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("[ai] backup failed, keeping the file as it is: {}", e);
            return Report::default();
        }
    };
    let Ok(json) = serde_json::to_string_pretty(&*cfg) else {
        return Report::default();
    };
    if let Err(e) = secrets::write_private(path, json.as_bytes()) {
        tracing::warn!("[ai] could not rewrite the config: {}", e);
        return Report::default();
    }
    tracing::info!(
        "[ai] moved {} key(s) out of {} into the secret store (backup at {})",
        moved,
        path.display(),
        backup.display()
    );
    Report {
        moved,
        backup: Some(backup),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_carry_the_provider_and_the_entry() {
        assert_eq!(key_account("openai", "abc"), "openai:abc");
        assert_eq!(token_account("newapi", "abc"), "newapi:abc#access_token");
    }

    #[test]
    fn taking_secrets_blanks_the_entry() {
        let mut entries = vec![KeyEntry {
            id: "e1".into(),
            kind: "openai".into(),
            key: " sk-live ".into(),
            access_token: String::new(),
            ..Default::default()
        }];
        assert!(entry_has_plaintext(&entries[0]));
        let pairs = take_secrets(&mut entries);
        assert_eq!(pairs[0], ("openai:e1".to_string(), "sk-live".to_string()));
        assert_eq!(pairs[1].1, "");
        assert!(entries[0].key.is_empty());
        assert!(!entry_has_plaintext(&entries[0]));
        let json = serde_json::to_string(&entries).unwrap();
        assert!(!json.contains("sk-live"), "{json}");
    }

    #[test]
    fn the_backup_sits_next_to_the_file() {
        assert_eq!(
            backup_path(Path::new("/a/b/ai-keys.json")),
            PathBuf::from("/a/b/ai-keys.json.bak")
        );
    }
}
