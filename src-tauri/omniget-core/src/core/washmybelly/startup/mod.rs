macro_rules! argv {
    ($($x:expr),* $(,)?) => { vec![$(String::from($x)),*] };
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Impact {
    pub ms: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StartupItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub scope: String,
    pub location: String,
    pub command: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    #[serde(default)]
    pub signer: Option<String>,
    #[serde(default)]
    pub signer_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default)]
    pub installed: Option<i64>,
    #[serde(default)]
    pub impact: Option<Impact>,
    pub can_disable: bool,
    pub can_enable: bool,
    pub needs_admin: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub recommended: bool,
    pub third_party: bool,
    #[serde(skip)]
    pub off: Option<Action>,
    #[serde(skip)]
    pub on: Option<Action>,
}

impl StartupItem {
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

#[derive(Debug, Clone, Default)]
pub struct Output {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub trait Sys: Sync {
    fn run(&self, argv: &[String]) -> Result<Output, String>;

    fn run_env(&self, argv: &[String], env: &[(String, String)]) -> Result<Output, String> {
        let _ = env;
        self.run(argv)
    }

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

#[derive(Debug, Clone)]
pub struct Env {
    pub uid: u32,
    pub home: PathBuf,
    pub autostart_user: PathBuf,
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

pub fn file_time(path: &std::path::Path) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    m.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

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

pub fn windows_approved_enabled(hex: &str) -> Option<bool> {
    let b = u8::from_str_radix(hex.trim().get(0..2)?, 16).ok()?;
    match b {
        0x02 | 0x06 => Some(true),
        0x03 | 0x07 => Some(false),
        _ => None,
    }
}

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
    let source = super::plan::source_of(&a);
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

pub fn valid_label(label: &str) -> bool {
    static RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"^[A-Za-z0-9._\-]{1,255}$").unwrap());
    RE.is_match(label)
}

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

pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
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
