use std::path::PathBuf;
use std::sync::Arc;

use omniget_core::core::canvas::model::AgentKind;
use omniget_core::core::canvas::roles::{self, Role, RoleInfo, RoleInput, RoleLaunch, Scope};
use omniget_core::core::canvas::store::{CanvasStore, ORIGIN_USER};
use tauri::AppHandle;

fn store(app: &AppHandle) -> Arc<CanvasStore> {
    crate::canvas::chat::bind(app);
    crate::canvas::doc::host().bind(app).store().clone()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

fn scope_ws(scope: &str, ws: &str) -> Result<Option<String>, String> {
    match Scope::parse(scope) {
        Some(Scope::Workspace) => Ok(Some(ws.to_string())),
        Some(Scope::Global) => Ok(None),
        None => Err(format!("CANVAS_INVALID: scope {scope}")),
    }
}

#[tauri::command]
pub async fn canvas_roles_list(
    app: AppHandle,
    workspace_id: String,
) -> Result<Vec<RoleInfo>, String> {
    let s = store(&app);
    blocking(move || Ok(roles::list(&s, &workspace_id))).await
}

#[tauri::command]
pub async fn canvas_roles_get(
    app: AppHandle,
    workspace_id: String,
    role_id: String,
) -> Result<Option<Role>, String> {
    let s = store(&app);
    blocking(move || Ok(roles::get(&s, &workspace_id, &role_id).map(|(r, _)| r))).await
}

#[tauri::command]
pub async fn canvas_roles_save(
    app: AppHandle,
    workspace_id: String,
    scope: String,
    input: RoleInput,
) -> Result<Role, String> {
    let s = store(&app);
    blocking(move || {
        let target = scope_ws(&scope, &workspace_id)?;
        if let Some(id) = input.id.clone() {
            if let Some((_, from)) = roles::get(&s, &workspace_id, &id) {
                let wanted = if target.is_some() {
                    Scope::Workspace
                } else {
                    Scope::Global
                };
                if from != wanted {
                    roles::set_scope(&s, &workspace_id, &id, wanted)?;
                }
            }
        }
        let before = input
            .id
            .as_deref()
            .and_then(|id| roles::get(&s, &workspace_id, id))
            .map(|(r, _)| r.prompt);
        let role = roles::save(&s, target.as_deref(), input)?;
        if target.is_some() && before.as_deref().is_some_and(|p| p != role.prompt) {
            crate::canvas::roles::restart_holders(&workspace_id, &role.id);
        }
        Ok(role)
    })
    .await
}

#[tauri::command]
pub async fn canvas_roles_delete(
    app: AppHandle,
    workspace_id: String,
    role_id: String,
    scope: String,
    force: Option<bool>,
) -> Result<(), String> {
    let s = store(&app);
    blocking(move || {
        let target = scope_ws(&scope, &workspace_id)?;
        if target.is_some() {
            let used = crate::canvas::roles::nodes_using(&s, &workspace_id, &role_id).len();
            if used > 0 && !force.unwrap_or(false) {
                return Err(format!(
                    "CANVAS_CONFLICT: role in use by {used} terminal(s)"
                ));
            }
            if used > 0 {
                crate::canvas::roles::unassign_all(&workspace_id, &role_id, ORIGIN_USER)?;
            }
        }
        roles::delete(&s, target.as_deref(), &role_id)
    })
    .await
}

#[tauri::command]
pub async fn canvas_roles_assign(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    role_id: Option<String>,
) -> Result<Option<Role>, String> {
    let _ = store(&app);
    blocking(move || {
        crate::canvas::roles::assign(&workspace_id, &node_id, role_id.as_deref(), ORIGIN_USER)
    })
    .await
}

#[tauri::command]
pub async fn canvas_roles_launch(
    app: AppHandle,
    workspace_id: String,
    role_id: String,
    agent: String,
    cwd: Option<String>,
) -> Result<RoleLaunch, String> {
    let s = store(&app);
    blocking(move || {
        roles::launch_plan(
            &s,
            &workspace_id,
            &role_id,
            &AgentKind::parse(&agent),
            cwd.as_deref(),
        )
    })
    .await
}

#[tauri::command]
pub async fn canvas_roles_discover(
    app: AppHandle,
    workspace_id: String,
    dir: Option<String>,
) -> Result<Vec<Role>, String> {
    let s = store(&app);
    blocking(move || {
        let dir = match dir.filter(|d| !d.trim().is_empty()) {
            Some(d) => PathBuf::from(d),
            None => match s.workspace(&workspace_id)?.working_directory {
                Some(d) => PathBuf::from(d),
                None => return Ok(Vec::new()),
            },
        };
        Ok(roles::discover(&dir))
    })
    .await
}

#[tauri::command]
pub async fn canvas_roles_import(
    app: AppHandle,
    workspace_id: String,
    dir: Option<String>,
    names: Vec<String>,
) -> Result<Vec<Role>, String> {
    let s = store(&app);
    blocking(move || {
        let dir = match dir.filter(|d| !d.trim().is_empty()) {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(
                s.workspace(&workspace_id)?
                    .working_directory
                    .ok_or("CANVAS_INVALID: the workspace has no folder")?,
            ),
        };
        let done = roles::import_discovered(&s, &workspace_id, &dir, &names)?;
        for (r, updated) in &done {
            if *updated {
                crate::canvas::roles::restart_holders(&workspace_id, &r.id);
            }
        }
        Ok(done.into_iter().map(|(r, _)| r).collect())
    })
    .await
}
