use serde_json::{json, Value};
use tauri::AppHandle;

use omniget_core::core::canvas::workspace_io::{self as wio, ExportOptions, ImportOptions};

use super::{blocking, flag, need, ready, st, to_value, ws_of};
use crate::commands::canvas::workspace_io as cmd;

pub async fn call(app: &AppHandle, name: &str, a: &Value) -> Result<Value, String> {
    let s = ready(app);
    match name {
        "workspace_export" => {
            let ws = ws_of(a);
            s.workspace(&ws)?;
            let opts = ExportOptions {
                include_secrets: flag(a, "includeSecrets").unwrap_or(false),
                include_chat: flag(a, "includeChat").unwrap_or(true),
            };
            to_value(cmd::export(s, ws, need(a, "path")?, opts).await?)
        }
        "workspace_import" => {
            let path = need(a, "path")?;
            if !flag(a, "confirm").unwrap_or(false) {
                let p = cmd::import_preview(s, path).await?;
                return Ok(json!({ "preview": p, "created": false }));
            }
            let opts = ImportOptions {
                name: st(a, "name"),
                dir: st(a, "dir"),
            };
            let r = cmd::import(s, path, opts).await?;
            Ok(json!({
                "created": true,
                "workspaceId": r.workspace_id,
                "name": r.name,
                "nodeIds": r.node_ids,
                "floors": r.floors,
                "routines": r.routines,
            }))
        }
        "workspace_backup_list" => {
            let ws = ws_of(a);
            let now = flag(a, "backupNow").unwrap_or(false);
            let s2 = s.clone();
            let ws2 = ws.clone();
            let created = if now {
                Some(blocking(move || wio::create_backup(&s2, &ws2, "manual")).await?)
            } else {
                None
            };
            let list = blocking(move || wio::list_backups(&s, &ws)).await?;
            Ok(json!({ "backups": list, "created": created }))
        }
        "workspace_backup_restore" => {
            let ws = ws_of(a);
            let id = need(a, "backupId")?;
            let (s2, ws2, id2) = (s.clone(), ws.clone(), id.clone());
            let diff = blocking(move || wio::restore_diff(&s2, &ws2, &id2)).await?;
            if !flag(a, "confirm").unwrap_or(false) {
                return Ok(json!({ "restored": false, "diff": diff }));
            }
            let w = cmd::restore(s, ws, id).await?;
            Ok(json!({ "restored": true, "diff": diff, "workspace": w }))
        }
        _ => Err("TOOL_NOT_AUTHORIZED".into()),
    }
}
