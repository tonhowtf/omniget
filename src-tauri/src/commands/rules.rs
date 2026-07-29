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

/// O que a regra decidiu, para o enfileiramento aplicar.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RuleOutcome {
    pub rule_name: Option<String>,
    pub output_dir: Option<String>,
    pub quality: Option<String>,
    pub audio_only: Option<bool>,
    pub subtitles: Option<bool>,
    pub tags: Vec<String>,
}

/// Resolve a regra para uma URL.
///
/// Retorna vazio quando nada casa — o caminho de quem nunca criou regra nenhuma,
/// que e a maioria, e que nao pode pagar nada por isso.
pub fn resolve_for(url: &str, platform: Option<&str>) -> RuleOutcome {
    let rules = load_rules();
    match first_match(&rules, url, platform) {
        Some(r) => RuleOutcome {
            rule_name: Some(r.name.clone()),
            output_dir: r.then.output_dir.clone(),
            quality: r.then.quality.clone(),
            audio_only: r.then.audio_only,
            subtitles: r.then.subtitles,
            tags: r.then.tags.clone(),
        },
        None => RuleOutcome::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rules::{Actions, Condition};

    fn regra(nome: &str, host: &str, pasta: &str) -> Rule {
        Rule {
            enabled: true,
            name: nome.to_string(),
            when: Condition::HostIs {
                value: host.to_string(),
            },
            then: Actions {
                output_dir: Some(pasta.to_string()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn sem_regra_nada_e_decidido() {
        // O caminho da maioria dos usuarios. Precisa custar zero e nao inventar
        // pasta nenhuma.
        let out = RuleOutcome::default();
        assert!(out.rule_name.is_none());
        assert!(out.output_dir.is_none());
    }

    #[test]
    fn a_primeira_regra_que_casa_vence() {
        // A ordem e a prioridade — e por isso que a UI precisa deixar reordenar.
        let rules = vec![
            regra("primeira", "youtube.com", "/a"),
            regra("segunda", "youtube.com", "/b"),
        ];
        let m = first_match(&rules, "https://youtube.com/watch?v=x", None).expect("casa");
        assert_eq!(m.name, "primeira");
        assert_eq!(m.then.output_dir.as_deref(), Some("/a"));
    }

    #[test]
    fn regra_desligada_e_pulada() {
        let mut desligada = regra("desligada", "youtube.com", "/a");
        desligada.enabled = false;
        let rules = vec![desligada, regra("ligada", "youtube.com", "/b")];
        let m = first_match(&rules, "https://youtube.com/watch?v=x", None).expect("casa");
        assert_eq!(m.name, "ligada", "a desligada nao pode vencer");
    }

    #[test]
    fn regra_sem_nome_e_recusada_ao_salvar() {
        // Sem nome, o usuario nao consegue reconhecer a regra depois, e a lista
        // vira um monte de linhas iguais.
        let mut r = regra("  ", "youtube.com", "/a");
        r.name = "   ".to_string();
        let erro = save_rules(vec![r]).unwrap_err();
        assert!(erro.contains("nome"));
    }
}
