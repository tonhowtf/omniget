use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::policy::Principal;
use omniget_core::core::washmybelly::{approvals, plan::Source, store::Ctx, tools};

pub async fn call(app: &AppHandle, p: &Principal, name: &str, a: Value) -> Result<Value, String> {
    match name {
        "wmb_execute" | "wmb_restore" => {
            let client = p.name.clone();
            let is_exec = name == "wmb_execute";
            let target = a
                .get(if is_exec { "plan_id" } else { "receipt_id" })
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if target.is_empty() {
                return Err("INVALID_ARGUMENTS".into());
            }
            let approval = tauri::async_runtime::spawn_blocking(move || {
                let ctx = Ctx::app()?;
                if is_exec {
                    approvals::request_execute(&ctx, &target, &client)
                } else {
                    approvals::request_restore(&ctx, &target, &client)
                }
            })
            .await
            .map_err(|e| e.to_string())??;
            let _ = app.emit("washmybelly-approval", &approval);
            Ok(json!({
                "status": "pending_approval",
                "approval_id": approval.id,
                "message": "Waiting for the person to approve in the OmniGet window. Poll wmb_plan_show (or wmb_receipts for a restore).",
            }))
        }
        _ => tools::call_as(Source::External, name, a).await,
    }
}

fn o(props: Value) -> Value {
    json!({ "type": "object", "properties": props })
}

fn t(kind: &str) -> Value {
    json!({ "type": kind })
}

fn plan_preview() -> Value {
    o(json!({
        "plan_id": t("string"), "kind": t("string"), "status": t("string"),
        "item_count": t("integer"), "total_bytes": t("integer"), "needs_admin": t("boolean"),
        "max_risk": t("string"), "items": t("array"), "receipt_id": {"type": ["string", "null"]},
    }))
}

pub fn output_schema(name: &str) -> Option<Value> {
    if !name.starts_with("wmb_") {
        return None;
    }
    Some(match name {
        "wmb_capabilities" => o(
            json!({ "os": t("string"), "os_version": t("string"), "arch": t("string"), "modules": t("array"), "tools": t("array"), "source_url": t("string"), "privacy_source_url": t("string") }),
        ),
        "wmb_system_status" => o(
            json!({ "score": t("integer"), "factors": t("array"), "cpu": t("object"), "memory": t("object") }),
        ),
        "wmb_sensors" => o(
            json!({ "temperatures": t("array"), "fans": t("array"), "power": t("object"), "gpu": t("array") }),
        ),
        "wmb_processes" => o(json!({ "processes": t("array"), "total": t("integer") })),
        "wmb_battery_health" => o(
            json!({ "present": t("boolean"), "cycles": t("integer"), "health_percent": t("number") }),
        ),
        "wmb_disk_volumes" => o(json!({ "volumes": t("array") })),
        "wmb_disk_usage" => {
            o(json!({ "name": t("string"), "bytes": t("integer"), "children": t("array") }))
        }
        "wmb_disk_health" => o(json!({ "disks": t("array") })),
        "wmb_find_large_files"
        | "wmb_find_empty_folders"
        | "wmb_junk_scan"
        | "wmb_dev_artifacts_scan"
        | "wmb_app_leftovers" => o(
            json!({ "items": t("array"), "total_bytes": t("integer"), "plan_id": {"type": ["string", "null"]} }),
        ),
        "wmb_find_duplicates" => o(
            json!({ "groups": t("array"), "total_bytes": t("integer"), "plan_id": {"type": ["string", "null"]} }),
        ),
        "wmb_apps_list" => {
            o(json!({ "apps": t("array"), "total": t("integer"), "partial": t("boolean") }))
        }
        "wmb_startup_list"
        | "wmb_privacy_catalog"
        | "wmb_tweaks_list"
        | "wmb_updates_list"
        | "wmb_persistence_list" => {
            o(json!({ "items": t("array"), "plan_id": {"type": ["string", "null"]} }))
        }
        "wmb_plan" | "wmb_plan_show" => plan_preview(),
        "wmb_execute" => o(
            json!({ "receipt_id": t("string"), "bytes_freed": t("integer"), "done": t("integer"), "skipped": t("integer"), "failed": t("integer"), "status": t("string"), "approval_id": t("string") }),
        ),
        "wmb_receipts" => o(json!({ "receipts": t("array") })),
        "wmb_restore" => {
            o(json!({ "receipt_id": t("string"), "items": t("array"), "status": t("string") }))
        }
        "wmb_power_status" => o(
            json!({ "profile": {"type": ["string", "null"]}, "manager": {"type": ["string", "null"]}, "conflicts": t("array"), "assertions": t("array") }),
        ),
        "wmb_power_hold" => {
            o(json!({ "held": t("boolean"), "until": {"type": ["integer", "null"]} }))
        }
        "wmb_game_mode" => o(json!({ "active": t("boolean"), "method": t("string") })),
        "wmb_perf_capabilities" => {
            o(json!({ "available": t("boolean"), "method": t("string"), "missing": t("array") }))
        }
        "wmb_perf_capture_start" | "wmb_perf_capture_stop" => {
            o(json!({ "session_id": t("string"), "status": t("string") }))
        }
        "wmb_perf_sessions" => o(json!({ "sessions": t("array") })),
        "wmb_perf_summary" => o(
            json!({ "session_id": t("string"), "fps_avg": t("number"), "low_1": t("number"), "low_01": t("number"), "stutters": t("integer") }),
        ),
        "wmb_perf_compare" => {
            o(json!({ "before": t("object"), "after": t("object"), "delta": t("object") }))
        }
        _ => o(json!({})),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_outside_caller_never_answers_its_own_approval() {
        let served = include_str!("washmybelly.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!served.contains("approvals::answer"));
        assert!(served.contains("pending_approval"));
    }
}
