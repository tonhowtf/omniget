//! Gemini CLI on the canvas: `AfterAgent` (turn end, with the response),
//! `BeforeAgent` (the prompt was taken) and `Notification` hooks in the
//! `.gemini/settings.json` of the role folder (written by the roles module).
//! The hooks run `omniget canvas hook --tool gemini`, which maps Gemini's
//! events to Claude's.

pub const HOOK_COMMAND: &str = r#""${OMNIGET_CLI:-omniget}" canvas hook --tool gemini"#;

/// Gemini event name -> Claude event name.
pub fn claude_event(native: &str) -> Option<&'static str> {
    Some(match native {
        "BeforeAgent" => "UserPromptSubmit",
        "AfterAgent" => "Stop",
        "Notification" => "Notification",
        "BeforeTool" => "PreToolUse",
        "AfterTool" => "PostToolUse",
        "SessionStart" => "SessionStart",
        "SessionEnd" => "SessionEnd",
        "PreCompress" => "PreCompact",
        _ => return None,
    })
}
