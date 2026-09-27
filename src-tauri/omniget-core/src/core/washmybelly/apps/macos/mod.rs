//! Apps on macOS (F3): inventory, leftovers by evidence, uninstall plan.
//!
//! - [`list`]: app bundles in `/Applications`, `~/Applications` (and one
//!   folder level below, e.g. `Setapp/`), `/System/Applications` (listed,
//!   not removable), Homebrew casks and formulae, third-party `.pkg`
//!   receipts. Read-only: `Info.plist` (XML or binary, parsed here),
//!   sizes with `fsops::measure`, `mdls` for dates, `codesign` for the Team
//!   ID, `brew` and `pkgutil` for the rest.
//! - [`uninstall_items`]: the plan (see `items.rs`), leftovers found by
//!   evidence (`leftovers.rs`), and the alternatives (Homebrew zap, reset).
//! - Plan hooks and the helper side live in `actions.rs`.
//!
//! Ideas from the study of Pearcleaner (Apache + Commons Clause: ideas
//! only, no code or lists), Mole (sibling guard) and MacSai; all code and
//! lists here are our own.
//!
//! Real system calls are behind `#[cfg(target_os = "macos")]`; the parsers,
//! the leftover search and the item builder are pure and tested everywhere.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

pub mod actions;
pub mod items;
pub mod leftovers;
pub mod parse;
pub mod plist;

use std::path::Path;

use serde_json::{json, Value};

use super::super::elevate::ElevatedOp;
use super::super::plan::{Outcome, PlanItem};
use super::super::receipt::Undo;
use super::super::store::Ctx;
use super::App;

#[cfg(target_os = "macos")]
mod inventory;

/// Every app of this Mac (cached for 30 s: a leftover search right after a
/// listing reuses it).
pub fn list() -> Result<Vec<App>, String> {
    #[cfg(target_os = "macos")]
    return inventory::list_cached(false);
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

pub fn uninstall_items(ctx: &Ctx, app: &App) -> Result<(Vec<PlanItem>, Value), String> {
    let _ = ctx;
    #[cfg(target_os = "macos")]
    {
        let env = inventory::env_for(app)?;
        let built = items::build(app, &env);
        // Unticked candidates stay pickable through `wmb_plan` but are not
        // in the default plan the caller offers.
        let unticked: Vec<PlanItem> = built.unticked.iter().map(|u| u.item.clone()).collect();
        super::super::plan::register("uninstall", &unticked);
        let mut extra = built.extra;
        extra["unticked"] = items::unticked_json(&built.unticked);
        return Ok((built.ticked, extra));
    }
    #[allow(unreachable_code)]
    Err(format!("{}: macOS only", app.id))
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    actions::plan_execute(ctx, receipt_id, action, args)
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    actions::plan_elevated(ctx, receipt_id, action, args)
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    actions::plan_undo(ctx, action, args)
}

pub fn elevated_pkg_forget(id: &str) -> Result<String, String> {
    actions::elevated_pkg_forget(id)
}

/// Whether a process runs from inside the app bundle (or, when its path is
/// unknown, has the bundle's executable name).
pub fn running(app_path: &Path, executable: Option<&str>) -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let own = std::process::id();
    sys.processes().values().any(|p| {
        if p.pid().as_u32() == own {
            return false;
        }
        match p.exe() {
            Some(exe) => exe.starts_with(app_path),
            None => executable.is_some_and(|e| e.len() >= 4 && p.name().to_string_lossy() == e),
        }
    })
}

/// Screen-only actions (`washmybelly_ui` with `apps.mac.<action>`):
/// - `icon {app_id | path, size?}` → `{ data_url }` (PNG from the `.icns`,
///   `null` when the bundle has none);
/// - `quit {app_id}` → asks the app to quit (an Apple Event, like ⌘Q; never
///   a kill, so it may ask to save);
/// - `reveal {path}` → shows it in Finder.
pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    let s = |k: &str| a.get(k).and_then(Value::as_str).map(String::from);
    match action {
        "icon" => {
            let (id, path) = (s("app_id"), s("path"));
            let size = a
                .get("size")
                .and_then(Value::as_u64)
                .unwrap_or(64)
                .clamp(16, 256) as u32;
            super::super::tools::blocking(move || {
                let path = match (path, id) {
                    (Some(p), _) => p,
                    (None, Some(id)) => {
                        app_by_id(&id)?.path.ok_or("icon: the app has no bundle")?
                    }
                    _ => return Err("icon: app_id or path".into()),
                };
                let png = icon_png(Path::new(&path), size)?;
                Ok(json!({ "data_url": png.map(|b| format!("data:image/png;base64,{b}")) }))
            })
            .await
        }
        "quit" => {
            let id = s("app_id").ok_or("quit: app_id")?;
            super::super::tools::blocking(move || {
                let app = app_by_id(&id)?;
                let bid = app
                    .native_id
                    .clone()
                    .filter(|b| parse::valid_bundle_id(b))
                    .ok_or("quit: no bundle id")?;
                quit(&bid).map(|_| json!({ "ok": true }))
            })
            .await
        }
        "reveal" => {
            let path = crate::core::llm::tool_table::need_str(&a, "path")?;
            super::super::tools::blocking(move || {
                reveal(Path::new(&path)).map(|_| json!({ "ok": true }))
            })
            .await
        }
        _ => Err(format!("apps.mac: unknown screen action {action}")),
    }
}

fn app_by_id(id: &str) -> Result<App, String> {
    list()?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("{id}: not installed"))
}

/// Apps a quit request never goes to: the desktop itself and OmniGet.
const NEVER_QUIT: &[&str] = &[
    "com.apple.finder",
    "com.apple.dock",
    "com.apple.loginwindow",
    "com.apple.systemuiserver",
    "com.apple.WindowServer",
    "wtf.tonho.omniget",
];

fn quit(bundle_id: &str) -> Result<(), String> {
    if NEVER_QUIT.iter().any(|n| n.eq_ignore_ascii_case(bundle_id)) {
        return Err(format!("quit: {bundle_id} stays open"));
    }
    #[cfg(target_os = "macos")]
    {
        // The id travels as an argument of the script, never inside its text.
        let out = crate::core::process::std_command("/usr/bin/osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "tell application id (item 1 of argv) to quit",
                "-e",
                "end run",
                bundle_id,
            ])
            .output()
            .map_err(|e| format!("osascript: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "quit: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// The `.icns` a bundle names in its `Info.plist`.
pub fn icon_file(app: &Path) -> Option<std::path::PathBuf> {
    let info = plist::read(&app.join("Contents/Info.plist")).ok()?;
    let name = parse::bundle_info(&info)
        .icon_file
        .unwrap_or_else(|| "AppIcon".into());
    let name = if name.ends_with(".icns") {
        name
    } else {
        format!("{name}.icns")
    };
    let p = app.join("Contents/Resources").join(name);
    p.is_file().then_some(p)
}

fn icon_png(app: &Path, size: u32) -> Result<Option<String>, String> {
    if !app.is_absolute() || app.extension().is_none_or(|e| e != "app") || !app.is_dir() {
        return Err("icon: not an app bundle".into());
    }
    let Some(icns) = icon_file(app) else {
        return Ok(None);
    };
    #[cfg(target_os = "macos")]
    {
        use base64::Engine;
        let ctx = Ctx::app()?;
        let dir = ctx.dir.join("icons");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mtime = std::fs::metadata(&icns)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let key =
            xxhash_rust::xxh3::xxh3_64(format!("{}|{mtime}|{size}", icns.display()).as_bytes());
        let png = dir.join(format!("{key:016x}.png"));
        if !png.is_file() {
            let out = crate::core::process::std_command("/usr/bin/sips")
                .args(["-s", "format", "png", "-Z", &size.to_string()])
                .arg(&icns)
                .arg("--out")
                .arg(&png)
                .output()
                .map_err(|e| format!("sips: {e}"))?;
            if !out.status.success() {
                return Ok(None);
            }
        }
        let bytes = std::fs::read(&png).map_err(|e| e.to_string())?;
        return Ok(Some(
            base64::engine::general_purpose::STANDARD.encode(bytes),
        ));
    }
    #[allow(unreachable_code)]
    {
        let _ = (icns, size);
        Ok(None)
    }
}

fn reveal(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || std::fs::symlink_metadata(path).is_err() {
        return Err("reveal: no such path".into());
    }
    #[cfg(target_os = "macos")]
    {
        crate::core::process::std_command("/usr/bin/open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
