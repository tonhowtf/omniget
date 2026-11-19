use serde_json::{json, Value};

use super::Os;
use crate::core::llm::tool_table::{BoxFut, CostHint, ToolEntry, ToolImpl, ToolRisk};

const EVERY_OS: &[Os] = &[];
const WINDOWS: &[Os] = &[Os::Windows];
const MACOS: &[Os] = &[Os::Macos];

pub const PLAN_KINDS: &[&str] = &[
    "cleanup",
    "dev_artifacts",
    "duplicates",
    "empty_folders",
    "large_files",
    "uninstall",
    "startup",
    "privacy",
    "tweaks",
    "packages",
    "hosts",
    "maintenance",
    "power",
];

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

struct Def {
    name: &'static str,
    description: &'static str,
    schema: Value,
    call: fn(Value) -> BoxFut,
    cost: CostHint,
    platforms: &'static [Os],
    risk: ToolRisk,
    always_ask: bool,
}

macro_rules! body {
    ($name:literal) => {
        |a: Value| -> BoxFut { Box::pin(async move { call($name, a).await }) }
    };
}

fn defs() -> Vec<Def> {
    use CostHint::{Cheap, Local};
    use ToolRisk::{Destructive, Read, Reversible};
    let risk_filter = json!({ "type": "string", "enum": ["reversible", "destructive"] });
    let roots = json!({ "type": "array", "items": { "type": "string" }, "maxItems": 20 });
    let d = |name, description, schema, call, cost, platforms, risk| Def {
        name,
        description,
        schema,
        call,
        cost,
        platforms,
        risk,
        always_ask: false,
    };
    vec![
        d("wmb_capabilities", "What this computer is and what washmybelly can do on it: OS, version, arch, admin, package managers, power manager, Full Disk Access, the available modules with the reason for each missing one, and the source link of the embedded data. Call it first.",
            obj(json!({ "refresh": { "type": "boolean" } }), &[]), body!("wmb_capabilities"), Cheap, EVERY_OS, Read),
        d("wmb_system_status", "Health score 0-100 with what lowers it, plus a snapshot of CPU, memory, disk, temperature and battery.",
            obj(json!({}), &[]), body!("wmb_system_status"), Local, EVERY_OS, Read),
        d("wmb_sensors", "Temperatures, fans, power draw and GPU load and frequency, as far as this machine exposes them without root.",
            obj(json!({}), &[]), body!("wmb_sensors"), Local, EVERY_OS, Read),
        d("wmb_processes", "Top processes by cpu, memory or disk, with pid, name, user and command. Read only.",
            obj(json!({ "sort": { "type": "string", "enum": ["cpu", "memory", "disk"] }, "limit": { "type": "integer", "minimum": 1, "maximum": 200 }, "query": { "type": "string" } }), &[]), body!("wmb_processes"), Local, EVERY_OS, Read),
        d("wmb_battery_health", "Battery cycles, current capacity against design capacity, temperature and charge state. Empty when there is no battery.",
            obj(json!({}), &[]), body!("wmb_battery_health"), Cheap, EVERY_OS, Read),
        d("wmb_disk_volumes", "Mounted volumes with total, free and (macOS) purgeable space.",
            obj(json!({}), &[]), body!("wmb_disk_volumes"), Cheap, EVERY_OS, Read),
        d("wmb_disk_usage", "Disk usage tree of a folder in allocated bytes, hardlinks counted once, cut by depth and minimum fraction; small children fold into an \"other\" node. A folder inside a root scanned in the last 10 minutes answers from memory (cached: true); refresh forces a new scan.",
            obj(json!({ "path": { "type": "string" }, "depth": { "type": "integer", "minimum": 1, "maximum": 6 }, "min_fraction": { "type": "number", "minimum": 0, "maximum": 0.5 }, "refresh": { "type": "boolean" } }), &[]), body!("wmb_disk_usage"), Local, EVERY_OS, Read),
        d("wmb_disk_health", "Physical disk health: SMART or the OS reliability counters, where readable without root.",
            obj(json!({}), &[]), body!("wmb_disk_health"), Local, EVERY_OS, Read),
        d("wmb_find_large_files", "Largest files under the given folders (default: home). Returns items and a ready plan_id that moves chosen files to the Trash.",
            obj(json!({ "roots": roots.clone(), "min_mb": { "type": "integer", "minimum": 1 }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 } }), &[]), body!("wmb_find_large_files"), Local, EVERY_OS, Read),
        d("wmb_find_duplicates", "Exact duplicate files (size, then partial hash, then full hash; hardlinks are not duplicates). Returns groups and a plan_id that keeps one file per group.",
            obj(json!({ "roots": roots.clone(), "min_kb": { "type": "integer", "minimum": 0 } }), &["roots"]), body!("wmb_find_duplicates"), Local, EVERY_OS, Read),
        d("wmb_find_empty_folders", "Empty folders under the given roots, with a plan_id.",
            obj(json!({ "roots": roots.clone() }), &["roots"]), body!("wmb_find_empty_folders"), Local, EVERY_OS, Read),
        d("wmb_junk_scan", "Find junk on this system: caches, logs, trash, browser and developer caches. Only rules that found something are returned. Changes nothing. plan_id holds only the items ticked by default; unticked ones (history and cookies, admin, open apps) can still be chosen with wmb_plan.",
            obj(json!({ "categories": { "type": "array", "items": { "type": "string" } }, "max_risk": risk_filter.clone() }), &[]), body!("wmb_junk_scan"), Local, EVERY_OS, Read),
        d("wmb_dev_artifacts_scan", "Build artifacts of projects (target/, node_modules, .gradle, __pycache__, ...) under the given roots, with age and size, and a plan_id.",
            obj(json!({ "roots": roots.clone(), "older_than_days": { "type": "integer", "minimum": 0 } }), &[]), body!("wmb_dev_artifacts_scan"), Local, EVERY_OS, Read),
        d("wmb_apps_list", "Installed apps and packages with origin (app bundle, Homebrew, MSI, winget, Store, apt, flatpak, snap...), size and version.",
            obj(json!({ "query": { "type": "string" }, "source": { "type": "string" }, "quick": { "type": "boolean", "description": "Answer at once with what is cheap to list and mark the result partial when slower sources are still missing" } }), &[]), body!("wmb_apps_list"), Local, EVERY_OS, Read),
        d("wmb_app_leftovers", "Leftovers of an installed or already removed app, each with a confidence level, and a plan_id to uninstall with them.",
            obj(json!({ "app_id": { "type": "string" } }), &["app_id"]), body!("wmb_app_leftovers"), Local, EVERY_OS, Read),
        d("wmb_startup_list", "What opens at login or runs in the background: login items, launch agents and daemons, Run keys, scheduled tasks, autostart and user services, with signer and state.",
            obj(json!({ "include_system": { "type": "boolean", "description": "Also list the operating system's own items (read only)" } }), &[]), body!("wmb_startup_list"), Local, EVERY_OS, Read),
        d("wmb_privacy_catalog", "Privacy items of this system with their state read from the system and a recommendation. Items that turn off security protections are marked and never offered to agents.",
            obj(json!({ "category": { "type": "string" }, "preset": { "type": "string", "enum": ["standard", "strict"] } }), &[]), body!("wmb_privacy_catalog"), Local, EVERY_OS, Read),
        d("wmb_tweaks_list", "Windows tweaks (performance, gaming, UI, bloat) with their state read from the system, risk and whether a restart is needed.",
            obj(json!({ "category": { "type": "string" } }), &[]), body!("wmb_tweaks_list"), Local, WINDOWS, Read),
        d("wmb_updates_list", "Pending updates from the system package manager (winget, Homebrew, apt, dnf, pacman, zypper, flatpak, snap).",
            obj(json!({}), &[]), body!("wmb_updates_list"), Local, EVERY_OS, Read),
        d("wmb_persistence_list", "Everything that persists on this Mac: launch agents and daemons, background tasks, privileged helpers, profiles, system and network extensions, with signature, notarization and Team ID.",
            obj(json!({}), &[]), body!("wmb_persistence_list"), Local, MACOS, Read),
        d("wmb_plan", "Build a plan from item ids returned by a scan. Returns a preview with bytes, risks and whether admin is needed. Changes nothing; the plan expires in 10 minutes.",
            obj(json!({ "kind": { "type": "string", "enum": PLAN_KINDS }, "item_ids": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 5000 } }), &["kind", "item_ids"]), body!("wmb_plan"), Cheap, EVERY_OS, Read),
        d("wmb_plan_show", "One plan: items, bytes, risks, status (ready, pending_approval, executed, denied, expired) and the receipt id once it ran.",
            obj(json!({ "plan_id": { "type": "string" } }), &["plan_id"]), body!("wmb_plan_show"), Cheap, EVERY_OS, Read),
        Def { always_ask: true, ..d("wmb_execute", "Execute a plan. Always asks the person in the OmniGet window first. Each item is checked again and skipped if it changed. Returns the receipt.",
            obj(json!({ "plan_id": { "type": "string" }, "idempotencyKey": { "type": "string", "minLength": 1, "maxLength": 100 } }), &["plan_id"]), body!("wmb_execute"), Local, EVERY_OS, Destructive) },
        d("wmb_receipts", "History of executed plans: what left, how many bytes, and whether it can be restored. With receipt_id, that receipt with its items.",
            obj(json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 100 }, "receipt_id": { "type": "string" } }), &[]), body!("wmb_receipts"), Cheap, EVERY_OS, Read),
        Def { always_ask: true, ..d("wmb_restore", "Undo a receipt: bring files back from quarantine or Trash and restore saved settings snapshots. Asks the person first.",
            obj(json!({ "receipt_id": { "type": "string" }, "idempotencyKey": { "type": "string", "minLength": 1, "maxLength": 100 } }), &["receipt_id"]), body!("wmb_restore"), Local, EVERY_OS, Reversible) },
        d("wmb_power_status", "Current power profile and who manages it, conflicts between managers, and what keeps the machine awake.",
            obj(json!({}), &[]), body!("wmb_power_status"), Cheap, EVERY_OS, Read),
        d("wmb_power_hold", "Hold the performance profile (or keep the machine awake) for at most max_minutes; released on its own. action release ends it early.",
            obj(json!({ "action": { "type": "string", "enum": ["hold", "release"] }, "max_minutes": { "type": "integer", "minimum": 1, "maximum": 240 }, "reason": { "type": "string" } }), &["action"]), body!("wmb_power_hold"), Cheap, EVERY_OS, Reversible),
        d("wmb_game_mode", "Game mode status, or turn it on for a running game's pid; it turns off by itself when the process exits.",
            obj(json!({ "action": { "type": "string", "enum": ["status", "enable", "disable"] }, "pid": { "type": "integer", "minimum": 1 } }), &["action"]), body!("wmb_game_mode"), Cheap, EVERY_OS, Reversible),
        d("wmb_perf_capabilities", "Whether FPS and frametime can be measured here and how (PresentMon, MangoHud, Metal HUD), and what setup is missing.",
            obj(json!({}), &[]), body!("wmb_perf_capabilities"), Cheap, EVERY_OS, Read),
        d("wmb_perf_capture_start", "Start measuring a running game (pid or process_name). Windows: PresentMon, reads Windows events and never injects. Linux: call without pid to get the MangoHud launch command; the game must be started with it. macOS: sensors only for a running game (frames need the game opened from the OmniGet screen).",
            obj(json!({ "pid": { "type": "integer", "minimum": 1 }, "process_name": { "type": "string" }, "max_seconds": { "type": "integer", "minimum": 5, "maximum": 3600 }, "label": { "type": "string" } }), &[]), body!("wmb_perf_capture_start"), Local, EVERY_OS, Reversible),
        d("wmb_perf_capture_stop", "Stop the running capture and compute its summary.",
            obj(json!({ "session_id": { "type": "string" } }), &[]), body!("wmb_perf_capture_stop"), Local, EVERY_OS, Read),
        d("wmb_perf_sessions", "Captured sessions, newest first.",
            obj(json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 100 } }), &[]), body!("wmb_perf_sessions"), Cheap, EVERY_OS, Read),
        d("wmb_perf_summary", "Statistics of one session: average FPS, percentiles, 1% and 0.1% lows, stutters, freezes and sensor correlation.",
            obj(json!({ "session_id": { "type": "string" } }), &["session_id"]), body!("wmb_perf_summary"), Cheap, EVERY_OS, Read),
        d("wmb_perf_compare", "Compare two sessions (before and after) metric by metric.",
            obj(json!({ "before": { "type": "string" }, "after": { "type": "string" } }), &["before", "after"]), body!("wmb_perf_compare"), Cheap, EVERY_OS, Read),
    ]
}

pub fn entries() -> Vec<ToolEntry> {
    defs()
        .into_iter()
        .map(|d| ToolEntry {
            name: d.name,
            description: d.description,
            input_schema: d.schema,
            call: ToolImpl::Core(d.call),
            category: "system",
            cost_hint: d.cost,
            platforms: d.platforms,
            feature: Some("washmybelly"),
            risk: Some(d.risk),
            always_ask: d.always_ask,
        })
        .collect()
}

pub fn all_names() -> Vec<&'static str> {
    defs().into_iter().map(|d| d.name).collect()
}

pub async fn call(name: &str, a: Value) -> Result<Value, String> {
    call_as(super::plan::Source::Agent, name, a).await
}

pub async fn call_as(
    source: super::plan::Source,
    name: &str,
    mut a: Value,
) -> Result<Value, String> {
    use super::*;
    if !a.is_object() {
        a = json!({});
    }
    let tag = match source {
        plan::Source::Ui => "ui",
        plan::Source::Agent => "agent",
        plan::Source::External => "external",
        plan::Source::Cli => "cli",
    };
    a["_source"] = json!(tag);
    match name {
        "wmb_capabilities" => caps::tool_capabilities(a).await,
        "wmb_system_status" => monitor::tool_system_status(a).await,
        "wmb_sensors" => monitor::tool_sensors(a).await,
        "wmb_processes" => monitor::tool_processes(a).await,
        "wmb_battery_health" => power::tool_battery_health(a).await,
        "wmb_disk_volumes" => disk::tool_volumes(a).await,
        "wmb_disk_usage" => disk::tool_usage(a).await,
        "wmb_disk_health" => disk::tool_health(a).await,
        "wmb_find_large_files" => disk::tool_large_files(a).await,
        "wmb_find_duplicates" => clean::files::tool_duplicates(a).await,
        "wmb_find_empty_folders" => clean::files::tool_empty_folders(a).await,
        "wmb_junk_scan" => clean::tool_junk_scan(a).await,
        "wmb_dev_artifacts_scan" => clean::artifacts::tool_scan(a).await,
        "wmb_apps_list" => apps::tool_list(a).await,
        "wmb_app_leftovers" => apps::tool_leftovers(a).await,
        "wmb_startup_list" => startup::tool_list(a).await,
        "wmb_privacy_catalog" => privacy::tool_catalog(a).await,
        "wmb_tweaks_list" => tweaks::tool_list(a).await,
        "wmb_updates_list" => packages::tool_updates(a).await,
        "wmb_persistence_list" => security::tool_persistence(a).await,
        "wmb_plan" => plan::tool_plan(a).await,
        "wmb_plan_show" => plan::tool_show(a).await,
        "wmb_execute" => plan::tool_execute(a).await,
        "wmb_receipts" => receipt::tool_list(a).await,
        "wmb_restore" => receipt::tool_restore(a).await,
        "wmb_power_status" => power::tool_status(a).await,
        "wmb_power_hold" => power::tool_hold(a).await,
        "wmb_game_mode" => game::tool_game_mode(a).await,
        "wmb_perf_capabilities" => perf::tool_capabilities(a).await,
        "wmb_perf_capture_start" => perf::tool_capture_start(a).await,
        "wmb_perf_capture_stop" => perf::tool_capture_stop(a).await,
        "wmb_perf_sessions" => perf::tool_sessions(a).await,
        "wmb_perf_summary" => perf::tool_summary(a).await,
        "wmb_perf_compare" => perf::tool_compare(a).await,
        _ => Err(format!("unknown tool: {name}")),
    }
}
pub async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}
