pub mod claude;
pub mod codex;
pub mod gemini;
pub mod native;
pub mod pty;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Pty,
    Native,
    ShellOnly,
}

pub fn delivery_of(agent: &str) -> Delivery {
    match agent {
        "native" => Delivery::Native,
        "shell" | "" => Delivery::ShellOnly,
        _ => Delivery::Pty,
    }
}

pub fn delivery_for(agent: &str, own_command: bool) -> Delivery {
    match delivery_of(agent) {
        Delivery::ShellOnly if own_command => Delivery::Pty,
        d => d,
    }
}

pub fn has_turn_hooks(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "gemini" | "native")
}

pub fn bracketed(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "gemini" | "opencode")
}

pub fn reads_envelope(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "gemini" | "opencode" | "native")
}

pub fn launch_args(
    agent: &str,
    cli: &std::path::Path,
    plugin_dir: &std::path::Path,
) -> Vec<String> {
    match agent {
        "claude" => claude::launch_args(plugin_dir),
        "codex" => codex::notify_args(cli),
        _ => Vec::new(),
    }
}

pub fn agents_md_snippet() -> String {
    format!("## OmniGet canvas\n\n{}\n", claude::SKILL_BODY.trim())
}
