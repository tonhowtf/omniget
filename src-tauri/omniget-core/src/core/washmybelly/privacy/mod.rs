//! Privacy items compiled from privacy.sexy (AGPL-3.0, embedded intact in
//! `data/washmybelly/privacy-sexy/`), with the state of each item read from
//! the system, a snapshot before every change and a real undo; the macOS
//! privacy permissions (TCC) live in [`tcc`].
//!
//! - [`compiler`]: the privacy.sexy template and function-call compiler.
//! - [`catalog`]: items of the three systems as typed facts (registry
//!   values, services, tasks, `defaults`, systemd units, Firefox prefs...),
//!   minus the left-out list (Defender, CPU mitigations...).
//! - [`state`]: what each fact reads as right now.
//! - [`exec`]: one item at a time through typed steps; free-form code is
//!   never run (`code_only`: the screen offers "copy").
//!
//! AGPL obligation: the catalog answer and `wmb_capabilities` carry the
//! link to the source, since the MCP can serve this output over a network.

pub mod catalog;
pub mod compiler;
pub mod exec;
pub mod state;
pub mod tcc;

use serde_json::{json, Value};

use self::catalog::{Catalog, Item};
use self::exec::{Action, Snapshot};
use self::state::{Env, ItemState, Reader};
use super::elevate::{self, ElevatedOp, OpResult};
use super::plan::{self, Op, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::startup::{RealSys, Sys};
use super::store::Ctx;
use super::{current_os, Os, Risk};

/// Plan kind of privacy items (shared with the TCC list).
pub const PLAN_KIND: &str = "privacy";
pub const LICENSE: &str = "AGPL-3.0";

fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

/// First paragraph of the docs, for lists (the screen asks `details` for the rest).
fn short_docs(d: &str, max: usize) -> String {
    let first = d
        .split("\n\n")
        .find(|p| !p.trim().is_empty())
        .unwrap_or("")
        .trim();
    if first.chars().count() > max {
        format!("{}…", first.chars().take(max).collect::<String>())
    } else {
        first.to_string()
    }
}

fn default_id(it: &Item) -> String {
    it.id.replacen("privacy:", "privacy-default:", 1)
}

fn plan_item(it: &Item, action: Action) -> PlanItem {
    let (id, label) = match action {
        Action::Apply => (it.id.clone(), it.name.clone()),
        Action::RevertDefault => (default_id(it), format!("Back to the default: {}", it.name)),
    };
    PlanItem {
        id,
        label,
        detail: Some(it.path.join(" › ")),
        group: it.top().to_string(),
        bytes: 0,
        risk: it.risk(),
        admin: it.admin(),
        op: Op::Module {
            module: "privacy".into(),
            action: action.as_str().into(),
            args: json!({ "id": it.id }),
        },
    }
}

fn in_preset(it: &Item, preset: &str) -> bool {
    matches!(
        (preset, it.recommend.as_deref()),
        (_, Some("standard")) | ("strict", Some("strict"))
    )
}

/// Read every item, register the plan candidates, offer agents a plan with
/// the preset, and leave the pending count for the health score.
pub fn catalog_and_offer(
    ctx: &Ctx,
    sys: &dyn Sys,
    env: &Env,
    os: Os,
    category: &str,
    preset: &str,
    source: Source,
) -> Result<Value, String> {
    let cat: &Catalog = catalog::catalog(os)?;
    let reader = Reader::new(sys, env);
    let here = os == env.os;
    let all: Vec<&Item> = cat.items.iter().collect();
    super::progress(
        "privacy_catalog",
        "read",
        0,
        all.len() as u64,
        "reading privacy settings",
    );
    if here {
        reader.prefetch(&all);
    }
    let states: Vec<ItemState> = state::par_map(&all, |it| {
        if here {
            reader.item(it)
        } else {
            ItemState::Unknown("another system".into())
        }
    });
    super::progress(
        "privacy_catalog",
        "done",
        all.len() as u64,
        all.len() as u64,
        "",
    );

    let cat_filter = category.trim().to_lowercase();
    let wanted =
        |it: &Item| cat_filter.is_empty() || it.path.iter().any(|p| p.to_lowercase() == cat_filter);
    let include_code = source == Source::Ui;
    let mut items = Vec::new();
    let mut candidates = Vec::new();
    let mut suggested = Vec::new();
    let mut not_present = 0;
    let mut pending = 0u32;
    let (mut applied, mut not_applied, mut unknown, mut code_only) = (0, 0, 0, 0);
    let mut categories: Vec<(String, usize)> = Vec::new();
    for (it, st) in all.iter().zip(&states) {
        if *st == ItemState::NotPresent {
            not_present += 1;
            continue;
        }
        let executable = it.executable();
        let risk = it.risk();
        if it.recommend.as_deref() == Some("standard")
            && !it.cleanup
            && risk != Risk::Extreme
            && executable
            && *st == ItemState::NotApplied
        {
            pending += 1;
        }
        if !wanted(it) {
            continue;
        }
        match st {
            ItemState::Applied => applied += 1,
            ItemState::NotApplied => not_applied += 1,
            _ => unknown += 1,
        }
        if !executable {
            code_only += 1;
        }
        match categories.iter_mut().find(|(c, _)| c == it.top()) {
            Some(c) => c.1 += 1,
            None => categories.push((it.top().to_string(), 1)),
        }
        // A setting whose state cannot be read stays disabled (never guess).
        let unreadable_setting = !it.cleanup && matches!(st, ItemState::Unknown(_));
        let can_apply = here && executable && !unreadable_setting && *st != ItemState::Applied;
        let can_default = here && !unreadable_setting && it.default_parts().is_some();
        if can_apply {
            let p = plan_item(it, Action::Apply);
            if source != Source::Ui
                && risk == Risk::Reversible
                && in_preset(
                    it,
                    if preset.is_empty() {
                        "standard"
                    } else {
                        preset
                    },
                )
            {
                suggested.push(p.clone());
            }
            candidates.push(p);
        }
        if can_default {
            candidates.push(plan_item(it, Action::RevertDefault));
        }
        let reason = match (st, executable) {
            (ItemState::Unknown(r), _) => Some(r.clone()),
            (_, false) => Some(
                it.opaque_reason
                    .clone()
                    .unwrap_or_else(|| "runs free-form code: copy it and run it yourself".into()),
            ),
            _ => None,
        };
        let mut v = json!({
            "id": it.id,
            "name": it.name,
            "category": it.category(),
            "path": it.path,
            "top": it.top(),
            // Agents get a shorter summary: a whole catalog must fit a context.
            "docs": short_docs(&it.docs, if include_code { 400 } else { 160 }),
            "recommend": it.recommend,
            "risk": risk,
            "state": st.as_str(),
            "reason": reason,
            "admin": it.admin(),
            "restart": it.restart,
            "kind": if it.cleanup { "cleanup" } else { "setting" },
            "code_only": !executable,
            "can_apply": can_apply,
            "can_revert_default": can_default,
            "default_id": can_default.then(|| default_id(it)),
        });
        if include_code && !executable {
            if let Ok(c) = it.compile(&cat.collection) {
                v["code"] = json!(c.code);
                v["revert_code"] = json!(c.revert);
            }
        }
        items.push(v);
    }
    plan::register(PLAN_KIND, &candidates);
    let mut plan_id = Value::Null;
    if source != Source::Ui && !suggested.is_empty() {
        plan_id = json!(plan::offer(ctx, PLAN_KIND, suggested, source)?.id);
    }
    if here {
        save_score(ctx, pending);
    }
    let preset_ids = |p: &str| -> Vec<&str> {
        all.iter()
            .zip(&states)
            .filter(|(it, st)| {
                **st != ItemState::NotPresent && it.risk() == Risk::Reversible && in_preset(it, p)
            })
            .map(|(it, _)| it.id.as_str())
            .collect()
    };
    Ok(json!({
        "os": os.as_str(),
        "source_url": super::PRIVACY_SEXY_SOURCE_URL,
        "license": LICENSE,
        "attribution": "Privacy items from privacy.sexy by undergroundwires (AGPL-3.0)",
        "items": items,
        "categories": categories.into_iter().map(|(name, count)| json!({ "name": name, "count": count })).collect::<Vec<_>>(),
        "counts": {
            "shown": applied + not_applied + unknown,
            "applied": applied,
            "not_applied": not_applied,
            "unknown": unknown,
            "code_only": code_only,
            "not_present": not_present,
            "left_out": cat.left_out,
            "pending_standard": pending,
        },
        "presets": { "standard": preset_ids("standard"), "strict": preset_ids("strict") },
        "total_bytes": 0,
        "plan_id": plan_id,
    }))
}

fn save_score(ctx: &Ctx, pending: u32) {
    use super::monitor::score::ScoreInputs;
    let mut s: ScoreInputs = ctx.kv_get("score_inputs").unwrap_or_default();
    s.privacy_pending = Some(pending);
    s.updated
        .get_or_insert_with(Default::default)
        .insert("privacy_pending".into(), super::store::now());
    let _ = ctx.kv_set("score_inputs", &s);
}

pub async fn tool_catalog(a: Value) -> Result<Value, String> {
    let source = source_of(&a);
    let category = crate::core::llm::tool_table::s(&a, "category");
    let preset = crate::core::llm::tool_table::s(&a, "preset");
    if !matches!(preset.as_str(), "" | "standard" | "strict") {
        return Err(format!("unknown preset {preset}"));
    }
    let os = match a.get("os").and_then(Value::as_str) {
        Some(o) => Os::parse(o).ok_or_else(|| format!("unknown system {o}"))?,
        None => current_os(),
    };
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        catalog_and_offer(&ctx, &RealSys, &Env::real(), os, &category, &preset, source)
    })
    .await
}

// ── Executing ─────────────────────────────────────────────────────────

/// The item behind a plan argument, only for this system.
fn item_of(args: &Value, os: Os) -> Result<&'static Item, String> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or("privacy: item id is missing")?;
    let cat = catalog::catalog(os)?;
    cat.find(id)
        .ok_or_else(|| format!("privacy: unknown item {id}"))
}

/// Snapshot, then run the person's steps; returns the helper's operations.
/// `preview` builds the operations without running or saving anything
/// (the "copy command" view of the plan).
#[allow(clippy::too_many_arguments)]
pub fn execute_item(
    ctx: &Ctx,
    sys: &dyn Sys,
    env: &Env,
    receipt_id: &str,
    it: &Item,
    action: Action,
    allow_admin: bool,
    preview: bool,
) -> Result<(Outcome, Vec<ElevatedOp>), String> {
    let reader = Reader::new(sys, env);
    let (steps, priors) = exec::steps(&reader, it, action)?;
    let ops = exec::elevated_ops(&steps);
    if !ops.is_empty() && !allow_admin {
        return Err(format!("{} needs the administrator helper", it.name));
    }
    if steps.is_empty() {
        return Ok((
            Outcome {
                detail: "already in that state".into(),
                bytes: 0,
                undo: None,
            },
            ops,
        ));
    }
    if preview {
        return Ok((Outcome::default(), ops));
    }
    let undo = if priors.is_empty() {
        None
    } else {
        let snap = Snapshot {
            item_id: it.id.clone(),
            action: action.as_str().into(),
            priors,
        };
        let id = ctx.snapshot_put(receipt_id, "privacy", &it.id, &snap)?;
        Some(Undo::Module {
            module: "privacy".into(),
            action: "restore_snapshot".into(),
            args: json!({ "snapshot": id }),
        })
    };
    let n = exec::run_user_steps(sys, &steps)?;
    let detail = match action {
        Action::Apply => format!(
            "applied ({} change{})",
            n + ops.len(),
            if n + ops.len() == 1 { "" } else { "s" }
        ),
        Action::RevertDefault => "back to the default".into(),
    };
    Ok((
        Outcome {
            detail,
            bytes: 0,
            undo,
        },
        ops,
    ))
}

/// Runs a batch through the administrator helper (tests pass a fake).
pub type Elevate<'a> = &'a dyn Fn(&[ElevatedOp]) -> Result<Vec<OpResult>, String>;

/// Restore a snapshot: the person's steps here, the rest through the helper.
pub fn restore(
    ctx: &Ctx,
    sys: &dyn Sys,
    env: &Env,
    snapshot_id: &str,
    elevate: Elevate,
) -> Result<String, String> {
    let snap: Snapshot = ctx.snapshot_get(snapshot_id)?;
    let steps = exec::restore_steps(env, &snap);
    let n = exec::run_user_steps(sys, &steps)?;
    let ops = exec::elevated_ops(&steps);
    if !ops.is_empty() {
        let results = elevate(&ops)?;
        if let Some(bad) = results.iter().find(|r| !r.ok) {
            return Err(bad.detail.clone());
        }
    }
    Ok(format!(
        "{} setting{} put back",
        n + ops.len(),
        if n + ops.len() == 1 { "" } else { "s" }
    ))
}

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "privacy" }`) ──

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    if action == "tcc_reset" {
        return tcc::plan_tcc_reset(args);
    }
    let act = Action::parse(action)?;
    let it = item_of(args, current_os())?;
    execute_item(
        ctx,
        &RealSys,
        &Env::real(),
        receipt_id,
        it,
        act,
        false,
        false,
    )
    .map(|(o, _)| o)
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    let act = Action::parse(action)?;
    let it = item_of(args, current_os())?;
    // `plan::copy_commands` asks with a throwaway state folder and this id:
    // nothing may run then.
    let preview = receipt_id == "preview";
    let (o, ops) = execute_item(
        ctx,
        &RealSys,
        &Env::real(),
        receipt_id,
        it,
        act,
        true,
        preview,
    )?;
    Ok((ops, o.undo))
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    if action != "restore_snapshot" {
        return Err(format!("privacy: cannot undo {action}"));
    }
    let id = args
        .get("snapshot")
        .and_then(Value::as_str)
        .ok_or("privacy: undo without a snapshot")?;
    restore(ctx, &RealSys, &Env::real(), id, &|ops| {
        elevate::run(ctx, ops)
    })
}

/// Actions only the screen may run (`washmybelly_ui` with `privacy.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action {
        "tcc_list" => tcc::tcc_list(a).await,
        // Full docs, the compiled code and revert code, and what we would run.
        "details" => super::tools::blocking(move || details(&a)).await,
        other => Err(format!("privacy: unknown screen action {other}")),
    }
}

fn details(a: &Value) -> Result<Value, String> {
    let id = a.get("id").and_then(Value::as_str).ok_or("id is missing")?;
    let os = id
        .split(':')
        .nth(1)
        .and_then(Os::parse)
        .ok_or("not a privacy item id")?;
    let cat = catalog::catalog(os)?;
    let it = cat.find(id).ok_or_else(|| format!("unknown item {id}"))?;
    let compiled = it.compile(&cat.collection)?;
    let mut steps = Vec::new();
    if os == current_os() && it.executable() {
        let env = Env::real();
        let reader = Reader::new(&RealSys, &env);
        if let Ok((st, _)) = exec::steps(&reader, it, Action::Apply) {
            steps = st.iter().map(exec::describe).collect();
        }
    }
    Ok(json!({
        "id": it.id,
        "name": it.name,
        "docs": it.docs,
        "code": compiled.code,
        "revert_code": compiled.revert,
        "steps": steps,
        "source_url": super::PRIVACY_SEXY_SOURCE_URL,
        "license": LICENSE,
    }))
}
