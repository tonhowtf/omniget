//! Windows tweaks as data, with their state read from the system and a
//! snapshot before every change (F4).
//!
//! - [`catalog`]: the embedded catalog (Win11Debloat, winutil, Sophia and
//!   our own items), checked against [`guard`] on load.
//! - [`state`]: what the system has now; nothing is remembered as "applied".
//! - [`apply`]: snapshot, change, undo; the helper's typed operations.
//! - `wmb_tweaks_list` → [`tool_list`]; plan items are
//!   `Op::Module { module: "tweaks", action: "apply" | "default", args: {id} }`
//!   in plans of kind `tweaks`.
//!
//! Design notes ported with attribution: undo that restores the real old
//! value and type (optimizerDuck, GPL-3.0; Win11Debloat, MIT), state read
//! live and items disabled when it cannot be read, HAGS gating (SophiApp,
//! MIT). A restore point is created before the first administrator batch of
//! the day with `Checkpoint-Computer`, which keeps Windows' own once-a-day
//! limit (Sparkle lifts that limit; we do not).

pub mod apply;
pub mod catalog;
pub mod guard;
pub mod regfile;
pub mod regfiles;
pub mod state;

use std::sync::OnceLock;

use serde_json::{json, Value};

use super::elevate::{ElevatedOp, OpResult};
use super::plan::{self, Op, Outcome, PlanItem};
use super::receipt::Undo;
use super::startup::{RealSys, Sys};
use super::store::Ctx;
use super::Risk;
use apply::{KeySets, Prev, Snapshot};
use catalog::{Tweak, TweakOp};
use state::Facts;

pub const PLAN_KIND: &str = "tweaks";
const ACTIONS: &[&str] = &["apply", "default"];
const KV_ULTIMATE: &str = "tweaks.ultimate_guid";
const KV_RESTORE_DAY: &str = "tweaks.restore_point_day";

/// `omniget-elevate` routes `ElevatedOp::Maintenance { id: "win_restore_point" }`
/// to [`elevated_restore_point`]. Until the orchestrator wires that arm
/// (handoff `shared_changes`), no restore point is requested, so a missing
/// route never fails a batch.
pub const HELPER_HAS_RESTORE_POINT: bool = true;
pub const RESTORE_POINT_ID: &str = "win_restore_point";

/// Fixed script: the description comes from `OMNIGET_WMB_RP_DESC`. Windows
/// refuses a second point within 24 hours; that refusal is reported, and
/// `SystemRestorePointCreationFrequency` is never touched.
pub const RESTORE_POINT_SCRIPT: &str = "$ErrorActionPreference='Stop'; $d=$env:OMNIGET_WMB_RP_DESC; if (-not $d) { $d='OmniGet washmybelly' }; try { Checkpoint-Computer -Description $d -RestorePointType MODIFY_SETTINGS -WarningVariable w -WarningAction SilentlyContinue; if ($w) { 'skipped: ' + $w[0] } else { 'created' } } catch { 'skipped: ' + $_.Exception.Message }";

pub fn restore_point_argv() -> Vec<String> {
    [
        "powershell",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        RESTORE_POINT_SCRIPT,
    ]
    .map(String::from)
    .to_vec()
}

/// The helper's executor for `win_restore_point`. It never fails the batch:
/// a point Windows declines (System Protection off, one already today) is
/// reported in the detail.
pub fn elevated_restore_point() -> Result<String, String> {
    if !cfg!(windows) {
        return Err("restore points exist only on Windows".into());
    }
    let argv = restore_point_argv();
    let out = RealSys.run_env(
        &argv,
        &[("OMNIGET_WMB_RP_DESC".into(), "OmniGet washmybelly".into())],
    );
    Ok(match out {
        Ok(o) => format!("restore point: {}", o.stdout.trim()),
        Err(e) => format!("restore point skipped: {e}"),
    })
}

fn key_sets() -> Result<&'static KeySets, String> {
    static SETS: OnceLock<Result<KeySets, String>> = OnceLock::new();
    SETS.get_or_init(|| {
        let c = catalog::catalog()?;
        Ok(KeySets {
            catalog: c.reg_keys(),
            whole: c.whole_key_deletes(),
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

// ── Listing ──────────────────────────────────────────────────────────

fn item_id(t: &Tweak, default: bool) -> String {
    if default {
        format!("tweaks:{}:default", t.id)
    } else {
        format!("tweaks:{}", t.id)
    }
}

fn plan_item(t: &Tweak, default: bool) -> PlanItem {
    let label = if default {
        format!("Back to the Windows default: {}", t.label)
    } else {
        t.label.clone()
    };
    let detail = if t.needs_restart {
        Some("Takes effect after a restart.".to_string())
    } else if t.description.is_empty() {
        None
    } else {
        Some(t.description.clone())
    };
    PlanItem {
        id: item_id(t, default),
        label,
        detail,
        group: t.category.clone(),
        bytes: 0,
        risk: if default { Risk::Reversible } else { t.risk },
        admin: if default {
            t.default_admin()
        } else {
            t.admin()
        },
        op: Op::Module {
            module: "tweaks".into(),
            action: if default { "default" } else { "apply" }.into(),
            args: json!({ "id": t.id }),
        },
    }
}

/// Why a tweak cannot be offered here, if it cannot.
fn unavailable(t: &Tweak, facts: &Facts, state: &str) -> Option<String> {
    if !t.applies.edition_ok(&facts.edition) {
        return Some("edition".into());
    }
    if t.requires.iter().any(|r| r == "hags") {
        if let Some(why) = facts.hags_blocked {
            return Some(why.into());
        }
    }
    if t.ops
        .iter()
        .any(|o| matches!(o, TweakOp::OptionalFeature { .. }))
    {
        return Some("needs_helper_update".into());
    }
    if t.admin()
        && t.ops
            .iter()
            .any(|o| matches!(o, TweakOp::PowerSetting { .. }))
    {
        return Some("needs_helper_update".into());
    }
    if state == "unknown" {
        return Some("unreadable".into());
    }
    None
}

/// Every tweak for this build, with its state read now. Items for other
/// builds are left out; items this machine cannot take (edition, hardware)
/// stay with the reason. Registers the plan candidates; builds no plan.
pub fn list_with(sys: &dyn Sys, facts: &Facts, category: Option<&str>) -> Result<Value, String> {
    let cat = catalog::catalog()?;
    let tweaks: Vec<&Tweak> = cat
        .tweaks
        .iter()
        .filter(|t| t.applies.build_ok(facts.build))
        .filter(|t| category.is_none_or(|c| c.is_empty() || t.category == c))
        .collect();
    let reader = state::Reader::new(sys);
    let mut keys: Vec<String> = tweaks
        .iter()
        .flat_map(|t| t.ops.iter())
        .filter_map(|op| op.reg_target().map(|(h, k, _)| state::full_key(h, k, None)))
        .collect();
    keys.sort_by_key(|k| k.to_ascii_uppercase());
    keys.dedup_by_key(|k| k.to_ascii_uppercase());
    reader.prefetch(&keys);

    let mut items = Vec::new();
    let mut candidates = Vec::new();
    let mut states = std::collections::HashMap::new();
    for t in &tweaks {
        let per_op: Vec<_> = t
            .ops
            .iter()
            .map(|op| state::op_state(&reader, op, facts))
            .collect();
        let st = state::aggregate(&per_op);
        states.insert(t.id.clone(), st);
        let why = unavailable(t, facts, st);
        let has_default = t.default_ops.is_some();
        if why.is_none() {
            candidates.push(plan_item(t, false));
            if has_default {
                candidates.push(plan_item(t, true));
            }
        }
        items.push(json!({
            "id": t.id,
            "item_id": item_id(t, false),
            "default_item_id": has_default.then(|| item_id(t, true)),
            "label": t.label,
            "label_key": t.label_key,
            "description": t.description,
            "category": t.category,
            "group": t.group,
            "risk": t.risk,
            "admin": t.admin(),
            "default_admin": t.default_admin(),
            "needs_restart": t.needs_restart,
            "warning": t.warning,
            "source": t.source,
            "license": t.license,
            "state": st,
            "available": why.is_none(),
            "reason": why,
            "has_default": has_default,
        }));
    }
    plan::register(PLAN_KIND, &candidates);

    let visible: std::collections::HashSet<&str> = tweaks.iter().map(|t| t.id.as_str()).collect();
    let groups: Vec<Value> = cat
        .groups
        .iter()
        .filter_map(|g| {
            let members: Vec<&String> = g
                .members
                .iter()
                .filter(|m| visible.contains(m.as_str()))
                .collect();
            if members.is_empty() {
                return None;
            }
            let active = members
                .iter()
                .find(|m| states.get(m.as_str()) == Some(&"applied"));
            Some(json!({
                "id": g.id, "label": g.label, "description": g.description,
                "members": members, "default": g.default, "active": active,
            }))
        })
        .collect();
    let mut categories: Vec<&str> = catalog::CATEGORIES
        .iter()
        .copied()
        .filter(|c| tweaks.iter().any(|t| t.category == *c))
        .collect();
    categories.dedup();
    Ok(json!({
        "build": facts.build,
        "edition": facts.edition,
        "categories": categories,
        "groups": groups,
        "items": items,
        "total_bytes": 0,
        "plan_id": Value::Null,
        "restore_point": HELPER_HAS_RESTORE_POINT,
    }))
}

pub async fn tool_list(a: Value) -> Result<Value, String> {
    if super::current_os() != super::Os::Windows {
        return Err("WINDOWS_ONLY: tweaks exist only on Windows".into());
    }
    let category = crate::core::llm::tool_table::s(&a, "category");
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        let cat = catalog::catalog()?;
        let want_hags = cat
            .tweaks
            .iter()
            .any(|t| t.requires.iter().any(|r| r == "hags"));
        let mut facts = state::read_facts(&RealSys, want_hags);
        facts.ultimate_guid = ctx.kv_get(KV_ULTIMATE);
        list_with(&RealSys, &facts, Some(category.as_str()))
    })
    .await
}

// ── Plan hooks ───────────────────────────────────────────────────────

fn tweak_and_ops(
    action: &str,
    args: &Value,
) -> Result<(&'static Tweak, &'static [TweakOp]), String> {
    if !ACTIONS.contains(&action) {
        return Err(format!("tweaks: unknown action {action}"));
    }
    let id = args["id"].as_str().ok_or("tweaks: id missing")?;
    let t = catalog::catalog()?
        .get(id)
        .ok_or_else(|| format!("tweaks: unknown tweak {id}"))?;
    let ops: &[TweakOp] = if action == "default" {
        t.default_ops
            .as_deref()
            .ok_or_else(|| format!("tweaks: {id} has no Windows default to go back to"))?
    } else {
        &t.ops
    };
    Ok((t, ops))
}

fn needs_admin(ops: &[TweakOp]) -> bool {
    ops.iter().any(TweakOp::needs_admin)
}

fn save_snapshot(ctx: &Ctx, receipt_id: &str, snap: &Snapshot) -> Result<Undo, String> {
    let id = ctx.snapshot_put(
        receipt_id,
        "tweaks",
        &format!("{}:{}", snap.tweak, snap.action),
        snap,
    )?;
    Ok(Undo::Module {
        module: "tweaks".into(),
        action: "restore_snapshot".into(),
        args: json!({ "snapshot": id }),
    })
}

fn remember_ultimate(ctx: &Ctx, entries: &[Prev]) {
    for p in entries {
        if let Prev::Power {
            created: Some(g), ..
        } = p
        {
            let _ = ctx.kv_set(KV_ULTIMATE, g);
        }
    }
}

pub fn execute_with(
    sys: &dyn Sys,
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    let (t, ops) = tweak_and_ops(action, args)?;
    if needs_admin(ops) {
        return Err("tweaks: this change needs the administrator".into());
    }
    let entries = apply::apply_direct(sys, ops, key_sets()?)?;
    remember_ultimate(ctx, &entries);
    let snap = Snapshot {
        tweak: t.id.clone(),
        action: action.into(),
        elevated: false,
        entries,
    };
    let undo = save_snapshot(ctx, receipt_id, &snap)?;
    let restart = if t.needs_restart {
        " (takes effect after a restart)"
    } else {
        ""
    };
    Ok(Outcome {
        detail: format!("{} applied{restart}", t.label),
        bytes: 0,
        undo: Some(undo),
    })
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    execute_with(&RealSys, ctx, receipt_id, action, args)
}

fn today() -> i64 {
    super::store::now() / 86_400
}

pub fn elevated_with(
    sys: &dyn Sys,
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    let (t, ops) = tweak_and_ops(action, args)?;
    let sid = if ops.iter().any(|o| {
        o.reg_target()
            .is_some_and(|(h, _, _)| h == catalog::Hive::Hkcu)
    }) {
        state::current_sid(sys)
    } else {
        None
    };
    let (entries, mut elevated) = apply::plan_for_helper(sys, ops, sid.as_deref(), key_sets()?)?;
    let snap = Snapshot {
        tweak: t.id.clone(),
        action: action.into(),
        elevated: true,
        entries,
    };
    let undo = save_snapshot(ctx, receipt_id, &snap)?;
    // A restore point before the first administrator batch of the day.
    if HELPER_HAS_RESTORE_POINT
        && receipt_id != "preview"
        && ctx.kv_get::<i64>(KV_RESTORE_DAY) != Some(today())
    {
        let _ = ctx.kv_set(KV_RESTORE_DAY, &today());
        elevated.insert(
            0,
            ElevatedOp::Maintenance {
                id: RESTORE_POINT_ID.into(),
            },
        );
    }
    Ok((elevated, Some(undo)))
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    elevated_with(&RealSys, ctx, receipt_id, action, args)
}

/// Runs elevated operations through the helper.
type HelperFn<'a> = dyn Fn(&[ElevatedOp]) -> Result<Vec<OpResult>, String> + 'a;

/// Undo one item: its snapshot back in place, here or through the helper.
pub fn undo_with(
    sys: &dyn Sys,
    ctx: &Ctx,
    action: &str,
    args: &Value,
    helper: &HelperFn<'_>,
) -> Result<String, String> {
    if action != "restore_snapshot" {
        return Err(format!("tweaks: cannot undo {action}"));
    }
    let id = args["snapshot"]
        .as_str()
        .ok_or("tweaks: snapshot id missing")?;
    let snap: Snapshot = ctx.snapshot_get(id)?;
    if snap.elevated {
        let ops = apply::restore_for_helper(&snap)?;
        if ops.is_empty() {
            return Ok("nothing to put back".into());
        }
        let results = helper(&ops)?;
        if let Some(bad) = results.iter().find(|r| !r.ok) {
            return Err(bad.detail.clone());
        }
        return Ok(format!("{}: {} change(s) put back", snap.tweak, ops.len()));
    }
    let n = apply::run_restore(sys, &snap)?;
    Ok(format!("{}: {n} change(s) put back", snap.tweak))
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    undo_with(&RealSys, ctx, action, args, &|ops| {
        super::elevate::run(ctx, ops)
    })
}

// ── The helper's registry executors ─────────────────────────────────

pub fn elevated_reg_set(key: &str, name: &str, kind: &str, data: &str) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("reg_set: only on Windows".into());
    }
    apply::helper_reg_set(&RealSys, key, name, kind, data)
}

pub fn elevated_reg_delete(key: &str, name: Option<&str>) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("reg_delete: only on Windows".into());
    }
    apply::helper_reg_delete(&RealSys, key, name, key_sets()?)
}

/// Actions only the screen may run (`washmybelly_ui` with `tweaks.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
        // Create a restore point now (one UAC prompt).
        "restore_point" => {
            if !HELPER_HAS_RESTORE_POINT {
                return Err("NOT_AVAILABLE: restore points need the helper update".into());
            }
            super::tools::blocking(|| {
                let ctx = Ctx::app()?;
                let r = super::elevate::run(
                    &ctx,
                    &[ElevatedOp::Maintenance {
                        id: RESTORE_POINT_ID.into(),
                    }],
                )?;
                let _ = ctx.kv_set(KV_RESTORE_DAY, &today());
                Ok(json!({ "detail": r.first().map(|x| x.detail.clone()).unwrap_or_default() }))
            })
            .await
        }
        _ => Err(format!("tweaks: unknown screen action {action}")),
    }
}
