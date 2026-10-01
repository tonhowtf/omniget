use std::path::Path;

pub fn notify_args(cli: &Path) -> Vec<String> {
    let argv = [
        cli.to_string_lossy().into_owned(),
        "canvas".into(),
        "hook".into(),
        "--tool".into(),
        "codex".into(),
    ];
    let arr = argv
        .iter()
        .map(|a| serde_json::to_string(a).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",");
    vec!["-c".into(), format!("notify=[{arr}]")]
}

pub fn notify_to_hook(payload: &serde_json::Value) -> Option<(String, serde_json::Value)> {
    let ty = payload.get("type").and_then(|t| t.as_str())?;
    if ty != "agent-turn-complete" {
        return None;
    }
    let msg = payload
        .get("last-assistant-message")
        .and_then(|m| m.as_str())
        .unwrap_or("");
    Some((
        "Stop".into(),
        serde_json::json!({
            "hook_event_name": "Stop",
            "last_assistant_message": msg,
            "codex": payload,
        }),
    ))
}
