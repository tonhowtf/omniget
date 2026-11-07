#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

pub mod actions;
pub mod items;
pub mod leftovers;
pub mod parse;
pub mod plist;

use std::path::Path;

use serde_json::Value;

use super::super::elevate::ElevatedOp;
use super::super::plan::{Outcome, PlanItem};
use super::super::receipt::Undo;
use super::super::store::Ctx;
use super::App;

#[cfg(target_os = "macos")]
mod inventory;

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
