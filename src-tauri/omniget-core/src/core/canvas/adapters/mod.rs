//! How a message reaches each kind of node (estudo 98/03 §B.8) and what each
//! agent needs at launch to report its state and find the cords.
//!
//! - [`pty`]: any CLI in a terminal: gate, bracketed paste, Enter apart,
//!   confirmation and retry. The default.
//! - [`claude`]: `--plugin-dir <app_data>/canvas/plugin` (skill + hooks for
//!   that session only).
//! - [`codex`]: `-c notify=[...]` per process (turn complete + last message).
//! - [`gemini`]: `AfterAgent`/`Notification` hooks for `.gemini/settings.json`.
//! - [`opencode`]: its own HTTP server (prompt_async + SSE `session.idle`),
//!   a spike: delivery still goes through the PTY.
//! - [`native`]: our runtime; the message becomes a user message of the
//!   node's conversation, state and turn end come from the bus.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod native;
pub mod opencode;
pub mod pty;

/// Delivery path of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Paste into the terminal.
    Pty,
    /// A turn of our own runtime.
    Native,
    /// A plain shell: no `ask`, only `--raw`.
    ShellOnly,
}

/// The agent label of a node's data (`claude`, `codex`, `shell`, ...).
pub fn delivery_of(agent: &str) -> Delivery {
    match agent {
        "native" => Delivery::Native,
        "shell" | "" => Delivery::ShellOnly,
        _ => Delivery::Pty,
    }
}

/// Delivery path of a node from its agent label and whether it runs a
/// command of its own (a user preset or a custom CLI). The label of such a
/// node is `shell` when its program is not a known agent, but it is still a
/// program that reads prompts: it takes asks through the PTY.
pub fn delivery_for(agent: &str, own_command: bool) -> Delivery {
    match delivery_of(agent) {
        Delivery::ShellOnly if own_command => Delivery::Pty,
        d => d,
    }
}

/// Agents that report their turns through hooks (`Stop`, `notify`,
/// `AfterAgent`). Any other program's turn ends when its screen goes quiet.
pub fn has_turn_hooks(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "gemini" | "native")
}

/// Agents known to turn on bracketed paste (mode 2004) in their TUI. For the
/// others the envelope is flattened to one line before the paste, so a
/// newline is never read as Enter.
pub fn bracketed(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "gemini" | "opencode")
}

/// Agents that read the `<omniget-message>` envelope (their skill or
/// `AGENTS.md` explains it). Any other CLI gets the prompt itself, as a
/// person would type it.
pub fn reads_envelope(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "gemini" | "opencode" | "native")
}

/// Extra arguments for the agent's command line when a canvas node starts it.
/// `cli` = the multi-call `omniget` (`<app_data>/bin/omniget`), `plugin_dir`
/// = `<app_data>/canvas/plugin`.
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

/// Text for the `AGENTS.md` of a role folder (Codex, Gemini, OpenCode read
/// it; Claude gets the same rules from the plugin's skill).
pub fn agents_md_snippet() -> String {
    format!("## OmniGet canvas\n\n{}\n", claude::SKILL_BODY.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_or_custom_command_takes_asks() {
        assert_eq!(delivery_for("shell", false), Delivery::ShellOnly);
        assert_eq!(delivery_for("shell", true), Delivery::Pty);
        assert_eq!(delivery_for("", true), Delivery::Pty);
        assert_eq!(delivery_for("native", true), Delivery::Native);
        assert_eq!(delivery_for("claude", false), Delivery::Pty);
        assert!(!has_turn_hooks("shell"));
        assert!(!reads_envelope("shell"));
        assert!(reads_envelope("claude"));
    }
}
