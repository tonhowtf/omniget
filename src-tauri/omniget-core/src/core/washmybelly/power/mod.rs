//! Power profile, battery health, holds, conflicts between managers (F5).
//!
//! - `wmb_power_status` reads: the profile and who manages it, conflicts
//!   between Linux managers ("one manager only"), what keeps the machine
//!   awake, Windows plans and power mode, macOS Low Power Mode, battery
//!   charge limits. It changes nothing; the changes it can offer are
//!   registered as `power` plan candidates (`actions`).
//! - `wmb_power_hold` holds the performance profile / keeps the machine
//!   awake for at most four hours and lets go on its own ([`hold`]).
//! - Plan actions (closed list, [`ACTIONS`]): switch a Windows plan, add
//!   Ultimate Performance, Low Power Mode, apply EPP/governor once, battery
//!   charge limit, resolve a manager conflict. Each snapshots the state it
//!   changes and its receipt undoes it.

pub mod arbiter;
pub mod hold;
pub mod parse;
pub mod sysfs;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::elevate::ElevatedOp;
use super::plan::{Op, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::store::Ctx;
use super::Risk;

pub use hold::{
    auto_hold_begin, hold_for, hold_while_pid, pid_alive, set_auto_hold, HoldState, HoldToken,
};

const MODULE: &str = "power";
/// Plan kind of the candidates `wmb_power_status` registers.
pub const PLAN_KIND: &str = "power";

/// Every action a `power` plan item may carry.
pub const ACTIONS: &[&str] = &[
    "set_scheme",
    "ultimate",
    "lowpowermode",
    "cpu_epp",
    "cpu_governor",
    "charge_limit",
    "resolve_conflict",
];

/// Keys `ElevatedOp::PowerSetting` accepts. Values are checked again by
/// the helper against what the system lists as available.
pub const SETTING_KEYS: &[&str] = &[
    "cpu_epp",
    "cpu_governor",
    "charge_limit",
    "lowpowermode",
    "manager_off",
    "manager_on",
];

/// Profiles of the power-profiles API.
pub const PROFILES: &[&str] = &["power-saver", "balanced", "performance"];

/// Battery health lives with the monitor readers (`monitor::battery`, one
/// parser per system, tested with fixtures); this is the tool's entry point.
pub async fn tool_battery_health(a: Value) -> Result<Value, String> {
    super::monitor::tool_battery_health(a).await
}

// ── Hold backend per system ──────────────────────────────────────────

fn backend() -> Box<dyn hold::Backend> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::DbusBackend)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::CaffeinateBackend)
    }
    #[cfg(windows)]
    {
        Box::new(windows::PowerRequestBackend)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        struct None_;
        impl hold::Backend for None_ {
            fn name(&self) -> &'static str {
                "none"
            }
            fn acquire(&self, _: &str, _: Option<u64>) -> Result<Box<dyn Send>, String> {
                Err("power hold: unsupported system".into())
            }
        }
        Box::new(None_)
    }
}

// ── wmb_power_hold ───────────────────────────────────────────────────

pub async fn tool_hold(a: Value) -> Result<Value, String> {
    let action = crate::core::llm::tool_table::need_str(&a, "action")?;
    let minutes = a
        .get("max_minutes")
        .and_then(Value::as_u64)
        .unwrap_or(60)
        .min(u64::from(hold::MAX_MINUTES)) as u32;
    let who = match a.get("_source").and_then(Value::as_str) {
        Some("ui") => "held from the OmniGet screen",
        Some("external") => "held by an external MCP client",
        _ => "held by an OmniGet agent",
    };
    let reason: String = a
        .get("reason")
        .and_then(Value::as_str)
        .map(|r| r.chars().filter(|c| !c.is_control()).take(120).collect())
        .filter(|r: &String| !r.trim().is_empty())
        .unwrap_or_else(|| who.to_string());
    super::tools::blocking(move || {
        let h = hold::global();
        let st = match action.as_str() {
            "hold" => {
                let st = h.hold(minutes, &reason)?;
                if let Some(until) = st.until {
                    hold::arm_timer(until);
                }
                st
            }
            "release" => h.release(),
            other => return Err(format!("unknown action {other}; use hold or release")),
        };
        hold::persist(&st);
        serde_json::to_value(&st).map_err(|e| e.to_string())
    })
    .await
}

// ── wmb_power_status ─────────────────────────────────────────────────

fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

fn item(
    id: String,
    label: String,
    detail: Option<String>,
    admin: bool,
    action: &str,
    args: Value,
) -> PlanItem {
    PlanItem {
        id,
        label,
        detail,
        group: "power".into(),
        bytes: 0,
        risk: Risk::Reversible,
        admin,
        op: Op::Module {
            module: MODULE.into(),
            action: action.into(),
            args,
        },
    }
}

fn action_view(it: &PlanItem) -> Value {
    let (action, args) = match &it.op {
        Op::Module { action, args, .. } => (action.clone(), args.clone()),
        _ => (String::new(), Value::Null),
    };
    json!({ "id": it.id, "label": it.label, "detail": it.detail, "admin": it.admin, "risk": it.risk, "action": action, "args": args })
}

pub async fn tool_status(a: Value) -> Result<Value, String> {
    let _source = source_of(&a);
    super::tools::blocking(move || {
        let (mut v, items) = read_status();
        super::plan::register(PLAN_KIND, &items);
        v["actions"] = Value::Array(items.iter().map(action_view).collect());
        v["plan_kind"] = json!(PLAN_KIND);
        v["hold"] = serde_json::to_value(hold::global().state()).unwrap_or(Value::Null);
        Ok(v)
    })
    .await
}

#[cfg(target_os = "linux")]
fn read_status() -> (Value, Vec<PlanItem>) {
    let sys = std::path::Path::new("/sys");
    let mut reasons = Map::new();
    let mut running = linux::running_managers();
    let profiles = match linux::profiles() {
        Ok(p) => p,
        Err(e) => {
            reasons.insert("profiles".into(), json!(e));
            None
        }
    };
    // The D-Bus owner counts even when its unit has another name.
    if let Some(m) = profiles
        .as_ref()
        .and_then(|p| p["daemon"].as_str())
        .and_then(|d| arbiter::Manager::ALL.into_iter().find(|m| m.id() == d))
    {
        if !running.contains(&m) {
            running.push(m);
        }
    }
    let verdict = arbiter::arbitrate(&running);
    let knobs = parse::cpu_knobs(sys);
    let limits = parse::charge_limits(sys);
    let tlp = running
        .contains(&arbiter::Manager::Tlp)
        .then(linux::tlp_stat)
        .flatten();
    let blockers = linux::sleep_blockers().unwrap_or_else(|e| {
        reasons.insert("sleep_blockers".into(), json!(e));
        Vec::new()
    });
    let items = linux_items(&running, &verdict, &knobs, &limits);
    let v = json!({
        "os": "linux",
        "profile": profiles.as_ref().map(|p| json!({
            "current": p["active"],
            "available": p["profiles"],
            "holds": p["holds"],
            "degraded": p["degraded"],
            "can_switch": true,
        })),
        "manager": verdict.manager,
        "managers_running": running.iter().map(|m| m.id()).collect::<Vec<_>>(),
        "conflicts": verdict.conflicts,
        "suggestion": verdict.suggestion,
        "tlp": tlp,
        "cpu": knobs,
        "charge_limits": limits,
        "sleep_blockers": blockers,
        "sleep_blockers_need_admin": false,
        "reasons": reasons,
    });
    (v, items)
}

/// Plan candidates on Linux, from what runs and what the kernel exposes.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn linux_items(
    running: &[arbiter::Manager],
    verdict: &arbiter::Verdict,
    knobs: &parse::CpuKnobs,
    limits: &[parse::ChargeLimit],
) -> Vec<PlanItem> {
    let mut items = Vec::new();
    if let Some(c) = verdict.conflicts.first() {
        let keep = c.keep;
        let units = arbiter::units_to_disable(running, keep);
        items.push(item(
            format!("power:resolve_conflict:{keep}"),
            format!("Keep {keep}, turn off {}", units.join(", ")),
            Some(
                "Stops, disables and masks the other power managers. Undo turns them back on."
                    .into(),
            ),
            true,
            "resolve_conflict",
            json!({ "keep": keep, "units": units }),
        ));
    }
    // "Apply now" only when nobody manages the knobs: otherwise the manager
    // overwrites it within seconds.
    if verdict.manager.is_none() && verdict.conflicts.is_empty() {
        for e in &knobs.epps {
            if Some(e) != knobs.epp.as_ref() && e != "default" {
                items.push(item(
                    format!("power:cpu_epp:{e}"),
                    format!("CPU energy preference: {e}"),
                    Some("Applies now, until the next restart.".into()),
                    true,
                    "cpu_epp",
                    json!({ "value": e }),
                ));
            }
        }
        for g in &knobs.governors {
            if Some(g) != knobs.governor.as_ref() {
                items.push(item(
                    format!("power:cpu_governor:{g}"),
                    format!("CPU governor: {g}"),
                    Some("Applies now, until the next restart.".into()),
                    true,
                    "cpu_governor",
                    json!({ "value": g }),
                ));
            }
        }
    }
    // TLP and auto-cpufreq restore their own thresholds at boot.
    let limit_owner = running
        .iter()
        .any(|m| matches!(m, arbiter::Manager::Tlp | arbiter::Manager::AutoCpufreq));
    if !limit_owner {
        for l in limits {
            for pct in [60u8, 80, 100] {
                if pct != l.end {
                    items.push(item(
                        format!("power:charge_limit:{}:{pct}", l.battery),
                        if pct == 100 {
                            format!("Charge {} fully", l.battery)
                        } else {
                            format!("Stop charging {} at {pct}%", l.battery)
                        },
                        Some(
                            "A lower limit slows battery wear on a laptop that stays plugged in."
                                .into(),
                        ),
                        true,
                        "charge_limit",
                        json!({ "battery": l.battery, "percent": pct }),
                    ));
                }
            }
        }
    }
    items
}

#[cfg(target_os = "macos")]
fn read_status() -> (Value, Vec<PlanItem>) {
    let mut reasons = Map::new();
    let s = macos::settings().unwrap_or_else(|e| {
        reasons.insert("pmset".into(), json!(e));
        parse::PmsetSettings::default()
    });
    let blockers = macos::assertions().unwrap_or_else(|e| {
        reasons.insert("sleep_blockers".into(), json!(e));
        Vec::new()
    });
    let items = mac_items(&s);
    let v = json!({
        "os": "macos",
        "profile": {
            "current": match s.low_power_mode { Some(true) => "low-power", Some(false) => "automatic", None => "unknown" },
            "available": ["automatic", "low-power"],
            "can_switch": false,
        },
        "manager": "pmset",
        "managers_running": [],
        "conflicts": [],
        "low_power_mode": s.low_power_mode,
        "power_mode": s.power_mode,
        "sleep_minutes": s.sleep_minutes,
        "display_sleep_minutes": s.display_sleep_minutes,
        "sleep_prevented_by": s.sleep_prevented_by,
        "sleep_blockers": blockers,
        "sleep_blockers_need_admin": false,
        "reasons": reasons,
    });
    (v, items)
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn mac_items(s: &parse::PmsetSettings) -> Vec<PlanItem> {
    let Some(on) = s.low_power_mode else {
        return Vec::new();
    };
    let next = !on;
    vec![item(
        format!("power:lowpowermode:{}", if next { "on" } else { "off" }),
        if next {
            "Turn on Low Power Mode".into()
        } else {
            "Turn off Low Power Mode".into()
        },
        Some("Changes pmset for battery and power adapter. Needs your password.".into()),
        true,
        "lowpowermode",
        json!({ "on": next }),
    )]
}

#[cfg(windows)]
fn read_status() -> (Value, Vec<PlanItem>) {
    let mut reasons = Map::new();
    let list = windows::schemes().unwrap_or_else(|e| {
        reasons.insert("plans".into(), json!(e));
        Vec::new()
    });
    let active = windows::active_scheme()
        .ok()
        .flatten()
        .or_else(|| list.iter().find(|s| s.active).cloned());
    let (ac, dc) = windows::overlay();
    let (blockers, need_admin) = match windows::requests() {
        Ok(b) => (b, false),
        Err(_) => (Vec::new(), true),
    };
    let created: Option<String> = Ctx::app()
        .ok()
        .and_then(|c| c.kv_get("power_ultimate_guid"));
    let items = win_items(&list, created.as_deref());
    let v = json!({
        "os": "windows",
        "profile": {
            "current": ac.or(dc),
            "overlay": { "ac": ac, "dc": dc },
            "available": ["best_efficiency", "balanced", "best_performance"],
            "can_switch": false,
        },
        "manager": "windows",
        "managers_running": [],
        "conflicts": [],
        "active_plan": active,
        "plans": list,
        "ultimate": { "present": ultimate_present(&list, created.as_deref()) },
        "sleep_blockers": blockers,
        "sleep_blockers_need_admin": need_admin,
        "reasons": reasons,
    });
    (v, items)
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn ultimate_present(list: &[parse::Scheme], created: Option<&str>) -> bool {
    list.iter().any(|s| {
        s.guid == parse::ULTIMATE_TEMPLATE
            || Some(s.guid.as_str()) == created
            || s.name.to_ascii_lowercase().contains("ultimate")
    })
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn win_items(list: &[parse::Scheme], created: Option<&str>) -> Vec<PlanItem> {
    let mut items: Vec<PlanItem> = list
        .iter()
        .filter(|s| !s.active)
        .map(|s| {
            item(
                format!("power:set_scheme:{}", s.guid),
                format!("Switch to the {} plan", s.name),
                None,
                false,
                "set_scheme",
                json!({ "guid": s.guid, "name": s.name }),
            )
        })
        .collect();
    if !ultimate_present(list, created) {
        items.push(item(
            "power:ultimate".into(),
            "Add the Ultimate Performance plan and switch to it".into(),
            Some(
                "Windows hides this plan on most editions. Undo switches back and removes it."
                    .into(),
            ),
            false,
            "ultimate",
            json!({}),
        ));
    }
    items
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn read_status() -> (Value, Vec<PlanItem>) {
    (
        json!({ "os": "unknown", "reasons": { "power": "unsupported system" } }),
        Vec::new(),
    )
}

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "power" }`) ──

/// What a receipt needs to put things back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Snap {
    /// Windows: the plan that was active, and the one we created (removed on undo).
    Scheme {
        previous: String,
        created: Option<String>,
    },
    /// Administrator settings to apply again, in order.
    Settings { ops: Vec<(String, String)> },
}

fn check_action(action: &str) -> Result<(), String> {
    if ACTIONS.contains(&action) {
        Ok(())
    } else {
        Err(format!("power: unknown action {action}"))
    }
}

fn undo_of(id: String) -> Option<Undo> {
    Some(Undo::Module {
        module: MODULE.into(),
        action: "restore_snapshot".into(),
        args: json!({ "snapshot": id }),
    })
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    check_action(action)?;
    #[cfg(windows)]
    {
        let previous = windows::active_scheme()?
            .ok_or("no active power plan")?
            .guid;
        match action {
            "set_scheme" => {
                let guid = args["guid"].as_str().ok_or("set_scheme: guid")?;
                if !parse::is_guid(guid) {
                    return Err("set_scheme: invalid GUID".into());
                }
                let snap = ctx.snapshot_put(
                    receipt_id,
                    MODULE,
                    "scheme",
                    &Snap::Scheme {
                        previous,
                        created: None,
                    },
                )?;
                windows::set_active(guid)?;
                return Ok(Outcome {
                    detail: format!("switched to {}", args["name"].as_str().unwrap_or(guid)),
                    bytes: 0,
                    undo: undo_of(snap),
                });
            }
            "ultimate" => {
                let created = windows::duplicate_ultimate()?;
                let snap = ctx.snapshot_put(
                    receipt_id,
                    MODULE,
                    "scheme",
                    &Snap::Scheme {
                        previous,
                        created: Some(created.clone()),
                    },
                )?;
                let _ = ctx.kv_set("power_ultimate_guid", &created);
                windows::set_active(&created)?;
                return Ok(Outcome {
                    detail: "Ultimate Performance added and active".into(),
                    bytes: 0,
                    undo: undo_of(snap),
                });
            }
            _ => {}
        }
    }
    let _ = (ctx, receipt_id, args);
    Err(format!(
        "power: {action} needs an administrator on this system"
    ))
}

/// The administrator settings an action changes: `(key, new, old)`.
#[allow(unused_variables)]
fn setting_changes(action: &str, args: &Value) -> Result<Vec<(String, String, String)>, String> {
    match action {
        #[cfg(target_os = "macos")]
        "lowpowermode" => {
            let on = args["on"].as_bool().ok_or("lowpowermode: on")?;
            let cur = macos::settings()?
                .low_power_mode
                .ok_or("this Mac has no Low Power Mode")?;
            Ok(vec![("lowpowermode".into(), bit(on), bit(cur))])
        }
        #[cfg(target_os = "linux")]
        "cpu_epp" | "cpu_governor" => {
            let value = args["value"].as_str().ok_or("value")?.to_string();
            let k = parse::cpu_knobs(std::path::Path::new("/sys"));
            let (cur, list) = if action == "cpu_epp" {
                (k.epp, k.epps)
            } else {
                (k.governor, k.governors)
            };
            let cur = cur.ok_or("this CPU does not expose it")?;
            if !list.contains(&value) {
                return Err(format!("{value} is not available on this CPU"));
            }
            Ok(vec![(action.to_string(), value, cur)])
        }
        #[cfg(target_os = "linux")]
        "charge_limit" => {
            let bat = args["battery"].as_str().ok_or("battery")?;
            let pct = args["percent"].as_u64().ok_or("percent")?;
            let cur = parse::charge_limits(std::path::Path::new("/sys"))
                .into_iter()
                .find(|c| c.battery == bat)
                .ok_or("that battery has no charge limit")?;
            Ok(vec![(
                "charge_limit".into(),
                format!("{bat}:{pct}"),
                format!("{bat}:{}", cur.end),
            )])
        }
        #[cfg(target_os = "linux")]
        "resolve_conflict" => {
            let keep = args["keep"].as_str().ok_or("keep")?;
            let running = linux::running_managers();
            let units = arbiter::units_to_disable(&running, keep);
            if units.is_empty() {
                return Err("there is no conflict any more".into());
            }
            Ok(units
                .into_iter()
                .map(|u| ("manager_off".to_string(), u.to_string(), u.to_string()))
                .collect())
        }
        _ => Err(format!(
            "power: action {action} has no administrator form here"
        )),
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn bit(b: bool) -> String {
    if b { "1" } else { "0" }.into()
}

/// How to put one administrator setting back.
fn reverse(key: &str, old: &str) -> (String, String) {
    match key {
        "manager_off" => ("manager_on".into(), old.into()),
        k => (k.into(), old.into()),
    }
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    check_action(action)?;
    let changes = setting_changes(action, args)?;
    for (k, v, _) in &changes {
        validate_setting(k, v)?;
    }
    let back: Vec<(String, String)> = changes
        .iter()
        .rev()
        .map(|(k, _, old)| reverse(k, old))
        .collect();
    let snap = ctx.snapshot_put(receipt_id, MODULE, action, &Snap::Settings { ops: back })?;
    let ops = changes
        .into_iter()
        .map(|(key, value, _)| ElevatedOp::PowerSetting { key, value })
        .collect();
    Ok((ops, undo_of(snap)))
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    if action != "restore_snapshot" {
        return Err(format!("power: cannot undo {action}"));
    }
    let id = args["snapshot"].as_str().ok_or("snapshot id")?;
    match ctx.snapshot_get::<Snap>(id)? {
        Snap::Scheme { previous, created } => restore_scheme(&previous, created.as_deref()),
        Snap::Settings { ops } => {
            for (k, v) in &ops {
                validate_setting(k, v)?;
            }
            let batch: Vec<ElevatedOp> = ops
                .into_iter()
                .map(|(key, value)| ElevatedOp::PowerSetting { key, value })
                .collect();
            let results = super::elevate::run(ctx, &batch)?;
            match results.iter().find(|r| !r.ok) {
                Some(r) => Err(r.detail.clone()),
                None => Ok("power settings restored".into()),
            }
        }
    }
}

#[allow(unused_variables)]
fn restore_scheme(previous: &str, created: Option<&str>) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::set_active(previous)?;
        if let Some(c) = created {
            windows::delete_scheme(c)?;
            if let Ok(ctx) = Ctx::app() {
                let _ = ctx.kv_set("power_ultimate_guid", &Value::Null);
            }
        }
        return Ok("previous power plan restored".into());
    }
    #[allow(unreachable_code)]
    Err("power plans only exist on Windows".into())
}

// ── Administrator helper ─────────────────────────────────────────────

/// Shape check shared by the app (before signing) and the helper (before
/// touching anything). The helper then checks the value against what the
/// system lists as available.
pub fn validate_setting(key: &str, value: &str) -> Result<(), String> {
    if !SETTING_KEYS.contains(&key) {
        return Err(format!("power_setting: key {key:?} is not allowed"));
    }
    let token = |v: &str| {
        !v.is_empty()
            && v.len() <= 40
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    let ok = match key {
        "cpu_epp" | "cpu_governor" => token(value),
        "lowpowermode" => value == "0" || value == "1",
        "charge_limit" => value.split_once(':').is_some_and(|(b, p)| {
            parse::valid_battery_name(b) && p.parse::<u8>().is_ok_and(|p| (20..=100).contains(&p))
        }),
        "manager_off" | "manager_on" => arbiter::Manager::from_unit(value).is_some(),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "power_setting: value {value:?} is not valid for {key}"
        ))
    }
}

/// Executor of `ElevatedOp::PowerSetting` (runs as administrator).
pub fn elevated_setting(key: &str, value: &str) -> Result<String, String> {
    validate_setting(key, value)?;
    #[cfg(target_os = "linux")]
    {
        let sys = std::path::Path::new("/sys");
        return match key {
            "cpu_epp" => sysfs::write_cpu_knob(
                sys,
                "energy_performance_preference",
                "energy_performance_available_preferences",
                value,
            ),
            "cpu_governor" => sysfs::write_cpu_knob(
                sys,
                "scaling_governor",
                "scaling_available_governors",
                value,
            ),
            "charge_limit" => sysfs::write_charge_limit(sys, value),
            "manager_off" => linux::manager_unit(value, false),
            "manager_on" => linux::manager_unit(value, true),
            _ => Err(format!("power_setting: {key} is not a Linux setting")),
        };
    }
    #[cfg(target_os = "macos")]
    {
        return match key {
            "lowpowermode" => macos::set_low_power_mode(value == "1"),
            _ => Err(format!("power_setting: {key} is not a macOS setting")),
        };
    }
    #[allow(unreachable_code)]
    Err(format!(
        "power_setting: {key} is not available on this system"
    ))
}

/// The same setting as a terminal command, for people who would rather run
/// it themselves (`elevate::copy_command`). Display only.
pub fn command_line(key: &str, value: &str) -> String {
    if validate_setting(key, value).is_err() {
        return format!("# invalid power setting {key}");
    }
    match key {
        "cpu_epp" => format!("echo {value} | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference"),
        "cpu_governor" => format!("echo {value} | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor"),
        "charge_limit" => {
            let (b, p) = value.split_once(':').unwrap_or_default();
            format!("echo {p} | sudo tee /sys/class/power_supply/{b}/charge_control_end_threshold")
        }
        "lowpowermode" => format!("sudo pmset -a lowpowermode {value}"),
        "manager_off" => format!("sudo systemctl disable --now {value} && sudo systemctl mask {value}"),
        "manager_on" => format!("sudo systemctl unmask {value} && sudo systemctl enable --now {value}"),
        _ => format!("# power setting {key}"),
    }
}

// ── Screen-only actions ──────────────────────────────────────────────

/// Actions only the screen may run (`washmybelly_ui` with `power.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action {
        // The same switch as the desktop's own menu (no password, polkit).
        "set_profile" => {
            let p = crate::core::llm::tool_table::need_str(&a, "profile")?;
            if !PROFILES.contains(&p.as_str()) {
                return Err(format!("unknown profile {p}"));
            }
            super::tools::blocking(move || {
                #[cfg(target_os = "linux")]
                {
                    linux::set_profile(&p)?;
                    return Ok(json!({ "profile": p }));
                }
                #[allow(unreachable_code)]
                Err::<Value, String>(format!("power profiles are a Linux feature ({p})"))
            })
            .await
        }
        "battery_report" => {
            super::tools::blocking(|| {
                #[cfg(windows)]
                {
                    let dir = super::data_dir()?.join("reports");
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let path = dir.join("battery-report.html");
                    windows::battery_report(&path)?;
                    // Our own file in our own folder: explorer opens it
                    // with the default browser. Path in argv.
                    let _ = crate::core::process::std_command("explorer")
                        .arg(&path)
                        .spawn();
                    return Ok(json!({ "path": path }));
                }
                #[allow(unreachable_code)]
                Err::<Value, String>("the battery report is a Windows feature".into())
            })
            .await
        }
        "hold_state" => Ok(serde_json::to_value(hold::global().state()).unwrap_or(Value::Null)),
        _ => Err(format!("power: unknown screen action {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_allowlist() {
        for (k, v) in [
            ("cpu_epp", "balance_power"),
            ("cpu_governor", "performance"),
            ("charge_limit", "BAT0:80"),
            ("lowpowermode", "1"),
            ("manager_off", "power-profiles-daemon.service"),
            ("manager_on", "tlp.service"),
        ] {
            validate_setting(k, v).unwrap_or_else(|e| panic!("{k}={v}: {e}"));
        }
        for (k, v) in [
            ("hibernatemode", "0"),
            ("cpu_epp", "power; reboot"),
            ("cpu_epp", ""),
            ("cpu_governor", "../../x"),
            ("charge_limit", "BAT0:5"),
            ("charge_limit", "BAT0:101"),
            ("charge_limit", "../BAT0:80"),
            ("lowpowermode", "2"),
            ("manager_off", "sshd.service"),
            ("manager_off", "tlp"),
        ] {
            assert!(validate_setting(k, v).is_err(), "{k}={v} must be refused");
            assert!(elevated_setting(k, v).is_err());
        }
    }

    #[test]
    fn unknown_actions_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        assert!(plan_execute(&ctx, "r", "rm_rf", &json!({})).is_err());
        assert!(plan_elevated(&ctx, "r", "rm_rf", &json!({})).is_err());
        assert!(plan_undo(&ctx, "rm_rf", &json!({})).is_err());
    }

    #[test]
    fn undo_of_a_conflict_turns_managers_back_on_in_reverse() {
        let changes = [
            (
                "manager_off".to_string(),
                "power-profiles-daemon.service".to_string(),
                "power-profiles-daemon.service".to_string(),
            ),
            (
                "manager_off".to_string(),
                "tuned.service".to_string(),
                "tuned.service".to_string(),
            ),
        ];
        let back: Vec<_> = changes
            .iter()
            .rev()
            .map(|(k, _, o)| reverse(k, o))
            .collect();
        assert_eq!(back[0], ("manager_on".into(), "tuned.service".into()));
        assert_eq!(
            back[1],
            ("manager_on".into(), "power-profiles-daemon.service".into())
        );
        assert_eq!(
            reverse("cpu_epp", "balance_performance"),
            ("cpu_epp".into(), "balance_performance".into())
        );
    }

    #[test]
    fn snapshot_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let id = ctx
            .snapshot_put(
                "r",
                MODULE,
                "scheme",
                &Snap::Scheme {
                    previous: parse::BALANCED.into(),
                    created: Some("x".into()),
                },
            )
            .unwrap();
        let s: Snap = ctx.snapshot_get(&id).unwrap();
        assert_eq!(
            s,
            Snap::Scheme {
                previous: parse::BALANCED.into(),
                created: Some("x".into())
            }
        );
    }

    #[test]
    fn command_lines_are_display_only_and_validated() {
        assert_eq!(
            command_line("lowpowermode", "1"),
            "sudo pmset -a lowpowermode 1"
        );
        assert!(
            command_line("charge_limit", "BAT0:80").contains("/BAT0/charge_control_end_threshold")
        );
        assert!(command_line("cpu_epp", "x; rm -rf /").starts_with('#'));
    }

    #[test]
    fn linux_items_follow_the_one_manager_rule() {
        use arbiter::Manager::*;
        let knobs = parse::CpuKnobs {
            governor: Some("powersave".into()),
            governors: vec!["performance".into(), "powersave".into()],
            epp: Some("balance_performance".into()),
            epps: vec![
                "default".into(),
                "performance".into(),
                "balance_performance".into(),
            ],
            ..Default::default()
        };
        let limits = vec![parse::ChargeLimit {
            battery: "BAT0".into(),
            end: 100,
            start: None,
        }];
        // Nobody manages: apply-now and charge limit offered.
        let items = linux_items(&[], &arbiter::arbitrate(&[]), &knobs, &limits);
        let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"power:cpu_epp:performance"));
        assert!(!ids.contains(&"power:cpu_epp:default"));
        assert!(
            !ids.contains(&"power:cpu_epp:balance_performance"),
            "current value not offered"
        );
        assert!(ids.contains(&"power:cpu_governor:performance"));
        assert!(ids.contains(&"power:charge_limit:BAT0:80"));
        assert!(items.iter().all(|i| i.admin && i.risk == Risk::Reversible));
        // PPD manages: no apply-now.
        let items = linux_items(
            &[PowerProfilesDaemon],
            &arbiter::arbitrate(&[PowerProfilesDaemon]),
            &knobs,
            &limits,
        );
        assert!(items.iter().all(|i| !i.id.starts_with("power:cpu_")));
        // TLP + PPD: conflict item, no charge limit (TLP owns it).
        let running = [Tlp, PowerProfilesDaemon];
        let items = linux_items(&running, &arbiter::arbitrate(&running), &knobs, &limits);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "power:resolve_conflict:tlp");
    }

    #[test]
    fn windows_items_offer_other_plans_and_ultimate_once() {
        let list = parse::powercfg_list(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/washmybelly_fixtures/power/powercfg_list.txt"),
            )
            .unwrap(),
        );
        let items = win_items(&list, None);
        let ids: Vec<_> = items.iter().map(|i| i.id.clone()).collect();
        assert!(ids.contains(&format!("power:set_scheme:{}", parse::HIGH_PERFORMANCE)));
        assert!(
            !ids.contains(&format!("power:set_scheme:{}", parse::BALANCED)),
            "the active plan is not offered"
        );
        assert!(ids.contains(&"power:ultimate".to_string()));
        assert!(items.iter().all(|i| !i.admin));
        let with = parse::powercfg_list(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/washmybelly_fixtures/power/powercfg_list_ultimate.txt"),
            )
            .unwrap(),
        );
        assert!(!win_items(&with, None)
            .iter()
            .any(|i| i.id == "power:ultimate"));
    }

    #[test]
    fn mac_offers_the_opposite_low_power_mode() {
        let s = parse::PmsetSettings {
            low_power_mode: Some(false),
            ..Default::default()
        };
        let items = mac_items(&s);
        assert_eq!(items[0].id, "power:lowpowermode:on");
        assert!(items[0].admin);
        assert!(
            mac_items(&parse::PmsetSettings::default()).is_empty(),
            "no Low Power Mode, no item"
        );
    }
}
