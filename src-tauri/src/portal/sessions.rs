use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use omniget_core::core::canvas::model::NodeKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

use super::storage::{self, Scope};

pub const KEY_PREFIX: &str = "portal:named:";
pub const EVENT_SESSIONS: &str = "canvas://portal-sessions";
const LABEL_MAX: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalSession {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub created_at: i64,
}

fn path() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("portals").join("sessions.json"))
}

fn lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(Default::default)
}

pub fn list() -> Vec<PortalSession> {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(all: &[PortalSession]) -> Result<(), String> {
    let p = path().ok_or("PORTAL_SESSION: no app data dir")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("PORTAL_SESSION: {e}"))?;
    }
    let text = serde_json::to_string_pretty(all).map_err(|e| format!("PORTAL_SESSION: {e}"))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("PORTAL_SESSION: {e}"))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("PORTAL_SESSION: {e}"))
}

pub fn get(id: &str) -> Option<PortalSession> {
    list().into_iter().find(|s| s.id == id)
}

pub fn key(id: &str) -> String {
    format!("{KEY_PREFIX}{id}")
}

pub fn session_of(data: &Value) -> Option<String> {
    data.get("session")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn session_of_node(node_id: &str) -> Option<String> {
    let store = crate::canvas::doc::host().store();
    let (ws, floor) = store.locate_node(node_id)?;
    store
        .read(&ws, &floor, |d| {
            d.node(node_id).and_then(|n| session_of(&n.data))
        })
        .ok()
        .flatten()
}

pub fn scope_for(scope: Scope, node_id: &str) -> Scope {
    let store = crate::canvas::doc::host().store();
    store
        .locate_node(node_id)
        .and_then(|(ws, floor)| {
            store
                .read(&ws, &floor, |d| d.node(node_id).map(storage::scope_of))
                .ok()
                .flatten()
        })
        .unwrap_or(scope)
}

pub fn supported() -> bool {
    !cfg!(target_os = "macos") || storage::named_stores_available()
}

pub fn check_key(key: &str) -> Result<(), String> {
    let Some(id) = key.strip_prefix(KEY_PREFIX) else {
        return Ok(());
    };
    if !supported() {
        return Err(
            "PORTAL_SESSION_UNSUPPORTED: named portal sessions need macOS 14 or later".into(),
        );
    }
    if get(id).is_none() {
        return Err(format!(
            "PORTAL_SESSION: the portal session '{id}' no longer exists; pick another session"
        ));
    }
    Ok(())
}

pub fn clean_label(
    all: &[PortalSession],
    label: &str,
    except: Option<&str>,
) -> Result<String, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("PORTAL_SESSION: the name is empty".into());
    }
    if label.chars().count() > LABEL_MAX {
        return Err(format!(
            "PORTAL_SESSION: names have at most {LABEL_MAX} characters"
        ));
    }
    if all
        .iter()
        .any(|s| Some(s.id.as_str()) != except && s.label.trim().eq_ignore_ascii_case(label))
    {
        return Err(format!(
            "PORTAL_SESSION: a session named '{label}' already exists"
        ));
    }
    Ok(label.to_string())
}

fn clean_color(c: Option<String>) -> Option<String> {
    c.map(|c| c.trim().to_string()).filter(|c| {
        c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())
    })
}

fn changed() {
    crate::canvas::chat::emit(EVENT_SESSIONS, json!({ "sessions": list() }));
}

pub fn create(label: &str, color: Option<String>) -> Result<PortalSession, String> {
    if !supported() {
        return Err(
            "PORTAL_SESSION_UNSUPPORTED: named portal sessions need macOS 14 or later".into(),
        );
    }
    let _g = lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut all = list();
    let s = PortalSession {
        id: uuid::Uuid::new_v4().to_string(),
        label: clean_label(&all, label, None)?,
        color: clean_color(color),
        created_at: chrono::Utc::now().timestamp_millis(),
    };
    all.push(s.clone());
    save(&all)?;
    drop(_g);
    changed();
    Ok(s)
}

pub fn update(
    id: &str,
    label: Option<&str>,
    color: Option<Option<String>>,
) -> Result<PortalSession, String> {
    let _g = lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut all = list();
    let label = match label {
        Some(l) => Some(clean_label(&all, l, Some(id))?),
        None => None,
    };
    let slot = all
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("PORTAL_SESSION: no session '{id}'"))?;
    if let Some(l) = label {
        slot.label = l;
    }
    if let Some(c) = color {
        slot.color = clean_color(c);
    }
    let out = slot.clone();
    save(&all)?;
    drop(_g);
    changed();
    Ok(out)
}

pub fn users(id: &str) -> Vec<Value> {
    let store = crate::canvas::doc::host().store();
    let mut out = Vec::new();
    for w in store.workspaces() {
        for f in &w.workspace.floors {
            let found = store
                .read(&w.workspace.id, &f.id, |d| {
                    d.nodes
                        .iter()
                        .filter(|n| n.kind == NodeKind::Portal)
                        .filter(|n| session_of(&n.data).as_deref() == Some(id))
                        .map(|n| {
                            json!({
                                "workspaceId": w.workspace.id,
                                "workspaceName": w.workspace.name,
                                "floorId": f.id,
                                "nodeId": n.id,
                                "title": n.title.clone().or_else(|| n.data_str("name").map(str::to_string)),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            out.extend(found);
        }
    }
    out
}

pub async fn delete(app: &AppHandle, id: &str) -> Result<Value, String> {
    let session = get(id).ok_or_else(|| format!("PORTAL_SESSION: no session '{id}'"))?;
    let portals = users(id);
    for p in &portals {
        if let Some(n) = p["nodeId"].as_str() {
            if super::host().has(n) {
                let _ = super::close(app, n);
            }
        }
    }
    let k = key(id);
    #[cfg(target_vendor = "apple")]
    if storage::named_stores_available() {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        if let Err(e) = app.remove_data_store(storage::store_id(&k)).await {
            tracing::warn!("[portal] removing the data store of session {id}: {e}");
        }
    }
    if let Some(dir) = storage::data_dir(&k) {
        let _ = std::fs::remove_dir_all(dir);
    }
    {
        let _g = lock().lock().unwrap_or_else(|e| e.into_inner());
        let all: Vec<PortalSession> = list().into_iter().filter(|s| s.id != id).collect();
        save(&all)?;
    }
    changed();
    Ok(json!({ "removed": session, "portals": portals }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, label: &str) -> PortalSession {
        PortalSession {
            id: id.into(),
            label: label.into(),
            color: None,
            created_at: 0,
        }
    }

    #[test]
    fn labels_are_trimmed_unique_and_bounded() {
        let all = vec![s("a", "Teams work"), s("b", "Google personal")];
        assert_eq!(clean_label(&all, "  New  ", None).unwrap(), "New");
        assert!(clean_label(&all, "teams WORK", None).is_err());
        assert_eq!(
            clean_label(&all, "Teams Work", Some("a")).unwrap(),
            "Teams Work"
        );
        assert!(clean_label(&all, "   ", None).is_err());
        assert!(clean_label(&all, &"x".repeat(61), None).is_err());
    }

    #[test]
    fn the_store_key_follows_the_id_not_the_label() {
        assert_eq!(key("123"), "portal:named:123");
        assert_eq!(
            session_of(&json!({"session": " 123 "})).as_deref(),
            Some("123")
        );
        assert!(session_of(&json!({"session": ""})).is_none());
        assert!(session_of(&json!({"session": null})).is_none());
    }

    #[test]
    fn colors_must_be_hex() {
        assert_eq!(
            clean_color(Some("#A1b2C3".into())).as_deref(),
            Some("#A1b2C3")
        );
        assert!(clean_color(Some("red".into())).is_none());
        assert!(clean_color(Some("#12345".into())).is_none());
    }

    #[test]
    fn old_registries_without_color_still_load() {
        let v: Vec<PortalSession> =
            serde_json::from_str(r#"[{"id":"x","label":"Work","createdAt":5}]"#).unwrap();
        assert_eq!(v[0].label, "Work");
        assert!(v[0].color.is_none());
    }
}
