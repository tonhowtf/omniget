//! What opens at login or runs in the background, with an off switch that
//! never deletes anything.
//!
//! - macOS ([`macos`]): LaunchAgents and LaunchDaemons (state from
//!   `launchctl print-disabled` and `launchctl print`), legacy Login Items
//!   (System Events, read only for the list), the Background Task
//!   Management database (`sfltool dumpbtm`) and the signer (`codesign -dv`).
//! - Windows ([`windows`]): Run/RunOnce keys, the Startup folders and their
//!   `StartupApproved` state (how Task Manager turns items off), logon and
//!   boot tasks, and third-party automatic services.
//! - Linux ([`linux`]): XDG autostart (user and system) and `systemd --user`.
//!
//! A scan only reads. Turning an item off or on is an [`Action`] carried in
//! an `Op::Module { module: "startup" }` plan item; the plan engine calls
//! [`plan_execute`] (or [`plan_elevated`] for admin items), which saves the
//! state before the change so [`plan_undo`] can put it back. Every command
//! is argv; [`actions`] builds them apart from running them, so the tests
//! check the exact commands without touching `launchctl` or the registry.

/// argv from string literals and owned values (visible in the submodules).
macro_rules! argv {
    ($($x:expr),* $(,)?) => { vec![$(String::from($x)),*] };
}

/// The same argv as a fixed array, for commands that are only borrowed.
macro_rules! argv_array {
    ($($x:expr),* $(,)?) => { [$(String::from($x)),*] };
}

pub mod actions;
#[cfg(any(target_os = "linux", test))]
pub mod linux;
#[cfg(any(target_os = "macos", test))]
pub mod macos;
#[cfg(any(windows, test))]
pub mod windows;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use actions::Action;

use super::elevate::ElevatedOp;
use super::plan::{self, Op, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::store::Ctx;
use super::Risk;

pub const MODULE: &str = "startup";
pub const PLAN_KIND: &str = "startup";

/// Measured cost of an item at login, when the system reports one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Impact {
    pub ms: u64,
    /// `low`, `medium`, `high`.
    pub level: String,
}

impl Impact {
    pub fn from_ms(ms: u64) -> Impact {
        let level = if ms >= 1000 {
            "high"
        } else if ms >= 300 {
            "medium"
        } else {
            "low"
        };
        Impact {
            ms,
            level: level.into(),
        }
    }
}

/// One thing that starts with the session or the machine.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StartupItem {
    /// Stable id: `startup:<kind>:<key>`.
    pub id: String,
    pub name: String,
    /// `login_item`, `agent`, `daemon`, `run`, `run_once`, `startup_folder`,
    /// `task`, `service`, `autostart`, `user_service`.
    pub kind: String,
    /// `user` or `system`.
    pub scope: String,
    /// Where it is registered (plist, registry key, task path, .desktop, unit).
    pub location: String,
    pub command: String,
    pub enabled: bool,
    /// Running right now, when the system says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    /// Who signed the program (Developer ID name, Authenticode subject).
    #[serde(default)]
    pub signer: Option<String>,
    /// The signature checks out against a trusted root.
    #[serde(default)]
    pub signer_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// Unix seconds: when the entry was written (file time).
    #[serde(default)]
    pub installed: Option<i64>,
    #[serde(default)]
    pub impact: Option<Impact>,
    /// We can turn it off / on (without deleting anything).
    pub can_disable: bool,
    pub can_enable: bool,
    /// Changing it needs the administrator password.
    pub needs_admin: bool,
    /// Why it cannot be changed here, or what to know first.
    #[serde(default)]
    pub reason: Option<String>,
    /// Safe to turn off for most people (update checkers).
    #[serde(default)]
    pub recommended: bool,
    /// Not part of the operating system.
    pub third_party: bool,
    /// The flip to "off", when there is one.
    #[serde(skip)]
    pub off: Option<Action>,
    /// The flip to "on", when there is one.
    #[serde(skip)]
    pub on: Option<Action>,
}

impl StartupItem {
    /// The plan item that flips this entry to the other state.
    pub fn plan_item(&self) -> Option<PlanItem> {
        let action = if self.enabled {
            self.off.clone()?
        } else {
            self.on.clone()?
        };
        let (name, args) = action.split();
        Some(PlanItem {
            id: self.id.clone(),
            label: self.name.clone(),
            detail: Some(format!(
                "{} · {}",
                if self.enabled { "turn off" } else { "turn on" },
                self.location
            )),
            group: self.kind.clone(),
            bytes: 0,
            risk: Risk::Reversible,
            admin: self.needs_admin,
            op: Op::Module {
                module: MODULE.into(),
                action: name,
                args,
            },
        })
    }
}

/// A command's result. `ok` is exit status 0.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub ok: bool,
    /// Exit code, when the process exited normally.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs argv commands. Production is [`RealSys`]; tests record the calls.
pub trait Sys: Sync {
    fn run(&self, argv: &[String]) -> Result<Output, String>;

    /// Run with extra environment variables (data for a fixed script).
    fn run_env(&self, argv: &[String], env: &[(String, String)]) -> Result<Output, String> {
        let _ = env;
        self.run(argv)
    }

    /// Run and require success; the error carries stderr.
    fn run_ok(&self, argv: &[String]) -> Result<String, String> {
        let o = self.run(argv)?;
        if o.ok {
            Ok(o.stdout)
        } else {
            let msg = if o.stderr.trim().is_empty() {
                o.stdout.trim()
            } else {
                o.stderr.trim()
            };
            Err(format!(
                "{}: {}",
                argv.first().map(String::as_str).unwrap_or("?"),
                msg
            ))
        }
    }
}

pub struct RealSys;

impl Sys for RealSys {
    fn run(&self, argv: &[String]) -> Result<Output, String> {
        self.run_env(argv, &[])
    }

    fn run_env(&self, argv: &[String], env: &[(String, String)]) -> Result<Output, String> {
        let (prog, args) = argv.split_first().ok_or("empty command")?;
        let o = crate::core::process::std_command(prog)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .output()
            .map_err(|e| format!("{prog}: {e}"))?;
        Ok(Output {
            ok: o.status.success(),
            code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        })
    }
}

/// Where the user's things live; tests point it at a temporary HOME.
#[derive(Debug, Clone)]
pub struct Env {
    pub uid: u32,
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME/autostart`.
    pub autostart_user: PathBuf,
    /// `$XDG_CONFIG_DIRS/autostart` (usually `/etc/xdg/autostart`).
    pub autostart_system: Vec<PathBuf>,
}

impl Env {
    pub fn real() -> Env {
        let home = dirs::home_dir().unwrap_or_default();
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let system = std::env::var("XDG_CONFIG_DIRS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "/etc/xdg".into())
            .split(':')
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .map(|p| p.join("autostart"))
            .collect();
        Env {
            uid: current_uid(),
            home,
            autostart_user: config.join("autostart"),
            autostart_system: system,
        }
    }
}

#[cfg(unix)]
pub fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
pub fn current_uid() -> u32 {
    0
}

/// Update checkers: they wake up on their own, and the app still updates
/// when it is opened. The only items we suggest turning off.
pub fn looks_like_updater(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    [
        "updater",
        "update.agent",
        "autoupdate",
        "softwareupdate",
        "keystone",
        "update checker",
        "updatetask",
        "update service",
        "update.exe --processstart",
    ]
    .iter()
    .any(|p| t.contains(p))
        || t.ends_with(".update")
        || t.contains(".updater")
}

/// Unix seconds of a file's modification time.
pub fn file_time(path: &std::path::Path) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    m.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

// ── Parsers the actions need on every system ────────────────────────

/// `launchctl print-disabled <domain>`: label → disabled.
pub fn print_disabled(text: &str) -> std::collections::HashMap<String, bool> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.trim().split_once(" => ")?;
            let label = k.trim().trim_matches('"');
            let off = match v.trim() {
                "disabled" | "true" => true,
                "enabled" | "false" => false,
                _ => return None,
            };
            Some((label.to_string(), off))
        })
        .collect()
}

pub fn macos_disabled_lookup(text: &str, label: &str) -> bool {
    print_disabled(text).get(label).copied().unwrap_or(false)
}

/// First byte of a `StartupApproved` value: 02/06 on, 03/07 off.
pub fn windows_approved_enabled(hex: &str) -> Option<bool> {
    let b = u8::from_str_radix(hex.trim().get(0..2)?, 16).ok()?;
    match b {
        0x02 | 0x06 => Some(true),
        0x03 | 0x07 => Some(false),
        _ => None,
    }
}

/// `sc.exe qc <name>` start type as a `service_startup` mode.
pub fn windows_start_mode(text: &str) -> Option<String> {
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("START_TYPE"))?;
    let rest = line.split_once(':')?.1;
    let code: u32 = rest.split_whitespace().next()?.parse().ok()?;
    Some(
        match code {
            2 if rest.contains("DELAYED") => "delayed",
            2 => "automatic",
            3 => "manual",
            4 => "disabled",
            _ => return None,
        }
        .into(),
    )
}

// ── Listing ──────────────────────────────────────────────────────────

/// Everything this system starts, plus notes on what could not be read.
pub struct Collected {
    pub items: Vec<StartupItem>,
    pub notes: Vec<String>,
}

fn collect(sys: &dyn Sys, env: &Env, include_system: bool) -> Collected {
    #[cfg(target_os = "macos")]
    {
        macos::collect(sys, env, include_system)
    }
    #[cfg(windows)]
    {
        windows::collect(sys, env, include_system)
    }
    #[cfg(target_os = "linux")]
    {
        linux::collect(sys, env, include_system)
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (sys, env, include_system);
        Collected {
            items: Vec::new(),
            notes: vec!["unsupported system".into()],
        }
    }
}

fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

/// List, register every flip as a `wmb_plan` candidate, and leave the count
/// of third-party items that are on for the health score.
pub fn list_and_offer(
    ctx: &Ctx,
    sys: &dyn Sys,
    env: &Env,
    include_system: bool,
    source: Source,
) -> Result<Value, String> {
    let Collected { mut items, notes } = collect(sys, env, include_system);
    add_turned_off(ctx, &mut items);
    items.sort_by(|a, b| {
        (!a.third_party, a.kind.as_str(), a.name.to_lowercase()).cmp(&(
            !b.third_party,
            b.kind.as_str(),
            b.name.to_lowercase(),
        ))
    });
    let plan_items: Vec<PlanItem> = items.iter().filter_map(StartupItem::plan_item).collect();
    plan::register(PLAN_KIND, &plan_items);
    // Agents get a ready plan with the suggested flips only; the screen
    // builds its own from what the person picks.
    let mut plan_id = Value::Null;
    if source != Source::Ui {
        let suggested: Vec<PlanItem> = items
            .iter()
            .filter(|i| i.recommended && i.enabled && !i.needs_admin)
            .filter_map(StartupItem::plan_item)
            .collect();
        if !suggested.is_empty() {
            plan_id = json!(plan::offer(ctx, PLAN_KIND, suggested, source)?.id);
        }
    }
    let on_third_party = items.iter().filter(|i| i.enabled && i.third_party).count() as u32;
    save_score(ctx, on_third_party);
    Ok(json!({
        "items": items,
        "total_bytes": 0,
        "plan_id": plan_id,
        "enabled_third_party": on_third_party,
        "include_system": include_system,
        "notes": notes,
    }))
}

/// Items we turned off that the system stopped listing come back as "off",
/// so the screen can turn them on again. One listed as on again is dropped.
fn add_turned_off(ctx: &Ctx, items: &mut Vec<StartupItem>) {
    let mut list = actions::turned_off(ctx);
    let before = list.len();
    list.retain(|t| !items.iter().any(|i| i.id == t.id && i.enabled));
    if list.len() != before {
        actions::set_turned_off(ctx, &list);
    }
    for t in list {
        if items.iter().any(|i| i.id == t.id) {
            continue;
        }
        let location = match &t.on {
            Action::LoginItem { path, .. } => path.clone().unwrap_or_default(),
            Action::SystemdUser { unit, .. } => unit.clone(),
            Action::WinService { name, .. } => {
                format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{name}")
            }
            _ => String::new(),
        };
        items.push(StartupItem {
            id: t.id.clone(),
            name: t.name.clone(),
            kind: t.kind.clone(),
            scope: if t.admin {
                "system".into()
            } else {
                "user".into()
            },
            command: location.clone(),
            location,
            enabled: false,
            third_party: true,
            can_enable: true,
            needs_admin: t.admin,
            reason: Some("turned_off_here".into()),
            on: Some(t.on.clone()),
            ..Default::default()
        });
    }
}

fn save_score(ctx: &Ctx, login_items: u32) {
    use super::monitor::score::ScoreInputs;
    let mut s: ScoreInputs = ctx.kv_get("score_inputs").unwrap_or_default();
    s.login_items = Some(login_items);
    s.updated
        .get_or_insert_with(Default::default)
        .insert("login_items".into(), super::store::now());
    let _ = ctx.kv_set("score_inputs", &s);
}

pub async fn tool_list(a: Value) -> Result<Value, String> {
    let source = source_of(&a);
    let include_system = a
        .get("include_system")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        list_and_offer(&ctx, &RealSys, &Env::real(), include_system, source)
    })
    .await
}

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "startup" }`) ──

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    let a = Action::join(action, args)?;
    actions::execute(ctx, &RealSys, &Env::real(), receipt_id, &a)
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    let a = Action::join(action, args)?;
    actions::elevated(ctx, &RealSys, receipt_id, &a)
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    if action != "restore_snapshot" {
        return Err(format!("startup: cannot undo {action}"));
    }
    let id = args
        .get("snapshot")
        .and_then(Value::as_str)
        .ok_or("startup: undo without a snapshot")?;
    let snap: actions::Snapshot = ctx.snapshot_get(id)?;
    actions::restore(ctx, &RealSys, &Env::real(), &snap)
}

// ── Administrator executors (run inside `omniget-elevate`) ────────────

/// Windows services and systemd units that are never touched: security,
/// updates, networking (SharedAccess runs the mobile hotspot), the session.
const SERVICE_DENY: &[&str] = &[
    "windefend",
    "wdnissvc",
    "securityhealthservice",
    "wscsvc",
    "mpssvc",
    "wuauserv",
    "usosvc",
    "waasmedicsvc",
    "bits",
    "cryptsvc",
    "trustedinstaller",
    "rpcss",
    "rpceptmapper",
    "dcomlaunch",
    "ucpd",
    "sharedaccess",
    "eventlog",
    "dhcp",
    "dnscache",
    "nsi",
    "lsm",
    "samss",
    "profsvc",
    "audiosrv",
    "audioendpointbuilder",
    "plugplay",
    "power",
    "winmgmt",
    "bfe",
    "sgrmbroker",
    "dbus",
    "dbus-broker",
    "polkit",
    "networkmanager",
    "systemd-logind",
    "systemd-journald",
    "systemd-udevd",
    "systemd-resolved",
    "gdm",
    "sddm",
    "lightdm",
    "display-manager",
];

/// A Windows service name or a systemd unit name we accept from a plan.
pub fn valid_service_name(name: &str) -> bool {
    static RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9_.@:\-]{0,254}$").unwrap()
    });
    RE.is_match(name) && !name.contains("..")
}

fn denied_service(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let base = n.strip_suffix(".service").unwrap_or(&n);
    SERVICE_DENY.contains(&base) || base.starts_with("systemd-") || base.starts_with("getty@")
}

/// Windows start types and systemd verbs the helper accepts.
pub fn service_argv(name: &str, mode: &str, windows: bool) -> Result<Vec<String>, String> {
    if !valid_service_name(name) {
        return Err(format!("service_startup: invalid service name {name:?}"));
    }
    if denied_service(name) {
        return Err(format!("PROTECTED: {name} is never changed by washmybelly"));
    }
    if windows {
        let start = match mode {
            "automatic" | "auto" => "auto",
            "delayed" | "delayed-auto" => "delayed-auto",
            "manual" | "demand" => "demand",
            "disabled" => "disabled",
            _ => return Err(format!("service_startup: unknown mode {mode}")),
        };
        Ok(argv!["sc.exe", "config", name, "start=", start])
    } else {
        let verb = match mode {
            "enable" | "disable" => mode,
            _ => return Err(format!("service_startup: unknown mode {mode}")),
        };
        Ok(argv!["systemctl", verb, "--", name])
    }
}

pub fn elevated_service_startup(name: &str, mode: &str) -> Result<String, String> {
    if !(cfg!(windows) || cfg!(target_os = "linux")) {
        return Err("service_startup: only on Windows and Linux".into());
    }
    let argv = service_argv(name, mode, cfg!(windows))?;
    RealSys.run_ok(&argv)?;
    Ok(format!("{name}: {mode}"))
}

/// A scheduled task path: `\Folder\Name`, no quotes or wildcards.
pub fn valid_task_path(path: &str) -> bool {
    static RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r#"^\\[^"<>|*?\x00-\x1f]{1,259}$"#).unwrap()
    });
    RE.is_match(path) && !path.split('\\').any(|p| p == "..")
}

pub fn task_argv(path: &str, enabled: bool) -> Result<Vec<String>, String> {
    if !valid_task_path(path) {
        return Err(format!("task_state: invalid task path {path:?}"));
    }
    Ok(argv![
        "schtasks",
        "/Change",
        "/TN",
        path,
        if enabled { "/ENABLE" } else { "/DISABLE" }
    ])
}

pub fn elevated_task_state(path: &str, enabled: bool) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("task_state: only on Windows".into());
    }
    RealSys.run_ok(&task_argv(path, enabled)?)?;
    Ok(format!(
        "{path}: {}",
        if enabled { "enabled" } else { "disabled" }
    ))
}

/// A launchd label (`com.vendor.agent`).
pub fn valid_label(label: &str) -> bool {
    static RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"^[A-Za-z0-9._\-]{1,255}$").unwrap());
    RE.is_match(label)
}

/// `system` or `gui/<uid>`.
pub fn valid_domain(domain: &str) -> bool {
    domain == "system"
        || domain.strip_prefix("gui/").is_some_and(|n| {
            !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit())
        })
}

pub fn bootout_argv(domain: &str, label: &str) -> Result<Vec<String>, String> {
    if !valid_domain(domain) {
        return Err(format!("launchctl_bootout: invalid domain {domain:?}"));
    }
    if !valid_label(label) || label.starts_with("com.apple.") {
        return Err(format!("launchctl_bootout: refused label {label:?}"));
    }
    Ok(argv![
        "/bin/launchctl",
        "bootout",
        format!("{domain}/{label}")
    ])
}

/// `launchctl bootout` of a job that is not loaded (exit 3, "No such
/// process", "Could not find service") already has the state we want.
pub fn bootout_succeeded(o: &Output) -> bool {
    o.ok || o.code == Some(3)
        || o.stderr.contains("No such process")
        || o.stderr.contains("Could not find service")
        || o.stderr.contains("not loaded")
}

pub fn elevated_bootout(domain: &str, label: &str) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("launchctl_bootout: only on macOS".into());
    }
    let argv = bootout_argv(domain, label)?;
    let o = RealSys.run(&argv)?;
    if bootout_succeeded(&o) {
        Ok(format!("{domain}/{label} stopped"))
    } else {
        Err(format!("launchctl bootout: {}", o.stderr.trim()))
    }
}

/// Ready for a future `ElevatedOp::LaunchctlEnable` (see the handoff): turns
/// a LaunchDaemon off (or on) for good with `launchctl disable|enable`.
pub fn launchctl_state_argv(
    domain: &str,
    label: &str,
    enabled: bool,
) -> Result<Vec<String>, String> {
    if !valid_domain(domain) || !valid_label(label) || label.starts_with("com.apple.") {
        return Err(format!("launchctl: refused {domain}/{label}"));
    }
    Ok(argv![
        "launchctl",
        if enabled { "enable" } else { "disable" },
        format!("{domain}/{label}")
    ])
}

pub fn elevated_launchctl_state(
    domain: &str,
    label: &str,
    enabled: bool,
) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("launchctl: only on macOS".into());
    }
    RealSys.run_ok(&launchctl_state_argv(domain, label, enabled)?)?;
    Ok(format!(
        "{domain}/{label} {}",
        if enabled { "enabled" } else { "disabled" }
    ))
}

// ── Screen-only actions ──────────────────────────────────────────────

/// Actions only the screen may run (`washmybelly_ui` with `startup.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
        // Items only the system settings can switch (BTM login items and
        // agents of sandboxed apps): open the right pane.
        "open_settings" => {
            super::tools::blocking(|| {
                let argv: Vec<String> = if cfg!(target_os = "macos") {
                    argv![
                        "open",
                        "x-apple.systempreferences:com.apple.LoginItems-Settings.extension"
                    ]
                } else if cfg!(windows) {
                    argv!["explorer.exe", "ms-settings:startupapps"]
                } else {
                    return Err("NOT_AVAILABLE: no startup settings pane on this system".into());
                };
                RealSys.run(&argv)?;
                Ok(json!({ "ok": true }))
            })
            .await
        }
        _ => Err(format!("startup: unknown screen action {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_validation_refuses_odd_names() {
        assert!(bootout_argv("system", "com.vendor.daemon").is_ok());
        assert!(bootout_argv("gui/501", "com.vendor.agent").is_ok());
        assert!(bootout_argv("user/501", "x").is_err());
        assert!(bootout_argv("gui/", "x").is_err());
        assert!(bootout_argv("system", "com.apple.Finder").is_err());
        assert!(bootout_argv("system", "x; rm").is_err());
        assert!(bootout_argv("system", "a/b").is_err());
        assert_eq!(
            bootout_argv("gui/501", "com.vendor.agent").unwrap(),
            vec!["/bin/launchctl", "bootout", "gui/501/com.vendor.agent"]
        );
        let not_loaded = Output {
            ok: false,
            code: Some(3),
            stdout: String::new(),
            stderr: "Boot-out failed: 3: No such process".into(),
        };
        assert!(bootout_succeeded(&not_loaded));
        let denied = Output {
            ok: false,
            code: Some(1),
            stdout: String::new(),
            stderr: "Operation not permitted".into(),
        };
        assert!(!bootout_succeeded(&denied));
        assert!(task_argv("\\Vendor\\Task", false).is_ok());
        assert!(task_argv("Vendor", false).is_err());
        assert!(task_argv("\\a\\..\\b", false).is_err());
        assert!(task_argv("\\a\"b", false).is_err());
        assert_eq!(
            service_argv("AdobeARMservice", "manual", true).unwrap(),
            vec!["sc.exe", "config", "AdobeARMservice", "start=", "demand"]
        );
        assert_eq!(
            service_argv("syncthing.service", "disable", false).unwrap(),
            vec!["systemctl", "disable", "--", "syncthing.service"]
        );
        assert!(service_argv("SharedAccess", "manual", true)
            .unwrap_err()
            .starts_with("PROTECTED"));
        assert!(service_argv("WinDefend", "disabled", true).is_err());
        assert!(service_argv("systemd-logind.service", "disable", false).is_err());
        assert!(service_argv("-rf", "manual", true).is_err());
        assert!(service_argv("x", "mask", false).is_err());
    }

    #[test]
    fn updaters_are_the_only_suggestions() {
        assert!(looks_like_updater("com.google.GoogleUpdater.wake"));
        assert!(looks_like_updater("com.microsoft.update.agent"));
        assert!(looks_like_updater("us.zoom.updater"));
        assert!(!looks_like_updater("com.spotify.client.startuphelper"));
        assert!(!looks_like_updater("net.mullvad.daemon"));
    }

    #[test]
    fn impact_levels() {
        assert_eq!(Impact::from_ms(2105).level, "high");
        assert_eq!(Impact::from_ms(873).level, "medium");
        assert_eq!(Impact::from_ms(95).level, "low");
    }

    /// Reads this machine (changes nothing; state goes to a temp folder):
    /// `cargo test -p omniget-core --lib washmybelly::startup::tests::live_list -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the real startup items of this machine"]
    fn live_list() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let t0 = std::time::Instant::now();
        let v = list_and_offer(&ctx, &RealSys, &Env::real(), false, Source::Ui).unwrap();
        for it in v["items"].as_array().unwrap() {
            println!(
                "{:<12} {:<5} {:<5} {:<5} {:<40} {:<28} {}",
                it["kind"].as_str().unwrap(),
                it["enabled"],
                it["can_disable"],
                it["needs_admin"],
                it["name"].as_str().unwrap(),
                it["signer"].as_str().unwrap_or("-"),
                it["reason"].as_str().unwrap_or("")
            );
        }
        println!(
            "notes {:?} · on {} · {:?}",
            v["notes"],
            v["enabled_third_party"],
            t0.elapsed()
        );
    }
}
