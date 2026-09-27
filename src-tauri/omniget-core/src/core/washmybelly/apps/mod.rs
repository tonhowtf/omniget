//! Installed apps and packages, leftovers with confidence, uninstall (F3).
//!
//! One shape for every system ([`App`], [`Leftover`]); each system fills it
//! in its own file (`macos.rs`, `windows.rs`, `linux.rs`). Those files hold
//! the real work behind `#[cfg(target_os)]` and their text parsers as pure
//! functions compiled everywhere in tests. This file only dispatches.

pub mod linux;
pub mod macos;
pub mod windows;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::elevate::ElevatedOp;
use super::plan::{self, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::store::Ctx;

/// One installed app or package.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct App {
    /// Stable id: `<source>:<native id>` (`app:com.apple.Numbers`,
    /// `msi:{GUID}`, `winget:Mozilla.Firefox`, `apt:vlc`, `flatpak:org.gimp.GIMP`).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    /// `app_bundle`, `homebrew_cask`, `homebrew_formula`, `pkg`, `msi`,
    /// `registry`, `winget`, `appx`, `steam`, `scoop`, `apt`, `dnf`,
    /// `pacman`, `zypper`, `snap`, `flatpak`, `appimage`, `desktop_file`.
    pub source: String,
    #[serde(default)]
    pub path: Option<String>,
    /// Allocated bytes when known.
    #[serde(default)]
    pub bytes: Option<u64>,
    /// Bundle id (macOS), product code (MSI), package name (Linux).
    #[serde(default)]
    pub native_id: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub installed: Option<i64>,
    #[serde(default)]
    pub last_used: Option<i64>,
    /// Can we uninstall it at all (system apps, protected Appx: false).
    pub removable: bool,
    /// Why not, when `removable` is false (i18n key or English text).
    #[serde(default)]
    pub reason: Option<String>,
    /// Needs administrator rights to uninstall.
    #[serde(default)]
    pub needs_admin: bool,
    /// Per-system details (winget id, Appx classification, cask token...).
    #[serde(default)]
    pub extra: Value,
}

/// Something an app left behind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leftover {
    pub path: String,
    pub bytes: u64,
    /// `high` (bundle id / Team ID / receipt / registry link), `medium`,
    /// `low` (name match only: never ticked by default).
    pub confidence: String,
    /// What tied it to the app, in plain words.
    pub evidence: String,
    /// `file`, `folder`, `registry`, `service`, `launchd`, `task`.
    pub kind: String,
}

fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

/// Every app of this system.
pub fn list() -> Result<Vec<App>, String> {
    #[cfg(target_os = "macos")]
    return macos::list();
    #[cfg(windows)]
    return windows::list();
    #[cfg(target_os = "linux")]
    return linux::list();
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

/// Plan items to uninstall `app` with its leftovers, plus extra facts for
/// the screen (`{ leftovers: [...], notes: [...] }`).
pub fn uninstall_items(ctx: &Ctx, app: &App) -> Result<(Vec<PlanItem>, Value), String> {
    let _ = ctx;
    #[cfg(target_os = "macos")]
    return macos::uninstall_items(ctx, app);
    #[cfg(windows)]
    return windows::uninstall_items(ctx, app);
    #[cfg(target_os = "linux")]
    return linux::uninstall_items(ctx, app);
    #[allow(unreachable_code)]
    Err(format!("{}: uninstall is not available here", app.id))
}

pub async fn tool_list(a: Value) -> Result<Value, String> {
    let query = crate::core::llm::tool_table::s(&a, "query").to_lowercase();
    let source = crate::core::llm::tool_table::s(&a, "source");
    super::tools::blocking(move || {
        let mut apps = list()?;
        apps.retain(|app| {
            (query.is_empty()
                || app.name.to_lowercase().contains(&query)
                || app.id.to_lowercase().contains(&query))
                && (source.is_empty() || app.source == source)
        });
        apps.sort_by_key(|a| a.name.to_lowercase());
        let total = apps.len();
        Ok(json!({ "apps": apps, "total": total }))
    })
    .await
}

pub async fn tool_leftovers(a: Value) -> Result<Value, String> {
    let app_id = crate::core::llm::tool_table::need_str(&a, "app_id")?;
    let source = source_of(&a);
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        let apps = list()?;
        // An app already removed still has leftovers: build a bare App from
        // the id so the system module can search by it.
        let app = apps.into_iter().find(|x| x.id == app_id).unwrap_or_else(|| App {
            id: app_id.clone(),
            name: app_id.split(':').nth(1).unwrap_or(&app_id).to_string(),
            source: app_id.split(':').next().unwrap_or("").to_string(),
            removable: false,
            reason: Some("not installed anymore".into()),
            ..Default::default()
        });
        let (items, extra) = uninstall_items(&ctx, &app)?;
        let total: u64 = items.iter().map(|i| i.bytes).sum();
        let mut preview: Vec<Value> = items
            .iter()
            .map(|i| json!({ "id": i.id, "label": i.label, "detail": i.detail, "group": i.group, "bytes": i.bytes, "risk": i.risk, "admin": i.admin }))
            .collect();
        // Candidates left out of the default plan (the app is open, needs a
        // password, name-only match) are listed too, marked unticked, so the
        // screen never says "nothing left behind" while there is something.
        if let Some(extra_items) = extra.get("unticked").and_then(Value::as_array) {
            for u in extra_items {
                let mut u = u.clone();
                u["unticked"] = json!(true);
                preview.push(u);
            }
        }
        let plan_id = if items.is_empty() {
            Value::Null
        } else {
            json!(plan::offer(&ctx, "uninstall", items, source)?.id)
        };
        Ok(json!({ "app": app, "items": preview, "total_bytes": total, "plan_id": plan_id, "extra": extra }))
    })
    .await
}

// ── Plan hooks: actions are `<os>.<action>` and go to that system's file ──

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    match action.split_once('.') {
        Some(("mac", a)) => macos::plan_execute(ctx, receipt_id, a, args),
        Some(("win", a)) => windows::plan_execute(ctx, receipt_id, a, args),
        Some(("linux", a)) => linux::plan_execute(ctx, receipt_id, a, args),
        _ => Err(format!("apps: unknown action {action}")),
    }
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match action.split_once('.') {
        Some(("mac", a)) => macos::plan_elevated(ctx, receipt_id, a, args),
        Some(("win", a)) => windows::plan_elevated(ctx, receipt_id, a, args),
        Some(("linux", a)) => linux::plan_elevated(ctx, receipt_id, a, args),
        _ => Err(format!("apps: action {action} has no administrator form")),
    }
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    match action.split_once('.') {
        Some(("mac", a)) => macos::plan_undo(ctx, a, args),
        Some(("win", a)) => windows::plan_undo(ctx, a, args),
        Some(("linux", a)) => linux::plan_undo(ctx, a, args),
        _ => Err(format!("apps: cannot undo {action}")),
    }
}

/// `pkgutil --forget` after a `.pkg`'s files are gone (runs as root).
pub fn elevated_pkg_forget(id: &str) -> Result<String, String> {
    macos::elevated_pkg_forget(id)
}

/// Screen-only actions: `apps.<os>.<action>` or `apps.icon` etc.
pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action.split_once('.') {
        Some(("mac", x)) => macos::ui_action(x, a).await,
        Some(("win", x)) => windows::ui_action(x, a).await,
        Some(("linux", x)) => linux::ui_action(x, a).await,
        _ => Err(format!("apps: unknown screen action {action}")),
    }
}
