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

pub fn default_home() -> Option<std::path::PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

pub fn session_exists_in(session_id: &str, codex_home: Option<&Path>) -> bool {
    if !super::claude::valid_session_id(session_id) {
        return false;
    }
    let root = match codex_home {
        Some(d) => Some(d.to_path_buf()),
        None => default_home(),
    };
    let Some(root) = root else { return false };
    ["sessions", "archived_sessions"]
        .iter()
        .any(|d| holds_rollout(&root.join(d), session_id, 4))
}

fn holds_rollout(dir: &Path, session_id: &str, depth: u8) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            depth > 0 && holds_rollout(&path, session_id, depth - 1)
        } else {
            name.ends_with(".jsonl") && name.contains(session_id)
        }
    })
}
