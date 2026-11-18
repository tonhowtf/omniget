use omniget_core::core::quicktools::awake::{AwakeView, Mode, MODES};
use omniget_core::core::quicktools::monitor::{self, Stats};
use omniget_core::core::quicktools::snippets::Snippet;
use omniget_core::core::quicktools::volume::{self, Target};
use omniget_core::core::quicktools::{capabilities, Capabilities, QuickError};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::quick_panel::{self, ClipboardView};

pub const TOOL: &str = "quick_tools";
pub const READ_OPS: &[&str] = &["status", "snippets_list", "system_stats", "volume_list"];
const WRITE_OPS: &[&str] = &["awake_set", "snippet_save", "snippet_delete", "volume_set"];
const DEFAULT_SNIPPETS: usize = 50;
const PREVIEW_CHARS: usize = 200;

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, QuickError> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.wire())
}

fn mode(raw: &str) -> Result<Mode, QuickError> {
    Mode::parse(raw).ok_or_else(|| {
        QuickError::InvalidInput(format!("mode must be one of {}", MODES.join(", ")))
    })
}

#[derive(Serialize)]
pub struct Status {
    capabilities: Capabilities,
    awake: AwakeView,
    clipboard_enabled: bool,
    snippets: Vec<Snippet>,
}

#[tauri::command]
pub async fn quicktools_status() -> Result<Status, String> {
    blocking(|| {
        let store = quick_panel::store();
        Ok(Status {
            capabilities: capabilities(),
            awake: quick_panel::awake_view(),
            clipboard_enabled: store.clipboard_history,
            snippets: store.snippets,
        })
    })
    .await
}

#[tauri::command]
pub async fn quicktools_awake_set(mode: String) -> Result<AwakeView, String> {
    blocking(move || quick_panel::awake_set(self::mode(&mode)?)).await
}

#[tauri::command]
pub async fn quicktools_system_stats(app: AppHandle) -> Result<Stats, String> {
    let open = quick_panel::visible(&app);
    blocking(move || {
        let stats = monitor::sample();
        if !open {
            monitor::release();
        }
        Ok(stats)
    })
    .await
}

#[tauri::command]
pub async fn quicktools_volume_list() -> Result<Vec<Target>, String> {
    volume::list().await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn quicktools_volume_set(
    id: String,
    volume: Option<u8>,
    muted: Option<bool>,
) -> Result<(), String> {
    volume::set(&id, volume, muted).await.map_err(|e| e.wire())
}

#[tauri::command]
pub async fn quicktools_snippet_save(
    id: Option<u32>,
    name: String,
    text: String,
) -> Result<Vec<Snippet>, String> {
    blocking(move || {
        quick_panel::snippet_save(id, &name, &text)?;
        Ok(quick_panel::store().snippets)
    })
    .await
}

#[tauri::command]
pub async fn quicktools_snippet_delete(id: u32) -> Result<Vec<Snippet>, String> {
    blocking(move || {
        quick_panel::snippet_delete(id)?;
        Ok(quick_panel::store().snippets)
    })
    .await
}

#[tauri::command]
pub async fn quicktools_snippet_copy(app: AppHandle, id: u32) -> Result<(), String> {
    blocking(move || {
        let text = quick_panel::store().snippet(id)?.text.clone();
        quick_panel::copy_text(&app, text)
    })
    .await
}

#[tauri::command]
pub async fn quicktools_clipboard(
    app: AppHandle,
    enabled: Option<bool>,
) -> Result<ClipboardView, String> {
    blocking(move || {
        if let Some(on) = enabled {
            quick_panel::clipboard_enable(&app, on)?;
        }
        Ok(quick_panel::clipboard_view())
    })
    .await
}

#[tauri::command]
pub async fn quicktools_clipboard_act(
    app: AppHandle,
    op: String,
    id: Option<u64>,
) -> Result<ClipboardView, String> {
    blocking(move || {
        quick_panel::clipboard_act(&app, &op, id)?;
        Ok(quick_panel::clipboard_view())
    })
    .await
}

#[tauri::command]
pub async fn quicktools_open(app: AppHandle, target: String) -> Result<(), String> {
    quick_panel::open(&app, &target).map_err(|e| e.wire())
}

#[tauri::command]
pub async fn quicktools_close(app: AppHandle) -> Result<(), String> {
    quick_panel::hide(&app);
    Ok(())
}

pub fn def(can_write: bool) -> (&'static str, &'static str, Value, Value) {
    let ops: Vec<&str> = if can_write {
        READ_OPS.iter().chain(WRITE_OPS).copied().collect()
    } else {
        READ_OPS.to_vec()
    };
    (
        TOOL,
        "Everyday utilities of this computer behind one op: status (what works here and whether keep awake is on), awake_set (stop the computer from sleeping for 30m, 1h, 2h or until turned off), snippets_list, snippet_save and snippet_delete (the person's saved texts), system_stats (CPU, memory, free disk, network and the five busiest processes) and volume_list and volume_set (per app on Windows and Linux, the master output on macOS). Use it to keep the machine awake during a long task, check its load or quiet a noisy app; the clipboard history of the panel is never available here.",
        json!({
            "op": {"type": "string", "enum": ops},
            "mode": {"type": "string", "enum": MODES, "description": "awake_set: off, 30m, 1h, 2h or on (until turned off)"},
            "snippetId": {"type": "integer", "minimum": 1, "description": "snippets_list: return this one with its full text; snippet_save: edit it instead of adding; snippet_delete: the one to remove"},
            "name": {"type": "string", "minLength": 1, "maxLength": 240, "description": "snippet_save: label, up to 60 characters"},
            "text": {"type": "string", "minLength": 1, "maxLength": 16384, "description": "snippet_save: the text to keep"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "snippets_list: rows returned (default 50)"},
            "id": {"type": "string", "maxLength": 32, "description": "volume_set: id from volume_list"},
            "volume": {"type": "integer", "minimum": 0, "maximum": 100, "description": "volume_set: level in percent"},
            "muted": {"type": "boolean", "description": "volume_set: mute or unmute"},
        }),
        json!(["op"]),
    )
}

fn shown<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn required<'a>(a: &'a Value, key: &str, op: &str) -> Result<&'a Value, String> {
    match &a[key] {
        Value::Null => Err(QuickError::InvalidInput(format!("{op} needs {key}")).wire()),
        value => Ok(value),
    }
}

fn snippet_row(s: &Snippet) -> Value {
    let chars = s.text.chars().count();
    json!({
        "id": s.id,
        "name": s.name,
        "chars": chars,
        "preview": s.text.chars().take(PREVIEW_CHARS).collect::<String>(),
        "truncated": chars > PREVIEW_CHARS,
    })
}

pub async fn call(a: &Value, can_write: bool, panel_open: bool) -> Result<Value, String> {
    let op = a["op"].as_str().unwrap_or_default();
    if WRITE_OPS.contains(&op) && !can_write {
        return Err(format!(
            "TOOL_NOT_AUTHORIZED: quick_tools op={op} needs the system_change scope"
        ));
    }
    let snippet_id = a["snippetId"].as_u64().and_then(|n| u32::try_from(n).ok());
    match op {
        "status" => {
            blocking(|| {
                Ok(json!({
                    "capabilities": capabilities(),
                    "awake": quick_panel::awake_view(),
                    "snippets": quick_panel::store().snippets.len(),
                }))
            })
            .await
        }
        "awake_set" => {
            let raw = required(a, "mode", op)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            let view = blocking(move || quick_panel::awake_set(mode(&raw)?)).await?;
            Ok(json!({ "awake": view }))
        }
        "snippets_list" => {
            let limit = a["limit"].as_u64().map_or(DEFAULT_SNIPPETS, |n| n as usize);
            blocking(move || {
                let store = quick_panel::store();
                if let Some(id) = snippet_id {
                    return Ok(json!({ "snippet": store.snippet(id)? }));
                }
                let total = store.snippets.len();
                let mut out = json!({
                    "snippets": store.snippets.iter().take(limit).map(snippet_row).collect::<Vec<_>>(),
                    "total": total,
                    "truncated": total > limit,
                });
                if total == 0 {
                    out["reason"] = json!("no_snippets_saved");
                }
                Ok(out)
            })
            .await
        }
        "snippet_save" => {
            let name = required(a, "name", op)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            let text = required(a, "text", op)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            let saved =
                blocking(move || quick_panel::snippet_save(snippet_id, &name, &text)).await?;
            Ok(json!({ "saved": snippet_row(&saved) }))
        }
        "snippet_delete" => {
            required(a, "snippetId", op)?;
            let id = snippet_id.unwrap_or_default();
            blocking(move || quick_panel::snippet_delete(id)).await?;
            Ok(json!({ "deleted": id }))
        }
        "system_stats" => {
            let stats = blocking(move || {
                let stats = monitor::sample_settled();
                if !panel_open {
                    monitor::release();
                }
                Ok(stats)
            })
            .await?;
            shown(stats)
        }
        "volume_list" => {
            let kind = volume::kind();
            let targets = volume::list().await.map_err(|e| e.wire())?;
            let mut out = json!({ "kind": kind, "targets": targets, "total": targets.len() });
            if targets.is_empty() {
                out["reason"] = json!(if kind == volume::Kind::Apps {
                    "nothing_is_playing"
                } else {
                    "output_has_no_volume_control"
                });
            }
            Ok(out)
        }
        "volume_set" => {
            let id = required(a, "id", op)?.as_str().unwrap_or_default();
            let level = a["volume"].as_u64().and_then(|n| u8::try_from(n).ok());
            let muted = a["muted"].as_bool();
            volume::set(id, level, muted).await.map_err(|e| e.wire())?;
            Ok(json!({ "id": id, "volume": level, "muted": muted, "done": true }))
        }
        _ => Err(QuickError::InvalidInput(format!("unknown op \"{op}\"")).wire()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(error: &str) -> (String, Value) {
        let (code, detail) = error.split_once(": ").unwrap();
        (code.to_string(), serde_json::from_str(detail).unwrap())
    }

    #[test]
    fn the_tool_is_one_dense_definition_and_setters_need_the_write_scope() {
        let (name, description, props, required) = def(true);
        assert_eq!(name, TOOL);
        assert!(description.matches(". ").count() <= 1, "two sentences");
        assert_eq!(required, json!(["op"]));
        assert!(props["op"].get("default").is_none());
        assert_eq!(props["op"]["enum"].as_array().unwrap().len(), 8);
        let (_, _, props, _) = def(false);
        assert_eq!(props["op"]["enum"], json!(READ_OPS));
        assert!(!description.contains("clipboard_"));
    }

    #[tokio::test]
    async fn the_tool_runs_end_to_end() {
        let env = crate::secrets::test_env::EnvGuard::acquire();
        let dir = tempfile::tempdir().unwrap();
        env.set("OMNIGET_DATA_DIR", Some(dir.path().to_str().unwrap()));
        quick_panel::forget_store();

        let status = call(&json!({"op": "status"}), false, false).await.unwrap();
        assert_eq!(status["awake"]["active"], false);
        assert_eq!(status["snippets"], 0);
        assert!(status["capabilities"]["volume"].is_string());
        assert!(status.get("clipboard").is_none());

        let empty = call(&json!({"op": "snippets_list"}), false, false)
            .await
            .unwrap();
        assert_eq!(empty["total"], 0);
        assert_eq!(empty["reason"], "no_snippets_saved");

        let refused = call(
            &json!({"op": "snippet_save", "name": "Email", "text": "me@example.com"}),
            false,
            false,
        )
        .await
        .unwrap_err();
        assert!(refused.starts_with("TOOL_NOT_AUTHORIZED"), "{refused}");

        let saved = call(
            &json!({"op": "snippet_save", "name": " Email ", "text": "me@example.com"}),
            true,
            false,
        )
        .await
        .unwrap();
        assert_eq!(saved["saved"]["id"], 1);
        assert_eq!(saved["saved"]["name"], "Email");
        assert!(dir.path().join("quick-tools.json").is_file());

        let listed = call(&json!({"op": "snippets_list"}), false, false)
            .await
            .unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["snippets"][0]["preview"], "me@example.com");
        let one = call(
            &json!({"op": "snippets_list", "snippetId": 1}),
            false,
            false,
        )
        .await
        .unwrap();
        assert_eq!(one["snippet"]["text"], "me@example.com");

        let (code, error) = detail(
            &call(
                &json!({"op": "snippet_save", "name": "", "text": "x"}),
                true,
                false,
            )
            .await
            .unwrap_err(),
        );
        assert_eq!(code, "INVALID_INPUT");
        assert_eq!(error["retryable"], false);
        assert!(error["next_step"].is_string());

        let (code, _) = detail(
            &call(
                &json!({"op": "snippet_delete", "snippetId": 9}),
                true,
                false,
            )
            .await
            .unwrap_err(),
        );
        assert_eq!(code, "NOT_FOUND");
        let gone = call(
            &json!({"op": "snippet_delete", "snippetId": 1}),
            true,
            false,
        )
        .await
        .unwrap();
        assert_eq!(gone["deleted"], 1);

        let off = call(&json!({"op": "awake_set", "mode": "off"}), true, false)
            .await
            .unwrap();
        assert_eq!(off["awake"]["mode"], "off");
        let (code, _) = detail(
            &call(&json!({"op": "awake_set", "mode": "forever"}), true, false)
                .await
                .unwrap_err(),
        );
        assert_eq!(code, "INVALID_INPUT");

        let stats = call(&json!({"op": "system_stats"}), false, false)
            .await
            .unwrap();
        assert!(stats["memory_total"].as_u64().unwrap() > 0);
        assert!(stats["cpu_percent"].is_number());
        assert!(stats["top"].as_array().unwrap().len() <= 5);

        let (code, _) = detail(
            &call(&json!({"op": "volume_set", "id": "1"}), true, false)
                .await
                .unwrap_err(),
        );
        assert_eq!(code, "INVALID_INPUT");
        let (code, _) = detail(&call(&json!({"op": "nope"}), true, false).await.unwrap_err());
        assert_eq!(code, "INVALID_INPUT");
        quick_panel::forget_store();
    }
}
