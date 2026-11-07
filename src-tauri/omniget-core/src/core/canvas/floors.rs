use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::model::{new_id, now_rfc3339, CanvasNode, FloorMeta, NodeKind, GROUND_FLOOR_ID};
use super::notes;
use super::ops::Op;
use super::persist;
use super::store::{CanvasStore, ORIGIN_SYSTEM};
use crate::core::vcs::repo::{self, GitCtx};
use crate::core::vcs::runner::{tail, Invocation};
use crate::core::vcs::worktree::{sanitize_branch_name, slug};

pub const PORT_BASE: u16 = 41000;
pub const PORT_STEP: u16 = 10;
const LAND_REF_PREFIX: &str = "refs/omniget/floors/";
const HOOK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FloorMode {
    Clone,
    Worktree,
    #[default]
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HookCommand {
    #[serde(default = "new_id")]
    pub id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookKind {
    Setup,
    Run,
    Teardown,
}

impl HookKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "setup" => Ok(Self::Setup),
            "run" => Ok(Self::Run),
            "teardown" => Ok(Self::Teardown),
            o => Err(format!(
                "CANVAS_INVALID: hook kind `{o}` (setup|run|teardown)"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FloorHooks {
    #[serde(default)]
    pub setup: Vec<HookCommand>,
    #[serde(default)]
    pub run: Vec<HookCommand>,
    #[serde(default)]
    pub teardown: Vec<HookCommand>,
    #[serde(default = "yes")]
    pub auto_run_setup: bool,
}

impl FloorHooks {
    pub fn of(&self, kind: HookKind) -> &[HookCommand] {
        match kind {
            HookKind::Setup => &self.setup,
            HookKind::Run => &self.run,
            HookKind::Teardown => &self.teardown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FloorRecord {
    pub mode: FloorMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FloorsConfig {
    #[serde(default)]
    pub hooks: FloorHooks,
    #[serde(default)]
    pub floors: BTreeMap<String, FloorRecord>,
}

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn config_path(store: &CanvasStore, ws: &str) -> PathBuf {
    store.workspace_dir(ws).join("floors.json")
}

pub fn load_config(store: &CanvasStore, ws: &str) -> FloorsConfig {
    std::fs::read(config_path(store, ws))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| FloorsConfig {
            hooks: FloorHooks {
                auto_run_setup: true,
                ..Default::default()
            },
            ..Default::default()
        })
}

pub fn edit_config<R>(
    store: &CanvasStore,
    ws: &str,
    f: impl FnOnce(&mut FloorsConfig) -> Result<R, String>,
) -> Result<R, String> {
    let _g = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = load_config(store, ws);
    let r = f(&mut cfg)?;
    let bytes = serde_json::to_vec_pretty(&cfg).map_err(|e| e.to_string())?;
    persist::write_atomic(&config_path(store, ws), &bytes)?;
    Ok(r)
}

pub fn set_hooks(store: &CanvasStore, ws: &str, hooks: FloorHooks) -> Result<FloorHooks, String> {
    store.workspace(ws)?;
    let mut hooks = hooks;
    for list in [&mut hooks.setup, &mut hooks.run, &mut hooks.teardown] {
        list.retain(|h| !h.command.trim().is_empty());
        for h in list.iter_mut() {
            if h.id.is_empty() {
                h.id = new_id();
            }
        }
    }
    edit_config(store, ws, |c| {
        c.hooks = hooks.clone();
        Ok(hooks)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorView {
    #[serde(flatten)]
    pub meta: FloorMeta,
    pub mode: FloorMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    pub is_ground: bool,
    pub node_count: usize,
    pub terminal_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty: Option<u32>,
    #[serde(default)]
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorProgress {
    pub workspace_id: String,
    pub floor_id: String,
    pub name: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub type ProgressFn<'a> = &'a (dyn Fn(FloorProgress) + Send + Sync);

fn ctx(dir: &Path) -> GitCtx {
    GitCtx::Repo {
        work_tree: dir.to_path_buf(),
    }
}

async fn git_text(dir: &Path, args: &[&str]) -> Result<String, String> {
    ctx(dir)
        .git()
        .args(args)
        .run()
        .await
        .map(|o| o.trimmed())
        .map_err(|e| e.to_string())
}

async fn git_ok(dir: &Path, args: &[&str]) -> bool {
    ctx(dir)
        .git()
        .args(args)
        .unchecked()
        .run()
        .await
        .map(|o| o.ok())
        .unwrap_or(false)
}

async fn current_branch(dir: &Path) -> Option<String> {
    let o = ctx(dir)
        .git()
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .unchecked()
        .run()
        .await
        .ok()?;
    o.ok().then(|| o.trimmed())
}

async fn branch_exists(dir: &Path, name: &str) -> bool {
    git_ok(
        dir,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )
    .await
}

async fn rev(dir: &Path, r: &str) -> Option<String> {
    let o = ctx(dir)
        .git()
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{r}^{{commit}}"))
        .unchecked()
        .run()
        .await
        .ok()?;
    o.ok().then(|| o.trimmed())
}

async fn tracked_dirty(dir: &Path) -> Vec<String> {
    let Ok(o) = ctx(dir)
        .git()
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=no"])
        .unchecked()
        .run()
        .await
    else {
        return Vec::new();
    };
    o.stdout
        .split(|b| *b == 0)
        .filter(|r| r.len() > 3)
        .map(|r| String::from_utf8_lossy(&r[3..]).into_owned())
        .collect()
}

async fn untracked(dir: &Path) -> Vec<String> {
    let Ok(o) = ctx(dir)
        .git()
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .unchecked()
        .run()
        .await
    else {
        return Vec::new();
    };
    o.stdout
        .split(|b| *b == 0)
        .filter(|r| r.starts_with(b"?? "))
        .map(|r| String::from_utf8_lossy(&r[3..]).into_owned())
        .collect()
}

pub fn branch_for(name: &str) -> String {
    let s = slug(name, 48);
    sanitize_branch_name(if s.is_empty() { "floor" } else { &s })
}

fn floor_trees_dir(store: &CanvasStore, ws: &str) -> PathBuf {
    store.workspace_dir(ws).join("floor-trees")
}

fn is_ours(store: &CanvasStore, ws: &str, path: &Path) -> bool {
    let root = floor_trees_dir(store, ws);
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    p.starts_with(&root) && p != root
}

fn ground_dir(store: &CanvasStore, ws: &str) -> Result<Option<PathBuf>, String> {
    Ok(store
        .workspace(ws)?
        .working_directory
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from))
}

fn allocate_port(store: &CanvasStore) -> Option<u16> {
    let mut used: HashSet<u16> = HashSet::new();
    for w in store.workspaces() {
        for r in load_config(store, &w.workspace.id).floors.values() {
            if let Some(p) = r.port {
                used.insert(p);
            }
        }
    }
    let mut p = PORT_BASE;
    while p < 60000 {
        if !used.contains(&p) && std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return Some(p);
        }
        p += PORT_STEP;
    }
    None
}

pub fn floor_env(store: &CanvasStore, ws: &str, floor: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let Ok(meta) = store.workspace(ws) else {
        return env;
    };
    let Some(f) = meta.floors.iter().find(|f| f.id == floor) else {
        return env;
    };
    let rec = load_config(store, ws)
        .floors
        .get(floor)
        .cloned()
        .unwrap_or_default();
    let root = rec
        .repo_root
        .clone()
        .or(meta.working_directory.clone())
        .unwrap_or_default();
    let path = f.path.clone().unwrap_or_else(|| root.clone());
    let project = Path::new(&root)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| meta.name.clone());
    let branch = f.branch.clone().unwrap_or_default();
    let pairs = [
        ("FLOOR_NAME", f.name.clone()),
        ("BRANCH_NAME", branch),
        ("FLOOR_PATH", path),
        ("ROOT_PATH", root),
        ("PROJECT_NAME", project),
    ];
    for (k, v) in pairs {
        env.insert(format!("OMNIGET_{k}"), v.clone());
        env.insert(format!("MAESTRI_{k}"), v);
    }
    env.insert("OMNIGET_FLOOR_ID".into(), f.id.clone());
    if let Some(p) = rec.port {
        env.insert("OMNIGET_FLOOR_PORT".into(), p.to_string());
        env.insert(
            "OMNIGET_FLOOR_PORT_RANGE".into(),
            format!("{p}-{}", p + PORT_STEP - 1),
        );
        env.insert("PORT".into(), p.to_string());
    }
    env
}

pub fn remap_cwd(store: &CanvasStore, ws: &str, floor: &str, cwd: Option<&str>) -> Option<String> {
    let meta = store.workspace(ws).ok()?;
    let f = meta.floors.iter().find(|f| f.id == floor)?;
    let Some(fpath) = f.path.clone().filter(|_| f.is_git_isolated) else {
        return cwd.map(str::to_string).or(meta.working_directory.clone());
    };
    let rec = load_config(store, ws)
        .floors
        .get(floor)
        .cloned()
        .unwrap_or_default();
    let Some(cwd) = cwd.filter(|c| !c.trim().is_empty()) else {
        return Some(map_under(&rec, meta.working_directory.as_deref(), &fpath).unwrap_or(fpath));
    };
    if Path::new(cwd).starts_with(&fpath) {
        return Some(cwd.to_string());
    }
    Some(map_under(&rec, Some(cwd), &fpath).unwrap_or_else(|| cwd.to_string()))
}

fn map_under(rec: &FloorRecord, path: Option<&str>, floor_path: &str) -> Option<String> {
    let path = Path::new(path?);
    let root = Path::new(rec.repo_root.as_deref()?);
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let rel = path
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .or_else(|_| canon(path).strip_prefix(canon(root)).map(Path::to_path_buf))
        .ok()?;
    Some(
        Path::new(floor_path)
            .join(rel)
            .to_string_lossy()
            .into_owned(),
    )
}

pub async fn list(store: &CanvasStore, ws: &str) -> Result<Vec<FloorView>, String> {
    let meta = store.workspace(ws)?;
    let cfg = load_config(store, ws);
    let mut out = Vec::new();
    for f in &meta.floors {
        let rec = cfg.floors.get(&f.id).cloned().unwrap_or_default();
        let (nodes, terms) = store
            .read(ws, &f.id, |d| {
                (
                    d.nodes.len(),
                    d.nodes.iter().filter(|n| n.is_terminal()).count(),
                )
            })
            .unwrap_or((0, 0));
        let mut v = FloorView {
            meta: f.clone(),
            mode: rec.mode,
            port: rec.port,
            base_branch: rec.base_branch.clone(),
            is_ground: f.id == GROUND_FLOOR_ID,
            node_count: nodes,
            terminal_count: terms,
            ahead: None,
            dirty: None,
            missing: false,
        };
        if rec.mode != FloorMode::Plain {
            if let Some(p) = f.path.as_deref().map(PathBuf::from) {
                if !p.exists() {
                    v.missing = true;
                } else {
                    v.dirty = Some(tracked_dirty(&p).await.len() as u32);
                    if let Some(base) = rec.base_commit.as_deref() {
                        v.ahead = git_text(&p, &["rev-list", "--count", &format!("{base}..HEAD")])
                            .await
                            .ok()
                            .and_then(|s| s.parse().ok());
                    }
                }
            }
        }
        out.push(v);
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFloor {
    pub name: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub existing_branch: bool,
    #[serde(default)]
    pub no_git: bool,
    #[serde(default)]
    pub copy_ground: bool,
    #[serde(default)]
    pub prefer_worktree: bool,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub run_setup: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateResult {
    pub floor: FloorView,
    #[serde(default)]
    pub setup: Vec<HookRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[cfg(target_os = "macos")]
async fn cow_clone(src: &Path, dest: &Path) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let (s, d) = (src.to_path_buf(), dest.to_path_buf());
    let direct = tokio::task::spawn_blocking(move || {
        let cs = CString::new(s.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let cd = CString::new(d.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        // SAFETY: two valid NUL-terminated paths; flags 0 = follow nothing special.
        let rc = unsafe { libc::clonefile(cs.as_ptr(), cd.as_ptr(), 0) };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    if direct.is_ok() {
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(dest);
    let o = Invocation::new("cp")
        .arg("-c")
        .arg("-R")
        .arg(src)
        .arg(dest)
        .timeout(Some(Duration::from_secs(1800)))
        .unchecked()
        .run()
        .await
        .map_err(|e| e.to_string())?;
    if o.ok() {
        Ok(())
    } else {
        let _ = std::fs::remove_dir_all(dest);
        Err(format!("clone failed: {}", o.err_text().trim()))
    }
}

#[cfg(not(target_os = "macos"))]
async fn cow_clone(_src: &Path, _dest: &Path) -> Result<(), String> {
    Err("copy-on-write clones need APFS (macOS)".into())
}

pub async fn create(
    store: &CanvasStore,
    ws: &str,
    req: CreateFloor,
    progress: ProgressFn<'_>,
) -> Result<CreateResult, String> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err("CANVAS_INVALID: floor name is empty".into());
    }
    let meta = store.workspace(ws)?;
    let floor_id = new_id();
    let say = |phase: &str, message: Option<String>| {
        progress(FloorProgress {
            workspace_id: ws.to_string(),
            floor_id: floor_id.clone(),
            name: name.clone(),
            phase: phase.to_string(),
            message,
        })
    };
    let mut warnings = Vec::new();
    let ground = ground_dir(store, ws)?;
    let repo_paths = match (&ground, req.no_git) {
        (Some(g), false) => repo::discover(g).await.ok(),
        _ => None,
    };

    let mut record = FloorRecord {
        port: allocate_port(store),
        ..Default::default()
    };
    let mut fmeta = FloorMeta {
        id: floor_id.clone(),
        name: name.clone(),
        color: req.color.clone(),
        branch: None,
        is_git_isolated: false,
        path: None,
        created_at: now_rfc3339(),
    };

    if let Some(paths) = repo_paths {
        let root = paths.root.clone();
        let branch = req
            .branch
            .as_deref()
            .map(sanitize_branch_name)
            .unwrap_or_else(|| branch_for(&name));
        let exists = branch_exists(&root, &branch).await;
        if exists && !req.existing_branch {
            return Err(format!(
                "CANVAS_CONFLICT: branch `{branch}` already exists (use the existing branch or pick another name)"
            ));
        }
        if !exists && req.existing_branch {
            return Err(format!("CANVAS_NOT_FOUND: no branch `{branch}`"));
        }
        record.repo_root = Some(root.to_string_lossy().into_owned());
        record.base_branch = current_branch(&root).await;
        record.base_commit = rev(&root, "HEAD").await;
        let repo_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".into());
        let dest = floor_trees_dir(store, ws).join(&floor_id).join(&repo_name);
        std::fs::create_dir_all(dest.parent().expect("has parent"))
            .map_err(|e| format!("CANVAS_IO: {e}"))?;

        say("clone", None);
        let mut mode = FloorMode::Worktree;
        if cfg!(target_os = "macos") && !req.prefer_worktree && !paths.linked_worktree {
            match cow_clone(&root, &dest).await {
                Ok(()) => mode = FloorMode::Clone,
                Err(e) => warnings.push(format!(
                    "copy-on-write clone failed, using a git worktree: {e}"
                )),
            }
        }
        say("branch", Some(branch.clone()));
        let made = match mode {
            FloorMode::Clone => {
                let args: Vec<&str> = if exists {
                    vec!["checkout", &branch]
                } else {
                    vec!["checkout", "-b", &branch]
                };
                let r = git_text(&dest, &args).await;
                if r.is_ok() {
                    let _ = git_ok(&dest, &["remote", "remove", "omniget-ground"]).await;
                    let _ = git_ok(
                        &dest,
                        &["remote", "add", "omniget-ground", &root.to_string_lossy()],
                    )
                    .await;
                }
                r
            }
            _ => {
                let d = dest.to_string_lossy().into_owned();
                let args: Vec<&str> = if exists {
                    vec!["worktree", "add", &d, &branch]
                } else {
                    vec!["worktree", "add", "-b", &branch, &d, "HEAD"]
                };
                git_text(&root, &args).await
            }
        };
        if let Err(e) = made {
            if mode == FloorMode::Clone && dest.exists() && is_ours(store, ws, &dest) {
                let _ = std::fs::remove_dir_all(dest.parent().expect("has parent"));
            }
            say("failed", Some(e.clone()));
            return Err(format!("CANVAS_GIT: {e}"));
        }
        record.mode = mode;
        fmeta.branch = Some(branch);
        fmeta.is_git_isolated = true;
        fmeta.path = Some(dest.to_string_lossy().into_owned());
    } else {
        record.mode = FloorMode::Plain;
        record.repo_root = ground.as_ref().map(|g| g.to_string_lossy().into_owned());
        if ground.is_some() && !req.no_git {
            warnings
                .push("the workspace folder is not a git repository: the floor shares it".into());
        }
    }

    edit_config(store, ws, |c| {
        c.floors.insert(floor_id.clone(), record.clone());
        Ok(())
    })?;
    store.create_floor(ws, fmeta.clone())?;

    if req.copy_ground {
        say("layout", None);
        if let Err(e) = copy_ground_layout(store, ws, &floor_id) {
            warnings.push(format!("ground layout not copied: {e}"));
        }
    }

    let hooks = load_config(store, ws).hooks;
    let mut setup = Vec::new();
    if req.run_setup.unwrap_or(hooks.auto_run_setup) && !hooks.setup.is_empty() {
        say("setup", None);
        setup = run_hooks(store, ws, &floor_id, HookKind::Setup, None).await?;
    }
    say("done", None);
    let floor = list(store, ws)
        .await?
        .into_iter()
        .find(|f| f.meta.id == floor_id)
        .ok_or("CANVAS_NOT_FOUND: floor vanished")?;
    let _ = meta;
    Ok(CreateResult {
        floor,
        setup,
        warnings,
    })
}

pub fn copy_ground_layout(store: &CanvasStore, ws: &str, floor: &str) -> Result<usize, String> {
    let ground = store.read(ws, GROUND_FLOOR_ID, |d| d.clone())?;
    let keep = |n: &CanvasNode| {
        matches!(
            n.kind,
            NodeKind::Note
                | NodeKind::Terminal
                | NodeKind::Agent
                | NodeKind::Text
                | NodeKind::FileTree
        ) && n.stack_id.is_none()
    };
    let moved = |p: &str| remap_cwd(store, ws, floor, Some(p)).unwrap_or_else(|| p.to_string());
    let mut ids = BTreeMap::new();
    let mut ops = Vec::new();
    let mut notes_to_copy = Vec::new();
    for n in ground.nodes.iter().filter(|n| keep(n)) {
        let mut c = n.clone();
        c.id = new_id();
        c.group_id = None;
        if let Some(data) = c.data.as_object_mut() {
            match c.kind {
                NodeKind::Terminal | NodeKind::Agent => {
                    data.insert("terminalId".into(), Value::String(new_id()));
                    for k in ["sessionId", "resumeSessionId", "codexSessionId"] {
                        data.remove(k);
                    }
                    let cwd = data.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
                    match remap_cwd(store, ws, floor, cwd.as_deref()) {
                        Some(p) => {
                            data.insert("cwd".into(), Value::String(p));
                        }
                        None => {
                            data.remove("cwd");
                        }
                    }
                }
                NodeKind::FileTree => {
                    for key in ["rootPath", "currentPath"] {
                        if let Some(p) = data.get(key).and_then(|v| v.as_str()).map(str::to_string)
                        {
                            data.insert(key.into(), Value::String(moved(&p)));
                        }
                    }
                    for key in ["pinnedPaths", "expanded"] {
                        if let Some(list) = data.get_mut(key).and_then(|v| v.as_array_mut()) {
                            for v in list.iter_mut() {
                                if let Some(p) = v.as_str().map(str::to_string) {
                                    *v = Value::String(moved(&p));
                                }
                            }
                        }
                    }
                }
                NodeKind::Note => {
                    let external = data
                        .get("isExternal")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !external {
                        data.remove("fileName");
                        notes_to_copy.push((n.id.clone(), c.id.clone()));
                    }
                }
                _ => {}
            }
        }
        ids.insert(n.id.clone(), c.id.clone());
        ops.push(Op::AddNode { node: c });
    }
    for con in &ground.connections {
        let (Some(from), Some(to)) = (ids.get(&con.from), ids.get(&con.to)) else {
            continue;
        };
        let mut c = con.clone();
        c.id = new_id();
        c.from = from.clone();
        c.to = to.clone();
        c.points.clear();
        c.is_active = None;
        c.from_note_id = None;
        c.to_note_id = None;
        ops.push(Op::AddConnection { connection: c });
    }
    let count = ids.len();
    if ops.is_empty() {
        return Ok(0);
    }
    store.apply(ws, floor, ops, ORIGIN_SYSTEM, None)?;
    for (old, new) in notes_to_copy {
        if let Ok(c) = notes::read(store, ws, GROUND_FLOOR_ID, &old) {
            if !c.text.is_empty() {
                let _ = notes::write(store, ws, floor, &new, &c.text, None, ORIGIN_SYSTEM);
            }
        }
    }
    Ok(count)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookRun {
    pub id: String,
    pub kind: HookKind,
    pub command: String,
    pub code: Option<i32>,
    pub ok: bool,
    pub output: String,
    pub duration_ms: u64,
}

fn floor_dir(store: &CanvasStore, ws: &str, floor: &str) -> Result<PathBuf, String> {
    let meta = store.workspace(ws)?;
    let f = meta
        .floors
        .iter()
        .find(|f| f.id == floor)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: floor {floor}"))?;
    f.path
        .clone()
        .or(meta.working_directory.clone())
        .map(PathBuf::from)
        .ok_or_else(|| "CANVAS_INVALID: the workspace has no folder".to_string())
}

pub async fn run_shell(
    dir: &Path,
    command: &str,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<(Option<i32>, String), String> {
    let mut inv = if cfg!(windows) {
        Invocation::new("cmd").arg("/C").arg(command)
    } else {
        Invocation::new("sh").arg("-c").arg(command)
    };
    inv = inv
        .cwd(dir)
        .timeout(Some(timeout))
        .max_output(4 * 1024 * 1024)
        .unchecked();
    inv = inv.env_remove("LC_ALL").env_remove("LANG");
    for (k, v) in env {
        inv = inv.env(k, v);
    }
    let o = inv.run().await.map_err(|e| e.to_string())?;
    let mut text = o.text();
    let err = o.err_text();
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
    }
    Ok((o.code, text))
}

pub async fn run_hooks(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    kind: HookKind,
    only: Option<&str>,
) -> Result<Vec<HookRun>, String> {
    let dir = floor_dir(store, ws, floor)?;
    let env = floor_env(store, ws, floor);
    let hooks = load_config(store, ws).hooks;
    let mut out = Vec::new();
    for h in hooks.of(kind) {
        if only.is_some_and(|o| o != h.id) || (only.is_none() && !h.enabled) {
            continue;
        }
        let t0 = std::time::Instant::now();
        let (code, output, ok) = match run_shell(&dir, &h.command, &env, HOOK_TIMEOUT).await {
            Ok((code, text)) => (code, tail(&text, 60, 8000), code == Some(0)),
            Err(e) => (None, e, false),
        };
        out.push(HookRun {
            id: h.id.clone(),
            kind,
            command: h.command.clone(),
            code,
            ok,
            output,
            duration_ms: t0.elapsed().as_millis() as u64,
        });
        if !ok {
            break;
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub path: String,
    pub added: Option<u32>,
    pub deleted: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LandPreview {
    pub floor_branch: String,
    pub target: String,
    pub creates_target: bool,
    pub up_to_date: bool,
    pub fast_forward: bool,
    pub commits: Vec<String>,
    pub files: Vec<FileStat>,
    pub conflicts: Vec<String>,
    pub floor_dirty: Vec<String>,
    #[serde(default)]
    pub floor_untracked: Vec<String>,
    pub ground_dirty: Vec<String>,
    pub target_checked_out: bool,
    pub branches: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

struct LandCtx {
    ground: PathBuf,
    floor_path: PathBuf,
    branch: String,
    floor_ref: String,
    name: String,
    rec: FloorRecord,
}

async fn land_ctx(store: &CanvasStore, ws: &str, floor: &str) -> Result<LandCtx, String> {
    if floor == GROUND_FLOOR_ID {
        return Err("CANVAS_INVALID: the ground floor cannot be landed".into());
    }
    let meta = store.workspace(ws)?;
    let f = meta
        .floors
        .iter()
        .find(|f| f.id == floor)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: floor {floor}"))?;
    let rec = load_config(store, ws)
        .floors
        .get(floor)
        .cloned()
        .unwrap_or_default();
    if rec.mode == FloorMode::Plain || !f.is_git_isolated {
        return Err(
            "CANVAS_INVALID: this floor shares the ground's folder; there is nothing to land"
                .into(),
        );
    }
    let ground = PathBuf::from(
        rec.repo_root
            .clone()
            .ok_or("CANVAS_INVALID: floor has no repository")?,
    );
    let floor_path = PathBuf::from(
        f.path
            .clone()
            .ok_or("CANVAS_INVALID: floor has no folder")?,
    );
    let branch = f
        .branch
        .clone()
        .ok_or("CANVAS_INVALID: floor has no branch")?;
    let floor_ref = match rec.mode {
        FloorMode::Clone => format!("{LAND_REF_PREFIX}{floor}"),
        _ => format!("refs/heads/{branch}"),
    };
    Ok(LandCtx {
        ground,
        floor_path,
        branch,
        floor_ref,
        name: f.name.clone(),
        rec,
    })
}

async fn checked_out(ground: &Path) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    let Ok(text) = git_text(ground, &["worktree", "list", "--porcelain"]).await else {
        return out;
    };
    let mut path: Option<PathBuf> = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            if let Some(p) = &path {
                out.insert(b.to_string(), p.clone());
            }
        }
    }
    out
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let ca = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

pub fn parse_merge_tree(out: &str) -> (String, Vec<String>) {
    let mut lines = out.lines();
    let tree = lines.next().unwrap_or("").trim().to_string();
    let mut seen = HashSet::new();
    let files = lines
        .take_while(|l| !l.is_empty())
        .filter(|l| seen.insert(l.to_string()))
        .map(str::to_string)
        .collect();
    (tree, files)
}

async fn preview_inner(
    lc: &LandCtx,
    into: Option<&str>,
) -> Result<(LandPreview, Option<String>, Option<String>), String> {
    let g = &lc.ground;
    let mut p = LandPreview {
        floor_branch: lc.branch.clone(),
        ..Default::default()
    };
    if !lc.floor_path.exists() {
        return Err(format!(
            "CANVAS_NOT_FOUND: floor folder {} is gone",
            lc.floor_path.display()
        ));
    }
    p.floor_dirty = tracked_dirty(&lc.floor_path).await;
    p.floor_untracked = untracked(&lc.floor_path).await;
    if lc.rec.mode == FloorMode::Clone {
        git_text(
            g,
            &[
                "fetch",
                "--no-tags",
                "--quiet",
                &lc.floor_path.to_string_lossy(),
                &format!("+refs/heads/{}:{}", lc.branch, lc.floor_ref),
            ],
        )
        .await
        .map_err(|e| format!("CANVAS_GIT: fetch from the floor failed: {e}"))?;
    }
    let tip = rev(g, &lc.floor_ref)
        .await
        .ok_or_else(|| format!("CANVAS_GIT: floor branch {} has no commit", lc.branch))?;
    p.branches = git_text(
        g,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
    .await
    .map(|t| t.lines().map(str::to_string).collect())
    .unwrap_or_default();
    let target = match into.map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => sanitize_branch_name(t),
        None => match current_branch(g).await.or(lc.rec.base_branch.clone()) {
            Some(b) => b,
            None => {
                return Err(
                    "CANVAS_INVALID: the ground is on a detached HEAD; choose a target branch"
                        .into(),
                )
            }
        },
    };
    p.target = target.clone();
    let wt = checked_out(g).await;
    if let Some(at) = wt.get(&format!("refs/heads/{target}")) {
        if same_dir(at, g) {
            p.target_checked_out = true;
            p.ground_dirty = tracked_dirty(g).await;
        } else if !same_dir(at, &lc.floor_path) || target != lc.branch {
            p.blocker = Some(format!(
                "`{target}` is checked out in {}: switch it there first",
                at.display()
            ));
        }
    }
    let old = rev(g, &format!("refs/heads/{target}")).await;
    let range_base = old
        .clone()
        .unwrap_or_else(|| lc.rec.base_commit.clone().unwrap_or_default());
    p.creates_target = old.is_none();
    if !range_base.is_empty() {
        p.commits = git_text(
            g,
            &[
                "log",
                "--oneline",
                "--no-decorate",
                "-n",
                "50",
                &format!("{range_base}..{tip}"),
            ],
        )
        .await
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default();
        p.files = git_text(g, &["diff", "--numstat", &format!("{range_base}...{tip}")])
            .await
            .map(|t| {
                t.lines()
                    .filter_map(|l| {
                        let mut it = l.splitn(3, '\t');
                        let a = it.next()?;
                        let d = it.next()?;
                        let path = it.next()?.to_string();
                        Some(FileStat {
                            path,
                            added: a.parse().ok(),
                            deleted: d.parse().ok(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let mut tree = None;
    if let Some(old) = &old {
        p.up_to_date = git_ok(g, &["merge-base", "--is-ancestor", &tip, old]).await;
        p.fast_forward =
            !p.up_to_date && git_ok(g, &["merge-base", "--is-ancestor", old, &tip]).await;
        if !p.up_to_date && !p.fast_forward {
            let o = ctx(g)
                .git()
                .args([
                    "merge-tree",
                    "--write-tree",
                    "--name-only",
                    "--no-messages",
                    old,
                    &tip,
                ])
                .timeout(Some(Duration::from_secs(300)))
                .unchecked()
                .run()
                .await
                .map_err(|e| e.to_string())?;
            match o.code {
                Some(0) => tree = Some(parse_merge_tree(&o.text()).0),
                Some(1) => p.conflicts = parse_merge_tree(&o.text()).1,
                _ => {
                    return Err(format!(
                        "CANVAS_GIT: merge-tree failed (git 2.38+ needed): {}",
                        o.err_text().trim()
                    ))
                }
            }
        }
    } else {
        p.fast_forward = true;
    }
    if p.blocker.is_none() {
        p.blocker = if !p.floor_dirty.is_empty() {
            Some(format!(
                "the floor has uncommitted changes ({}): commit them first",
                p.floor_dirty
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        } else if !p.floor_untracked.is_empty() {
            Some(format!(
                "the floor has untracked files ({}): add and commit them, or remove them, first",
                p.floor_untracked
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        } else if !p.ground_dirty.is_empty() {
            Some(format!(
                "the ground has uncommitted changes on `{target}` ({}): commit or stash them first",
                p.ground_dirty
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        } else if !p.conflicts.is_empty() {
            Some(format!(
                "merging into `{target}` conflicts in {}: resolve it in the floor first (merge `{target}` there)",
                p.conflicts.iter().take(8).cloned().collect::<Vec<_>>().join(", ")
            ))
        } else {
            None
        };
    }
    Ok((p, old, tree.or(Some(tip))))
}

pub async fn land_preview(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    into: Option<&str>,
) -> Result<LandPreview, String> {
    let lc = land_ctx(store, ws, floor).await?;
    Ok(preview_inner(&lc, into).await?.0)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LandOptions {
    #[serde(default)]
    pub into: Option<String>,
    #[serde(default)]
    pub delete_branch: bool,
    #[serde(default)]
    pub keep_floor: bool,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LandOutcome {
    pub status: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub fast_forward: bool,
    pub commits: usize,
    pub floor_removed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teardown: Vec<HookRun>,
}

pub async fn land(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    opts: LandOptions,
) -> Result<LandOutcome, String> {
    let lc = land_ctx(store, ws, floor).await?;
    let (p, old, tree_or_tip) = preview_inner(&lc, opts.into.as_deref()).await?;
    if let Some(b) = &p.blocker {
        return Err(format!("CANVAS_LAND_BLOCKED: {b}"));
    }
    super::workspace_io::before(store, ws, "floor-land");
    let g = &lc.ground;
    let tip = rev(g, &lc.floor_ref)
        .await
        .ok_or("CANVAS_GIT: floor tip vanished")?;
    let mut out = LandOutcome {
        target: p.target.clone(),
        fast_forward: p.fast_forward,
        commits: p.commits.len(),
        ..Default::default()
    };
    if p.up_to_date {
        out.status = "up_to_date".into();
    } else {
        let new = if p.fast_forward {
            tip.clone()
        } else {
            let tree = tree_or_tip.ok_or("CANVAS_GIT: no merge tree")?;
            let msg = opts
                .message
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| {
                    format!("Land floor '{}' ({}) into {}", lc.name, lc.branch, p.target)
                });
            let old = old.clone().ok_or("CANVAS_GIT: target vanished")?;
            git_text(
                g,
                &["commit-tree", &tree, "-p", &old, "-p", &tip, "-m", &msg],
            )
            .await
            .map_err(|e| format!("CANVAS_GIT: commit-tree failed: {e}"))?
        };
        let target_ref = format!("refs/heads/{}", p.target);
        let expect = old.clone().unwrap_or_else(|| "0".repeat(40));
        git_text(
            g,
            &[
                "update-ref",
                "-m",
                "omniget: land floor",
                &target_ref,
                &new,
                &expect,
            ],
        )
        .await
        .map_err(|e| format!("CANVAS_GIT: update-ref failed: {e}"))?;
        if p.target_checked_out {
            if let Some(old) = &old {
                if let Err(e) = git_text(g, &["read-tree", "-m", "-u", old, &new]).await {
                    let _ = git_text(g, &["update-ref", &target_ref, old, &new]).await;
                    return Err(format!(
                        "CANVAS_GIT: updating the ground's files failed, target restored: {e}"
                    ));
                }
            }
        }
        out.status = "landed".into();
        out.commit = Some(new);
    }
    if lc.rec.mode == FloorMode::Clone && lc.branch != p.target && !opts.delete_branch {
        let _ = git_ok(
            g,
            &["update-ref", &format!("refs/heads/{}", lc.branch), &tip],
        )
        .await;
    }
    if lc.rec.mode == FloorMode::Clone {
        let _ = git_ok(g, &["update-ref", "-d", &lc.floor_ref]).await;
    }
    if !opts.keep_floor {
        let d = discard(
            store,
            ws,
            floor,
            DiscardOptions {
                keep_branch: !opts.delete_branch || lc.branch == p.target,
                force: false,
            },
        )
        .await?;
        out.floor_removed = true;
        out.teardown = d.teardown;
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardOptions {
    #[serde(default)]
    pub keep_branch: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiscardOutcome {
    pub removed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_kept: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_deleted: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teardown: Vec<HookRun>,
}

pub async fn discard(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    opts: DiscardOptions,
) -> Result<DiscardOutcome, String> {
    if floor == GROUND_FLOOR_ID {
        return Err("CANVAS_INVALID: the ground floor cannot be deleted".into());
    }
    super::workspace_io::before(store, ws, "floor-delete");
    let meta = store.workspace(ws)?;
    let f = meta
        .floors
        .iter()
        .find(|f| f.id == floor)
        .cloned()
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: floor {floor}"))?;
    let rec = load_config(store, ws)
        .floors
        .get(floor)
        .cloned()
        .unwrap_or_default();
    let mut out = DiscardOutcome::default();
    let path = f.path.as_deref().map(PathBuf::from);
    let alive = path.as_ref().is_some_and(|p| p.exists());

    if alive && rec.mode != FloorMode::Plain && !opts.force {
        let dirty = tracked_dirty(path.as_ref().expect("alive")).await;
        if !dirty.is_empty() {
            return Err(format!(
                "CANVAS_FLOOR_DIRTY: the floor has uncommitted changes ({}); delete with force to drop them",
                dirty.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if alive || rec.mode == FloorMode::Plain {
        out.teardown = run_hooks(store, ws, floor, HookKind::Teardown, None)
            .await
            .unwrap_or_default();
    }
    let ground = rec.repo_root.as_deref().map(PathBuf::from);
    match (rec.mode, &path, &ground, &f.branch) {
        (FloorMode::Clone, Some(p), Some(g), Some(b)) => {
            if alive && opts.keep_branch {
                let _ = git_ok(
                    g,
                    &[
                        "fetch",
                        "--no-tags",
                        "--quiet",
                        &p.to_string_lossy(),
                        &format!("+refs/heads/{b}:refs/heads/{b}"),
                    ],
                )
                .await;
            }
            if alive {
                if !is_ours(store, ws, p) {
                    return Err(format!(
                        "CANVAS_UNSAFE: {} is not a floor folder",
                        p.display()
                    ));
                }
                let dir = p
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| p.clone());
                tokio::task::spawn_blocking(move || std::fs::remove_dir_all(dir))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("CANVAS_IO: {e}"))?;
            }
            if opts.keep_branch {
                out.branch_kept = Some(b.clone());
            } else if branch_exists(g, b).await
                && current_branch(g).await.as_deref() != Some(b)
                && git_ok(g, &["branch", "-D", "--", b]).await
            {
                out.branch_deleted = Some(b.clone());
            }
            let _ = git_ok(
                g,
                &["update-ref", "-d", &format!("{LAND_REF_PREFIX}{floor}")],
            )
            .await;
        }
        (FloorMode::Worktree, Some(p), Some(g), b) => {
            if alive {
                let mut args = vec!["worktree", "remove"];
                if opts.force {
                    args.push("--force");
                }
                let ps = p.to_string_lossy().into_owned();
                args.push(&ps);
                git_text(g, &args)
                    .await
                    .map_err(|e| format!("CANVAS_GIT: {e}"))?;
            }
            let _ = git_ok(g, &["worktree", "prune"]).await;
            if let Some(b) = b {
                if opts.keep_branch {
                    out.branch_kept = Some(b.clone());
                } else if git_ok(g, &["branch", "-D", "--", b]).await {
                    out.branch_deleted = Some(b.clone());
                }
            }
            if let Some(parent) = p.parent() {
                if is_ours(store, ws, parent) {
                    let _ = std::fs::remove_dir_all(parent);
                }
            }
        }
        _ => {}
    }
    store.delete_floor(ws, floor)?;
    edit_config(store, ws, |c| {
        c.floors.remove(floor);
        Ok(())
    })?;
    out.removed = true;
    Ok(out)
}

pub fn update_meta(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    name: Option<String>,
    color: Option<Option<String>>,
) -> Result<(), String> {
    let mut patch = Map::new();
    if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
        patch.insert("name".into(), json!(n.trim()));
    }
    if let Some(c) = color {
        patch.insert("color".into(), c.map(Value::String).unwrap_or(Value::Null));
    }
    if !patch.is_empty() {
        store.update_floor(ws, floor, patch)?;
    }
    Ok(())
}
