use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::text::Replacement;

pub const HISTORY_CAP: usize = 200;
pub const MAX_SECONDS_RANGE: (u32, u32) = (5, 600);

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Hold,
    Toggle,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    #[default]
    Paste,
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub shortcut: String,
    pub mode: Mode,
    pub device: Option<String>,
    pub language: String,
    pub model: String,
    pub output: Output,
    pub restore_clipboard: bool,
    pub capitalize: bool,
    pub remove_fillers: bool,
    pub replacements: Vec<Replacement>,
    pub max_seconds: u32,
    pub save_history: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            shortcut: super::shortcut::default_shortcut(cfg!(target_os = "macos")).to_string(),
            mode: Mode::Hold,
            device: None,
            language: "auto".into(),
            model: crate::core::asr::DEFAULT_MODEL.into(),
            output: Output::Paste,
            restore_clipboard: true,
            capitalize: true,
            remove_fillers: true,
            replacements: Vec::new(),
            max_seconds: 120,
            save_history: true,
        }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        self.max_seconds = self
            .max_seconds
            .clamp(MAX_SECONDS_RANGE.0, MAX_SECONDS_RANGE.1);
        self.device = self
            .device
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        let lang = self.language.trim().to_ascii_lowercase();
        self.language = if lang.is_empty() { "auto".into() } else { lang };
        if crate::core::asr::models::find(&self.model).is_none() {
            self.model = crate::core::asr::DEFAULT_MODEL.into();
        }
        self.replacements = self
            .replacements
            .into_iter()
            .map(|r| Replacement {
                find: r.find.trim().to_string(),
                replace: r.replace,
            })
            .filter(|r| !r.find.is_empty())
            .take(500)
            .collect();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub text: String,
    pub raw: String,
    pub created_at: i64,
    pub duration_ms: u64,
    pub language: String,
    pub model: String,
    pub outcome: String,
}

pub fn dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("dictation"))
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("path without a parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "data".into());
    let tmp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replacing {}", path.display()));
    }
    Ok(())
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

pub fn load_settings(dir: &Path) -> Settings {
    read_json::<Settings>(&dir.join("settings.json")).sanitized()
}

pub fn save_settings(dir: &Path, settings: &Settings) -> anyhow::Result<()> {
    write_json(&dir.join("settings.json"), settings)
}

pub fn load_history(dir: &Path) -> Vec<Entry> {
    read_json(&dir.join("history.json"))
}

pub fn push_history(dir: &Path, entry: Entry) -> anyhow::Result<Vec<Entry>> {
    let mut list = load_history(dir);
    list.retain(|e| e.id != entry.id);
    list.insert(0, entry);
    list.truncate(HISTORY_CAP);
    write_json(&dir.join("history.json"), &list)?;
    Ok(list)
}

pub fn delete_history(dir: &Path, id: &str) -> anyhow::Result<Vec<Entry>> {
    let mut list = load_history(dir);
    list.retain(|e| e.id != id);
    write_json(&dir.join("history.json"), &list)?;
    Ok(list)
}

pub fn clear_history(dir: &Path) -> anyhow::Result<()> {
    write_json(&dir.join("history.json"), &Vec::<Entry>::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            text: format!("text {id}"),
            raw: String::new(),
            created_at: 1,
            duration_ms: 1000,
            language: "en".into(),
            model: "whisper-base".into(),
            outcome: "pasted".into(),
        }
    }

    #[test]
    fn history_is_newest_first_and_capped() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..(HISTORY_CAP + 5) {
            push_history(tmp.path(), entry(&i.to_string())).unwrap();
        }
        let list = load_history(tmp.path());
        assert_eq!(list.len(), HISTORY_CAP);
        assert_eq!(list[0].id, (HISTORY_CAP + 4).to_string());
        let after = delete_history(tmp.path(), &list[0].id).unwrap();
        assert_eq!(after.len(), HISTORY_CAP - 1);
        clear_history(tmp.path()).unwrap();
        assert!(load_history(tmp.path()).is_empty());
        let leftovers = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn corrupt_files_fall_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("history.json"), b"{nope").unwrap();
        std::fs::write(tmp.path().join("settings.json"), b"[]").unwrap();
        assert!(load_history(tmp.path()).is_empty());
        assert_eq!(load_settings(tmp.path()), Settings::default().sanitized());
    }

    #[test]
    fn settings_round_trip_and_sanitize() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Settings {
            max_seconds: 9999,
            device: Some("  ".into()),
            language: " PT ".into(),
            model: "nope".into(),
            replacements: vec![
                Replacement {
                    find: "  ".into(),
                    replace: "x".into(),
                },
                Replacement {
                    find: " omniget ".into(),
                    replace: "OmniGet".into(),
                },
            ],
            mode: Mode::Toggle,
            ..Settings::default()
        };
        save_settings(tmp.path(), &s).unwrap();
        let back = load_settings(tmp.path());
        assert_eq!(back.max_seconds, 600);
        assert_eq!(back.device, None);
        assert_eq!(back.language, "pt");
        assert_eq!(back.model, crate::core::asr::DEFAULT_MODEL);
        assert_eq!(back.replacements.len(), 1);
        assert_eq!(back.replacements[0].find, "omniget");
        assert_eq!(back.mode, Mode::Toggle);
    }

    #[test]
    fn partial_settings_keep_defaults() {
        let s: Settings = serde_json::from_str(r#"{"mode":"toggle"}"#).unwrap();
        assert_eq!(s.mode, Mode::Toggle);
        assert!(s.remove_fillers);
        assert_eq!(s.max_seconds, 120);
    }
}
