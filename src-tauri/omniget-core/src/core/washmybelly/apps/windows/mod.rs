//! Apps on Windows (F3): inventory, uninstall, leftovers.
//!
//! This module ports the design of Bulk Crap Uninstaller
//! (https://github.com/Klocman/Bulk-Crap-Uninstaller, Copyright (c) 2017
//! Marcin Szeniak, Apache License 2.0; NOTICE in
//! `src-tauri/omniget-core/data/washmybelly/bcu/NOTICE`). The C# code was
//! read and rewritten in Rust with changes: winget support, plans and
//! receipts, the Trash for files and `.reg` backups for keys, no UI
//! automation, no shell. Appx data comes from Win11Debloat, Sophia Script
//! and winutil (MIT), see `catalog.rs`.
//!
//! Layout: `parse` (pure text parsers), `catalog` (Appx classification),
//! `inventory` (sources → apps, merge), `leftovers` (confidence), `run`
//! (commands and the stall check) are compiled everywhere and tested on
//! every OS; `registry` and `sys` touch Windows and exist only there.

#![cfg_attr(not(windows), allow(dead_code))]

pub mod catalog;
pub mod inventory;
pub mod leftovers;
pub mod parse;
#[cfg(windows)]
pub mod registry;
pub mod run;
#[cfg(windows)]
pub mod sys;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::elevate::ElevatedOp;
use super::super::fsops;
use super::super::plan::{Op, Outcome, PlanItem};
use super::super::protect::{self, Guard};
use super::super::receipt::Undo;
use super::super::store::Ctx;
use super::super::{Destination, Risk};
use super::{App, Leftover};
use inventory::x;
use leftovers::Found;

// ── Inventory ───────────────────────────────────────────────────────

pub fn list() -> Result<Vec<App>, String> {
    #[cfg(windows)]
    {
        if let Some(apps) = cached() {
            return Ok(apps);
        }
        let apps = sys::list()?;
        if let Ok(ctx) = Ctx::app() {
            remember(&ctx, &apps);
        }
        store_cache(&apps);
        return Ok(apps);
    }
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

/// A full listing takes seconds (winget, PowerShell): `wmb_app_leftovers`
/// right after `wmb_apps_list` reuses it. Any uninstall clears it.
const CACHE_SECS: u64 = 90;

static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<App>)>> =
    std::sync::Mutex::new(None);

fn cached() -> Option<Vec<App>> {
    let g = CACHE.lock().ok()?;
    let (at, apps) = g.as_ref()?;
    (at.elapsed().as_secs() < CACHE_SECS).then(|| apps.clone())
}

fn store_cache(apps: &[App]) {
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((std::time::Instant::now(), apps.to_vec()));
    }
}

fn forget_cache() {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
}

/// What we remember of apps seen in a listing, so leftovers of an app
/// that is gone can still be searched by its publisher and folder.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Seen {
    pub name: String,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub reg_root: Option<String>,
    #[serde(default)]
    pub reg_key: Option<String>,
}

const SEEN_KEY: &str = "apps.win.seen";

pub fn remember(ctx: &Ctx, apps: &[App]) {
    let mut m: BTreeMap<String, Seen> = ctx.kv_get(SEEN_KEY).unwrap_or_default();
    if m.len() > 4000 {
        m.clear();
    }
    for a in apps
        .iter()
        .filter(|a| !matches!(a.source.as_str(), "appx" | "orphan_dir"))
    {
        m.insert(
            a.id.clone(),
            Seen {
                name: a.name.clone(),
                publisher: a.publisher.clone(),
                path: a.path.clone(),
                reg_root: x(a, "reg_root").map(str::to_string),
                reg_key: x(a, "reg_key").map(str::to_string),
            },
        );
    }
    let _ = ctx.kv_set(SEEN_KEY, &m);
}

/// Fill a bare app (already removed) from what a listing saw: by id, or
/// by name for the screen's `removed:<name>`.
pub fn recall(ctx: &Ctx, app: &App) -> App {
    let m: BTreeMap<String, Seen> = ctx.kv_get(SEEN_KEY).unwrap_or_default();
    let wanted = parse::normalize_name(&app.name);
    let hit = m
        .get(&app.id)
        .map(|s| (app.id.clone(), s.clone()))
        .or_else(|| {
            m.iter()
                .find(|(_, s)| !wanted.is_empty() && parse::normalize_name(&s.name) == wanted)
                .map(|(k, s)| (k.clone(), s.clone()))
        });
    let mut out = app.clone();
    if let Some((id, s)) = hit {
        out.name = s.name;
        out.publisher = out.publisher.or(s.publisher);
        out.path = out.path.or(s.path);
        let mut extra = if out.extra.is_object() {
            out.extra.clone()
        } else {
            json!({})
        };
        extra["reg_root"] = json!(s.reg_root);
        extra["reg_key"] = json!(s.reg_key);
        extra["seen_as"] = json!(id);
        out.extra = extra;
    }
    out
}

fn installed(app: &App) -> bool {
    app.source != "removed" && app.reason.as_deref() != Some("not installed anymore")
}

// ── Uninstall items ─────────────────────────────────────────────────

/// A candidate kept out of the default plan, with the reason.
#[derive(Debug, Clone)]
pub struct Unticked {
    pub item: PlanItem,
    pub why: String,
}

/// What `uninstall_items` builds before registering anything.
#[derive(Debug, Clone, Default)]
pub struct Built {
    pub ticked: Vec<PlanItem>,
    pub unticked: Vec<Unticked>,
    pub leftovers: Vec<Value>,
    pub notes: Vec<String>,
    pub command: Option<String>,
}

/// The command a person would run, for display.
pub fn command_preview(app: &App) -> Option<String> {
    let sr = r"C:\Windows";
    match app.source.as_str() {
        "appx" => Some(format!(
            "Get-AppxPackage -Name {} | Remove-AppxPackage",
            app.extra["appx"]["name"].as_str().unwrap_or("")
        )),
        "steam" => x(app, "steam_appid").map(|id| format!("steam.exe steam://uninstall/{id}")),
        "scoop" => x(app, "scoop_name").map(|n| {
            format!(
                "scoop uninstall {n}{}",
                if app.extra["scoop_global"] == true {
                    " --global"
                } else {
                    ""
                }
            )
        }),
        "orphan_dir" => app
            .path
            .as_ref()
            .map(|p| format!("Remove-Item -LiteralPath \"{p}\" -Recurse")),
        _ => {
            if let Some(id) = x(app, "winget_id") {
                return run::winget_uninstall(id).ok().map(|p| p.display());
            }
            if x(app, "installer_kind") == Some("msi") {
                if let Some(code) = x(app, "product_code") {
                    return run::msi_uninstall(code, sr).ok().map(|p| {
                        p.display()
                            .replace(r"C:\Windows\System32\msiexec.exe", "msiexec.exe")
                    });
                }
            }
            x(app, "quiet_uninstall_string")
                .or_else(|| x(app, "uninstall_string"))
                .map(str::to_string)
        }
    }
}

/// The item that removes the app itself (group `app`), and whether it is
/// ticked. `Err` is the reason there is none.
pub fn main_item(app: &App, running: &[String]) -> Result<(PlanItem, Option<String>), String> {
    if !app.removable {
        return Err(app
            .reason
            .clone()
            .unwrap_or_else(|| "Windows does not let this app be removed".into()));
    }
    let label = format!("Uninstall {}", app.name);
    let id = format!("uninstall:win:app:{}", app.id);
    let detail = command_preview(app).or_else(|| app.path.clone());
    let bytes = app.bytes.unwrap_or(0);
    let item = |risk: Risk, admin: bool, op: Op| PlanItem {
        id: id.clone(),
        label: label.clone(),
        detail: detail.clone(),
        group: "app".into(),
        bytes,
        risk,
        admin,
        op,
    };
    let running_why = (!running.is_empty()).then(|| {
        format!(
            "{} is open ({}). Close it first; it stays unticked until then.",
            app.name,
            running.join(", ")
        )
    });
    match app.source.as_str() {
        "appx" => {
            let a = &app.extra["appx"];
            let name = a["name"].as_str().unwrap_or("");
            let class = a["classification"].as_str().unwrap_or("optional");
            let store = a["store_id"].as_str();
            let risk = match (class, store) {
                ("dangerous", _) => Risk::Extreme,
                (_, Some(_)) => Risk::Reversible,
                _ => Risk::Destructive,
            };
            let op = Op::Module {
                module: "apps".into(),
                action: "win.appx_remove".into(),
                args: json!({ "app_id": app.id, "name": name, "family": a["family"], "store_id": store }),
            };
            let why = if class == "dangerous" {
                Some("Removing it breaks parts of Windows. Only pick it if you know you do not need it.".to_string())
            } else {
                running_why
            };
            Ok((item(risk, false, op), why))
        }
        "orphan_dir" => {
            let path = PathBuf::from(app.path.as_deref().ok_or("no folder")?);
            let root = path.parent().ok_or("no parent folder")?.to_path_buf();
            protect::check(&path, Guard::Picked, Some(&root))?;
            let fp = fsops::fingerprint(&path)?;
            let op = Op::RemovePath {
                path,
                destination: Destination::Trash,
                fp,
                root: Some(root),
                guard: Guard::Picked,
                contents_only: false,
            };
            Ok((
                PlanItem { detail: app.path.clone(), ..item(Risk::Destructive, true, op) },
                Some("No installed program claims this folder, but nothing proves it is unused. Check it first.".into()),
            ))
        }
        _ => {
            let kind = match app.source.as_str() {
                "steam" => "steam",
                "scoop" => "scoop",
                "winget" => "winget",
                _ => x(app, "installer_kind").unwrap_or("other"),
            };
            let args = json!({
                "app_id": app.id,
                "name": app.name,
                "kind": kind,
                "path": app.path,
                "winget_id": x(app, "winget_id"),
                "product_code": x(app, "product_code"),
                "reg_root": x(app, "reg_root"),
                "reg_key": x(app, "reg_key"),
                "steam_appid": x(app, "steam_appid"),
                "scoop_name": x(app, "scoop_name"),
                "scoop_global": app.extra.get("scoop_global").cloned().unwrap_or(Value::Bool(false)),
            });
            let op = Op::Module {
                module: "apps".into(),
                action: "win.uninstall".into(),
                args,
            };
            let why = if app.needs_admin {
                Some(format!(
                    "Installed for every account: it needs an administrator, and OmniGet cannot run uninstallers as administrator yet. Run it yourself: {}",
                    detail.clone().unwrap_or_else(|| "Settings > Apps".into())
                ))
            } else {
                running_why
            };
            Ok((item(Risk::Destructive, app.needs_admin, op), why))
        }
    }
}

fn leftover_item(f: &Found) -> Option<PlanItem> {
    let leaf = parse::win_file_name(&f.path).to_string();
    let id = format!("uninstall:win:{}:{}", f.kind, f.path);
    let detail = Some(f.path.clone());
    match f.kind {
        "folder" | "file" => {
            let path = PathBuf::from(&f.path);
            let root = f
                .base
                .clone()
                .or_else(|| path.parent().map(Path::to_path_buf))?;
            protect::check(&path, Guard::Picked, Some(&root)).ok()?;
            let fp = fsops::fingerprint(&path).ok()?;
            Some(PlanItem {
                id,
                label: if f.kind == "file" {
                    format!("Shortcut {leaf}")
                } else {
                    format!("Folder {leaf}")
                },
                detail,
                group: "files".into(),
                bytes: f.bytes,
                // The helper cannot use the Trash: as administrator it is gone for good.
                risk: if f.admin {
                    Risk::Destructive
                } else {
                    Risk::Reversible
                },
                admin: f.admin,
                op: Op::RemovePath {
                    path,
                    destination: Destination::Trash,
                    fp,
                    root: Some(root),
                    guard: Guard::Picked,
                    contents_only: false,
                },
            })
        }
        "registry" => {
            run::reg_key_ok(&f.path).ok()?;
            Some(PlanItem {
                id,
                label: format!("Registry key {leaf}"),
                detail,
                group: "registry".into(),
                bytes: 0,
                risk: Risk::Reversible,
                admin: f.admin,
                op: Op::Module {
                    module: "apps".into(),
                    action: "win.reg_delete".into(),
                    args: json!({ "key": f.path }),
                },
            })
        }
        "service" => Some(PlanItem {
            id,
            label: format!(
                "Turn off the service {}",
                f.service.as_deref().unwrap_or(&leaf)
            ),
            detail,
            group: "startup".into(),
            bytes: 0,
            risk: Risk::Reversible,
            admin: true,
            op: Op::Module {
                module: "apps".into(),
                action: "win.service_disable".into(),
                args: json!({ "name": f.service }),
            },
        }),
        "task" => Some(PlanItem {
            id,
            label: format!("Turn off the scheduled task {leaf}"),
            detail,
            group: "startup".into(),
            bytes: 0,
            risk: Risk::Reversible,
            admin: true,
            op: Op::Module {
                module: "apps".into(),
                action: "win.task_disable".into(),
                args: json!({ "path": f.path }),
            },
        }),
        _ => None,
    }
}

/// Put the pieces together (pure: tested with fake leftovers).
pub fn assemble(app: &App, running: &[String], found: Vec<Found>) -> Built {
    let mut b = Built {
        command: command_preview(app),
        ..Default::default()
    };
    let is_installed = installed(app);
    let mut main_ticked = false;
    if is_installed {
        match main_item(app, running) {
            Ok((item, None)) => {
                main_ticked = true;
                b.ticked.push(item);
            }
            Ok((item, Some(why))) => {
                b.notes.push(why.clone());
                b.unticked.push(Unticked { item, why });
            }
            Err(reason) => b
                .notes
                .push(format!("{} cannot be uninstalled here: {reason}", app.name)),
        }
    } else {
        b.notes.push(format!(
            "{} is not installed anymore; these are the pieces it left.",
            app.name
        ));
    }
    if !running.is_empty() {
        b.notes
            .push(format!("{} is running: {}.", app.name, running.join(", ")));
    }
    // While the app stays installed, nothing of it goes on its own.
    let leftovers_may_tick = !is_installed || main_ticked;
    if is_installed && !main_ticked && !found.is_empty() {
        b.notes
            .push("Its leftovers stay unticked until the app itself is uninstalled.".into());
    }
    let mut found = found;
    found.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.cmp(&b.path)));
    for f in &found {
        let Some(item) = leftover_item(f) else {
            continue;
        };
        let lv = leftovers::level(f.score);
        let tick = leftovers_may_tick && leftovers::ticked(f.score);
        let lo = Leftover {
            path: f.path.clone(),
            bytes: f.bytes,
            confidence: leftovers::confidence_word(f.score).into(),
            evidence: f
                .reasons
                .iter()
                .map(|r| format!("{} ({:+})", r.text, r.delta))
                .collect::<Vec<_>>()
                .join("; "),
            kind: f.kind.into(),
        };
        let mut v = serde_json::to_value(&lo).unwrap_or(Value::Null);
        v["id"] = json!(item.id);
        v["level"] = json!(lv);
        v["score"] = json!(f.score);
        v["reasons"] = json!(f.reasons);
        v["admin"] = json!(f.admin);
        v["ticked"] = json!(tick);
        b.leftovers.push(v);
        if tick {
            b.ticked.push(item);
        } else {
            let why = if !leftovers_may_tick {
                "the app is still installed".to_string()
            } else {
                format!("{} confidence: check it first", lv.replace('_', " "))
            };
            b.unticked.push(Unticked { item, why });
        }
    }
    b
}

fn unticked_json(u: &[Unticked]) -> Value {
    Value::Array(
        u.iter()
            .map(|x| {
                json!({
                    "id": x.item.id, "label": x.item.label, "detail": x.item.detail, "group": x.item.group,
                    "bytes": x.item.bytes, "risk": x.item.risk, "admin": x.item.admin, "why": x.why,
                })
            })
            .collect(),
    )
}

fn target_of(app: &App) -> leftovers::Target {
    let mut extra_dirs = Vec::new();
    if let Some(us) = x(app, "uninstall_string") {
        if let Some(c) = parse::split_command(us, &|p| Path::new(p).is_file()) {
            let dir = parse::win_parent(&c.program).to_string();
            if dir.len() > 3 && !parse::norm_path(&dir).ends_with(r"\system32") {
                extra_dirs.push(dir);
            }
        }
    }
    leftovers::Target {
        name: app.name.clone(),
        publisher: app.publisher.clone(),
        location: app.path.clone(),
        extra_dirs,
    }
}

pub fn uninstall_items(ctx: &Ctx, app: &App) -> Result<(Vec<PlanItem>, Value), String> {
    let app = if installed(app) {
        app.clone()
    } else {
        recall(ctx, app)
    };
    // Store, Steam and Scoop clean up after themselves; their data folders
    // (saves, settings) are not guessed at by name.
    let search = !matches!(app.source.as_str(), "appx" | "steam" | "scoop");
    #[cfg(windows)]
    let (found, running) = {
        let found = if search {
            let others = sys::others_for(&app.id, &app.name);
            let key = if installed(&app) {
                None
            } else {
                inventory::reg_location(&app)
            };
            sys::leftovers(&target_of(&app), &others, key)
        } else {
            Vec::new()
        };
        let running = if installed(&app) {
            app.path.as_deref().map(sys::running_in).unwrap_or_default()
        } else {
            Vec::new()
        };
        (found, running)
    };
    #[cfg(not(windows))]
    let (found, running): (Vec<Found>, Vec<String>) = {
        let _ = (search, target_of(&app));
        (Vec::new(), Vec::new())
    };
    let b = assemble(&app, &running, found);
    let unticked: Vec<PlanItem> = b.unticked.iter().map(|u| u.item.clone()).collect();
    super::super::plan::register("uninstall", &unticked);
    let extra = json!({
        "leftovers": b.leftovers,
        "notes": b.notes,
        "running": !running.is_empty(),
        "unticked": unticked_json(&b.unticked),
        "command": b.command,
    });
    Ok((b.ticked, extra))
}

// ── Plan hooks ──────────────────────────────────────────────────────

/// Actions this file runs (`win.<action>`), a closed list.
pub const ACTIONS: &[&str] = &[
    "uninstall",
    "appx_remove",
    "reg_delete",
    "service_disable",
    "task_disable",
];

fn backup_file(ctx: &Ctx, receipt_id: &str, key: &str) -> PathBuf {
    let safe: String = receipt_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let hash = xxhash_rust::xxh3::xxh3_64(key.as_bytes());
    ctx.dir
        .join("regbackup")
        .join(if safe.is_empty() {
            "preview".into()
        } else {
            safe
        })
        .join(format!("{hash:016x}.reg"))
}

fn backup_ok(ctx: &Ctx, file: &Path) -> Result<(), String> {
    let root = ctx.dir.join("regbackup");
    let ok = file.extension().is_some_and(|e| e == "reg")
        && file.starts_with(&root)
        && !file
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    if ok {
        Ok(())
    } else {
        Err("PROTECTED: not one of our registry backups".into())
    }
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    if !ACTIONS.contains(&action) {
        return Err(format!("apps.windows: unknown action {action}"));
    }
    let _ = (ctx, receipt_id, args);
    #[cfg(windows)]
    {
        return match action {
            "uninstall" => {
                forget_cache();
                let (detail, bytes) = sys::uninstall(args)?;
                Ok(Outcome {
                    detail,
                    bytes,
                    undo: None,
                })
            }
            "appx_remove" => {
                forget_cache();
                let name = run::arg(args, "name").ok_or("package name missing")?;
                let detail = sys::appx_remove(name)?;
                let undo = run::arg(args, "store_id")
                    .filter(|s| parse::store_id_ok(s))
                    .map(|s| Undo::Module {
                        module: "apps".into(),
                        action: "win.appx_reinstall".into(),
                        args: json!({ "store_id": s, "name": name }),
                    });
                Ok(Outcome {
                    detail,
                    bytes: 0,
                    undo,
                })
            }
            "reg_delete" => {
                let key = run::arg(args, "key").ok_or("key missing")?;
                run::reg_key_ok(key)?;
                if !key.to_ascii_uppercase().starts_with(r"HKCU\") {
                    return Err("this key needs an administrator".into());
                }
                if !registry::exists(key) {
                    return Err(format!("{key} is already gone"));
                }
                let file = backup_file(ctx, receipt_id, key);
                sys::reg_export(key, &file)?;
                sys::reg_delete(key)?;
                Ok(Outcome {
                    detail: format!("removed {key} (backup in {})", file.display()),
                    bytes: 0,
                    undo: Some(Undo::Module {
                        module: "apps".into(),
                        action: "win.reg_import".into(),
                        args: json!({ "file": file, "key": key }),
                    }),
                })
            }
            _ => Err("this needs an administrator".into()),
        };
    }
    #[allow(unreachable_code)]
    Err("apps.windows: only on Windows".into())
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match action {
        "reg_delete" => {
            let key = run::arg(args, "key").ok_or("key missing")?;
            run::reg_key_ok(key)?;
            let file = backup_file(ctx, receipt_id, key);
            #[cfg(windows)]
            {
                if receipt_id != "preview" {
                    if !registry::exists(key) {
                        return Err(format!("{key} is already gone"));
                    }
                    // Reading HKLM needs no administrator: the backup is ours.
                    sys::reg_export(key, &file)?;
                }
            }
            Ok((
                vec![ElevatedOp::RegDelete {
                    key: key.to_string(),
                    name: None,
                }],
                Some(Undo::Module {
                    module: "apps".into(),
                    action: "win.reg_import".into(),
                    args: json!({ "file": file, "key": key }),
                }),
            ))
        }
        "service_disable" => {
            let name = run::arg(args, "name").ok_or("service name missing")?;
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' '))
            {
                return Err(format!("not a service name: {name}"));
            }
            Ok((
                vec![ElevatedOp::ServiceStartup {
                    name: name.to_string(),
                    mode: "disabled".into(),
                }],
                None,
            ))
        }
        "task_disable" => {
            let path = run::arg(args, "path").ok_or("task path missing")?;
            if !path.starts_with('\\')
                || path.to_ascii_lowercase().starts_with(r"\microsoft\")
                || path.contains(['"', '\0'])
            {
                return Err(format!("PROTECTED: {path}"));
            }
            Ok((
                vec![ElevatedOp::TaskState {
                    path: path.to_string(),
                    enabled: false,
                }],
                None,
            ))
        }
        "uninstall" => {
            // The helper reads the Uninstall entry again and refuses any
            // command that differs from the one rebuilt from it.
            #[cfg(windows)]
            {
                let app_id = run::arg(args, "app_id").ok_or("app id missing")?;
                if let Ok(entry) = sys::entry_of(args, app_id) {
                    let p = run::for_entry(&entry, &inventory::DiskProbe, &run::system_root())?;
                    let mut argv = p.args.clone();
                    if let Some(r) = &p.raw {
                        argv.extend(parse::argv_split(r));
                    }
                    return Ok((
                        vec![ElevatedOp::WinUninstall {
                            app_id: app_id.to_string(),
                            kind: p.kind.as_str().to_string(),
                            program: p.program.clone(),
                            args: argv,
                        }],
                        None,
                    ));
                }
            }
            Err(format!(
                "NEEDS_ELEVATED_UNINSTALL: OmniGet cannot run this uninstaller as administrator. Run it yourself: {}",
                elevated_command(args).unwrap_or_else(|| "Settings > Apps > Installed apps".into())
            ))
        }
        _ => Err(format!(
            "apps.windows: action {action} has no administrator form"
        )),
    }
}

/// The command an administrator would run for an `uninstall` item.
fn elevated_command(args: &Value) -> Option<String> {
    #[cfg(windows)]
    {
        let app_id = run::arg(args, "app_id")?;
        if let Some(id) = run::arg(args, "winget_id") {
            return run::winget_uninstall(id).ok().map(|p| p.display());
        }
        let entry = sys::entry_of(args, app_id).ok()?;
        return run::for_entry(&entry, &inventory::DiskProbe, &run::system_root())
            .ok()
            .map(|p| p.display());
    }
    #[allow(unreachable_code)]
    {
        let _ = args;
        None
    }
}

/// Executor of `ElevatedOp::WinUninstall { app_id, kind, program, args }`:
/// runs as administrator, reads the
/// Uninstall entry named by `app_id` again, rebuilds the command from it
/// and refuses anything that differs from `program`/`args`.
pub fn elevated_win_uninstall(
    app_id: &str,
    kind: &str,
    program: &str,
    args: &[String],
) -> Result<String, String> {
    let _ = (app_id, kind, program, args);
    #[cfg(windows)]
    {
        let (root, key) = if let Some(g) = app_id.strip_prefix("msi:") {
            // Machine MSI entries live in the 64-bit or the 32-bit view.
            let r = [parse::Root::Hklm, parse::Root::Hklm32]
                .into_iter()
                .find(|r| registry::read_entry(*r, g).is_some())
                .ok_or("NOT_INSTALLED")?;
            (r, g.to_string())
        } else {
            let rest = app_id.strip_prefix("reg:").ok_or("not a registry app")?;
            let (r, k) = rest.split_once(':').ok_or("bad app id")?;
            (parse::Root::parse(r).ok_or("bad root")?, k.to_string())
        };
        let entry =
            registry::read_entry(root, &key).ok_or_else(|| format!("NOT_INSTALLED: {app_id}"))?;
        let planned = run::for_entry(&entry, &inventory::DiskProbe, &run::system_root())?;
        let mut expected = planned.args.clone();
        if let Some(r) = &planned.raw {
            expected.extend(parse::argv_split(r));
        }
        if !planned.program.eq_ignore_ascii_case(program)
            || expected != args
            || planned.kind.as_str() != kind
        {
            return Err("REFUSED: the command does not match the registry entry".into());
        }
        let code = sys::run_planned(&planned, entry.get("DisplayName").unwrap_or(app_id))?;
        let m = parse::installer_exit(planned.kind, code);
        return if m.ok { Ok(m.text) } else { Err(m.text) };
    }
    #[allow(unreachable_code)]
    Err("only on Windows".into())
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    match action {
        "reg_import" => {
            let file = PathBuf::from(run::arg(args, "file").ok_or("backup file missing")?);
            backup_ok(ctx, &file)?;
            if !file.is_file() {
                return Err(format!("the backup {} is gone", file.display()));
            }
            #[cfg(windows)]
            {
                sys::reg_import(&file)?;
                return Ok(format!(
                    "restored {}",
                    run::arg(args, "key").unwrap_or("the key")
                ));
            }
            #[allow(unreachable_code)]
            Err("only on Windows".into())
        }
        "appx_reinstall" => {
            let id = run::arg(args, "store_id").ok_or("Store id missing")?;
            if !parse::store_id_ok(id) {
                return Err(format!("not a Store id: {id}"));
            }
            #[cfg(windows)]
            {
                forget_cache();
                return sys::store_reinstall(id);
            }
            #[allow(unreachable_code)]
            Err("only on Windows".into())
        }
        _ => Err(format!("apps.windows: cannot undo {action}")),
    }
}

/// Screen-only actions: `apps.win.details {app_id}`.
pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action {
        "details" => {
            let id = crate::core::llm::tool_table::need_str(&a, "app_id")?;
            super::super::tools::blocking(move || {
                let app = list()?
                    .into_iter()
                    .find(|x| x.id == id)
                    .ok_or_else(|| format!("{id} is not installed"))?;
                #[cfg(windows)]
                let facts = sys::details(&app);
                #[cfg(not(windows))]
                let facts = Value::Null;
                Ok(json!({ "app": app, "facts": facts, "command": command_preview(&app) }))
            })
            .await
        }
        _ => Err(format!("apps.windows: unknown screen action {action}")),
    }
}
