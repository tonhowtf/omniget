//! Regras do usuario: "link deste canal -> esta pasta, nesta qualidade".
//!
//! B40. O motor de decisao (`core::rules`) existia testado e sem chamador. Aqui
//! ele ganha persistencia, comandos e o ponto onde e consultado: o enfileiramento.
//!
//! Arquivo proprio em vez de campo no `settings.json`, pelo mesmo motivo dos
//! overrides de binario (#222): nao obriga migracao para todos por causa de
//! poucos, e uma regra malformada nao pode impedir as configuracoes de carregar.

use std::path::PathBuf;

use crate::core::rules::{first_match, Rule};

const RULES_FILE: &str = "rules.json";

fn file_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(RULES_FILE))
}

/// Le as regras do disco. Arquivo ilegivel vale como "sem regras": perder as
/// regras e um incomodo, nao conseguir baixar e um problema.
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

/// Grava a lista inteira. A ordem importa: a primeira regra que casar vence, e
/// e por isso que a UI deixa reordenar em vez de so ligar e desligar.
#[tauri::command]
pub fn save_rules(rules: Vec<Rule>) -> Result<(), String> {
    for r in &rules {
        if r.name.trim().is_empty() {
            return Err("Uma regra precisa de nome para voce reconhecer depois".to_string());
        }
    }
    save_to_disk(&rules)
}

/// Qual regra casaria com esta URL, sem baixar nada.
///
/// Existe para a UI poder responder "esta regra pega o que voce espera?" antes
/// de o usuario descobrir no download errado.
#[tauri::command]
pub fn preview_rule_match(url: String, platform: Option<String>) -> Option<Rule> {
    let rules = load_rules();
    first_match(&rules, &url, platform.as_deref()).cloned()
}
