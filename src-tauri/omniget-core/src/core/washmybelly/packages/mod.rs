//! Package managers: installed packages, removal previews, orphans and
//! pending updates, one [`PackageBackend`] per manager (F3/F4).
//!
//! - Linux: apt, dnf, pacman, zypper, snap (snapd REST API on its socket),
//!   flatpak. Removing needs administrator rights except for flatpak in the
//!   user installation; the helper runs [`elevated_remove`] as root.
//! - Windows: winget (updates, one process per package, winutil's exit code
//!   table for the outcome).
//! - macOS: Homebrew (updates).
//!
//! Every command is argv with `LC_ALL=C` (the parsers read English labels)
//! and `--` before package names; names are checked against a strict pattern
//! per manager before they reach any command. Every parser is a pure function
//! with a fixture in `tests/washmybelly_fixtures/packages/`.

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

// ── Shapes ───────────────────────────────────────────────────────────

/// What a removal would take with it, computed without changing anything.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RemovePreview {
    /// The packages asked for.
    pub removes: Vec<String>,
    /// Everything else the manager removes with them (dependents, unused
    /// dependencies, locale extensions).
    pub also_removes: Vec<String>,
    /// Bytes the removal frees, when the manager says.
    #[serde(default)]
    pub frees_bytes: Option<u64>,
    /// False when the manager could only give a partial answer (zypper
    /// without root falls back to `rpm -e --test`).
    #[serde(default = "yes")]
    pub exact: bool,
}

fn yes() -> bool {
    true
}

/// One pending update.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Update {
    /// Stable item id: `packages:<manager>[:<scope>]:<name>`.
    pub id: String,
    /// Package name as the manager knows it (winget id, flatpak ref...).
    pub name: String,
    pub manager: String,
    pub current: String,
    pub available: String,
    /// Download size when the manager says.
    #[serde(default)]
    pub size: Option<u64>,
    pub admin: bool,
    /// `user` or `system` (flatpak), `formula` or `cask` (Homebrew).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Shown instead of offering the update (a pinned formula, an id winget
    /// cut short). Such updates never enter a plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One package manager. Implementations only read, except through the argv
/// they hand back: removal and upgrade run elsewhere (the elevated helper or
/// a plan hook), after the names were validated again.
pub trait PackageBackend: Send + Sync {
    /// `apt`, `dnf`, `pacman`, `zypper`, `snap`, `flatpak`, `winget`, `brew`.
    fn id(&self) -> &'static str;
    /// The manager is installed and usable on this machine.
    fn available(&self) -> bool;
    /// Installed packages (every one, apps and libraries).
    fn list(&self) -> Result<Vec<App>, String>;
    /// What removing `names` would take along. Changes nothing.
    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String>;
    /// The argv (program name first, bare) that removes `names`
    /// non-interactively. With no names it is the command prefix shown in
    /// "copy the command".
    fn remove_argv(&self, names: &[String]) -> Vec<String>;
    /// Packages nothing needs anymore.
    fn orphans(&self) -> Result<Vec<App>, String>;
    /// Pending updates.
    fn updates(&self) -> Result<Vec<Update>, String>;
    /// The argv that upgrades `names` non-interactively.
    fn upgrade_argv(&self, names: &[String]) -> Vec<String>;
    /// Package owning each of `files` (only the ones owned), for telling
    /// apps (a package with a `.desktop` file) from libraries.
    fn desktop_owners(&self, _files: &[PathBuf]) -> HashMap<PathBuf, String> {
        HashMap::new()
    }
}

/// Managers the root helper may drive. Closed list.
pub const LINUX_MANAGERS: &[&str] = &["apt", "dnf", "pacman", "zypper", "snap", "flatpak"];

/// Folders the helper resolves programs in (never `PATH`).
const SYSTEM_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

/// Most names one removal or upgrade may carry.
pub const MAX_NAMES: usize = 200;

/// The backend for `manager`, whatever the OS (the parsers and argv are
/// pure; only `available`/`list`/... touch the system).
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

/// Backends of this OS that are installed, the distro's own manager first.
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

/// apt/dnf/pacman/zypper ordered by `/etc/os-release` (`ID`, `ID_LIKE`),
/// then snap and flatpak. The binary still has to be there: a Debian with
/// `rpm` installed does not make dnf the system manager.
fn linux_order() -> Vec<&'static str> {
    let rel = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let mut order = distro_order(&super::parse::os_release(&rel));
    order.extend(["snap", "flatpak"]);
    order
}

/// Pure: system managers by distro family, most likely first.
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

// ── Names ────────────────────────────────────────────────────────────

fn re(pattern: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static pattern"))
}

/// Whether `name` is a well-formed package name for `manager`. Nothing that
/// starts with `-`, has a space, a quote, `;`, `$`, `/..` or a control
/// character ever passes, so a name cannot turn into an option or a shell
/// word even in "copy the command".
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
        // Debian policy: lowercase, digits, + - . ; optional :arch.
        "apt" => re(r"^[a-z0-9][a-z0-9+.\-]{1,127}(:[a-z0-9\-]{2,20})?$", &APT),
        // rpm names: letters, digits, . _ + -.
        "dnf" | "zypper" => re(r"^[A-Za-z0-9_][A-Za-z0-9._+\-]{0,127}$", &RPM),
        // Arch: lowercase, digits, @ . _ + -, not starting with - or .
        "pacman" => re(r"^[a-z0-9@_+][a-z0-9@._+\-]{0,127}$", &PACMAN),
        // snapd: lowercase, digits, hyphens, at most 40, no hyphen at the ends.
        "snap" => re(r"^[a-z0-9](?:[a-z0-9\-]{0,38}[a-z0-9])?$", &SNAP),
        // Reverse-DNS id with at least three parts, or `id//branch`, or a
        // full ref `app|runtime/id/arch/branch`.
        "flatpak" => re(
            r"^(?:(?:app|runtime)/)?[A-Za-z_][A-Za-z0-9_\-]*(?:\.[A-Za-z_][A-Za-z0-9_\-]*){2,}(?:/[A-Za-z0-9_\-]*/[A-Za-z0-9_.\-]+|//[A-Za-z0-9_.\-]+)?$",
            &FLATPAK,
        ),
        // winget ids (`Mozilla.Firefox`, `7zip.7zip`, Store ids `9NBLGGH4NNS1`).
        "winget" => re(r"^[A-Za-z0-9][A-Za-z0-9._+\-]{0,127}$", &WINGET),
        // Homebrew formula/cask tokens, optionally `user/tap/name`.
        "brew" => re(
            r"^(?:[a-z0-9][a-z0-9_\-]*/[a-z0-9][a-z0-9_\-]*/)?[a-z0-9@][a-z0-9@._+\-]{0,127}$",
            &BREW,
        ),
        _ => return false,
    };
    r.is_match(name) && !name.contains("..")
}

/// Check a whole request: the manager is known, 1..=[`MAX_NAMES`] names,
/// each well formed, no duplicates.
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

// ── Running commands ─────────────────────────────────────────────────

/// A program on `PATH` or in the usual system folders.
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

/// What a read-only query printed.
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Run a read-only query with `LC_ALL=C`, stdin closed. Never fails on a
/// non-zero exit (several managers answer "nothing" that way); only when
/// the program cannot start.
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

/// The last lines of a command's output, for receipts and errors.
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

/// Sizes as package managers print them: `345.21 KiB`, `2.9 MiB`, `1.2 GB`
/// (GLib, SI), `30 M` (dnf4, binary), `40.0   B`, with a no-break space or
/// not. `None` when it is not a size.
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

/// Run a manager as root inside `omniget-elevate`: the program is looked up
/// only in the system folders, the environment is emptied, stdin closed.
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

/// `omniget-elevate`: remove packages as root. Revalidates the manager
/// against [`LINUX_MANAGERS`] and every name against its pattern; flatpak
/// here is always the system installation.
pub fn elevated_remove(manager: &str, names: &[String]) -> Result<String, String> {
    validate(manager, names, LINUX_MANAGERS)?;
    if !cfg!(target_os = "linux") {
        return Err("package_remove: Linux only".into());
    }
    let b = backend(manager).ok_or("unknown manager")?;
    run_as_root(&b.remove_argv(names), &root_env(manager))
}

/// `omniget-elevate`: upgrade packages as root (needs the
/// `ElevatedOp::PackageUpgrade` variant; see the handoff).
pub fn elevated_upgrade(manager: &str, names: &[String]) -> Result<String, String> {
    validate(manager, names, LINUX_MANAGERS)?;
    if !cfg!(target_os = "linux") {
        return Err("package_upgrade: Linux only".into());
    }
    let b = backend(manager).ok_or("unknown manager")?;
    run_as_root(&b.upgrade_argv(names), &root_env(manager))
}

/// The command a person would type to remove packages with `manager`
/// (display only, for "copy the command"; the names are appended quoted).
pub fn remove_command(manager: &str) -> String {
    match backend(manager) {
        Some(b) if LINUX_MANAGERS.contains(&manager) => b.remove_argv(&[]).join(" "),
        _ => format!("# {manager}: no administrator removal"),
    }
}

/// Same for upgrades.
pub fn upgrade_command(manager: &str) -> String {
    match backend(manager) {
        Some(b) if LINUX_MANAGERS.contains(&manager) => b.upgrade_argv(&[]).join(" "),
        _ => format!("# {manager}: no administrator upgrade"),
    }
}

// ── Updates ──────────────────────────────────────────────────────────

/// Managers whose upgrades run without administrator rights (in the app's
/// own process, one process per package).
const USER_UPGRADE: &[&str] = &["winget", "brew", "flatpak"];

/// Whether an upgrade by `manager` in `scope` needs the helper.
pub fn upgrade_needs_admin(manager: &str, scope: Option<&str>) -> bool {
    match manager {
        "winget" | "brew" => false,
        "flatpak" => scope != Some("user"),
        _ => true,
    }
}

/// Plan items for `updates`. pacman does not support partial upgrades, so
/// its updates become one "whole system" item.
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

/// Updates from every available manager, with the errors of the ones that
/// failed (a broken repo must not hide the others).
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

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "packages" }`) ──

/// Actions this module runs itself.
pub const ACTIONS: &[&str] = &["upgrade"];

fn scope_arg(args: &Value) -> Option<&str> {
    args.get("scope").and_then(Value::as_str)
}

/// Upgrade without administrator rights: winget and Homebrew, flatpak in the
/// user installation. One process per package.
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

/// The administrator half of an upgrade (apt, dnf, pacman, zypper, snap,
/// flatpak system).
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

/// The helper operation for an administrator upgrade, executed by
/// [`elevated_upgrade`] and shown by [`upgrade_command`].
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

/// Actions only the screen may run (`washmybelly_ui` with `packages.<action>`):
/// never exposed as MCP tools.
///
/// - `managers`: the managers found here.
/// - `remove_preview` `{manager, names}`: what else a removal takes along.
/// - `orphans` `{manager?}`: packages nothing needs.
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

// ── Shared helpers for backends ──────────────────────────────────────

/// A package as an [`App`] of source `manager`.
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

#[cfg(test)]
pub(crate) fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/washmybelly_fixtures/packages")
            .join(name),
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_could_become_options_or_shell_words_are_refused() {
        for m in [
            "apt", "dnf", "pacman", "zypper", "snap", "flatpak", "winget", "brew",
        ] {
            for bad in [
                "-rf", "--all", "a b", "a;b", "a$(x)", "a`b`", "a'b", "a\"b", "a\nb", "", "a|b",
                "a&b", "../x",
            ] {
                assert!(!valid_name(m, bad), "{m} accepted {bad:?}");
            }
        }
        assert!(valid_name("apt", "libc6:i386"));
        assert!(valid_name("apt", "g++"));
        assert!(!valid_name("apt", "Nano"));
        assert!(valid_name("dnf", "NetworkManager"));
        assert!(valid_name("pacman", "lib32-glibc"));
        assert!(!valid_name("snap", "bad;name"));
        assert!(!valid_name("snap", "-x"));
        assert!(valid_name("snap", "firefox"));
        assert!(valid_name("flatpak", "org.gimp.GIMP"));
        assert!(valid_name("flatpak", "org.gimp.GIMP//stable"));
        assert!(valid_name(
            "flatpak",
            "runtime/org.freedesktop.Platform.GL.default/x86_64/24.08"
        ));
        assert!(!valid_name("flatpak", "gimp"));
        assert!(!valid_name("flatpak", "com.example.Bad;rm"));
        assert!(valid_name("winget", "Microsoft.VisualStudioCode"));
        assert!(valid_name("winget", "9NBLGGH4NNS1"));
        assert!(!valid_name("winget", "Microsoft.EdgeWebView2Run…"));
        assert!(valid_name("brew", "python@3.12"));
        assert!(valid_name("brew", "homebrew/cask/firefox"));
        assert!(!valid_name("nope", "x"));
    }

    #[test]
    fn validate_checks_the_manager_count_and_duplicates() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(validate("apt", &n(&["nano"]), LINUX_MANAGERS).is_ok());
        assert!(validate("winget", &n(&["Git.Git"]), LINUX_MANAGERS)
            .unwrap_err()
            .starts_with("BAD_MANAGER"));
        assert!(validate("apt", &[], LINUX_MANAGERS).is_err());
        assert!(validate("apt", &n(&["nano", "nano"]), LINUX_MANAGERS).is_err());
        assert!(validate("apt", &n(&["nano", "-y"]), LINUX_MANAGERS)
            .unwrap_err()
            .starts_with("BAD_NAME"));
        assert!(validate("apt", &n(&["nano; rm -rf /"]), LINUX_MANAGERS).is_err());
        let many: Vec<String> = (0..=MAX_NAMES).map(|i| format!("pkg{i}")).collect();
        assert!(validate("apt", &many, LINUX_MANAGERS).is_err());
    }

    #[test]
    fn elevated_remove_refuses_before_running_anything() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(elevated_remove("rm", &n(&["x"]))
            .unwrap_err()
            .starts_with("BAD_MANAGER"));
        assert!(elevated_remove("apt", &n(&["--purge"]))
            .unwrap_err()
            .starts_with("BAD_NAME"));
        assert!(elevated_remove("pacman", &n(&["a b"]))
            .unwrap_err()
            .starts_with("BAD_NAME"));
        assert!(elevated_upgrade("brew", &n(&["x"]))
            .unwrap_err()
            .starts_with("BAD_MANAGER"));
    }

    #[test]
    fn argv_puts_double_dash_before_names() {
        let n = vec!["nano".to_string()];
        for m in LINUX_MANAGERS {
            let b = backend(m).unwrap();
            let argv = b.remove_argv(&n);
            let dd = argv
                .iter()
                .position(|a| a == "--")
                .unwrap_or_else(|| panic!("{m}: no --"));
            assert_eq!(&argv[dd + 1..], &n[..], "{m}");
            if *m != "pacman" {
                let up = b.upgrade_argv(&n);
                assert!(up.contains(&"--".to_string()), "{m} upgrade");
            }
        }
        assert_eq!(remove_command("apt"), "apt-get -y remove --");
        assert_eq!(remove_command("pacman"), "pacman -Rns --noconfirm --");
        assert_eq!(
            remove_command("flatpak"),
            "flatpak uninstall -y --noninteractive --system --"
        );
        assert!(remove_command("winget").starts_with('#'));
        assert_eq!(upgrade_command("pacman"), "pacman -Syu --noconfirm");
    }

    #[test]
    fn sizes_in_every_dialect() {
        assert_eq!(parse_size("345.21 KiB"), Some(353_495));
        assert_eq!(parse_size("2.9 MiB"), Some(3_040_870));
        assert_eq!(parse_size("1.2\u{a0}GB"), Some(1_200_000_000));
        assert_eq!(parse_size("30 M"), Some(31_457_280));
        assert_eq!(parse_size("40.0   B"), Some(40));
        assert_eq!(parse_size("< 24.1 MB"), Some(24_100_000));
        assert_eq!(parse_size("stable"), None);
    }

    #[test]
    fn distro_order_follows_os_release() {
        let rel = super::super::parse::os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n");
        assert_eq!(distro_order(&rel)[0], "apt");
        let rel =
            super::super::parse::os_release("ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n");
        assert_eq!(distro_order(&rel)[0], "zypper");
        let rel = super::super::parse::os_release("ID=cachyos\nID_LIKE=arch\n");
        assert_eq!(distro_order(&rel)[0], "pacman");
        assert_eq!(
            distro_order(&HashMap::new()),
            vec!["apt", "dnf", "pacman", "zypper"]
        );
    }

    #[test]
    fn update_plan_items_follow_the_manager_rules() {
        let u = |m: &str, n: &str, admin: bool, scope: Option<&str>| Update {
            id: format!("packages:{m}:{n}"),
            name: n.into(),
            manager: m.into(),
            current: "1".into(),
            available: "2".into(),
            admin,
            scope: scope.map(String::from),
            ..Default::default()
        };
        let mut pinned = u("brew", "node", false, Some("formula"));
        pinned.note = Some("pinned".into());
        let items = update_items(&[
            u("pacman", "bash", true, None),
            u("pacman", "systemd", true, None),
            u("winget", "Git.Git", false, None),
            u(
                "flatpak",
                "app/org.videolan.VLC/x86_64/stable",
                false,
                Some("user"),
            ),
            u("apt", "tzdata", true, None),
            pinned,
        ]);
        assert_eq!(
            items.len(),
            4,
            "pacman folds into one item, the pinned one stays out"
        );
        assert_eq!(items[0].id, "packages:pacman:system");
        assert!(items[0].admin);
        let Op::Module {
            module,
            action,
            args,
        } = &items[1].op
        else {
            panic!()
        };
        assert_eq!((module.as_str(), action.as_str()), ("packages", "upgrade"));
        assert_eq!(args["names"], json!(["Git.Git"]));
        assert!(!items[1].admin);
        assert_eq!(items[2].op_args()["scope"], "user");
        assert!(items[3].admin);
        assert!(items.iter().all(|i| i.risk == Risk::Destructive));
    }

    #[test]
    fn upgrade_hooks_refuse_the_wrong_side() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        // apt through the user path: refused (apt is not a user manager).
        let r = plan_execute(
            &ctx,
            "r",
            "upgrade",
            &json!({ "manager": "apt", "names": ["nano"] }),
        );
        assert!(r.unwrap_err().starts_with("BAD_MANAGER"));
        // flatpak system through the user path: refused.
        let r = plan_execute(
            &ctx,
            "r",
            "upgrade",
            &json!({ "manager": "flatpak", "scope": "system", "names": ["org.gimp.GIMP"] }),
        );
        assert!(r.unwrap_err().contains("administrator"));
        // A bad name never reaches a command.
        let r = plan_execute(
            &ctx,
            "r",
            "upgrade",
            &json!({ "manager": "winget", "names": ["--all"] }),
        );
        assert!(r.unwrap_err().starts_with("BAD_NAME"));
        assert!(plan_execute(&ctx, "r", "remove", &json!({})).is_err());
        let r = plan_elevated(
            &ctx,
            "r",
            "upgrade",
            &json!({ "manager": "winget", "names": ["Git.Git"] }),
        );
        assert!(r.unwrap_err().starts_with("BAD_MANAGER"));
    }

    /// Reads the real managers (never changes anything). Run on Linux:
    /// `cargo test -p omniget-core --lib washmybelly::packages::tests::live_read_only -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real package managers of the machine"]
    fn live_read_only() {
        for b in available() {
            let list = b.list().unwrap();
            println!("{}: {} installed", b.id(), list.len());
            if let Some(p) = list.iter().find(|a| a.name == "less" || a.name == "nano") {
                println!(
                    "  preview {}: {:?}",
                    p.name,
                    b.remove_preview(std::slice::from_ref(&p.name))
                );
            }
            println!("  orphans: {:?}", b.orphans().map(|o| o.len()));
            println!(
                "  updates: {:?}",
                b.updates()
                    .map(|u| u.into_iter().take(3).collect::<Vec<_>>())
            );
        }
        let apps = super::super::apps::list().unwrap();
        println!("apps: {}", apps.len());
    }

    /// Really removes `nano` as root through [`elevated_remove`]. Only in a
    /// throwaway container with `WMB_DISPOSABLE=1` (see the handoff).
    #[test]
    #[ignore = "removes a real package; disposable Linux container only"]
    fn live_root_remove_in_a_container() {
        if std::env::var("WMB_DISPOSABLE").as_deref() != Ok("1") {
            return;
        }
        let m = available()
            .into_iter()
            .find(|b| SYSTEM_MANAGERS_FOR_TEST.contains(&b.id()))
            .expect("a system manager");
        let out = elevated_remove(m.id(), &["nano".to_string()]).unwrap();
        println!("{}: {out}", m.id());
        assert!(m
            .list()
            .unwrap()
            .iter()
            .all(|a| a.native_id.as_deref() != Some("nano")));
    }

    const SYSTEM_MANAGERS_FOR_TEST: &[&str] = &["apt", "dnf", "pacman", "zypper"];

    trait OpArgs {
        fn op_args(&self) -> Value;
    }
    impl OpArgs for PlanItem {
        fn op_args(&self) -> Value {
            match &self.op {
                Op::Module { args, .. } => args.clone(),
                _ => Value::Null,
            }
        }
    }
}
