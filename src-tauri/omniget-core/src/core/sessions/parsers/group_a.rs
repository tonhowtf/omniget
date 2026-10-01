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
