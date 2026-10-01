use std::path::PathBuf;

use crate::core::rules::{first_match, Rule};

const RULES_FILE: &str = "rules.json";

fn file_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(RULES_FILE))
}

pub fn load_rules() -> Vec<Rule> {
    let Some(path) = file_path() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<Rule>>(&raw) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[rules] {} ilegivel, ignorando: {}", path.display(), e);
            Vec::new()
        }
    }
}

fn save_to_disk(rules: &[Rule]) -> Result<(), String> {
    let path = file_path().ok_or_else(|| "sem diretorio de dados".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_rules() -> Vec<Rule> {
    load_rules()
}

#[tauri::command]
pub fn save_rules(rules: Vec<Rule>) -> Result<(), String> {
    for r in &rules {
        if r.name.trim().is_empty() {
            return Err("Uma regra precisa de nome para voce reconhecer depois".to_string());
        }
    }
    save_to_disk(&rules)
}

#[tauri::command]
pub fn preview_rule_match(url: String, platform: Option<String>) -> Option<Rule> {
    let rules = load_rules();
    first_match(&rules, &url, platform.as_deref()).cloned()
}
