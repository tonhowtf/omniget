use std::path::{Path, PathBuf};

use base64::Engine as _;
use omniget_core::core::canvas::model::{self as cmodel, WorkspaceSummary};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::AppHandle;

use crate::canvas::doc::host;

pub const AGENT_FILES: [&str; 3] = ["CLAUDE.md", "AGENTS.md", "GEMINI.md"];

pub fn install() {
    cmodel::set_summary_enricher(enrich);
}

fn enrich(s: &mut WorkspaceSummary) {
    let term = crate::pty::term();
    let mut any = false;
    for t in s.terminals.iter_mut() {
        if let Some(id) = t.terminal_id.as_deref() {
            if term.info(id).is_some_and(|i| i.alive) {
                t.running = true;
                any = true;
            }
        }
    }
    s.loaded = Some(any);
    if let Some(p) = s.workspace.icon.as_deref().and_then(icon_file) {
        s.icon_revision = file_revision(&p);
    }
    if s.workspace.sync_agent_files {
        if let Some(dir) = s.workspace.working_directory.as_deref() {
            s.agent_files = Some(agent_status(Path::new(dir)));
        }
    }
}

fn icon_file(icon: &str) -> Option<PathBuf> {
    let p = icon.strip_prefix("file:").unwrap_or(icon).trim();
    let lower = p.to_ascii_lowercase();
    let image = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".ico"]
        .iter()
        .any(|e| lower.ends_with(e));
    (Path::new(p).is_absolute() && (icon.starts_with("file:") || image)).then(|| PathBuf::from(p))
}

fn file_revision(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    let t = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Some(format!("{t:x}-{:x}", m.len()))
}

fn mime_of(p: &Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        _ => "image/png",
    }
}

pub type IconBytes = (Vec<u8>, &'static str, String);

pub fn icon_bytes(workspace_id: &str) -> Result<Option<IconBytes>, String> {
    let ws = host().store().workspace(workspace_id)?;
    let Some(p) = ws.icon.as_deref().and_then(icon_file) else {
        return Ok(None);
    };
    let meta = std::fs::metadata(&p)
        .map_err(|e| format!("CANVAS_NOT_FOUND: icon {}: {e}", p.display()))?;
    if meta.len() > 4 * 1024 * 1024 {
        return Err("CANVAS_INVALID: icon larger than 4 MB".into());
    }
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    let rev = file_revision(&p).unwrap_or_default();
    Ok(Some((bytes, mime_of(&p), rev)))
}

#[tauri::command]
pub async fn canvas_workspace_icon(
    app: AppHandle,
    workspace_id: String,
) -> Result<Option<Value>, String> {
    host().bind(&app);
    tauri::async_runtime::spawn_blocking(move || {
        Ok(icon_bytes(&workspace_id)?.map(|(bytes, mime, rev)| {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            json!({ "dataUrl": format!("data:{mime};base64,{b64}"), "revision": rev })
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn read_opt(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn write_atomic(p: &Path, text: &str) -> Result<(), String> {
    let tmp = p.with_extension("md.omniget-tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", p.display()))?;
    std::fs::rename(&tmp, p).map_err(|e| format!("{}: {e}", p.display()))
}

pub fn agent_status(dir: &Path) -> Value {
    let files: Vec<(String, Option<String>)> = AGENT_FILES
        .iter()
        .map(|n| (n.to_string(), read_opt(&dir.join(n))))
        .collect();
    let reference = files.iter().find_map(|(_, c)| c.clone());
    let divergent: Vec<&str> = files
        .iter()
        .filter(|(_, c)| {
            c.as_ref().map(|c| c.trim_end()) != reference.as_ref().map(|r| r.trim_end())
        })
        .map(|(n, _)| n.as_str())
        .collect();
    json!({
        "synced": reference.is_some() && divergent.is_empty(),
        "divergent": divergent,
        "files": files.iter().map(|(n, c)| json!({
            "name": n,
            "exists": c.is_some(),
            "lines": c.as_ref().map(|c| c.lines().count()),
        })).collect::<Vec<_>>(),
    })
}

pub fn write_instructions(dir: &Path, text: &str, sync: bool) -> Result<Vec<String>, String> {
    if !dir.is_dir() {
        return Err(format!(
            "CANVAS_INVALID: working folder {} does not exist",
            dir.display()
        ));
    }
    let mut written = Vec::new();
    for n in AGENT_FILES.iter().take(if sync { 3 } else { 1 }) {
        write_atomic(&dir.join(n), text)?;
        written.push(n.to_string());
    }
    Ok(written)
}

pub fn resolve_agent_files(dir: &Path, winner: &str, confirm: bool) -> Result<Value, String> {
    let Some(win) = AGENT_FILES
        .iter()
        .find(|n| n.eq_ignore_ascii_case(winner.trim()))
    else {
        return Err(format!(
            "CANVAS_INVALID: '{winner}' is not one of {}",
            AGENT_FILES.join(", ")
        ));
    };
    let text = read_opt(&dir.join(win)).ok_or_else(|| {
        format!(
            "CANVAS_NOT_FOUND: {win} does not exist in {}",
            dir.display()
        )
    })?;
    let keep: std::collections::HashSet<&str> = text.lines().collect();
    let mut changes = Vec::new();
    for n in AGENT_FILES.iter().filter(|n| *n != win) {
        let old = read_opt(&dir.join(n)).unwrap_or_default();
        if old.trim_end() == text.trim_end() {
            continue;
        }
        let had: std::collections::HashSet<&str> = old.lines().collect();
        let lost: Vec<&str> = old
            .lines()
            .filter(|l| !l.trim().is_empty() && !keep.contains(l))
            .collect();
        let added = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !had.contains(l))
            .count();
        changes.push(json!({
            "file": n,
            "linesLost": lost.len(),
            "linesAdded": added,
            "lost": lost.iter().take(20).collect::<Vec<_>>(),
        }));
    }
    if confirm {
        for n in AGENT_FILES.iter().filter(|n| *n != win) {
            write_atomic(&dir.join(n), &text)?;
        }
    }
    Ok(json!({
        "winner": win,
        "applied": confirm,
        "changes": changes,
        "status": agent_status(dir),
    }))
}

#[derive(Default)]
pub struct AgentPatch {
    instructions: Option<String>,
    resolve: Option<String>,
    confirm: bool,
}

pub fn take_agent_patch(ws: &str, patch: &mut Map<String, Value>) -> Result<AgentPatch, String> {
    let mut out = AgentPatch::default();
    if let Some(v) = patch.remove("instructions") {
        out.instructions = Some(v.as_str().unwrap_or_default().to_string());
    }
    out.confirm = patch
        .remove("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if let Some(Value::String(s)) = patch.get("syncAgentFiles").cloned() {
        let winner = s.strip_prefix("resolve:").unwrap_or(&s).to_string();
        out.resolve = Some(winner);
        patch.insert("syncAgentFiles".into(), Value::Bool(true));
    }
    if let Some(Value::String(icon)) = patch.get("icon").cloned() {
        if let Some(e) = icon.strip_prefix("emoji:") {
            patch.insert("icon".into(), Value::String(e.trim().to_string()));
        }
    }
    if let Some(v) = patch.get("jumpNumber").cloned() {
        if !v.is_null() {
            let n = v
                .as_u64()
                .filter(|n| (1..=9).contains(n))
                .ok_or("CANVAS_INVALID: jumpNumber must be 1 to 9")?;
            let store = host().store().clone();
            for w in store.workspaces() {
                if w.workspace.id != ws && w.workspace.jump_number == Some(n as u8) {
                    let mut p = Map::new();
                    p.insert("jumpNumber".into(), Value::Null);
                    store.update_workspace(&w.workspace.id, p)?;
                }
            }
        }
    }
    Ok(out)
}

pub fn run_agent_patch(ws: &cmodel::Workspace, work: AgentPatch) -> Result<Option<Value>, String> {
    if work.instructions.is_none() && work.resolve.is_none() {
        return Ok(None);
    }
    let dir = ws
        .working_directory
        .as_deref()
        .ok_or("CANVAS_INVALID: the workspace has no working folder for CLAUDE.md/AGENTS.md")?;
    let dir = Path::new(dir);
    let mut out = Map::new();
    if let Some(text) = work.instructions {
        out.insert(
            "written".into(),
            json!(write_instructions(dir, &text, ws.sync_agent_files)?),
        );
    }
    if let Some(w) = work.resolve {
        out.insert(
            "resolve".into(),
            resolve_agent_files(dir, &w, work.confirm)?,
        );
    }
    out.insert("status".into(), agent_status(dir));
    Ok(Some(Value::Object(out)))
}

#[tauri::command]
pub async fn canvas_workspace_agent_files(
    app: AppHandle,
    workspace_id: String,
    resolve: Option<String>,
    confirm: Option<bool>,
) -> Result<Value, String> {
    host().bind(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let ws = host().store().workspace(&workspace_id)?;
        let Some(dir) = ws.working_directory.as_deref() else {
            return Ok(json!({ "status": null, "text": null }));
        };
        let dir = Path::new(dir);
        let resolved = match resolve {
            Some(w) => Some(resolve_agent_files(dir, &w, confirm.unwrap_or(false))?),
            None => None,
        };
        let text = AGENT_FILES.iter().find_map(|n| read_opt(&dir.join(n)));
        Ok(json!({ "status": agent_status(dir), "text": text, "resolve": resolved }))
    })
    .await
    .map_err(|e| e.to_string())?
}

const IDES: &[(&str, &str, &str)] = &[
    ("vscode", "Visual Studio Code", "code"),
    (
        "vscode-insiders",
        "Visual Studio Code - Insiders",
        "code-insiders",
    ),
    ("cursor", "Cursor", "cursor"),
    ("windsurf", "Windsurf", "windsurf"),
    ("zed", "Zed", "zed"),
    ("idea", "IntelliJ IDEA", "idea"),
    ("webstorm", "WebStorm", "webstorm"),
    ("pycharm", "PyCharm", "pycharm"),
    ("goland", "GoLand", "goland"),
    ("rustrover", "RustRover", "rustrover"),
    ("xcode", "Xcode", "xed"),
    ("sublime", "Sublime Text", "subl"),
];

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            d.join(cmd).is_file() || (cfg!(windows) && d.join(format!("{cmd}.cmd")).is_file())
        })
    })
}

fn mac_app(name: &str) -> bool {
    cfg!(target_os = "macos")
        && [
            PathBuf::from("/Applications"),
            dirs::home_dir().unwrap_or_default().join("Applications"),
        ]
        .iter()
        .any(|d| d.join(format!("{name}.app")).is_dir())
}

pub fn installed_ides() -> Vec<&'static str> {
    IDES.iter()
        .filter(|(_, app, cli)| mac_app(app) || on_path(cli))
        .map(|(id, _, _)| *id)
        .collect()
}

pub fn open_in_ide(workspace_id: &str, ide: Option<&str>) -> Result<Value, String> {
    let store = host().store().clone();
    let ws = store.workspace(workspace_id)?;
    let dir = ws
        .working_directory
        .clone()
        .ok_or("CANVAS_INVALID: the workspace has no working folder")?;
    let wanted = ide
        .map(str::to_string)
        .or(ws.ide.clone())
        .or_else(|| installed_ides().first().map(|s| s.to_string()))
        .ok_or("IDE_NOT_FOUND: no known editor is installed")?;
    let (id, app, cli) = IDES
        .iter()
        .find(|(id, app, _)| id.eq_ignore_ascii_case(&wanted) || app.eq_ignore_ascii_case(&wanted))
        .copied()
        .ok_or_else(|| {
            format!(
                "CANVAS_INVALID: unknown ide '{wanted}' (one of {})",
                IDES.iter().map(|i| i.0).collect::<Vec<_>>().join(", ")
            )
        })?;
    let spawned = if mac_app(app) {
        std::process::Command::new("open")
            .args(["-a", app, &dir])
            .spawn()
    } else if on_path(cli) {
        std::process::Command::new(cli).arg(&dir).spawn()
    } else {
        return Err(format!("IDE_NOT_FOUND: {app} is not installed"));
    };
    spawned.map_err(|e| format!("IDE_FAILED: {e}"))?;
    if ws.ide.as_deref() != Some(id) {
        let mut p = Map::new();
        p.insert("ide".into(), json!(id));
        store.update_workspace(workspace_id, p)?;
    }
    Ok(json!({ "workspaceId": workspace_id, "ide": id, "path": dir }))
}

#[tauri::command]
pub async fn canvas_workspace_open_ide(
    app: AppHandle,
    workspace_id: String,
    ide: Option<String>,
) -> Result<Value, String> {
    host().bind(&app);
    tauri::async_runtime::spawn_blocking(move || open_in_ide(&workspace_id, ide.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn canvas_ides() -> Result<Vec<&'static str>, String> {
    tauri::async_runtime::spawn_blocking(installed_ides)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    #[serde(default)]
    last_workspace_id: Option<String>,
    #[serde(default)]
    pending: Option<String>,
    #[serde(default)]
    attempts: u32,
}

fn session_path() -> PathBuf {
    host().store().root().join("session.json")
}

fn read_session() -> Session {
    std::fs::read(session_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_session(s: &Session) {
    if let Ok(b) = serde_json::to_vec_pretty(s) {
        let p = session_path();
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

const SAFE_AFTER: u32 = 2;

pub fn session_boot(preferred: Option<&str>) -> Result<Value, String> {
    let store = host().store().clone();
    let list = store.workspaces();
    let exists = |id: &str| list.iter().any(|w| w.workspace.id == id);
    let mut s = read_session();
    let target = s
        .last_workspace_id
        .clone()
        .filter(|id| exists(id))
        .or_else(|| preferred.filter(|id| exists(id)).map(str::to_string));
    if let Some(t) = target.as_deref() {
        if s.pending.as_deref() == Some(t) && s.attempts >= SAFE_AFTER {
            let other = match list.iter().find(|w| w.workspace.id != t) {
                Some(w) => w.workspace.id.clone(),
                None => {
                    store
                        .create_workspace(cmodel::WorkspaceInput {
                            name: "Home".into(),
                            ..Default::default()
                        })?
                        .id
                }
            };
            let name = list
                .iter()
                .find(|w| w.workspace.id == t)
                .map(|w| w.workspace.name.clone());
            s.pending = None;
            s.attempts = 0;
            s.last_workspace_id = Some(other.clone());
            write_session(&s);
            let label = name.clone().unwrap_or_else(|| t.to_string());
            crate::mcp::canvas_ui::record_notification(
                "canvas",
                "Safe launch",
                &format!("“{label}” failed to open twice, so another workspace opened."),
                false,
            );
            return Ok(json!({
                "workspaceId": other,
                "safeMode": true,
                "crashedWorkspaceId": t,
                "crashedName": name,
            }));
        }
        s.attempts = if s.pending.as_deref() == Some(t) {
            s.attempts + 1
        } else {
            1
        };
        s.pending = Some(t.to_string());
        write_session(&s);
        if crate::local_bridge_debug::enabled()
            && std::env::var("OMNIGET_TEST_CRASH_WORKSPACE")
                .ok()
                .as_deref()
                == Some(t)
        {
            std::process::abort();
        }
    }
    Ok(json!({ "workspaceId": target, "safeMode": false, "attempts": s.attempts }))
}

pub fn session_mark(workspace_id: &str, ok: bool) {
    let mut s = read_session();
    s.last_workspace_id = Some(workspace_id.to_string());
    if ok || s.pending.as_deref() != Some(workspace_id) {
        s.pending = None;
        s.attempts = 0;
    }
    write_session(&s);
}

#[tauri::command]
pub async fn canvas_session_boot(
    app: AppHandle,
    preferred: Option<String>,
) -> Result<Value, String> {
    host().bind(&app);
    tauri::async_runtime::spawn_blocking(move || session_boot(preferred.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn canvas_session_mark(
    app: AppHandle,
    workspace_id: String,
    ok: bool,
) -> Result<(), String> {
    host().bind(&app);
    let ws = workspace_id.clone();
    tauri::async_runtime::spawn_blocking(move || session_mark(&workspace_id, ok))
        .await
        .map_err(|e| e.to_string())?;
    if ok {
        crate::canvas::launch::wake_workspace(&ws);
    }
    Ok(())
}

fn redirect_file() -> Option<PathBuf> {
    omniget_core::core::paths::data_dir_redirect_file()
}

fn walk(root: &Path, rel: &Path, out: &mut Vec<(PathBuf, u64)>) -> std::io::Result<()> {
    for e in std::fs::read_dir(root.join(rel))? {
        let e = e?;
        let r = rel.join(e.file_name());
        let ft = e.file_type()?;
        if ft.is_dir() {
            walk(root, &r, out)?;
        } else if ft.is_file() {
            out.push((r, e.metadata()?.len()));
        }
    }
    Ok(())
}

pub fn move_data_dir(target: &str, confirm: bool) -> Result<Value, String> {
    if std::env::var_os("OMNIGET_DATA_DIR").is_some() {
        return Err("DATA_DIR_PINNED: OMNIGET_DATA_DIR is set for this process".into());
    }
    let from = omniget_core::core::paths::app_data_dir().ok_or("DATA_DIR: no data folder")?;
    let to = PathBuf::from(target.trim());
    if !to.is_absolute() {
        return Err("CANVAS_INVALID: the new data folder must be an absolute path".into());
    }
    if to.starts_with(&from) || from.starts_with(&to) {
        return Err(
            "CANVAS_INVALID: the new folder cannot be inside the current one (or contain it)"
                .into(),
        );
    }
    if to.exists() {
        let mut it = std::fs::read_dir(&to).map_err(|e| e.to_string())?;
        if it.any(|e| e.is_ok_and(|e| e.file_name() != ".DS_Store")) {
            return Err(
                "DATA_DIR_NOT_EMPTY: choose an empty folder; nothing in it was touched".into(),
            );
        }
    }
    let mut files = Vec::new();
    walk(&from, Path::new(""), &mut files).map_err(|e| e.to_string())?;
    let bytes: u64 = files.iter().map(|f| f.1).sum();
    if !confirm {
        return Ok(
            json!({ "from": from, "to": to, "files": files.len(), "bytes": bytes, "moved": false }),
        );
    }
    let _ = host().flush();
    for (rel, _) in &files {
        let dst = to.join(rel);
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::copy(from.join(rel), &dst).map_err(|e| format!("{}: {e}", rel.display()))?;
    }
    let mut bad = Vec::new();
    for (rel, _) in &files {
        let a = std::fs::metadata(from.join(rel)).map(|m| m.len()).ok();
        let b = std::fs::metadata(to.join(rel)).map(|m| m.len()).ok();
        if a.is_some() && a != b {
            bad.push(rel.display().to_string());
        }
    }
    if !bad.is_empty() {
        return Err(format!(
            "DATA_DIR_VERIFY: {} files differ after the copy ({}); the app keeps the old folder",
            bad.len(),
            bad.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let redirect = redirect_file().ok_or("DATA_DIR: no place for the pointer file")?;
    std::fs::write(&redirect, to.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    Ok(json!({
        "from": from, "to": to, "files": files.len(), "bytes": bytes, "moved": true,
        "restartRequired": true, "oldFolderKept": true,
    }))
}

#[tauri::command]
pub async fn canvas_data_dir_move(target: String, confirm: Option<bool>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || move_data_dir(&target, confirm.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())?
}
