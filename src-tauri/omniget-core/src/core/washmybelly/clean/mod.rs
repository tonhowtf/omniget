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
        if full {
            record_score(&ctx, out["selected_bytes"].as_u64().unwrap_or(0));
        }
        Ok(out)
    })
    .await
}

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

pub const ACTIONS: &[&str] = &["delete_files", "apfs_clone", "truncate", "sqlite_vacuum"];
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

fn revalidate(path: &Path, root: &Path, fp: &Fingerprint) -> Result<(), String> {
    protect::check(path, Guard::Rule, Some(root))?;
    fsops::unchanged(path, fp)
}

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
        return Ok("nothing to undo: the file kept identical content".into());
    }
    Err(format!(
        "clean: {action} cannot be undone (caches come back on their own)"
    ))
}

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

pub fn native_command_line(id: &str) -> String {
    match native_argv(id) {
        Ok((argv, _)) => format!("sudo {}", argv.join(" ")),
        Err(e) => format!("# {e}"),
    }
}

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

pub fn protection_match(path: &Path) -> Option<String> {
    protection::protection_match(path)
}

pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
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
