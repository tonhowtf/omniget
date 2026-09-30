//! Tauri commands of floors and environments. Logic lives in
//! `omniget_core::core::canvas::{floors, envs}`; the host
//! (`crate::canvas::floors`) emits `canvas://floor-progress` and `canvas://floors`.

use std::collections::HashMap;

use omniget_core::core::canvas::envs::{self, EnvRecord, EnvTest, Wrapped};
use omniget_core::core::canvas::floors::{
    self, CreateFloor, CreateResult, DiscardOptions, DiscardOutcome, FloorHooks, FloorView,
    HookKind, HookRun, LandOptions, LandOutcome, LandPreview,
};
use omniget_core::core::canvas::store::CanvasStore;
use tauri::AppHandle;

use crate::canvas::floors as host;

fn store(app: &AppHandle) -> std::sync::Arc<CanvasStore> {
    crate::canvas::routines::start(app);
    crate::canvas::doc::host().bind(app).store().clone()
}

#[tauri::command]
pub async fn canvas_floor_list(
    app: AppHandle,
    workspace_id: String,
) -> Result<Vec<FloorView>, String> {
    let s = store(&app);
    floors::list(&s, &workspace_id).await
}

#[tauri::command]
pub async fn canvas_floor_create(
    app: AppHandle,
    workspace_id: String,
    input: CreateFloor,
) -> Result<CreateResult, String> {
    store(&app);
    host::create(&workspace_id, input).await
}

/// Floors still being made (progress) or failed (error until dismissed).
#[tauri::command]
pub fn canvas_floor_pending(app: AppHandle, workspace_id: String) -> Vec<host::PendingFloor> {
    store(&app);
    host::pending_floors(&workspace_id)
}

/// Per floor: terminals needing the person, working, alive (floors overview).
#[tauri::command]
pub fn canvas_floor_activity(app: AppHandle, workspace_id: String) -> Vec<host::FloorActivity> {
    store(&app);
    host::activity(&workspace_id)
}

/// Dismisses a pending or failed floor (by name, floor id or token).
#[tauri::command]
pub fn canvas_floor_pending_dismiss(app: AppHandle, workspace_id: String, key: String) -> bool {
    store(&app);
    host::dismiss_pending(&workspace_id, &key)
}

#[tauri::command]
pub async fn canvas_floor_land_preview(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    into: Option<String>,
) -> Result<LandPreview, String> {
    let s = store(&app);
    floors::land_preview(&s, &workspace_id, &floor_id, into.as_deref()).await
}

#[tauri::command]
pub async fn canvas_floor_land(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    options: LandOptions,
) -> Result<LandOutcome, String> {
    store(&app);
    host::land(&workspace_id, &floor_id, options).await
}

#[tauri::command]
pub async fn canvas_floor_discard(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    options: DiscardOptions,
) -> Result<DiscardOutcome, String> {
    store(&app);
    host::discard(&workspace_id, &floor_id, options).await
}

/// Runs the hooks of `kind` (`setup` | `run` | `teardown`), or one hook by id.
#[tauri::command]
pub async fn canvas_floor_run_hook(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    kind: String,
    hook_id: Option<String>,
) -> Result<Vec<HookRun>, String> {
    let s = store(&app);
    let kind = HookKind::parse(&kind)?;
    let runs = floors::run_hooks(&s, &workspace_id, &floor_id, kind, hook_id.as_deref()).await?;
    host::emit_floors(&workspace_id, Some(&floor_id), "hooks");
    Ok(runs)
}

#[tauri::command]
pub async fn canvas_floor_hooks_get(
    app: AppHandle,
    workspace_id: String,
) -> Result<FloorHooks, String> {
    let s = store(&app);
    s.workspace(&workspace_id)?;
    Ok(floors::load_config(&s, &workspace_id).hooks)
}

#[tauri::command]
pub async fn canvas_floor_hooks_set(
    app: AppHandle,
    workspace_id: String,
    hooks: FloorHooks,
) -> Result<FloorHooks, String> {
    let s = store(&app);
    floors::set_hooks(&s, &workspace_id, hooks)
}

/// Rename or recolor a floor (`color: null` clears it).
#[tauri::command]
pub async fn canvas_floor_update(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    name: Option<String>,
    color: Option<Option<String>>,
) -> Result<(), String> {
    let s = store(&app);
    floors::update_meta(&s, &workspace_id, &floor_id, name, color)
}

/// "Fetch from Source Repo": a ground branch into a cloned floor.
#[tauri::command]
pub async fn canvas_floor_fetch(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    branch: String,
) -> Result<String, String> {
    let s = store(&app);
    floors::fetch_from_ground(&s, &workspace_id, &floor_id, &branch).await
}

/// What `term_open` should get for a terminal on this floor: cwd moved into
/// the floor, floor env merged, command wrapped for a remote environment.
#[tauri::command]
pub async fn canvas_floor_spawn_context(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    cwd: Option<String>,
    command: Vec<String>,
    env: HashMap<String, String>,
) -> Result<Wrapped, String> {
    store(&app);
    Ok(host::spawn_context(
        &workspace_id,
        &floor_id,
        cwd.as_deref(),
        &command,
        &env,
    ))
}

#[tauri::command]
pub async fn canvas_env_get(app: AppHandle, workspace_id: String) -> Result<EnvRecord, String> {
    let s = store(&app);
    s.workspace(&workspace_id)?;
    Ok(envs::load(&s, &workspace_id))
}

#[tauri::command]
pub async fn canvas_env_set(
    app: AppHandle,
    workspace_id: String,
    env: EnvRecord,
) -> Result<EnvRecord, String> {
    let s = store(&app);
    envs::save(&s, &workspace_id, env)
}

#[tauri::command]
pub async fn canvas_env_test(app: AppHandle, env: EnvRecord) -> Result<EnvTest, String> {
    let s = store(&app);
    let port = crate::local_bridge::ensure_running_for_canvas(&app).ok();
    Ok(envs::test_bridge(&envs::resolve(&s, env), port).await)
}

/// Hosts of `~/.ssh/config` for the SSH picker (alias, HostName, User, Port).
#[tauri::command]
pub async fn canvas_env_ssh_hosts() -> Result<Vec<serde_json::Value>, String> {
    Ok(envs::ssh_config_hosts())
}

/// The environment a terminal node runs in.
fn node_env(s: &CanvasStore, node_id: &str) -> Result<EnvRecord, String> {
    let (ws, _floor) = s
        .locate_node(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    Ok(envs::load_for(s, &ws, Some(node_id)))
}

/// Files on the machine where a terminal runs, for the composer's `#`
/// picker. `{remote: false}` for a local terminal (the front lists the
/// local folder itself).
#[tauri::command]
pub async fn canvas_env_files(
    app: AppHandle,
    node_id: String,
    dir: Option<String>,
    query: String,
) -> Result<serde_json::Value, String> {
    let s = store(&app);
    let rec = node_env(&s, &node_id)?;
    if !rec.kind.is_remote() {
        return Ok(serde_json::json!({ "remote": false }));
    }
    let (root, hits) = envs::list_files(&rec, dir.as_deref(), &query).await?;
    Ok(serde_json::json!({
        "remote": true,
        "env": envs::target_label(&rec),
        "root": root,
        "hits": hits.into_iter().map(|(p, d)| serde_json::json!({ "path": p, "isDir": d })).collect::<Vec<_>>(),
    }))
}

/// Local files (a drop, a pasted screenshot) for a terminal: on a remote
/// one they are copied to `/tmp/omniget-drops/` there and the remote paths
/// come back; a local terminal gets its paths unchanged.
#[tauri::command]
pub async fn canvas_env_drop(
    app: AppHandle,
    node_id: String,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let s = store(&app);
    let rec = node_env(&s, &node_id)?;
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        out.push(envs::push_file(&rec, std::path::Path::new(&p)).await?);
    }
    Ok(out)
}

/// Environment switches (which kinds are offered) and custom runtimes.
#[tauri::command]
pub async fn canvas_env_providers_get(app: AppHandle) -> Result<envs::EnvProviders, String> {
    let s = store(&app);
    Ok(envs::providers(&s))
}

/// Merges `{docker: false, runtimes: {podman: {...} | null}}` into them.
#[tauri::command]
pub async fn canvas_env_providers_set(
    app: AppHandle,
    patch: serde_json::Value,
) -> Result<envs::EnvProviders, String> {
    let s = store(&app);
    envs::set_providers(&s, &patch)
}

/// Persistent terminal sessions (tmux `omniget-*`) on this machine and in
/// the remote environments in use (SSH, Docker, Sandbox, runtimes).
#[tauri::command]
pub async fn canvas_env_sessions(app: AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let s = store(&app);
    Ok(envs::sessions(&s).await)
}

/// Ends one persistent session where it runs (`env` = its target label from
/// the list); the node stays.
#[tauri::command]
pub async fn canvas_env_session_kill(
    app: AppHandle,
    session: String,
    env: Option<String>,
) -> Result<(), String> {
    let s = store(&app);
    envs::session_kill(&s, &session, env.as_deref()).await
}

/// Settings → Canvas → Skills: where the canvas skill lives for each agent.
/// `install` writes it now into every folder that is set.
#[tauri::command]
pub async fn canvas_skills_info(
    app: AppHandle,
    install: Option<bool>,
) -> Result<serde_json::Value, String> {
    let _ = crate::canvas::cords::install_plugin();
    let mut agents = serde_json::Map::new();
    for agent in ["claude", "codex", "gemini", "opencode"] {
        let root = crate::canvas::cords::skill_root(&app, agent);
        let file = if install == Some(true) {
            crate::canvas::cords::install_skill_root(&app, agent)?
        } else {
            root.as_ref()
                .map(|r| r.join("omniget-canvas").join("SKILL.md"))
                .filter(|f| f.is_file())
        };
        agents.insert(
            agent.into(),
            serde_json::json!({
                "root": root.map(|r| r.to_string_lossy().into_owned()),
                "installed": file.map(|f| f.to_string_lossy().into_owned()),
            }),
        );
    }
    Ok(serde_json::json!({
        "pluginDir": crate::canvas::cords::plugin_dir().map(|p| p.to_string_lossy().into_owned()),
        "agents": agents,
    }))
}
