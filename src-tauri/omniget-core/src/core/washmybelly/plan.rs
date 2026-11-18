use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::elevate::{self, ElevatedOp};
use super::fsops::{self, Fingerprint};
use super::protect::{self, Guard};
use super::receipt::{ItemResult, ItemStatus, Receipt, Undo};
use super::store::{now, Ctx};
use super::{Destination, Risk};

pub const PLAN_TTL_SECS: i64 = 600;
const CANDIDATE_TTL: Duration = Duration::from_secs(1800);

pub const NATIVE_PROGRAMS: &[&str] = &[
    "brew",
    "npm",
    "pnpm",
    "yarn",
    "pip",
    "pip3",
    "go",
    "gradle",
    "xcrun",
    "docker",
    "flatpak",
    "snap",
    "journalctl",
    "tmutil",
    "qlmanage",
    "dscacheutil",
    "killall",
    "mdutil",
    "diskutil",
    "lsregister",
    "fstrim",
    "sfc",
    "DISM",
    "dism",
    "ipconfig",
    "winget",
    "apt-get",
    "dnf",
    "pacman",
    "paccache",
    "zypper",
    "conda",
    "composer",
    "dotnet",
    "nix-collect-garbage",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Ui,
    Agent,
    External,
    Cli,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    RemovePath {
        path: PathBuf,
        destination: Destination,
        fp: Fingerprint,
        #[serde(default)]
        root: Option<PathBuf>,
        guard: Guard,
        #[serde(default)]
        contents_only: bool,
    },
    Command {
        program: String,
        args: Vec<String>,
        #[serde(default)]
        measure: Vec<PathBuf>,
    },
    Module {
        module: String,
        action: String,
        #[serde(default)]
        args: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub group: String,
    pub bytes: u64,
    pub risk: Risk,
    #[serde(default)]
    pub admin: bool,
    pub op: Op,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Ready,
    PendingApproval,
    Executing,
    Executed,
    Denied,
    Expired,
}

impl PlanStatus {
    fn as_str(self) -> &'static str {
        match self {
            PlanStatus::Ready => "ready",
            PlanStatus::PendingApproval => "pending_approval",
            PlanStatus::Executing => "executing",
            PlanStatus::Executed => "executed",
            PlanStatus::Denied => "denied",
            PlanStatus::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub kind: String,
    pub source: Source,
    pub created: i64,
    pub expires: i64,
    pub status: PlanStatus,
    pub items: Vec<PlanItem>,
    pub total_bytes: u64,
    pub needs_admin: bool,
    pub max_risk: Risk,
    #[serde(default)]
    pub receipt_id: Option<String>,
}

impl Plan {
    pub fn preview(&self) -> Value {
        let mut by_risk: HashMap<&str, (usize, u64)> = HashMap::new();
        let mut by_group: HashMap<&str, (usize, u64)> = HashMap::new();
        for it in &self.items {
            let r = match it.risk {
                Risk::Read => "read",
                Risk::Reversible => "reversible",
                Risk::Destructive => "destructive",
                Risk::Extreme => "extreme",
            };
            let e = by_risk.entry(r).or_default();
            e.0 += 1;
            e.1 += it.bytes;
            let g = by_group.entry(it.group.as_str()).or_default();
            g.0 += 1;
            g.1 += it.bytes;
        }
        let items: Vec<Value> = self
            .items
            .iter()
            .take(500)
            .map(|it| {
                json!({
                    "id": it.id, "label": it.label, "detail": it.detail, "group": it.group,
                    "bytes": it.bytes, "risk": it.risk, "admin": it.admin, "action": op_summary(&it.op),
                })
            })
            .collect();
        json!({
            "plan_id": self.id,
            "kind": self.kind,
            "status": self.status.as_str(),
            "created": self.created,
            "expires": self.expires,
            "item_count": self.items.len(),
            "total_bytes": self.total_bytes,
            "needs_admin": self.needs_admin,
            "max_risk": self.max_risk,
            "by_risk": by_risk.into_iter().map(|(k, (n, b))| (k.to_string(), json!({"count": n, "bytes": b}))).collect::<serde_json::Map<_, _>>(),
            "by_group": by_group.into_iter().map(|(k, (n, b))| (k.to_string(), json!({"count": n, "bytes": b}))).collect::<serde_json::Map<_, _>>(),
            "receipt_id": self.receipt_id,
            "items": items,
            "items_truncated": self.items.len() > 500,
        })
    }
}

fn op_summary(op: &Op) -> Value {
    match op {
        Op::RemovePath {
            path,
            destination,
            contents_only,
            ..
        } => json!({
            "type": "remove", "path": path, "destination": destination, "contents_only": contents_only,
        }),
        Op::Command { program, args, .. } => {
            json!({ "type": "command", "argv": std::iter::once(program.clone()).chain(args.iter().cloned()).collect::<Vec<_>>() })
        }
        Op::Module { module, action, .. } => {
            json!({ "type": "module", "module": module, "action": action })
        }
    }
}

type Candidates = HashMap<String, (Instant, String, PlanItem)>;

static CANDIDATES: Mutex<Option<Candidates>> = Mutex::new(None);

fn remember(kind: &str, items: &[PlanItem]) {
    let Ok(mut guard) = CANDIDATES.lock() else {
        return;
    };
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, (at, _, _)| at.elapsed() < CANDIDATE_TTL);
    let now = Instant::now();
    for it in items {
        map.insert(it.id.clone(), (now, kind.to_string(), it.clone()));
    }
}

fn candidate(kind: &str, id: &str) -> Option<PlanItem> {
    let guard = CANDIDATES.lock().ok()?;
    let (at, k, item) = guard.as_ref()?.get(id)?;
    (k == kind && at.elapsed() < CANDIDATE_TTL).then(|| item.clone())
}

pub fn register(kind: &str, items: &[PlanItem]) {
    remember(kind, items);
}

pub fn offer(ctx: &Ctx, kind: &str, items: Vec<PlanItem>, source: Source) -> Result<Plan, String> {
    remember(kind, &items);
    let items = items
        .into_iter()
        .filter(|it| source == Source::Ui || it.risk != Risk::Extreme)
        .collect();
    build(ctx, kind, items, source)
}

pub fn from_ids(ctx: &Ctx, kind: &str, ids: &[String], source: Source) -> Result<Plan, String> {
    let mut items = Vec::with_capacity(ids.len());
    let mut missing = Vec::new();
    for id in ids {
        match candidate(kind, id) {
            Some(it) if it.risk == Risk::Extreme && source != Source::Ui => {
                return Err(format!(
                    "EXTREME_ITEM: {id} can only be chosen on the OmniGet screen"
                ))
            }
            Some(it) => items.push(it),
            None => missing.push(id.clone()),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "UNKNOWN_ITEMS: {} id(s) were not offered by a recent {kind} scan (scan again): {}",
            missing.len(),
            missing
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    build(ctx, kind, items, source)
}

fn build(ctx: &Ctx, kind: &str, items: Vec<PlanItem>, source: Source) -> Result<Plan, String> {
    for it in &items {
        validate_item(it)?;
    }
    let created = now();
    let plan = Plan {
        id: format!("plan_{}", uuid::Uuid::new_v4().simple()),
        kind: kind.to_string(),
        source,
        created,
        expires: created + PLAN_TTL_SECS,
        status: PlanStatus::Ready,
        total_bytes: items.iter().map(|i| i.bytes).sum(),
        needs_admin: items.iter().any(|i| i.admin),
        max_risk: items.iter().map(|i| i.risk).max().unwrap_or(Risk::Read),
        items,
        receipt_id: None,
    };
    save(ctx, &plan)?;
    Ok(plan)
}

fn validate_item(it: &PlanItem) -> Result<(), String> {
    match &it.op {
        Op::RemovePath {
            path, root, guard, ..
        } => protect::check(path, *guard, root.as_deref()),
        Op::Command { program, .. } => {
            if NATIVE_PROGRAMS.contains(&program.as_str()) {
                Ok(())
            } else {
                Err(format!(
                    "PROTECTED: {program} is not an allowed cleanup program"
                ))
            }
        }
        Op::Module { module, .. } => {
            if MODULES.contains(&module.as_str()) {
                Ok(())
            } else {
                Err(format!("unknown module {module}"))
            }
        }
    }
}

pub fn save(ctx: &Ctx, plan: &Plan) -> Result<(), String> {
    let body = serde_json::to_string(plan).map_err(|e| e.to_string())?;
    ctx.open()?
        .execute(
            "INSERT INTO plans(id, kind, created, expires, status, body) VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET status=excluded.status, body=excluded.body",
            params![plan.id, plan.kind, plan.created, plan.expires, plan.status.as_str(), body],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load(ctx: &Ctx, plan_id: &str) -> Result<Plan, String> {
    let body: Option<String> = ctx
        .open()?
        .query_row(
            "SELECT body FROM plans WHERE id=?1",
            params![plan_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mut plan: Plan =
        serde_json::from_str(&body.ok_or_else(|| format!("PLAN_NOT_FOUND: {plan_id}"))?)
            .map_err(|e| e.to_string())?;
    if matches!(plan.status, PlanStatus::Ready | PlanStatus::PendingApproval)
        && now() > plan.expires
    {
        plan.status = PlanStatus::Expired;
        save(ctx, &plan)?;
    }
    Ok(plan)
}

pub fn set_status(ctx: &Ctx, plan_id: &str, status: PlanStatus) -> Result<Plan, String> {
    let mut plan = load(ctx, plan_id)?;
    plan.status = status;
    save(ctx, &plan)?;
    Ok(plan)
}

pub const MODULES: &[&str] = &[
    "clean",
    "apps",
    "startup",
    "privacy",
    "tweaks",
    "packages",
    "hosts",
    "maintenance",
    "power",
];

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub detail: String,
    pub bytes: u64,
    pub undo: Option<Undo>,
}

fn module_execute(
    ctx: &Ctx,
    receipt_id: &str,
    module: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    use super::*;
    match module {
        "clean" => clean::plan_execute(ctx, receipt_id, action, args),
        "apps" => apps::plan_execute(ctx, receipt_id, action, args),
        "startup" => startup::plan_execute(ctx, receipt_id, action, args),
        "privacy" => privacy::plan_execute(ctx, receipt_id, action, args),
        "tweaks" => tweaks::plan_execute(ctx, receipt_id, action, args),
        "packages" => packages::plan_execute(ctx, receipt_id, action, args),
        "hosts" => hosts::plan_execute(ctx, receipt_id, action, args),
        "maintenance" => maintenance::plan_execute(ctx, receipt_id, action, args),
        "power" => power::plan_execute(ctx, receipt_id, action, args),
        _ => Err(format!("unknown module {module}")),
    }
}

fn module_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    module: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    use super::*;
    match module {
        "clean" => clean::plan_elevated(ctx, receipt_id, action, args),
        "apps" => apps::plan_elevated(ctx, receipt_id, action, args),
        "startup" => startup::plan_elevated(ctx, receipt_id, action, args),
        "privacy" => privacy::plan_elevated(ctx, receipt_id, action, args),
        "tweaks" => tweaks::plan_elevated(ctx, receipt_id, action, args),
        "packages" => packages::plan_elevated(ctx, receipt_id, action, args),
        "hosts" => hosts::plan_elevated(ctx, receipt_id, action, args),
        "maintenance" => maintenance::plan_elevated(ctx, receipt_id, action, args),
        "power" => power::plan_elevated(ctx, receipt_id, action, args),
        _ => Err(format!("unknown module {module}")),
    }
}

pub fn module_undo(ctx: &Ctx, module: &str, action: &str, args: &Value) -> Result<String, String> {
    use super::*;
    match module {
        "clean" => clean::plan_undo(ctx, action, args),
        "apps" => apps::plan_undo(ctx, action, args),
        "startup" => startup::plan_undo(ctx, action, args),
        "privacy" => privacy::plan_undo(ctx, action, args),
        "tweaks" => tweaks::plan_undo(ctx, action, args),
        "packages" => packages::plan_undo(ctx, action, args),
        "hosts" => hosts::plan_undo(ctx, action, args),
        "maintenance" => maintenance::plan_undo(ctx, action, args),
        "power" => power::plan_undo(ctx, action, args),
        _ => Err(format!("unknown module {module}")),
    }
}

pub fn execute(ctx: &Ctx, plan_id: &str) -> Result<Receipt, String> {
    let mut plan = load(ctx, plan_id)?;
    match plan.status {
        PlanStatus::Ready | PlanStatus::PendingApproval => {}
        PlanStatus::Expired => return Err("PLAN_EXPIRED: scan again".into()),
        PlanStatus::Executed => {
            return Err(format!(
                "PLAN_ALREADY_EXECUTED: receipt {}",
                plan.receipt_id.clone().unwrap_or_default()
            ))
        }
        PlanStatus::Denied => return Err("PLAN_DENIED: the person refused this plan".into()),
        PlanStatus::Executing => return Err("PLAN_RUNNING".into()),
    }
    plan.status = PlanStatus::Executing;
    save(ctx, &plan)?;

    let receipt_id = format!("rcpt_{}", uuid::Uuid::new_v4().simple());
    let total = plan.items.len() as u64;
    let mut results: Vec<Option<ItemResult>> = vec![None; plan.items.len()];
    let mut batch: Vec<ElevatedOp> = Vec::new();
    let mut batch_owner: Vec<(usize, usize, Option<Undo>)> = Vec::new();

    for (i, it) in plan.items.iter().enumerate() {
        super::progress("execute", "run", i as u64, total, it.label.clone());
        if it.admin {
            match elevated_ops(ctx, &receipt_id, it) {
                Ok((ops, undo)) => {
                    batch_owner.push((i, ops.len(), undo));
                    batch.extend(ops);
                }
                Err(e) => results[i] = Some(result(it, ItemStatus::Skipped, e, 0, None)),
            }
            continue;
        }
        results[i] = Some(run_item(ctx, &receipt_id, i, it));
    }

    if !batch.is_empty() {
        super::progress(
            "execute",
            "admin",
            0,
            batch.len() as u64,
            "waiting for the administrator password",
        );
        let outcome = elevate::run(ctx, &batch);
        let mut cursor = 0;
        for (i, n, undo) in batch_owner {
            let it = &plan.items[i];
            let r = match &outcome {
                Ok(per_op) => {
                    let mine = &per_op[cursor..cursor + n];
                    let failed: Vec<_> = mine.iter().filter(|r| !r.ok).collect();
                    let bytes = mine.iter().map(|r| r.bytes).sum();
                    if failed.is_empty() {
                        let detail = mine
                            .iter()
                            .map(|r| r.detail.trim())
                            .filter(|d| !d.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let detail = if detail.is_empty() {
                            "done as administrator".to_string()
                        } else {
                            detail
                        };
                        result(it, ItemStatus::Done, detail, bytes, undo)
                    } else {
                        result(
                            it,
                            ItemStatus::Failed,
                            failed[0].detail.clone(),
                            bytes,
                            None,
                        )
                    }
                }
                Err(e) => result(it, ItemStatus::Failed, e.clone(), 0, None),
            };
            cursor += n;
            results[i] = Some(r);
        }
    }

    let items: Vec<ItemResult> = results.into_iter().flatten().collect();
    let receipt = Receipt::new(receipt_id, &plan, items);
    super::receipt::save(ctx, &receipt)?;
    plan.status = PlanStatus::Executed;
    plan.receipt_id = Some(receipt.id.clone());
    save(ctx, &plan)?;
    super::progress(
        "execute",
        "done",
        total,
        total,
        format!("{} bytes freed", receipt.bytes_freed),
    );
    Ok(receipt)
}

fn result(
    it: &PlanItem,
    status: ItemStatus,
    detail: String,
    bytes: u64,
    undo: Option<Undo>,
) -> ItemResult {
    ItemResult {
        item_id: it.id.clone(),
        label: it.label.clone(),
        status,
        detail,
        bytes,
        undo,
    }
}

fn elevated_ops(
    ctx: &Ctx,
    receipt_id: &str,
    it: &PlanItem,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match &it.op {
        Op::RemovePath {
            path,
            fp,
            root,
            guard,
            contents_only,
            ..
        } => {
            protect::check(path, *guard, root.as_deref())?;
            fsops::unchanged(path, fp)?;
            Ok((
                vec![ElevatedOp::RemovePath {
                    path: path.clone(),
                    root: root.clone(),
                    guard: *guard,
                    fp: *fp,
                    contents_only: *contents_only,
                }],
                None,
            ))
        }
        Op::Command { .. } => Err("a native command never runs as administrator".into()),
        Op::Module {
            module,
            action,
            args,
        } => module_elevated(ctx, receipt_id, module, action, args),
    }
}

fn run_item(ctx: &Ctx, receipt_id: &str, index: usize, it: &PlanItem) -> ItemResult {
    match &it.op {
        Op::RemovePath {
            path,
            destination,
            fp,
            root,
            guard,
            contents_only,
        } => {
            if let Err(e) = protect::check(path, *guard, root.as_deref()) {
                return result(it, ItemStatus::Skipped, e, 0, None);
            }
            if let Err(e) = fsops::unchanged(path, fp) {
                return result(it, ItemStatus::Skipped, e, 0, None);
            }
            remove(
                ctx,
                receipt_id,
                index,
                it,
                path,
                *destination,
                *contents_only,
            )
        }
        Op::Command {
            program,
            args,
            measure,
        } => {
            if !NATIVE_PROGRAMS.contains(&program.as_str()) {
                return result(
                    it,
                    ItemStatus::Skipped,
                    format!("{program} is not allowed"),
                    0,
                    None,
                );
            }
            let before: u64 = measure.iter().map(|p| fsops::measure(p).0).sum();
            let out = crate::core::process::std_command(program)
                .args(args)
                .output();
            let after: u64 = measure.iter().map(|p| fsops::measure(p).0).sum();
            let freed = before.saturating_sub(after);
            match out {
                Ok(o) if o.status.success() => result(
                    it,
                    ItemStatus::Done,
                    format!("{program} finished"),
                    freed,
                    None,
                ),
                Ok(o) => result(it, ItemStatus::Failed, tail(&o.stderr), freed, None),
                Err(e) => result(it, ItemStatus::Failed, format!("{program}: {e}"), 0, None),
            }
        }
        Op::Module {
            module,
            action,
            args,
        } => match module_execute(ctx, receipt_id, module, action, args) {
            Ok(o) => result(it, ItemStatus::Done, o.detail, o.bytes, o.undo),
            Err(e) => result(it, ItemStatus::Failed, e, 0, None),
        },
    }
}

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let s = s.trim();
    let start = s.len().saturating_sub(400);
    s.get(start..).unwrap_or(s).to_string()
}

fn remove(
    ctx: &Ctx,
    receipt_id: &str,
    index: usize,
    it: &PlanItem,
    path: &std::path::Path,
    dest: Destination,
    contents_only: bool,
) -> ItemResult {
    let bytes = fsops::measure(path).0;
    match dest {
        Destination::Unlink => match fsops::remove_tree(path, contents_only) {
            Ok(freed) => result(it, ItemStatus::Done, "removed".into(), freed, None),
            Err(e) => result(it, ItemStatus::Failed, e, 0, None),
        },
        Destination::Quarantine if !contents_only && quarantine_ok(ctx, path) => {
            match super::quarantine::move_in(ctx, receipt_id, index, path) {
                Ok(q) => result(
                    it,
                    ItemStatus::Done,
                    "moved to quarantine".into(),
                    bytes,
                    Some(Undo::Quarantine {
                        from: q,
                        to: path.to_path_buf(),
                    }),
                ),
                Err(e) => result(it, ItemStatus::Failed, e, 0, None),
            }
        }
        Destination::Quarantine | Destination::Trash => {
            if contents_only {
                let mut freed = 0;
                let mut undo = Vec::new();
                if let Ok(entries) = std::fs::read_dir(path) {
                    for e in entries.flatten() {
                        let p = e.path();
                        let b = fsops::measure(&p).0;
                        if fsops::to_trash(&p).is_ok() {
                            freed += b;
                            undo.push(p);
                        }
                    }
                }
                return result(
                    it,
                    ItemStatus::Done,
                    "moved to the Trash".into(),
                    freed,
                    Some(Undo::TrashMany { originals: undo }),
                );
            }
            match fsops::to_trash(path) {
                Ok(()) => result(
                    it,
                    ItemStatus::Done,
                    "moved to the Trash".into(),
                    bytes,
                    Some(Undo::Trash {
                        original: path.to_path_buf(),
                    }),
                ),
                Err(e) => result(it, ItemStatus::Failed, e, 0, None),
            }
        }
    }
}

fn quarantine_ok(ctx: &Ctx, path: &std::path::Path) -> bool {
    super::quarantine::same_volume(path, &ctx.dir)
}

pub fn copy_commands(ctx: &Ctx, plan_id: &str) -> Result<Vec<String>, String> {
    let plan = load(ctx, plan_id)?;
    if !matches!(plan.status, PlanStatus::Ready | PlanStatus::PendingApproval) {
        return Err(format!("PLAN_NOT_READY: {}", plan.status.as_str()));
    }
    let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
    let dry = Ctx::at(scratch.path());
    let mut ops = Vec::new();
    let mut notes = Vec::new();
    for it in plan.items.iter().filter(|i| i.admin) {
        match &it.op {
            Op::RemovePath {
                path,
                fp,
                root,
                guard,
                contents_only,
                ..
            } => ops.push(ElevatedOp::RemovePath {
                path: path.clone(),
                root: root.clone(),
                guard: *guard,
                fp: *fp,
                contents_only: *contents_only,
            }),
            Op::Module {
                module,
                action,
                args,
            } => match module_elevated(&dry, "preview", module, action, args) {
                Ok((more, _)) => ops.extend(more),
                Err(e) => notes.push(format!("# {}: {e}", it.label)),
            },
            Op::Command { .. } => {}
        }
    }
    let mut out = elevate::copy_command(&ops);
    out.extend(notes);
    Ok(out)
}

pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action {
        "copy_commands" => {
            let id = crate::core::llm::tool_table::need_str(&a, "plan_id")?;
            super::tools::blocking(move || {
                let ctx = Ctx::app()?;
                Ok(json!({ "commands": copy_commands(&ctx, &id)? }))
            })
            .await
        }
        _ => Err(format!("plan: unknown screen action {action}")),
    }
}

pub fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

pub async fn tool_plan(a: Value) -> Result<Value, String> {
    let kind = crate::core::llm::tool_table::need_str(&a, "kind")?;
    if !super::tools::PLAN_KINDS.contains(&kind.as_str()) {
        return Err(format!("unknown plan kind {kind}"));
    }
    let ids = crate::core::llm::tool_table::list(&a, "item_ids");
    if ids.is_empty() {
        return Err("item_ids is empty".into());
    }
    let source = source_of(&a);
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        Ok(from_ids(&ctx, &kind, &ids, source)?.preview())
    })
    .await
}

pub async fn tool_show(a: Value) -> Result<Value, String> {
    let id = crate::core::llm::tool_table::need_str(&a, "plan_id")?;
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        let plan = load(&ctx, &id)?;
        let mut v = plan.preview();
        if let Some(rid) = &plan.receipt_id {
            if let Ok(r) = super::receipt::load(&ctx, rid) {
                v["receipt"] = r.summary();
            }
        }
        Ok(v)
    })
    .await
}

pub async fn tool_execute(a: Value) -> Result<Value, String> {
    let id = crate::core::llm::tool_table::need_str(&a, "plan_id")?;
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        Ok(execute(&ctx, &id)?.summary())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_item(path: &std::path::Path, root: &std::path::Path, dest: Destination) -> PlanItem {
        PlanItem {
            id: format!("t:{}", path.display()),
            label: "test".into(),
            detail: None,
            group: "test".into(),
            bytes: fsops::measure(path).0,
            risk: Risk::Reversible,
            admin: false,
            op: Op::RemovePath {
                path: path.to_path_buf(),
                destination: dest,
                fp: fsops::fingerprint(path).unwrap(),
                root: Some(root.to_path_buf()),
                guard: Guard::Rule,
                contents_only: false,
            },
        }
    }

    #[test]
    fn scan_plan_execute_restore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(&dir.path().join("state"));
        let root = dir.path().join("junk");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.cache");
        let b = root.join("b.cache");
        std::fs::write(&a, vec![1u8; 10_000]).unwrap();
        std::fs::write(&b, vec![1u8; 10_000]).unwrap();
        let items = vec![
            file_item(&a, &root, Destination::Quarantine),
            file_item(&b, &root, Destination::Unlink),
        ];
        let plan = offer(&ctx, "cleanup", items, Source::Agent).unwrap();
        let narrowed =
            from_ids(&ctx, "cleanup", &[plan.items[0].id.clone()], Source::Agent).unwrap();
        assert_eq!(narrowed.items.len(), 1);
        let receipt = execute(&ctx, &plan.id).unwrap();
        assert!(!a.exists() && !b.exists());
        assert!(receipt.bytes_freed > 0);
        assert!(execute(&ctx, &plan.id).is_err(), "a plan runs once");
        let restored = super::super::receipt::restore(&ctx, &receipt.id).unwrap();
        assert!(a.exists(), "{restored:?}");
    }

    #[test]
    fn a_file_changed_after_the_scan_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(&dir.path().join("state"));
        let root = dir.path().join("junk");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.log");
        std::fs::write(&a, "1").unwrap();
        let plan = offer(
            &ctx,
            "cleanup",
            vec![file_item(&a, &root, Destination::Unlink)],
            Source::Agent,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&a, vec![2u8; 9000]).unwrap();
        let receipt = execute(&ctx, &plan.id).unwrap();
        assert!(a.exists());
        assert_eq!(receipt.items[0].status, ItemStatus::Skipped);
    }

    #[test]
    fn ids_no_scan_offered_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let e = from_ids(&ctx, "cleanup", &["forged".into()], Source::Agent).unwrap_err();
        assert!(e.starts_with("UNKNOWN_ITEMS"), "{e}");
    }

    #[test]
    fn a_program_outside_the_allowlist_never_gets_a_plan() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let item = PlanItem {
            id: "x".into(),
            label: "x".into(),
            detail: None,
            group: String::new(),
            bytes: 0,
            risk: Risk::Destructive,
            admin: false,
            op: Op::Command {
                program: "sh".into(),
                args: vec!["-c".into(), "rm -rf ~".into()],
                measure: vec![],
            },
        };
        assert!(offer(&ctx, "cleanup", vec![item], Source::Agent).is_err());
    }

    #[cfg_attr(windows, ignore = "needs permission to create a junction")]
    #[test]
    fn canary_links_never_lead_outside_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(&dir.path().join("state"));
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("precious.txt"), "keep me").unwrap();
        let root = dir.path().join("junk");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/trash.tmp"), "x").unwrap();
        link_dir(&outside, &root.join("sub/link")).unwrap();
        link_dir(&outside, &root.join("top-link")).unwrap();

        let mut items = vec![PlanItem {
            id: "root".into(),
            label: "root".into(),
            detail: None,
            group: String::new(),
            bytes: 0,
            risk: Risk::Destructive,
            admin: false,
            op: Op::RemovePath {
                path: root.clone(),
                destination: Destination::Unlink,
                fp: fsops::fingerprint(&root).unwrap(),
                root: Some(dir.path().to_path_buf()),
                guard: Guard::Picked,
                contents_only: true,
            },
        }];
        let through = root.join("top-link").join("precious.txt");
        assert!(
            protect::check(&through, Guard::Picked, Some(&root)).is_err(),
            "a path through a link is refused"
        );
        items.push(file_item(
            &root.join("top-link"),
            &root,
            Destination::Unlink,
        ));
        let plan = offer(&ctx, "cleanup", items, Source::Ui).unwrap();
        let _ = execute(&ctx, &plan.id).unwrap();
        assert_eq!(
            std::fs::read_to_string(outside.join("precious.txt")).unwrap(),
            "keep me"
        );
        assert!(!root.join("sub/trash.tmp").exists());
    }

    #[cfg(unix)]
    fn link_dir(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn link_dir(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("mklink /J failed"))
        }
    }
}
