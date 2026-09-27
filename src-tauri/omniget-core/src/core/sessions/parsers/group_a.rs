//! Registro das fontes do grupo a (dono: worker s1-sessions): Claude Code,
//! Codex, Gemini CLI, Qwen Code, OpenCode + Kilo, Crush e Goose.

use std::path::PathBuf;

use crate::core::sessions::model::SessionSource;

#[path = "claude.rs"]
pub mod claude;
#[path = "codex.rs"]
pub mod codex;
#[path = "crush.rs"]
pub mod crush;
#[path = "gemini.rs"]
pub mod gemini;
#[path = "goose.rs"]
pub mod goose;
#[path = "opencode.rs"]
pub mod opencode;
#[path = "qwen.rs"]
pub mod qwen;

pub fn sources() -> Vec<Box<dyn SessionSource>> {
    vec![
        Box::new(claude::ClaudeSource::new()),
        Box::new(codex::CodexSource::new()),
        Box::new(gemini::GeminiSource::new()),
        Box::new(qwen::QwenSource::new()),
        Box::new(opencode::OpenCodeSource::opencode()),
        Box::new(opencode::OpenCodeSource::kilo()),
        Box::new(crush::CrushSource::new()),
        Box::new(goose::GooseSource::new()),
    ]
}

/// Contas isoladas do OmniGet para um CLI (`claude` | `codex`): o
/// `accounts.json` do `/llm` e os perfis em `<app_data>/llm/profiles/*`.
/// Só caminhos de diretório; nada de credencial é lido.
pub fn account_dirs(cli: &str) -> Vec<(String, PathBuf)> {
    use crate::core::llm::cli_runtime::accounts::{default_profile_dir, AccountStore};
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    if let Some(store) = AccountStore::default_store() {
        for a in store.list().iter() {
            if a.cli.as_str() != cli || a.config_dir.as_os_str().is_empty() {
                continue;
            }
            out.push((a.id.clone(), a.config_dir.clone()));
        }
    }
    // Perfis criados pelo OmniGet que não estão (mais) no accounts.json.
    if let Some(profiles) =
        default_profile_dir("x").and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        if let Ok(rd) = std::fs::read_dir(&profiles) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let id = e.file_name().to_string_lossy().to_string();
                if out.iter().any(|(i, d)| *i == id || *d == p) {
                    continue;
                }
                // O perfil é de um CLI só: decide pela pasta que ele tem.
                let has = match cli {
                    "claude" => p.join("projects").is_dir(),
                    "codex" => p.join("sessions").is_dir(),
                    _ => false,
                };
                if has {
                    out.push((id, p));
                }
            }
        }
    }
    out
}
