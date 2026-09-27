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

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::core::washmybelly::startup::{Output, Sys};

    /// Answers argv from a table and records every call.
    #[derive(Default)]
    pub struct FakeSys {
        answers: HashMap<String, Output>,
        any_ok: bool,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeSys {
        pub fn with(self, argv: &[&str], ok: bool, stdout: &str) -> Self {
            self.with_argv(argv.iter().map(|s| s.to_string()).collect(), ok, stdout)
        }

        pub fn with_argv(mut self, argv: Vec<String>, ok: bool, stdout: &str) -> Self {
            self.answers.insert(
                argv.join("\u{1}"),
                Output {
                    ok,
                    code: Some(if ok { 0 } else { 1 }),
                    stdout: stdout.into(),
                    stderr: String::new(),
                },
            );
            self
        }

        pub fn with_err(mut self, argv: &[&str], stderr: &str) -> Self {
            let key = argv.join("\u{1}");
            self.answers.insert(
                key,
                Output {
                    ok: false,
                    code: Some(1),
                    stdout: String::new(),
                    stderr: stderr.into(),
                },
            );
            self
        }

        /// Anything not in the table succeeds with no output.
        pub fn with_any_ok(mut self) -> Self {
            self.any_ok = true;
            self
        }

        pub fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Sys for FakeSys {
        fn run(&self, argv: &[String]) -> Result<Output, String> {
            self.calls.lock().unwrap().push(argv.to_vec());
            Ok(self
                .answers
                .get(&argv.join("\u{1}"))
                .cloned()
                .unwrap_or(Output {
                    ok: self.any_ok,
                    code: Some(if self.any_ok { 0 } else { 1 }),
                    stdout: String::new(),
                    stderr: if self.any_ok {
                        String::new()
                    } else {
                        "ERROR: The system was unable to find the specified registry key or value."
                            .into()
                    },
                }))
        }
    }

    pub fn fixture(name: &str) -> String {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/washmybelly_fixtures/privacy")
            .join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::catalog::Fact;
    use super::compiler::{Collection, Compiler, Node, ScriptData};
    use super::test_support::{fixture, FakeSys};
    use super::*;

    fn scripts(c: &Collection) -> Vec<&ScriptData> {
        fn walk<'a>(n: &'a [Node], out: &mut Vec<&'a ScriptData>) {
            for x in n {
                match x {
                    Node::Category(c) => walk(&c.children, out),
                    Node::Script(s) => out.push(s),
                }
            }
        }
        let mut out = Vec::new();
        for a in &c.actions {
            walk(&a.children, &mut out);
        }
        out
    }

    fn compare(os: Os, golden: &str) -> usize {
        let want: BTreeMap<String, serde_json::Value> = serde_json::from_str(golden).unwrap();
        let cat = catalog::catalog(os).unwrap();
        let comp = Compiler::new(&cat.collection);
        let by_name: BTreeMap<&str, &ScriptData> = scripts(&cat.collection)
            .into_iter()
            .map(|s| (s.name.as_str(), s))
            .collect();
        let mut bad = Vec::new();
        for (name, w) in &want {
            let s = by_name
                .get(name.as_str())
                .unwrap_or_else(|| panic!("{name} not in the collection"));
            let got = comp
                .compile_script(s)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            if got.code != w["code"].as_str().unwrap()
                || got.revert != w["revert"].as_str().unwrap()
            {
                bad.push(name.clone());
            }
        }
        assert!(
            bad.is_empty(),
            "{os:?}: {} of {} differ from the TypeScript compiler: {:?}",
            bad.len(),
            want.len(),
            &bad[..bad.len().min(10)]
        );
        want.len()
    }

    #[test]
    fn golden_windows() {
        assert!(compare(Os::Windows, &fixture("golden_windows.json")) >= 30);
    }

    #[test]
    fn golden_macos() {
        assert!(compare(Os::Macos, &fixture("golden_macos.json")) >= 30);
    }

    #[test]
    fn golden_linux() {
        assert!(compare(Os::Linux, &fixture("golden_linux.json")) >= 30);
    }

    /// Every script of the three systems against the full TypeScript output
    /// (`PRIVACY_GOLDEN_FULL` = the folder `privacy_golden.sh` prints).
    #[test]
    #[ignore = "needs the full outputs of scripts/washmybelly/privacy_golden.sh"]
    fn full_golden() {
        let dir = PathBuf::from(std::env::var("PRIVACY_GOLDEN_FULL").expect("PRIVACY_GOLDEN_FULL"));
        for os in [Os::Windows, Os::Macos, Os::Linux] {
            let text =
                std::fs::read_to_string(dir.join(format!("full_{}.json", os.as_str()))).unwrap();
            let n = compare(os, &text);
            eprintln!("{os:?}: {n} scripts identical");
        }
    }

    #[test]
    fn coverage_report() {
        for os in [Os::Windows, Os::Macos, Os::Linux] {
            let c = catalog::catalog(os).unwrap();
            eprintln!("{os:?}: {:?}", catalog::coverage(c));
            eprintln!("{os:?} blockers: {:?}", catalog::blockers(c));
        }
        let w = catalog::coverage(catalog::catalog(Os::Windows).unwrap());
        assert!(w["state_readable"] >= 400, "{w:?}");
    }

    fn mac_env(home: PathBuf) -> Env {
        Env {
            os: Os::Macos,
            home,
            root: PathBuf::from("/r"),
            path_dirs: vec![],
            user: "u".into(),
            system_root: PathBuf::from("/w"),
        }
    }

    #[test]
    fn catalog_reads_state_offers_presets_and_saves_the_score() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        // Every `defaults read` says the key is not set; everything else succeeds.
        let sys = MacFake;
        let env = mac_env(dir.path().join("home"));
        let v = catalog_and_offer(&ctx, &sys, &env, Os::Macos, "", "", Source::Agent).unwrap();
        assert_eq!(v["license"], "AGPL-3.0");
        assert_eq!(v["source_url"], super::super::PRIVACY_SEXY_SOURCE_URL);
        let items = v["items"].as_array().unwrap();
        let spell = items
            .iter()
            .find(|i| i["name"] == "Disable online spell correction")
            .unwrap();
        assert_eq!(spell["state"], "not_applied");
        assert_eq!(spell["can_apply"], true);
        let sudo = items
            .iter()
            .find(|i| i["name"] == "Enable application firewall")
            .unwrap();
        assert_eq!(sudo["code_only"], true);
        assert!(sudo.get("code").is_none(), "agents get no code dump");
        assert!(items
            .iter()
            .all(|i| i["risk"] != "extreme" || i["can_apply"] == true || i["code_only"] == true));
        let s: super::super::monitor::score::ScoreInputs = ctx.kv_get("score_inputs").unwrap();
        assert!(s.privacy_pending.unwrap() > 0);
        assert!(
            v["plan_id"].is_string(),
            "agents get a plan with the standard preset"
        );
        let plan = plan::load(&ctx, v["plan_id"].as_str().unwrap()).unwrap();
        assert!(plan.items.iter().all(|i| i.risk == Risk::Reversible));
        // The screen gets the code of code-only items and no plan.
        let ui =
            catalog_and_offer(&ctx, &sys, &env, Os::Macos, "Configure OS", "", Source::Ui).unwrap();
        assert!(ui["plan_id"].is_null());
        assert!(ui["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["path"][0] == "Configure OS"));
    }

    struct MacFake;
    impl Sys for MacFake {
        fn run(&self, argv: &[String]) -> Result<super::super::startup::Output, String> {
            let missing = argv.first().map(String::as_str) == Some("defaults")
                && argv.get(1).map(String::as_str) == Some("read");
            Ok(super::super::startup::Output {
                ok: !missing,
                code: Some(if missing { 1 } else { 0 }),
                stdout: String::new(),
                stderr: if missing {
                    "does not exist".into()
                } else {
                    String::new()
                },
            })
        }
    }

    #[test]
    fn apply_then_undo_puts_the_real_previous_value_back() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let env = mac_env(dir.path().to_path_buf());
        let it = catalog::catalog(Os::Macos)
            .unwrap()
            .items
            .iter()
            .find(|i| i.name == "Disable display of recent applications on Dock")
            .unwrap();
        let sys = FakeSys::default()
            .with(
                &["defaults", "read", "com.apple.dock", "show-recents"],
                true,
                "1\n",
            )
            .with(
                &["defaults", "read-type", "com.apple.dock", "show-recents"],
                true,
                "Type is boolean\n",
            )
            .with_any_ok();
        let (out, ops) = execute_item(
            &ctx,
            &sys,
            &env,
            "rcpt_test",
            it,
            Action::Apply,
            false,
            false,
        )
        .unwrap();
        assert!(ops.is_empty());
        let Some(Undo::Module { action, args, .. }) = out.undo else {
            panic!("no undo")
        };
        assert_eq!(action, "restore_snapshot");
        assert!(sys
            .calls()
            .iter()
            .any(|c| c.join(" ") == "defaults write com.apple.dock show-recents -bool false"));
        let never = |_: &[ElevatedOp]| -> Result<Vec<OpResult>, String> {
            panic!("no admin step expected")
        };
        let undo_sys = FakeSys::default().with_any_ok();
        restore(
            &ctx,
            &undo_sys,
            &env,
            args["snapshot"].as_str().unwrap(),
            &never,
        )
        .unwrap();
        assert_eq!(
            undo_sys.calls(),
            vec![vec![
                "defaults",
                "write",
                "com.apple.dock",
                "show-recents",
                "-bool",
                "true"
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()]
        );
    }

    #[test]
    fn admin_undo_goes_through_the_helper() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let env = Env {
            os: Os::Windows,
            home: dir.path().into(),
            root: dir.path().into(),
            path_dirs: vec![],
            user: "u".into(),
            system_root: dir.path().into(),
        };
        let it = catalog::catalog(Os::Windows)
            .unwrap()
            .items
            .iter()
            .find(|i| i.name == "Disable diagnostic and usage telemetry")
            .unwrap();
        let sys = FakeSys::default().with_any_ok();
        let (out, ops) =
            execute_item(&ctx, &sys, &env, "rcpt_t", it, Action::Apply, true, false).unwrap();
        assert!(ops.iter().all(|o| matches!(o, ElevatedOp::RegSet { .. })));
        assert!(!ops.is_empty());
        let Some(Undo::Module { args, .. }) = out.undo else {
            panic!()
        };
        let seen = std::sync::Mutex::new(Vec::new());
        let helper = |ops: &[ElevatedOp]| -> Result<Vec<OpResult>, String> {
            seen.lock().unwrap().extend(ops.iter().cloned());
            Ok(ops
                .iter()
                .map(|_| OpResult {
                    ok: true,
                    detail: "ok".into(),
                    bytes: 0,
                })
                .collect())
        };
        restore(
            &ctx,
            &FakeSys::default(),
            &env,
            args["snapshot"].as_str().unwrap(),
            &helper,
        )
        .unwrap();
        // The keys had no value before (the fake registry is empty): undo deletes them.
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .all(|o| matches!(o, ElevatedOp::RegDelete { name: Some(_), .. })));
        // Without the helper allowed, an admin item refuses to run.
        assert!(execute_item(&ctx, &sys, &env, "r2", it, Action::Apply, false, false).is_err());
    }

    #[test]
    fn preview_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let env = mac_env(dir.path().to_path_buf());
        let it = catalog::catalog(Os::Macos)
            .unwrap()
            .items
            .iter()
            .find(|i| i.name == "Disable display of recent applications on Dock")
            .unwrap();
        let sys = FakeSys::default()
            .with(
                &["defaults", "read", "com.apple.dock", "show-recents"],
                true,
                "1\n",
            )
            .with_any_ok();
        execute_item(&ctx, &sys, &env, "preview", it, Action::Apply, true, true).unwrap();
        assert!(!sys
            .calls()
            .iter()
            .any(|c| c.get(1).map(String::as_str) == Some("write")));
    }

    #[test]
    fn left_out_and_extreme_items_never_reach_an_agent_plan() {
        let c = catalog::catalog(Os::Windows).unwrap();
        for it in &c.items {
            if it.extreme() {
                assert_eq!(plan_item(it, Action::Apply).risk, Risk::Extreme);
            }
            assert!(!it
                .parts
                .iter()
                .any(|p| matches!(&p.fact, Fact::Opaque { what } if what == "SetMpPreference")));
        }
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let extreme: Vec<PlanItem> = c
            .items
            .iter()
            .filter(|i| i.extreme())
            .take(3)
            .map(|i| plan_item(i, Action::Apply))
            .collect();
        let p = plan::offer(&ctx, PLAN_KIND, extreme.clone(), Source::Agent).unwrap();
        assert!(
            p.items.is_empty(),
            "plan::offer drops extreme items for agents"
        );
        let ids: Vec<String> = extreme.iter().map(|i| i.id.clone()).collect();
        assert!(plan::from_ids(&ctx, PLAN_KIND, &ids, Source::Agent)
            .unwrap_err()
            .starts_with("EXTREME_ITEM"));
    }
}
