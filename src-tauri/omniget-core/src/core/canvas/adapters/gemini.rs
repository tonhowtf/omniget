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
