//! The install plan (plan §4.4 step 1–2): every file each component would touch
//! on each target, with a unified diff, conflicts resolved by the chosen policy,
//! losses of fidelity and the commands hooks/MCP servers will run. Nothing is
//! written here; [`super::writer::apply`] replays the plan.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::convert::{ConvertCtx, Dedupe, FileAction, PatchOp, PlannedFile, Registry};
use super::edit::{self, DocFormat, Seg};
use super::lock::{self, ComponentMeta, InstallRecord, WrittenAction};
use super::model::{Compat, Component, ComponentBody, ComponentKind};
use super::targets::{self, TargetAdapter};
use super::writer;
use super::{now_iso, sha256_hex, AgentkitError, Env, Result, Scope};

/// What to do when a file or entry of the same name already exists and is not ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Leave theirs, skip ours.
    Skip,
    /// Replace theirs (a backup is kept in the transaction).
    Overwrite,
    /// Install ours under `<category>-<name>` (default, plan §4.4.5).
    #[default]
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitStatus {
    New,
    /// Installed before with other content: the old install is removed first.
    Update,
    /// Same component and content already installed: nothing to do.
    Installed,
    Unsupported,
    /// Skipped by the conflict policy.
    Conflict,
    Error,
}

/// One component on one target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUnit {
    pub unit_id: String,
    pub component: ComponentMeta,
    pub target: String,
    pub target_name: String,
    pub status: UnitStatus,
    pub compat: Compat,
    /// Name it installs under (after a collision rename).
    pub install_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    pub files: Vec<PlannedFile>,
    #[serde(default)]
    pub losses: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One path as the UI shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePlan {
    pub path: PathBuf,
    /// `create | replace | merge | unchanged`.
    pub action: String,
    pub before_sha: Option<String>,
    pub after_sha: Option<String>,
    pub diff: String,
    pub targets: Vec<String>,
    pub components: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub executable: bool,
    /// The file on disk is an older copy of ours (update): replacing it is not a collision.
    #[serde(default)]
    pub owned_by_update: bool,
    /// A folder link to this path (skill folder of a tool that does not read
    /// `.agents/skills`); `before_sha`/`after_sha` hash [`writer::link_state`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_to: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    pub id: String,
    pub created_at: String,
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<PathBuf>,
    pub policy: ConflictPolicy,
    pub units: Vec<PlanUnit>,
    pub files: Vec<FilePlan>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl InstallPlan {
    /// Anything to write at all?
    pub fn has_changes(&self) -> bool {
        self.files.iter().any(|f| f.action != "unchanged")
    }
}

/// Inputs of [`plan`].
#[derive(Debug, Clone, Default)]
pub struct PlanRequest {
    pub components: Vec<Component>,
    pub targets: Vec<String>,
    pub scope: Option<Scope>,
    pub project_dir: Option<PathBuf>,
    pub policy: ConflictPolicy,
    /// Only for tools that cannot reference an env var (written in plain text, with a warning).
    pub secret_values: BTreeMap<String, String>,
}

// ------------------------------------------------------------------ catalog resolver

type Resolver = Arc<dyn Fn(&str) -> Result<Component> + Send + Sync>;

fn resolver_cell() -> &'static RwLock<Option<Resolver>> {
    static CELL: std::sync::OnceLock<RwLock<Option<Resolver>>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| RwLock::new(None))
}

/// Registers how catalog ids (`cct:agents/dev/x`) become components. The
/// catalog module (or the app setup) calls this once.
pub fn set_catalog_resolver(f: impl Fn(&str) -> Result<Component> + Send + Sync + 'static) {
    if let Ok(mut w) = resolver_cell().write() {
        *w = Some(Arc::new(f));
    }
}

/// Resolves a catalog id, or `path:<kind>:<absolute path>` straight from disk.
pub fn resolve_id(id: &str) -> Result<Component> {
    if let Some(rest) = id.strip_prefix("path:") {
        let (kind, p) = rest
            .split_once(':')
            .ok_or_else(|| AgentkitError::new("AGENTKIT_ID", "use path:<kind>:<path>"))?;
        let kind = ComponentKind::parse(kind)
            .ok_or_else(|| AgentkitError::new("AGENTKIT_ID", format!("unknown kind `{kind}`")))?;
        return super::parse::parse_path(kind, Path::new(p));
    }
    let r = resolver_cell().read().ok().and_then(|g| g.clone());
    match r {
        Some(f) => f(id),
        None => Err(AgentkitError::new(
            "AGENTKIT_NO_CATALOG",
            format!("no catalog is loaded to resolve `{id}`"),
        )),
    }
}

/// `agent:dev/test-runner` referenced from `cct:loops/eng/x` → `cct:agents/dev/test-runner`.
fn reference_id(reference: &str, from: &Component) -> Option<String> {
    let (kind, path) = reference.split_once(':')?;
    let kind = ComponentKind::parse(kind)?;
    let source = from.id.split_once(':').map(|(s, _)| s).unwrap_or("cct");
    let plural = match kind {
        ComponentKind::Mcp => "mcps".to_string(),
        ComponentKind::Setting => "settings".to_string(),
        k => format!("{}s", k.as_str()),
    };
    Some(format!("{source}:{plural}/{path}"))
}

/// Stacks, loops and templates bring their referenced components along.
fn expand(components: Vec<Component>, warnings: &mut Vec<String>) -> Vec<Component> {
    let mut out: Vec<Component> = Vec::new();
    let mut queue: Vec<Component> = components;
    let mut seen: Vec<String> = Vec::new();
    while let Some(c) = queue.pop() {
        if seen.contains(&c.id) {
            continue;
        }
        seen.push(c.id.clone());
        let refs: Vec<String> = match &c.body {
            ComponentBody::Stack(s) => s.components.clone(),
            ComponentBody::Loop(l) => l.components.clone(),
            ComponentBody::ProjectTemplate(t) => t.components.clone(),
            _ => vec![],
        };
        for r in refs {
            let id = if r.contains(":")
                && (r.starts_with("path:")
                    || r.contains(":agents/")
                    || r.contains(":commands/")
                    || r.contains(":skills/")
                    || r.contains(":hooks/")
                    || r.contains(":mcps/")
                    || r.contains(":settings/"))
            {
                r.clone()
            } else {
                reference_id(&r, &c).unwrap_or(r.clone())
            };
            match resolve_id(&id) {
                Ok(dep) => queue.insert(0, dep),
                Err(e) => warnings.push(format!(
                    "{}: reference `{r}` not installed ({})",
                    c.name, e.message
                )),
            }
        }
        if c.kind != ComponentKind::Stack {
            out.push(c);
        }
    }
    out
}

// ------------------------------------------------------------------ planning

fn read_opt(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

fn name_candidates(c: &Component, base: &str) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(cat) = &c.category {
        let cat = super::parse::sanitize_name(cat);
        if !base.starts_with(&format!("{cat}-")) {
            v.push(format!("{cat}-{base}"));
        }
    }
    for i in 2..6 {
        v.push(format!(
            "{}-{i}",
            v.first().cloned().unwrap_or_else(|| base.to_string())
        ));
    }
    v
}

/// Does `pf` land on something that exists and is not ours?
fn collides(pf: &PlannedFile, owned: &[PathBuf]) -> bool {
    if pf.action == FileAction::Link {
        let Some(to) = &pf.link_to else {
            return false;
        };
        return match writer::link_state(&pf.path, to) {
            None => false,
            Some(s) => s != writer::link_bytes(to) && !owned.contains(&pf.path),
        };
    }
    if pf.action != FileAction::Write {
        return false;
    }
    if let Some(root) = &pf.unit_root {
        if root.exists() && !owned.iter().any(|o| o.starts_with(root)) {
            // same bytes already there → not a collision (idempotent)
            return match (read_opt(&pf.path), &pf.content) {
                (Some(a), Some(b)) => &a != b,
                (None, _) => true,
                _ => true,
            };
        }
        return false;
    }
    if !pf.primary || !pf.path.exists() || owned.contains(&pf.path) {
        return false;
    }
    read_opt(&pf.path).as_ref() != pf.content.as_ref()
}

/// Makes a plan and stores it in memory; returns it.
pub fn plan(env: &Env, req: PlanRequest) -> Result<InstallPlan> {
    let scope = req.scope.unwrap_or(if req.project_dir.is_some() {
        Scope::Project
    } else {
        Scope::Global
    });
    if scope.is_project_bound() && req.project_dir.is_none() {
        return Err(AgentkitError::new(
            "AGENTKIT_SCOPE",
            "project scope needs a project folder",
        ));
    }
    if let Some(p) = &req.project_dir {
        if !p.is_absolute() || !p.is_dir() {
            return Err(AgentkitError::new(
                "AGENTKIT_SCOPE",
                format!("{} is not a folder", p.display()),
            ));
        }
    }
    let all = targets::load_targets(env);
    let claude = targets::find(&all, "claude")?.clone();
    let project = req.project_dir.clone();
    let project_str = project.as_ref().map(|p| p.display().to_string());
    let mut warnings = Vec::new();
    let components = expand(req.components.clone(), &mut warnings);
    let locks = lock::all_locks(env, project.as_deref());
    let find_existing = |cid: &str, t: &str| -> Option<InstallRecord> {
        locks
            .iter()
            .find_map(|(_, l)| l.find(cid, t, scope, project_str.as_deref()).cloned())
    };
    let registry = Registry::global();
    let mut units: Vec<PlanUnit> = Vec::new();

    for c in &components {
        for tid in &req.targets {
            let t: &TargetAdapter = targets::find(&all, tid)?;
            let unit_id = format!("{}@{}", c.id, t.id);
            let mut unit = PlanUnit {
                unit_id,
                component: ComponentMeta::from(c),
                target: t.id.clone(),
                target_name: t.name.clone(),
                status: UnitStatus::New,
                compat: Compat::Converted,
                install_name: super::parse::sanitize_name(&c.name),
                replaces: None,
                files: vec![],
                losses: vec![],
                notes: vec![],
                commands: vec![],
                error: None,
            };
            if !t.supports_scope(scope) {
                unit.status = UnitStatus::Unsupported;
                unit.compat = Compat::Unsupported {
                    reason: format!("{} has no {} scope", t.name, scope.as_str()),
                };
                units.push(unit);
                continue;
            }
            let existing = find_existing(&c.id, &t.id);
            if let Some(ex) = &existing {
                if ex.component.sha256 == c.sha256 {
                    unit.status = UnitStatus::Installed;
                    unit.compat = ex.compat.clone();
                    unit.install_name = ex.installed_name.clone();
                    unit.replaces = Some(ex.install_id.clone());
                    unit.notes
                        .push("already installed with this content".into());
                    units.push(unit);
                    continue;
                }
                unit.status = UnitStatus::Update;
                unit.replaces = Some(ex.install_id.clone());
                unit.install_name = ex.installed_name.clone();
            }
            let owned: Vec<PathBuf> = existing
                .as_ref()
                .map(|r| r.files.iter().map(|f| PathBuf::from(&f.path)).collect())
                .unwrap_or_default();
            let mut name_override = existing
                .as_ref()
                .map(|r| r.installed_name.clone())
                .filter(|n| *n != super::parse::sanitize_name(&c.name));
            let mut attempt = 0;
            let candidates = name_candidates(c, &super::parse::sanitize_name(&c.name));
            let conversion = loop {
                let ctx = ConvertCtx {
                    env,
                    project: project.as_deref(),
                    scope,
                    claude: &claude,
                    name_override: name_override.clone(),
                    secret_values: &req.secret_values,
                };
                let conv = match registry.convert(c, t, &ctx) {
                    Ok(x) => x,
                    Err(e) => {
                        unit.status = UnitStatus::Error;
                        unit.error = Some(e.to_string());
                        break None;
                    }
                };
                let hit = conv.files.iter().any(|f| collides(f, &owned));
                if !hit {
                    break Some(conv);
                }
                match req.policy {
                    ConflictPolicy::Overwrite => {
                        unit.notes.push(
                            "replaces an existing file of the same name (backup kept)".into(),
                        );
                        break Some(conv);
                    }
                    ConflictPolicy::Skip => {
                        unit.status = UnitStatus::Conflict;
                        unit.notes
                            .push("a different file with this name already exists; skipped".into());
                        break None;
                    }
                    ConflictPolicy::Rename => {
                        if attempt >= candidates.len() {
                            unit.status = UnitStatus::Conflict;
                            unit.notes
                                .push("no free name left for this component".into());
                            break None;
                        }
                        name_override = Some(candidates[attempt].clone());
                        attempt += 1;
                    }
                }
            };
            if let Some(n) = &name_override {
                if unit.status != UnitStatus::Conflict {
                    unit.install_name = n.clone();
                    if existing.is_none() {
                        unit.notes.push(format!(
                            "a different `{}` exists; installed as `{n}`",
                            c.name
                        ));
                    }
                }
            }
            if let Some(conv) = conversion {
                unit.compat = conv.compat.clone();
                if let Compat::Unsupported { reason } = &conv.compat {
                    unit.status = UnitStatus::Unsupported;
                    unit.notes.push(reason.clone());
                } else {
                    for f in &conv.files {
                        for l in &f.losses {
                            if !unit.losses.contains(l) {
                                unit.losses.push(l.clone());
                            }
                        }
                        for n in &f.notes {
                            if !unit.notes.contains(n) {
                                unit.notes.push(n.clone());
                            }
                        }
                        for cmd in &f.commands {
                            if !unit.commands.contains(cmd) {
                                unit.commands.push(cmd.clone());
                            }
                        }
                    }
                    unit.files = conv.files;
                }
            }
            units.push(unit);
        }
    }

    // ---- simulate every path, resolving entry conflicts by policy
    let mut order: Vec<PathBuf> = Vec::new();
    for u in &units {
        if !matches!(u.status, UnitStatus::New | UnitStatus::Update) {
            continue;
        }
        for f in &u.files {
            if !order.contains(&f.path) {
                order.push(f.path.clone());
            }
        }
    }
    let replaced: Vec<InstallRecord> = units
        .iter()
        .filter(|u| u.status == UnitStatus::Update)
        .filter_map(|u| u.replaces.as_ref())
        .filter_map(|id| locks.iter().find_map(|(_, l)| l.by_id(id).cloned()))
        .collect();
    let mut files: Vec<FilePlan> = Vec::new();
    for path in order {
        let link_to: Option<PathBuf> = units
            .iter()
            .filter(|u| matches!(u.status, UnitStatus::New | UnitStatus::Update))
            .flat_map(|u| u.files.iter())
            .filter(|f| f.path == path && f.action == FileAction::Link)
            .filter_map(|f| f.link_to.clone())
            .last();
        let before = match &link_to {
            Some(to) => writer::link_state(&path, to),
            None => read_opt(&path),
        };
        let before_sha = before.as_ref().map(|b| sha256_hex(b));
        let owned_by_update = replaced
            .iter()
            .any(|r| r.files.iter().any(|f| Path::new(&f.path) == path));
        let mut fp = FilePlan {
            path: path.clone(),
            action: String::new(),
            before_sha: before_sha.clone(),
            after_sha: None,
            diff: String::new(),
            targets: vec![],
            components: vec![],
            conflicts: vec![],
            commands: vec![],
            executable: false,
            owned_by_update,
            link_to: link_to.clone(),
        };
        let idxs: Vec<(usize, usize)> = units
            .iter()
            .enumerate()
            .filter(|(_, u)| matches!(u.status, UnitStatus::New | UnitStatus::Update))
            .flat_map(|(ui, u)| {
                u.files
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.path == path)
                    .map(move |(fi, _)| (ui, fi))
            })
            .collect();
        for &(ui, fi) in &idxs {
            let u = &units[ui];
            let f = &u.files[fi];
            if !fp.targets.contains(&u.target) {
                fp.targets.push(u.target.clone());
            }
            if !fp.components.contains(&u.component.name) {
                fp.components.push(u.component.name.clone());
            }
            fp.commands.extend(
                f.commands
                    .iter()
                    .filter(|c| !fp.commands.contains(c))
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            fp.executable |= f.executable;
        }
        let all_write = idxs.iter().all(|&(ui, fi)| {
            matches!(
                units[ui].files[fi].action,
                FileAction::Write | FileAction::Link
            )
        });
        let any_write = idxs
            .iter()
            .any(|&(ui, fi)| units[ui].files[fi].action == FileAction::Write);
        let (after, format): (Option<Vec<u8>>, Option<DocFormat>) = if let Some(to) = &link_to {
            (Some(writer::link_bytes(to)), None)
        } else if all_write {
            let contents: Vec<&Vec<u8>> = idxs
                .iter()
                .filter_map(|&(ui, fi)| units[ui].files[fi].content.as_ref())
                .collect();
            if contents.windows(2).any(|w| w[0] != w[1]) {
                fp.conflicts.push(
                    "several components write different content here; the last one wins".into(),
                );
            }
            (contents.last().map(|c| (*c).clone()), None)
        } else if any_write {
            fp.conflicts
                .push("a component writes this whole file while another merges into it".into());
            warnings.push(format!(
                "{}: mixed write and merge; merge skipped",
                path.display()
            ));
            let c = idxs
                .iter()
                .rev()
                .find_map(|&(ui, fi)| units[ui].files[fi].content.clone());
            (c, None)
        } else {
            let format = units[idxs[0].0].files[idxs[0].1]
                .format
                .unwrap_or(DocFormat::Json);
            let mut text = before
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();
            // pieces of an install being updated come out first
            for r in &replaced {
                for f in &r.files {
                    if Path::new(&f.path) == path {
                        if let WrittenAction::Merged {
                            format: fmt, undo, ..
                        } = &f.action
                        {
                            if let Ok((t, _, _)) = writer::undo_ops(*fmt, &text, undo, false) {
                                text = t;
                            }
                        }
                    }
                }
            }
            for &(ui, fi) in &idxs {
                let unit_label = format!("{} → {}", units[ui].component.name, units[ui].target);
                let category = units[ui].component.category.clone();
                let mut ops = std::mem::take(&mut units[ui].files[fi].ops);
                let mut kept = Vec::new();
                for op in ops.drain(..) {
                    match resolve_op(
                        format,
                        &text,
                        op,
                        req.policy,
                        category.as_deref(),
                        &mut fp.conflicts,
                        &unit_label,
                    ) {
                        Some(o) => kept.push(o),
                        None => {}
                    }
                }
                match writer::apply_ops(format, &text, &kept) {
                    Ok(a) => text = a.text,
                    Err(e) => {
                        fp.conflicts.push(format!("{unit_label}: {}", e.message));
                        units[ui].status = UnitStatus::Error;
                        units[ui].error = Some(e.to_string());
                    }
                }
                units[ui].files[fi].ops = kept;
            }
            (Some(text.into_bytes()), Some(format))
        };
        let _ = format;
        let after_sha = after.as_ref().map(|b| sha256_hex(b));
        fp.action = match (&before, &after) {
            (_, None) => "unchanged".into(),
            (None, Some(_)) => {
                if all_write {
                    "create".into()
                } else {
                    "merge".into()
                }
            }
            (Some(b), Some(a)) if b == a => "unchanged".into(),
            (Some(_), Some(_)) => {
                if all_write {
                    "replace".into()
                } else {
                    "merge".into()
                }
            }
        };
        fp.diff = if let Some(to) = &link_to {
            if before.as_deref() == after.as_deref() {
                String::new()
            } else {
                format!(
                    "--- /dev/null\n+++ b{}\n@@ link @@\n+→ {}\n",
                    path.display(),
                    to.display()
                )
            }
        } else {
            unified_diff(
                &before
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).to_string())
                    .unwrap_or_default(),
                &after
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).to_string())
                    .unwrap_or_default(),
                &path.display().to_string(),
                before.is_none(),
            )
        };
        fp.after_sha = after_sha;
        files.push(fp);
    }
    let plan = InstallPlan {
        id: format!("plan-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
        created_at: now_iso(),
        scope,
        project_dir: project,
        policy: req.policy,
        units,
        files,
        warnings,
    };
    store(plan.clone());
    Ok(plan)
}

/// Applies the conflict policy to one op against the current simulated text.
/// `None` = drop the op.
fn resolve_op(
    format: DocFormat,
    text: &str,
    op: PatchOp,
    policy: ConflictPolicy,
    category: Option<&str>,
    conflicts: &mut Vec<String>,
    unit: &str,
) -> Option<PatchOp> {
    if format == DocFormat::Markdown {
        return Some(op);
    }
    let base = if text.trim().is_empty() {
        format.empty_doc().to_string()
    } else {
        text.to_string()
    };
    let Ok(doc) = edit::parse_value(format, &base) else {
        conflicts.push(format!(
            "{unit}: the file does not parse; nothing will be merged"
        ));
        return None;
    };
    match op {
        PatchOp::Set {
            path,
            value,
            rename_at,
        } => {
            let cur = edit::value_at(&doc, &path);
            match cur {
                None => Some(PatchOp::Set {
                    path,
                    value,
                    rename_at,
                }),
                Some(v) if *v == value => Some(PatchOp::Set {
                    path,
                    value,
                    rename_at,
                }),
                Some(_) => {
                    let label = edit::path_display(&path);
                    match (policy, rename_at) {
                        (ConflictPolicy::Skip, _) => {
                            conflicts.push(format!(
                                "{unit}: {label} already exists with other content; skipped"
                            ));
                            None
                        }
                        (ConflictPolicy::Rename, Some(i)) => {
                            let Seg::Key(name) = &path[i] else {
                                return Some(PatchOp::Set {
                                    path,
                                    value,
                                    rename_at,
                                });
                            };
                            let mut cands = Vec::new();
                            if let Some(cat) = category {
                                cands.push(format!("{}-{name}", super::parse::sanitize_name(cat)));
                            }
                            for n in 2..10 {
                                cands.push(format!("{name}-{n}"));
                            }
                            for cand in cands {
                                let mut p = path.clone();
                                p[i] = Seg::Key(cand.clone());
                                match edit::value_at(&doc, &p) {
                                    None => {
                                        conflicts.push(format!("{unit}: {label} exists with other content; added as `{cand}`"));
                                        return Some(PatchOp::Set {
                                            path: p,
                                            value,
                                            rename_at,
                                        });
                                    }
                                    Some(v) if *v == value => {
                                        return Some(PatchOp::Set {
                                            path: p,
                                            value,
                                            rename_at,
                                        })
                                    }
                                    _ => {}
                                }
                            }
                            conflicts.push(format!("{unit}: {label}: no free name; skipped"));
                            None
                        }
                        _ => {
                            conflicts
                                .push(format!("{unit}: {label} will be replaced (backup kept)"));
                            Some(PatchOp::Set {
                                path,
                                value,
                                rename_at,
                            })
                        }
                    }
                }
            }
        }
        PatchOp::Append {
            path,
            value,
            dedupe,
        } => {
            let arr = edit::value_at(&doc, &path)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            match writer::already_there(&arr, &value, &dedupe) {
                Ok(_) => Some(PatchOp::Append {
                    path,
                    value,
                    dedupe,
                }),
                Err(why) => {
                    if let (ConflictPolicy::Rename, Dedupe::Fields { fields }) = (policy, &dedupe) {
                        if let Some(f) = fields.first() {
                            if let Some(Value::String(n)) = value.get(f) {
                                let mut v2 = value.clone();
                                let cand = category
                                    .map(|c| format!("{}-{n}", super::parse::sanitize_name(c)))
                                    .unwrap_or_else(|| format!("{n}-2"));
                                v2[f] = Value::String(cand.clone());
                                conflicts.push(format!("{unit}: {why}; added as `{cand}`"));
                                return Some(PatchOp::Append {
                                    path,
                                    value: v2,
                                    dedupe,
                                });
                            }
                        }
                    }
                    conflicts.push(format!("{unit}: {why}; skipped"));
                    None
                }
            }
        }
        other => Some(other),
    }
}

// ------------------------------------------------------------------ plan store

fn plans() -> &'static Mutex<HashMap<String, InstallPlan>> {
    static CELL: std::sync::OnceLock<Mutex<HashMap<String, InstallPlan>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store(plan: InstallPlan) {
    if let Ok(mut m) = plans().lock() {
        if m.len() >= 32 {
            if let Some(oldest) = m
                .values()
                .min_by(|a, b| a.created_at.cmp(&b.created_at))
                .map(|p| p.id.clone())
            {
                m.remove(&oldest);
            }
        }
        m.insert(plan.id.clone(), plan);
    }
}

/// A stored plan by id.
pub fn get(id: &str) -> Option<InstallPlan> {
    plans().lock().ok().and_then(|m| m.get(id).cloned())
}

/// Removes a stored plan (after apply).
pub fn take(id: &str) -> Option<InstallPlan> {
    plans().lock().ok().and_then(|mut m| m.remove(id))
}

// ------------------------------------------------------------------ unified diff

/// Unified diff with 3 lines of context (own LCS; big files get a summary).
pub fn unified_diff(a: &str, b: &str, path: &str, new_file: bool) -> String {
    if a == b {
        return String::new();
    }
    let al: Vec<&str> = a.lines().collect();
    let bl: Vec<&str> = b.lines().collect();
    let header = format!(
        "--- {}\n+++ {}\n",
        if new_file {
            "/dev/null".to_string()
        } else {
            format!("a{path}")
        },
        format!("b{path}")
    );
    if al.len() * bl.len() > 4_000_000 {
        return format!(
            "{header}@@ file too large for a line diff: {} → {} lines @@\n",
            al.len(),
            bl.len()
        );
    }
    // LCS table (suffix lengths)
    let (n, m) = (al.len(), bl.len());
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * (m + 1) + j] = if al[i] == bl[j] {
                t[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1])
            };
        }
    }
    // edit script: (tag, a_idx, b_idx)
    let mut ops: Vec<(char, usize, usize)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && al[i] == bl[j] {
            ops.push((' ', i, j));
            i += 1;
            j += 1;
        } else if j < m && (i == n || t[i * (m + 1) + j + 1] > t[(i + 1) * (m + 1) + j]) {
            ops.push(('+', i, j));
            j += 1;
        } else {
            ops.push(('-', i, j));
            i += 1;
        }
    }
    let mut out = header;
    let ctx = 3;
    let mut k = 0;
    while k < ops.len() {
        if ops[k].0 == ' ' {
            k += 1;
            continue;
        }
        let start = k.saturating_sub(ctx);
        let mut end = k;
        let mut last_change = k;
        while end < ops.len() {
            if ops[end].0 != ' ' {
                last_change = end;
            } else if end - last_change > ctx * 2 {
                break;
            }
            end += 1;
        }
        let end = (last_change + ctx + 1).min(ops.len());
        let a_start = ops[start].1;
        let b_start = ops[start].2;
        let a_len = ops[start..end].iter().filter(|o| o.0 != '+').count();
        let b_len = ops[start..end].iter().filter(|o| o.0 != '-').count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            if a_len == 0 { a_start } else { a_start + 1 },
            a_len,
            if b_len == 0 { b_start } else { b_start + 1 },
            b_len
        ));
        for o in &ops[start..end] {
            let line = match o.0 {
                '+' => bl[o.2],
                _ => al[o.1],
            };
            out.push(o.0);
            out.push_str(line);
            out.push('\n');
        }
        k = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::unified_diff;

    #[test]
    fn diff_shows_changes() {
        let d = unified_diff("a\nb\nc\n", "a\nB\nc\nd\n", "/x", false);
        assert!(d.contains("-b\n+B\n"));
        assert!(d.contains("+d\n"));
        assert!(d.starts_with("--- a/x\n+++ b/x\n@@"));
        assert_eq!(unified_diff("same", "same", "/x", false), "");
    }
}
