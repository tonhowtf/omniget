pub mod apt;
pub mod brew;
pub mod dnf;
pub mod flatpak;
pub mod pacman;
pub mod snap;
pub mod winget;
pub mod zypper;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::apps::App;
use super::elevate::ElevatedOp;
use super::plan::{self, Op, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::store::Ctx;
use super::Risk;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RemovePreview {
    pub removes: Vec<String>,
    pub also_removes: Vec<String>,
    #[serde(default)]
    pub frees_bytes: Option<u64>,
    #[serde(default = "yes")]
    pub exact: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Update {
    pub id: String,
    pub name: String,
    pub manager: String,
    pub current: String,
    pub available: String,
    #[serde(default)]
    pub size: Option<u64>,
    pub admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub trait PackageBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn available(&self) -> bool;
    fn list(&self) -> Result<Vec<App>, String>;
    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String>;
    fn remove_argv(&self, names: &[String]) -> Vec<String>;
    fn orphans(&self) -> Result<Vec<App>, String>;
    fn updates(&self) -> Result<Vec<Update>, String>;
    fn upgrade_argv(&self, names: &[String]) -> Vec<String>;
    fn desktop_owners(&self, _files: &[PathBuf]) -> HashMap<PathBuf, String> {
        HashMap::new()
    }
}

pub const LINUX_MANAGERS: &[&str] = &["apt", "dnf", "pacman", "zypper", "snap", "flatpak"];

const SYSTEM_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

pub const MAX_NAMES: usize = 200;

pub fn backend(manager: &str) -> Option<Box<dyn PackageBackend>> {
    Some(match manager {
        "apt" => Box::new(apt::Apt),
        "dnf" => Box::new(dnf::Dnf),
        "pacman" => Box::new(pacman::Pacman),
        "zypper" => Box::new(zypper::Zypper),
        "snap" => Box::new(snap::Snap),
        "flatpak" => Box::new(flatpak::Flatpak),
        "winget" => Box::new(winget::Winget),
        "brew" => Box::new(brew::Brew),
        _ => return None,
    })
}

pub fn available() -> Vec<Box<dyn PackageBackend>> {
    let ids: Vec<&str> = if cfg!(target_os = "linux") {
        linux_order()
    } else if cfg!(windows) {
        vec!["winget"]
    } else {
        vec!["brew"]
    };
    ids.into_iter()
        .filter_map(backend)
        .filter(|b| b.available())
        .collect()
}

fn linux_order() -> Vec<&'static str> {
    let rel = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let mut order = distro_order(&super::parse::os_release(&rel));
    order.extend(["snap", "flatpak"]);
    order
}

pub fn distro_order(rel: &HashMap<String, String>) -> Vec<&'static str> {
    let mut family: Vec<String> = rel.get("ID").cloned().into_iter().collect();
    family.extend(
        rel.get("ID_LIKE")
            .map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    let pick = |f: &str| match f {
        "debian" | "ubuntu" | "linuxmint" | "pop" | "elementary" | "zorin" | "kali"
        | "raspbian" | "neon" => Some("apt"),
        "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "nobara" | "ol" | "amzn" => {
            Some("dnf")
        }
        "arch" | "manjaro" | "endeavouros" | "cachyos" | "garuda" | "artix" | "steamos" => {
            Some("pacman")
        }
        "opensuse" | "opensuse-tumbleweed" | "opensuse-leap" | "suse" | "sles" => Some("zypper"),
        _ => None,
    };
    let mut out: Vec<&'static str> = Vec::new();
    for f in &family {
        if let Some(m) = pick(f) {
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    for m in ["apt", "dnf", "pacman", "zypper"] {
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

fn re(pattern: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static pattern"))
}

pub fn valid_name(manager: &str, name: &str) -> bool {
    static APT: OnceLock<Regex> = OnceLock::new();
    static RPM: OnceLock<Regex> = OnceLock::new();
    static PACMAN: OnceLock<Regex> = OnceLock::new();
    static SNAP: OnceLock<Regex> = OnceLock::new();
    static FLATPAK: OnceLock<Regex> = OnceLock::new();
    static WINGET: OnceLock<Regex> = OnceLock::new();
    static BREW: OnceLock<Regex> = OnceLock::new();
    if name.is_empty() || name.len() > 255 || name.starts_with('-') {
        return false;
    }
    let r = match manager {
        "apt" => re(r"^[a-z0-9][a-z0-9+.\-]{1,127}(:[a-z0-9\-]{2,20})?$", &APT),
        "dnf" | "zypper" => re(r"^[A-Za-z0-9_][A-Za-z0-9._+\-]{0,127}$", &RPM),
        "pacman" => re(r"^[a-z0-9@_+][a-z0-9@._+\-]{0,127}$", &PACMAN),
        "snap" => re(r"^[a-z0-9](?:[a-z0-9\-]{0,38}[a-z0-9])?$", &SNAP),
        "flatpak" => re(
            r"^(?:(?:app|runtime)/)?[A-Za-z_][A-Za-z0-9_\-]*(?:\.[A-Za-z_][A-Za-z0-9_\-]*){2,}(?:/[A-Za-z0-9_\-]*/[A-Za-z0-9_.\-]+|//[A-Za-z0-9_.\-]+)?$",
            &FLATPAK,
        ),
        "winget" => re(r"^[A-Za-z0-9][A-Za-z0-9._+\-]{0,127}$", &WINGET),
        "brew" => re(
            r"^(?:[a-z0-9][a-z0-9_\-]*/[a-z0-9][a-z0-9_\-]*/)?[a-z0-9@][a-z0-9@._+\-]{0,127}$",
            &BREW,
        ),
        _ => return false,
    };
    r.is_match(name) && !name.contains("..")
}

pub fn validate(manager: &str, names: &[String], allowed: &[&str]) -> Result<(), String> {
    if !allowed.contains(&manager) {
        return Err(format!("BAD_MANAGER: {manager} is not allowed here"));
    }
    if names.is_empty() || names.len() > MAX_NAMES {
        return Err(format!(
            "BAD_NAMES: between 1 and {MAX_NAMES} package names"
        ));
    }
    for (i, n) in names.iter().enumerate() {
        if !valid_name(manager, n) {
            return Err(format!(
                "BAD_NAME: {n:?} is not a valid {manager} package name"
            ));
        }
        if names[..i].contains(n) {
            return Err(format!("BAD_NAMES: {n} is listed twice"));
        }
    }
    Ok(())
}

fn names_arg(args: &Value) -> Vec<String> {
    args.get("names")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub fn find_program(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(SYSTEM_DIRS.iter().map(PathBuf::from));
    let exts: &[&str] = if cfg!(windows) {
        &["exe", "cmd", "bat"]
    } else {
        &[""]
    };
    for d in dirs {
        for e in exts {
            let p = if e.is_empty() {
                d.join(name)
            } else {
                d.join(format!("{name}.{e}"))
            };
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub fn query(program: &str, args: &[&str]) -> Result<Ran, String> {
    query_env(program, args, &[])
}

pub fn query_env(program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<Ran, String> {
    let mut cmd = crate::core::process::std_command(program);
    cmd.args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(std::process::Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().map_err(|e| format!("{program}: {e}"))?;
    Ok(Ran {
        code: o.status.code(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    })
}

pub fn tail(text: &str, lines: usize) -> String {
    let v: Vec<&str> = text
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let s = v[v.len().saturating_sub(lines)..].join("\n");
    let start = s.len().saturating_sub(1500);
    let mut i = start;
    while !s.is_char_boundary(i) {
        i += 1;
    }
    s[i..].to_string()
}

pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.replace('\u{a0}', " ");
    let s = s.trim().trim_start_matches("< ").trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.replace(',', ".").parse().ok()?;
    let mult: f64 = match unit.trim() {
        "" | "B" | "bytes" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "k" | "K" | "KiB" => 1024.0,
        "M" | "MiB" => 1024.0 * 1024.0,
        "G" | "GiB" => 1024.0 * 1024.0 * 1024.0,
        "T" | "TiB" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mult).round() as u64)
}

fn run_as_root(argv: &[String], extra_env: &[(&str, &str)]) -> Result<String, String> {
    let (prog, args) = argv.split_first().ok_or("empty command")?;
    let path = SYSTEM_DIRS
        .iter()
        .map(|d| Path::new(d).join(prog))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{prog} is not installed in the system folders"))?;
    let mut cmd = std::process::Command::new(&path);
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("HOME", "/root")
        .stdin(std::process::Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let o = cmd.output().map_err(|e| format!("{prog}: {e}"))?;
    let out = String::from_utf8_lossy(&o.stdout);
    if o.status.success() {
        Ok(tail(&out, 12))
    } else {
        let err = String::from_utf8_lossy(&o.stderr);
        Err(format!(
            "{prog} failed ({}): {}",
            o.status.code().unwrap_or(-1),
            tail(&format!("{out}\n{err}"), 12)
        ))
    }
}

fn root_env(manager: &str) -> Vec<(&'static str, &'static str)> {
    if manager == "apt" {
        vec![("DEBIAN_FRONTEND", "noninteractive")]
    } else {
        Vec::new()
    }
}

pub fn elevated_remove(manager: &str, names: &[String]) -> Result<String, String> {
    validate(manager, names, LINUX_MANAGERS)?;
    if !cfg!(target_os = "linux") {
        return Err("package_remove: Linux only".into());
    }
    let b = backend(manager).ok_or("unknown manager")?;
    run_as_root(&b.remove_argv(names), &root_env(manager))
}

pub fn elevated_upgrade(manager: &str, names: &[String]) -> Result<String, String> {
    validate(manager, names, LINUX_MANAGERS)?;
    if !cfg!(target_os = "linux") {
        return Err("package_upgrade: Linux only".into());
    }
    let b = backend(manager).ok_or("unknown manager")?;
    run_as_root(&b.upgrade_argv(names), &root_env(manager))
}

pub fn remove_command(manager: &str) -> String {
    match backend(manager) {
        Some(b) if LINUX_MANAGERS.contains(&manager) => b.remove_argv(&[]).join(" "),
        _ => format!("# {manager}: no administrator removal"),
    }
}

pub fn upgrade_command(manager: &str) -> String {
    match backend(manager) {
        Some(b) if LINUX_MANAGERS.contains(&manager) => b.upgrade_argv(&[]).join(" "),
        _ => format!("# {manager}: no administrator upgrade"),
    }
}

const USER_UPGRADE: &[&str] = &["winget", "brew", "flatpak"];

pub fn upgrade_needs_admin(manager: &str, scope: Option<&str>) -> bool {
    match manager {
        "winget" | "brew" => false,
        "flatpak" => scope != Some("user"),
        _ => true,
    }
}

pub fn update_items(updates: &[Update]) -> Vec<PlanItem> {
    let mut items = Vec::new();
    let pacman: Vec<&Update> = updates
        .iter()
        .filter(|u| u.manager == "pacman" && u.note.is_none())
        .collect();
    if !pacman.is_empty() {
        let names: Vec<String> = pacman.iter().map(|u| u.name.clone()).collect();
        items.push(PlanItem {
            id: "packages:pacman:system".into(),
            label: format!("Update the whole system ({} packages)", names.len()),
            detail: Some(format!(
                "Arch updates everything together: {}",
                preview_names(&names)
            )),
            group: "pacman".into(),
            bytes: 0,
            risk: Risk::Destructive,
            admin: true,
            op: Op::Module {
                module: "packages".into(),
                action: "upgrade".into(),
                args: json!({ "manager": "pacman", "names": names }),
            },
        });
    }
    for u in updates
        .iter()
        .filter(|u| u.manager != "pacman" && u.note.is_none())
    {
        let mut args = json!({ "manager": u.manager, "names": [u.name] });
        if let Some(s) = &u.scope {
            args["scope"] = json!(s);
        }
        items.push(PlanItem {
            id: u.id.clone(),
            label: format!("Update {} {} → {}", u.name, u.current, u.available),
            detail: Some(u.manager.clone()),
            group: u.manager.clone(),
            bytes: 0,
            risk: Risk::Destructive,
            admin: u.admin,
            op: Op::Module {
                module: "packages".into(),
                action: "upgrade".into(),
                args,
            },
        });
    }
    items
}

fn preview_names(names: &[String]) -> String {
    let mut s = names
        .iter()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > 12 {
        s.push_str(&format!(" and {} more", names.len() - 12));
    }
    s
}

fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

pub fn all_updates() -> (Vec<Update>, Vec<String>, Vec<Value>) {
    let mut updates = Vec::new();
    let mut managers = Vec::new();
    let mut errors = Vec::new();
    if cfg!(target_os = "linux") && crate::core::dependencies::is_flatpak() {
        errors.push(json!({ "manager": "*", "error": "washmybelly.reason.flatpak_sandbox" }));
        return (updates, managers, errors);
    }
    let found = available();
    let total = found.len() as u64;
    for (i, b) in found.iter().enumerate() {
        super::progress("updates_list", "scan", i as u64, total, b.id());
        managers.push(b.id().to_string());
        match b.updates() {
            Ok(u) => updates.extend(u),
            Err(e) => errors.push(json!({ "manager": b.id(), "error": e })),
        }
    }
    super::progress("updates_list", "done", total, total, "");
    (updates, managers, errors)
}

pub async fn tool_updates(a: Value) -> Result<Value, String> {
    let source = source_of(&a);
    super::tools::blocking(move || {
        let (updates, managers, errors) = all_updates();
        let items = update_items(&updates);
        let plan_id = if items.is_empty() {
            Value::Null
        } else {
            let ctx = Ctx::app()?;
            json!(plan::offer(&ctx, "packages", items, source)?.id)
        };
        Ok(json!({ "items": updates, "total": updates.len(), "managers": managers, "errors": errors, "plan_id": plan_id }))
    })
    .await
}

pub const ACTIONS: &[&str] = &["upgrade"];

fn scope_arg(args: &Value) -> Option<&str> {
    args.get("scope").and_then(Value::as_str)
}

pub fn plan_execute(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    if action != "upgrade" {
        return Err(format!("packages: unknown action {action}"));
    }
    let manager = args.get("manager").and_then(Value::as_str).unwrap_or("");
    let names = names_arg(args);
    validate(manager, &names, USER_UPGRADE)?;
    let scope = scope_arg(args);
    if upgrade_needs_admin(manager, scope) {
        return Err(format!(
            "packages: {manager} upgrades need administrator rights"
        ));
    }
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for name in &names {
        let r = match manager {
            "winget" => winget::upgrade_one(name),
            "brew" => brew::upgrade_one(name, scope.unwrap_or("formula")),
            "flatpak" => flatpak::upgrade_user(name),
            _ => Err("unsupported".into()),
        };
        match r {
            Ok(d) => done.push(format!("{name}: {d}")),
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    if failed.is_empty() {
        Ok(Outcome {
            detail: done.join("; "),
            bytes: 0,
            undo: None,
        })
    } else {
        Err(failed.join("; "))
    }
}

pub fn plan_elevated(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    if action != "upgrade" {
        return Err(format!(
            "packages: action {action} has no administrator form"
        ));
    }
    let manager = args.get("manager").and_then(Value::as_str).unwrap_or("");
    let names = names_arg(args);
    validate(manager, &names, LINUX_MANAGERS)?;
    Ok((vec![upgrade_op(manager, names)?], None))
}

fn upgrade_op(manager: &str, names: Vec<String>) -> Result<ElevatedOp, String> {
    Ok(ElevatedOp::PackageUpgrade {
        manager: manager.to_string(),
        names,
    })
}

pub fn plan_undo(_ctx: &Ctx, action: &str, _args: &Value) -> Result<String, String> {
    Err(format!(
        "packages: {action} cannot be undone (the manager keeps no previous version)"
    ))
}

pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    let action = action.to_string();
    super::tools::blocking(move || match action.as_str() {
        "managers" => {
            Ok(json!({ "managers": available().iter().map(|b| b.id()).collect::<Vec<_>>() }))
        }
        "remove_preview" => {
            let manager = a
                .get("manager")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let names = names_arg(&a);
            validate(&manager, &names, LINUX_MANAGERS)?;
            let b = backend(&manager).ok_or("unknown manager")?;
            Ok(json!(b.remove_preview(&names)?))
        }
        "orphans" => {
            let only = a.get("manager").and_then(Value::as_str).map(String::from);
            let mut out = Vec::new();
            for b in available() {
                if only.as_deref().is_some_and(|m| m != b.id()) {
                    continue;
                }
                out.extend(b.orphans().unwrap_or_default());
            }
            Ok(json!({ "orphans": out }))
        }
        other => Err(format!("packages: unknown screen action {other}")),
    })
    .await
}

pub fn package_app(
    manager: &str,
    name: &str,
    version: &str,
    bytes: Option<u64>,
    summary: &str,
    publisher: Option<String>,
) -> App {
    App {
        id: format!("{manager}:{name}"),
        name: name.to_string(),
        version: (!version.is_empty()).then(|| version.to_string()),
        publisher,
        source: manager.to_string(),
        bytes,
        native_id: Some(name.to_string()),
        removable: true,
        needs_admin: manager != "flatpak",
        extra: json!({ "summary": summary }),
        ..Default::default()
    }
}
