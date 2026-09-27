//! Junk cleanup: one rule engine fed by BleachBit CleanerML, Kudu, Mole,
//! PureMac and our own rules.
//!
//! - [`catalog`]: every rule of every source, in engine order;
//! - [`engine`]: scans the rules of this system (nothing changes);
//! - [`tool_junk_scan`]: the `wmb_junk_scan` tool, which hands the items to
//!   `plan::offer`;
//! - [`plan_execute`] / [`plan_elevated`]: the module actions the plan
//!   engine calls back (`delete_files`, `truncate`, `sqlite_vacuum`, and the
//!   administrator ones).

pub mod artifacts;
pub mod catalog;
pub mod cleanerml;
pub mod engine;
pub mod files;
pub mod kudu;
pub mod locks;
pub mod protection;
pub mod rule;
pub mod rules_linux;
pub mod rules_macos;
pub mod rules_ours;
pub mod rules_windows;
pub mod sqlite;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::elevate::ElevatedOp;
use super::fsops::{self, Fingerprint};
use super::plan::{self, Op, Outcome, Source};
use super::protect::{self, Guard};
use super::receipt::Undo;
use super::store::Ctx;
use super::{Destination, Os, Risk};

fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

fn risk_arg(a: &Value) -> Risk {
    a.get("max_risk")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(Risk::Extreme)
}

/// Scan, then offer: every item becomes a candidate for `wmb_plan`, and the
/// returned plan holds only the items ticked by default.
pub fn junk_scan(
    ctx: &Ctx,
    env: &engine::Env,
    categories: Vec<String>,
    max_risk: Risk,
    source: Source,
) -> Result<Value, String> {
    let rules = catalog::rules_for(env.os);
    let filter = engine::Filter {
        categories,
        max_risk,
    };
    let mut scan = engine::scan(&rules, env, &filter);
    if env.os == Os::Windows {
        mark_locked(&mut scan.items);
    }
    // What the plan engine would refuse never reaches it.
    scan.items.retain(|it| match &it.plan.op {
        Op::RemovePath {
            path, root, guard, ..
        } => protect::check(path, *guard, root.as_deref()).is_ok(),
        Op::Command { program, .. } => plan::NATIVE_PROGRAMS.contains(&program.as_str()),
        Op::Module { .. } => true,
    });
    scan.items.sort_by_key(|i| std::cmp::Reverse(i.plan.bytes));

    let mut categories: Vec<(String, u64, usize)> = Vec::new();
    for it in &scan.items {
        match categories.iter_mut().find(|c| c.0 == it.category) {
            Some(c) => {
                c.1 += it.plan.bytes;
                c.2 += 1;
            }
            None => categories.push((it.category.clone(), it.plan.bytes, 1)),
        }
    }
    categories.sort_by_key(|c| std::cmp::Reverse(c.1));

    let all: Vec<plan::PlanItem> = scan.items.iter().map(|i| i.plan.clone()).collect();
    let selected: Vec<plan::PlanItem> = scan
        .items
        .iter()
        .filter(|i| i.selected_by_default)
        .map(|i| i.plan.clone())
        .collect();
    let mut plan_id = Value::Null;
    if !all.is_empty() {
        // Every item is pickable through `wmb_plan`; the returned plan holds
        // only the ticked ones.
        plan::register("cleanup", &all);
        if !selected.is_empty() {
            plan_id = json!(plan::offer(ctx, "cleanup", selected, source)?.id);
        }
    }
    let total: u64 = scan.items.iter().map(|i| i.plan.bytes).sum();
    let selected_bytes: u64 = scan
        .items
        .iter()
        .filter(|i| i.selected_by_default)
        .map(|i| i.plan.bytes)
        .sum();
    Ok(json!({
        "items": scan.items.iter().map(engine::Item::to_json).collect::<Vec<_>>(),
        "total_bytes": total,
        "selected_bytes": selected_bytes,
        "plan_id": plan_id,
        "categories": categories.into_iter().map(|(id, bytes, count)| json!({ "id": id, "bytes": bytes, "count": count })).collect::<Vec<_>>(),
        "skipped_rules": scan.skipped_rules,
        "protected_paths": scan.protected,
        "needs_full_disk_access": scan.needs_full_disk_access,
    }))
}

/// Windows: files a process holds open (Restart Manager, `locks::lockers`)
/// leave their item unticked with "in use by <names>".
fn mark_locked(items: &mut [engine::Item]) {
    let mut owner: Vec<(usize, PathBuf)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        match &it.plan.op {
            Op::RemovePath { path, .. } => owner.push((i, path.clone())),
            Op::Module { action, args, .. } if action == "delete_files" => {
                for f in args["files"].as_array().into_iter().flatten() {
                    if let Some(p) = f["path"].as_str() {
                        owner.push((i, PathBuf::from(p)));
                    }
                }
            }
            _ => {}
        }
    }
    for chunk in owner.chunks(512) {
        let paths: Vec<PathBuf> = chunk.iter().map(|(_, p)| p.clone()).collect();
        for (path, who) in locks::lockers(&paths) {
            if who.is_empty() {
                continue;
            }
            for (i, p) in chunk {
                if *p == path && items[*i].running.is_none() {
                    items[*i].running = Some(format!("in use by {}", who.join(", ")));
                    items[*i].selected_by_default = false;
                }
            }
        }
    }
}

pub async fn tool_junk_scan(a: Value) -> Result<Value, String> {
    let categories = crate::core::llm::tool_table::list(&a, "categories");
    let max_risk = risk_arg(&a);
    let source = source_of(&a);
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        let full = categories.is_empty() && max_risk == Risk::Extreme;
        let out = junk_scan(&ctx, &engine::Env::system(), categories, max_risk, source)?;
        // Only a full scan speaks for the whole machine.
        if full {
            record_score(&ctx, out["selected_bytes"].as_u64().unwrap_or(0));
        }
        Ok(out)
    })
    .await
}

/// Leaves what a full scan would clean by default for the health score
/// (`score_inputs.junk_bytes`), keeping the other modules' fields.
fn record_score(ctx: &Ctx, bytes: u64) {
    let mut v: Value = ctx.kv_get("score_inputs").unwrap_or_else(|| json!({}));
    if !v.is_object() {
        v = json!({});
    }
    v["junk_bytes"] = json!(bytes);
    if !v["updated"].is_object() {
        v["updated"] = json!({});
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    v["updated"]["junk_bytes"] = json!(now);
    let _ = ctx.kv_set("score_inputs", &v);
}

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "clean" }`) ──

/// Actions this module runs itself (no administrator).
pub const ACTIONS: &[&str] = &["delete_files", "apfs_clone", "truncate", "sqlite_vacuum"];
/// Actions that only run through `omniget-elevate`.
pub const ADMIN_ACTIONS: &[&str] = &[
    "journal_vacuum",
    "tmutil_thin",
    "pkg_cache",
    "snap_revision",
    "maintenance",
    "remove_after_maintenance",
];

fn path_arg(args: &Value, key: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing {key}"))?,
    );
    if !p.is_absolute() {
        return Err(format!("{key} must be absolute"));
    }
    Ok(p)
}

fn fp_arg(v: &Value) -> Result<Fingerprint, String> {
    serde_json::from_value(v.get("fp").cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("bad fingerprint: {e}"))
}

/// A path the scan found, checked again: protection, root, fingerprint.
fn revalidate(path: &Path, root: &Path, fp: &Fingerprint) -> Result<(), String> {
    protect::check(path, Guard::Rule, Some(root))?;
    fsops::unchanged(path, fp)
}

/// The processes or lock files named by the rule, open right now.
fn still_running(args: &Value) -> Option<String> {
    let names: Vec<String> = args
        .get("running")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if names.is_empty() {
        return None;
    }
    let env = engine::Env::system();
    let mut probe = catalog::b("x", &[env.os], "x", "x").done();
    probe.running = names;
    env.running(&probe)
}

pub fn plan_execute(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    match action {
        "delete_files" => delete_files(args),
        // Duplicates on APFS: the copy becomes a clone of the kept file
        // (the files module does the checks and the swap).
        "apfs_clone" => {
            let keep = path_arg(args, "keep")?;
            let replace = path_arg(args, "replace")?;
            let fp = fp_arg(args)?;
            let freed = files::apfs_clone_replace(&keep, &replace, &fp)?;
            Ok(Outcome {
                detail: format!("{} is now a clone of {}", replace.display(), keep.display()),
                bytes: freed,
                undo: None,
            })
        }
        "truncate" | "sqlite_vacuum" => {
            let path = path_arg(args, "path")?;
            let root = path_arg(args, "root")?;
            revalidate(&path, &root, &fp_arg(args)?)?;
            if let Some(app) = still_running(args) {
                return Err(format!("RUNNING: {app} is open; close it and try again"));
            }
            let bytes = if action == "truncate" {
                sqlite::truncate(&path)?
            } else {
                sqlite::vacuum(&path)?
            };
            Ok(Outcome {
                detail: if action == "truncate" {
                    "emptied".into()
                } else {
                    "compacted".into()
                },
                bytes,
                undo: None,
            })
        }
        a if ADMIN_ACTIONS.contains(&a) => Err(format!("clean: {a} needs administrator rights")),
        _ => Err(format!("clean: unknown action {action}")),
    }
}

fn delete_files(args: &Value) -> Result<Outcome, String> {
    let root = path_arg(args, "root")?;
    let dest: Destination =
        serde_json::from_value(args.get("destination").cloned().unwrap_or(json!("unlink")))
            .map_err(|e| e.to_string())?;
    let files = args
        .get("files")
        .and_then(Value::as_array)
        .ok_or("missing files")?;
    let (mut removed, mut skipped, mut failed, mut bytes) = (0usize, 0usize, 0usize, 0u64);
    let mut first_problem: Option<String> = None;
    let mut trashed = Vec::new();
    for f in files {
        let path = path_arg(f, "path")?;
        if let Err(e) = revalidate(&path, &root, &fp_arg(f)?) {
            skipped += 1;
            first_problem.get_or_insert(e);
            continue;
        }
        let size = fsops::measure(&path).0;
        let r = match dest {
            Destination::Unlink => fsops::remove_tree(&path, false),
            // Many small files: the system Trash, restorable as a group.
            Destination::Trash | Destination::Quarantine => fsops::to_trash(&path).map(|()| {
                trashed.push(path.clone());
                size
            }),
        };
        match r {
            Ok(b) => {
                removed += 1;
                bytes += b;
            }
            Err(e) => {
                failed += 1;
                first_problem.get_or_insert(e);
            }
        }
    }
    if removed == 0 && (skipped + failed) > 0 {
        return Err(first_problem.unwrap_or_else(|| "nothing removed".into()));
    }
    let mut detail = format!("{removed} removed");
    if skipped + failed > 0 {
        detail.push_str(&format!(
            ", {} left ({})",
            skipped + failed,
            first_problem.unwrap_or_default()
        ));
    }
    let undo = (!trashed.is_empty()).then_some(Undo::TrashMany { originals: trashed });
    Ok(Outcome {
        detail,
        bytes,
        undo,
    })
}

pub fn plan_elevated(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match action {
        "journal_vacuum" => {
            let max_mb = args
                .get("max_mb")
                .and_then(Value::as_u64)
                .ok_or("missing max_mb")?;
            if !(16..=1_000_000).contains(&max_mb) {
                return Err("max_mb out of range".into());
            }
            Ok((vec![ElevatedOp::JournalVacuum { max_mb }], None))
        }
        "tmutil_thin" => {
            let bytes = args
                .get("bytes")
                .and_then(Value::as_u64)
                .ok_or("missing bytes")?;
            Ok((vec![ElevatedOp::TmutilThin { bytes }], None))
        }
        "maintenance" => {
            let id = args.get("id").and_then(Value::as_str).ok_or("missing id")?;
            windows_maintenance_id(id)?;
            Ok((vec![ElevatedOp::Maintenance { id: id.to_string() }], None))
        }
        "remove_after_maintenance" => {
            let id = args.get("id").and_then(Value::as_str).ok_or("missing id")?;
            windows_maintenance_id(id)?;
            let path = path_arg(args, "path")?;
            let root = path_arg(args, "root")?;
            let fp = fp_arg(args)?;
            revalidate(&path, &root, &fp)?;
            let contents_only = args
                .get("contents_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok((
                vec![
                    ElevatedOp::Maintenance { id: id.to_string() },
                    ElevatedOp::RemovePath {
                        path,
                        root: Some(root),
                        guard: Guard::Rule,
                        fp,
                        contents_only,
                    },
                ],
                None,
            ))
        }
        // The helper re-derives the argv from the id (closed table).
        "pkg_cache" | "snap_revision" => {
            let id = native_id(action, args)?;
            native_argv(&id)?;
            Ok((vec![ElevatedOp::CleanNative { id }], None))
        }
        _ => Err(format!("clean: action {action} has no administrator form")),
    }
}

fn windows_maintenance_id(id: &str) -> Result<(), String> {
    rules_windows::maintenance_argv(id, "C:\\Windows")
        .map(|_| ())
        .ok_or_else(|| format!("unknown maintenance {id}"))
}

pub fn plan_undo(_ctx: &Ctx, action: &str, _args: &Value) -> Result<String, String> {
    if action == "apfs_clone" {
        // Same bytes before and after: there is nothing to bring back.
        return Ok("nothing to undo: the file kept identical content".into());
    }
    Err(format!(
        "clean: {action} cannot be undone (caches come back on their own)"
    ))
}

// ── Administrator executors (run inside `omniget-elevate`) ──────────

/// A system program by absolute path. As root we never search PATH (the
/// app's PATH includes folders the user can write).
#[cfg(unix)]
fn system_program(name: &str) -> Result<PathBuf, String> {
    for dir in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(format!("{name} not found in the system folders"))
}

#[cfg(unix)]
fn run_root(program: &str, args: &[String]) -> Result<String, String> {
    let out = std::process::Command::new(system_program(program)?)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!(
            "{program} failed: {}",
            text.trim()
                .chars()
                .rev()
                .take(300)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        ))
    }
}

pub fn elevated_tmutil_thin(bytes: u64) -> Result<(String, u64), String> {
    #[cfg(target_os = "macos")]
    {
        if bytes == 0 {
            return Err("nothing to thin".into());
        }
        let out = run_root(
            "tmutil",
            &[
                "thinlocalsnapshots".into(),
                "/".into(),
                bytes.to_string(),
                "4".into(),
            ],
        )?;
        let n = rules_macos::parse_tm_dates(&out).len();
        Ok((format!("{n} local snapshot(s) thinned"), 0))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        Err("tmutil_thin: macOS only".into())
    }
}

pub fn elevated_journal_vacuum(max_mb: u64) -> Result<(String, u64), String> {
    #[cfg(target_os = "linux")]
    {
        if !(16..=1_000_000).contains(&max_mb) {
            return Err("max_mb out of range".into());
        }
        let usage = || {
            run_root("journalctl", &["--disk-usage".into()])
                .ok()
                .and_then(|t| rules_linux::parse_journal_usage(&t))
                .unwrap_or(0)
        };
        let before = usage();
        run_root("journalctl", &[format!("--vacuum-size={max_mb}M")])?;
        let freed = before.saturating_sub(usage());
        Ok((format!("journal kept at {max_mb} MB"), freed))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = max_mb;
        Err("journal_vacuum: Linux only".into())
    }
}

fn native_id(action: &str, args: &Value) -> Result<String, String> {
    match action {
        "pkg_cache" => Ok(format!(
            "pkg_cache:{}",
            args.get("manager")
                .and_then(Value::as_str)
                .ok_or("missing manager")?
        )),
        "snap_revision" => Ok(format!(
            "snap_revision:{}:{}",
            args.get("name")
                .and_then(Value::as_str)
                .ok_or("missing name")?,
            args.get("revision")
                .and_then(Value::as_str)
                .ok_or("missing revision")?
        )),
        _ => Err(format!("no native id for {action}")),
    }
}

/// The fixed argv and the folders measured before and after, for an
/// administrator cleanup id (`pkg_cache:apt`, `snap_revision:<name>:<rev>`).
/// The helper derives it again from the id; nothing else reaches root.
pub fn native_argv(id: &str) -> Result<(Vec<String>, Vec<&'static str>), String> {
    let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    if let Some(manager) = id.strip_prefix("pkg_cache:") {
        return Ok(match manager {
            "apt-get" | "apt" => (v(&["apt-get", "clean"]), vec!["/var/cache/apt/archives"]),
            "dnf" => (
                v(&["dnf", "clean", "packages"]),
                vec!["/var/cache/dnf", "/var/cache/libdnf5"],
            ),
            "paccache" => (v(&["paccache", "-rk2"]), vec!["/var/cache/pacman/pkg"]),
            "pacman" => (
                v(&["pacman", "-Sc", "--noconfirm"]),
                vec!["/var/cache/pacman/pkg"],
            ),
            "zypper" => (
                v(&["zypper", "--non-interactive", "clean"]),
                vec!["/var/cache/zypp/packages"],
            ),
            _ => return Err(format!("unknown package manager {manager}")),
        });
    }
    if let Some(rest) = id.strip_prefix("snap_revision:") {
        let (name, rev) = rest.split_once(':').ok_or("bad snap id")?;
        if !rules_linux::valid_snap(name, rev) {
            return Err("bad snap name or revision".into());
        }
        return Ok((
            vec![
                "snap".into(),
                "remove".into(),
                name.into(),
                format!("--revision={rev}"),
            ],
            Vec::new(),
        ));
    }
    Err(format!("unknown cleanup id {id}"))
}

/// The terminal equivalent of [`native_argv`] (display only).
pub fn native_command_line(id: &str) -> String {
    match native_argv(id) {
        Ok((argv, _)) => format!("sudo {}", argv.join(" ")),
        Err(e) => format!("# {e}"),
    }
}

/// Executor for a future `ElevatedOp::CleanNative { id }`: re-derives the
/// argv from the id, checks a snap revision is still disabled, measures.
pub fn elevated_native(id: &str) -> Result<(String, u64), String> {
    let (argv, measure) = native_argv(id)?;
    #[cfg(target_os = "linux")]
    {
        if let Some(rest) = id.strip_prefix("snap_revision:") {
            let (name, rev) = rest.split_once(':').ok_or("bad snap id")?;
            let body =
                rules_linux::snapd_get("/v2/snaps?select=all").ok_or("snapd did not answer")?;
            if !rules_linux::parse_snap_revisions(&body)
                .iter()
                .any(|s| s.name == name && s.revision == rev)
            {
                return Err("CHANGED: that revision is no longer a disabled one".into());
            }
        }
        let size = || {
            measure
                .iter()
                .map(|m| fsops::measure(Path::new(m)).0)
                .sum::<u64>()
        };
        let before = size();
        run_root(&argv[0], &argv[1..])?;
        Ok((
            format!("{} finished", argv[0]),
            before.saturating_sub(size()),
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (argv, measure);
        Err("native package cleanup: Linux only".into())
    }
}

/// App data that looks like cache but is not (the upstream protection list
/// and ours). Returns the matching pattern when `path` must never be
/// removed by a rule.
pub fn protection_match(path: &Path) -> Option<String> {
    protection::protection_match(path)
}

/// Actions only the screen may run (`washmybelly_ui` with `clean.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
        // The rules this system has, with their defaults, for the settings list.
        "catalog" => {
            let os = super::current_os();
            let rules: Vec<Value> = catalog::rules_for(os)
                .into_iter()
                .map(|r| {
                    json!({
                        "id": r.id, "label": r.label, "label_key": r.label_key, "category": r.category,
                        "description": r.description, "risk": r.risk, "destination": r.destination,
                        "admin": r.requires.contains(&rule::Requirement::Admin),
                        "selected_by_default": catalog::default_selected(r),
                        "source": r.source, "license": r.license,
                    })
                })
                .collect();
            Ok(json!({ "rules": rules }))
        }
        "stats" => Ok(catalog::stats()),
        _ => Err(format!("clean: unknown screen action {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ids_map_to_fixed_argv() {
        assert_eq!(
            native_argv("pkg_cache:apt-get").unwrap().0,
            vec!["apt-get", "clean"]
        );
        assert_eq!(
            native_argv("pkg_cache:paccache").unwrap().0,
            vec!["paccache", "-rk2"]
        );
        assert_eq!(
            native_argv("snap_revision:firefox:4793").unwrap().0,
            vec!["snap", "remove", "firefox", "--revision=4793"]
        );
        assert!(native_argv("pkg_cache:rm").is_err());
        assert!(native_argv("snap_revision:a;b:1").is_err());
        assert_eq!(
            native_command_line("pkg_cache:dnf"),
            "sudo dnf clean packages"
        );
    }

    #[test]
    fn elevated_actions_validate_their_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let (ops, _) =
            plan_elevated(&ctx, "r", "journal_vacuum", &json!({ "max_mb": 200 })).unwrap();
        assert_eq!(ops, vec![ElevatedOp::JournalVacuum { max_mb: 200 }]);
        assert!(plan_elevated(&ctx, "r", "journal_vacuum", &json!({ "max_mb": 1 })).is_err());
        assert!(plan_elevated(&ctx, "r", "maintenance", &json!({ "id": "anything" })).is_err());
        let (ops, _) =
            plan_elevated(&ctx, "r", "pkg_cache", &json!({ "manager": "apt-get" })).unwrap();
        let [ElevatedOp::CleanNative { id }] = ops.as_slice() else {
            panic!("{ops:?}")
        };
        assert!(native_command_line(id).contains("sudo apt-get clean"));
        assert!(plan_execute(&ctx, "r", "journal_vacuum", &json!({})).is_err());
        assert!(plan_execute(&ctx, "r", "rm_rf", &json!({})).is_err());
    }

    /// End to end on a fake home: scan, offer, execute, receipt.
    #[test]
    fn scan_then_execute_in_a_temporary_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let ctx = Ctx::at(&dir.path().join("state"));
        let w = |p: &str, n: usize| {
            let p = home.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![3u8; n]).unwrap();
        };
        w(".npm/_cacache/content/a", 20_000);
        w(".cache/someapp/blob", 20_000);
        w(".local/share/Trash/files/old.txt", 100);
        let env = engine::Env::fake(&home, Os::Linux);
        let out = junk_scan(&ctx, &env, Vec::new(), Risk::Extreme, Source::Agent).unwrap();
        let items = out["items"].as_array().unwrap();
        let rule_ids: Vec<&str> = items
            .iter()
            .map(|i| i["rule_id"].as_str().unwrap())
            .collect();
        assert!(rule_ids.contains(&"ours.npm"), "{rule_ids:?}");
        assert!(rule_ids.contains(&"linux.app_caches"), "{rule_ids:?}");
        // The Trash is found but only the screen may empty it.
        let trash = items
            .iter()
            .find(|i| i["rule_id"] == "linux.trash")
            .unwrap();
        assert_eq!(trash["selected_by_default"], false);
        assert_eq!(trash["risk"], "extreme");
        let plan_id = out["plan_id"].as_str().unwrap().to_string();
        let plan = plan::load(&ctx, &plan_id).unwrap();
        assert!(plan.items.iter().all(|i| i.risk != Risk::Extreme));
        let receipt = plan::execute(&ctx, &plan_id).unwrap();
        assert!(receipt.bytes_freed >= 40_000, "{}", receipt.bytes_freed);
        assert!(!home.join(".npm/_cacache/content/a").exists());
        assert!(home.join(".npm/_cacache").exists(), "the cache root stays");
        assert!(home.join(".local/share/Trash/files/old.txt").exists());
        // Unticked items stay pickable through wmb_plan.
        let trash_id = trash["id"].as_str().unwrap().to_string();
        assert!(
            plan::from_ids(&ctx, "cleanup", &[trash_id], Source::Agent).is_err(),
            "extreme items are screen-only"
        );
    }

    #[test]
    fn delete_files_revalidates_each_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("logs");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.log");
        let b = root.join("b.log");
        std::fs::write(&a, vec![1u8; 5000]).unwrap();
        std::fs::write(&b, vec![1u8; 5000]).unwrap();
        let fa = fsops::fingerprint(&a).unwrap();
        let fb = fsops::fingerprint(&b).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&b, vec![2u8; 9000]).unwrap();
        let args = json!({ "root": root, "destination": "unlink", "files": [{ "path": a, "fp": fa }, { "path": b, "fp": fb }] });
        let ctx = Ctx::at(dir.path());
        let o = plan_execute(&ctx, "r", "delete_files", &args).unwrap();
        assert!(!a.exists());
        assert!(b.exists(), "a file changed after the scan stays");
        assert!(o.detail.contains("1 removed"), "{}", o.detail);
        // A path outside the root is refused.
        let evil = json!({ "root": root, "destination": "unlink", "files": [{ "path": dir.path().join("x"), "fp": fa }] });
        assert!(plan_execute(&ctx, "r", "delete_files", &evil).is_err());
    }

    #[test]
    fn vacuum_through_the_plan_hook() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("places.sqlite");
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("CREATE TABLE t(x BLOB);").unwrap();
            for _ in 0..100 {
                c.execute("INSERT INTO t VALUES (zeroblob(4096))", [])
                    .unwrap();
            }
            c.execute_batch("DELETE FROM t;").unwrap();
        }
        let fp = fsops::fingerprint(&db).unwrap();
        let ctx = Ctx::at(&dir.path().join("state"));
        let args = json!({ "path": db, "root": dir.path(), "fp": fp, "running": [] });
        let o = plan_execute(&ctx, "r", "sqlite_vacuum", &args).unwrap();
        assert!(o.bytes > 100_000, "{}", o.bytes);
        // Second time the fingerprint no longer matches.
        assert!(plan_execute(&ctx, "r", "sqlite_vacuum", &args)
            .unwrap_err()
            .starts_with("CHANGED"));
    }

    /// Scans this machine for real (reads only; the plan goes to a temporary
    /// store) and prints a summary. `cargo test ... real_machine -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the real home and runs read-only probes (docker df, tmutil list)"]
    fn real_machine_scan_reads_only() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        if std::env::var_os("WMB_TIME_RULES").is_some() {
            let env = engine::Env::system();
            for r in catalog::rules_for(env.os) {
                let t = std::time::Instant::now();
                let s = engine::scan(&[r], &env, &engine::Filter::default());
                if t.elapsed().as_millis() > 500 {
                    eprintln!("slow {:?} {} ({} items)", t.elapsed(), r.id, s.items.len());
                }
            }
        }
        let t = std::time::Instant::now();
        let out = junk_scan(
            &ctx,
            &engine::Env::system(),
            Vec::new(),
            Risk::Extreme,
            Source::Ui,
        )
        .unwrap();
        eprintln!(
            "{:?} {} items, {} bytes, skipped {}, protected {}, fda {}",
            t.elapsed(),
            out["items"].as_array().unwrap().len(),
            out["total_bytes"],
            out["skipped_rules"],
            out["protected_paths"],
            out["needs_full_disk_access"]
        );
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&out["categories"]).unwrap()
        );
        for it in out["items"].as_array().unwrap().iter().take(40) {
            eprintln!(
                "{:>12} {:<11} {:<5} {:<40} {}",
                it["bytes"],
                it["risk"].as_str().unwrap(),
                it["selected_by_default"],
                it["rule_id"].as_str().unwrap(),
                it["path"].as_str().unwrap()
            );
        }
    }

    #[test]
    fn protection_hook_uses_the_lists() {
        let home = dirs::home_dir().unwrap();
        assert!(protection_match(&home.join("Library/Caches/com.1password.1password")).is_some());
        assert!(protection_match(&home.join(".cache/someapp")).is_none());
    }

    #[test]
    fn record_score_keeps_the_other_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(tmp.path());
        ctx.kv_set(
            "score_inputs",
            &json!({ "login_items": 4, "updated": { "login_items": 1 } }),
        )
        .unwrap();
        record_score(&ctx, 42);
        let v: Value = ctx.kv_get("score_inputs").unwrap();
        assert_eq!(v["junk_bytes"], 42);
        assert_eq!(v["login_items"], 4);
        assert_eq!(v["updated"]["login_items"], 1);
        assert!(v["updated"]["junk_bytes"].as_i64().unwrap() > 0);
    }
}
