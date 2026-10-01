//! The cleanup engine: every rule, whatever its source, goes through here.
//!
//! For each rule of the current system it checks `requires` (a path that
//! must exist, a program on PATH, Full Disk Access, admin), expands the
//! targets (`~`, `$VAR`, `%VAR%`, `{mount}`, globs in Rust), applies
//! `include`/`exclude`/`min_age_days`, measures with hardlinks counted once
//! across all rules, drops what `protect::check` refuses, and marks items
//! whose app is open. A rule that found nothing does not appear
//! (BleachBit's auto_hide). Nothing here changes the disk.
//!
//! Target and filter language (`Rule` fields keep their shape):
//!
//! - `Delete`: each match of each target is one item, removed whole.
//! - `DeleteContents`: each matching folder is emptied and kept. With no
//!   filter it is one item; if the folder itself is protected (a root such
//!   as `~/Library/Logs`, or a folder the protection list names) and the
//!   target was written literally, each child becomes its own item. With a
//!   `include` the matching entries inside are listed one by one and
//!   removed by the `delete_files` action; with only `min_age_days`, the
//!   direct children whose newest file is old enough (one item as well).
//!   `exclude` drops matched folders and, inside, the children it names.
//! - `include`/`exclude` entries: a wildcard on the name (`*.log`),
//!   `re:<regex>` on the name, `path-re:<regex>` on the whole path, and
//!   `type:f`/`type:d` (include only). Types must all match; names match
//!   if any does. Excluded folders are not entered.
//! - `running`: a process name, or a lock-file glob (anything with a path
//!   separator, `~` or a variable). An open app leaves the item unticked.
//! - `{mount}` in a target: every mounted volume (`/Volumes/*` on macOS,
//!   `/proc/self/mounts` on Linux), for per-volume Trash folders.
//!
//! Special variables besides the environment: `XDG_*` with their defaults,
//! `UID`, `BREW_CACHE` (`brew --cache`) and `PNPM_STORE` (`pnpm store path`),
//! both accepted only inside the home folder.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use regex::{Regex, RegexBuilder};
use serde_json::{json, Value};

use super::super::fsops::{self, Fingerprint};
use super::super::plan::{Op, PlanItem, NATIVE_PROGRAMS};
use super::super::protect::{self, Guard};
use super::super::{Destination, Os, Risk};
use super::catalog;
use super::rule::{Action, Requirement, Rule};

/// Files one `delete_files` item may carry; the rest waits for the next scan.
pub const MAX_FILES_PER_ITEM: usize = 5000;

type Lookup = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
type Run = Box<dyn Fn(&str, &[&str]) -> Option<String> + Send + Sync>;
type Get = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
type OnPath = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// What the engine may read from the machine. Tests build one over a
/// temporary home with no programs and no processes.
pub struct Env {
    pub os: Os,
    lookup: Lookup,
    run: Run,
    snapd: Get,
    on_path: OnPath,
    /// Lower-case process names (and exe basenames, without `.exe`).
    pub processes: HashSet<String>,
    pub full_disk_access: bool,
    pub mounts: Vec<PathBuf>,
    /// Run the probes that ask system tools (`docker`, `tmutil`, snapd).
    pub probes: bool,
    computed: Mutex<HashMap<String, Option<String>>>,
}

impl Env {
    /// The real machine.
    pub fn system() -> Env {
        Env {
            os: super::super::current_os(),
            lookup: Box::new(fsops::env_lookup),
            run: Box::new(|p, a| run_with_timeout(p, a, Duration::from_secs(20))),
            snapd: Box::new(super::rules_linux::snapd_get),
            on_path: Box::new(on_path),
            processes: process_names(),
            full_disk_access: full_disk_access(),
            mounts: mounts(),
            probes: true,
            computed: Mutex::new(HashMap::new()),
        }
    }

    /// A fake machine over `home`: no programs, no processes, no mounts.
    pub fn fake(home: &Path, os: Os) -> Env {
        let home = home.to_string_lossy().into_owned();
        Env {
            os,
            lookup: Box::new(move |k| (k == "HOME" || k == "USERPROFILE").then(|| home.clone())),
            run: Box::new(|_, _| None),
            snapd: Box::new(|_| None),
            on_path: Box::new(|_| false),
            processes: HashSet::new(),
            full_disk_access: true,
            mounts: Vec::new(),
            probes: false,
            computed: Mutex::new(HashMap::new()),
        }
    }

    pub fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        (self.run)(program, args)
    }

    pub fn snapd_get(&self, path: &str) -> Option<String> {
        (self.snapd)(path)
    }

    pub fn has_program(&self, name: &str) -> bool {
        (self.on_path)(name)
    }

    fn home(&self) -> Option<String> {
        (self.lookup)("HOME").or_else(|| (self.lookup)("USERPROFILE"))
    }

    /// A variable for `expand_vars`, with the XDG defaults and the computed ones.
    pub fn var(&self, name: &str) -> Option<String> {
        if let Some(v) = (self.lookup)(name).filter(|v| !v.is_empty()) {
            return Some(v);
        }
        if self.os == Os::Windows {
            if let Some(v) = (self.lookup)(&name.to_ascii_uppercase()).filter(|v| !v.is_empty()) {
                return Some(v);
            }
        }
        let home = self.home();
        match name {
            "XDG_CACHE_HOME" => return home.map(|h| format!("{h}/.cache")),
            "XDG_CONFIG_HOME" => return home.map(|h| format!("{h}/.config")),
            "XDG_DATA_HOME" => return home.map(|h| format!("{h}/.local/share")),
            "UID" => return uid(),
            _ => {}
        }
        let (program, args): (&str, &[&str]) = match name {
            "BREW_CACHE" => ("brew", &["--cache"]),
            "PNPM_STORE" => ("pnpm", &["store", "path"]),
            _ => return None,
        };
        if let Ok(cache) = self.computed.lock() {
            if let Some(v) = cache.get(name) {
                return v.clone();
            }
        }
        let value = self
            .output(program, args)
            .map(|s| s.trim().to_string())
            .filter(|p| {
                home.as_deref().is_some_and(|h| {
                    inside(p, h) && p.trim_end_matches('/') != h.trim_end_matches('/')
                })
            });
        if let Ok(mut cache) = self.computed.lock() {
            cache.insert(name.to_string(), value.clone());
        }
        value
    }

    /// Every existing path a target names.
    pub fn expand(&self, pattern: &str) -> Vec<PathBuf> {
        let patterns: Vec<String> = if pattern.contains("{mount}") {
            self.mounts
                .iter()
                .map(|m| pattern.replace("{mount}", m.to_string_lossy().trim_end_matches('/')))
                .collect()
        } else {
            vec![pattern.to_string()]
        };
        let mut out = Vec::new();
        for p in patterns {
            let Some(p) = fsops::expand_vars(&p, &|n| self.var(n)) else {
                continue;
            };
            for found in glob_fast(&p) {
                if !out.contains(&found) {
                    out.push(found);
                }
            }
        }
        out
    }

    /// The first open app or lock file that holds this rule's data.
    pub fn running(&self, rule: &Rule) -> Option<String> {
        for r in &rule.running {
            let lock_file = r.contains('/')
                || r.contains('\\')
                || r.starts_with('~')
                || r.starts_with('$')
                || r.starts_with('%');
            if lock_file {
                if !self.expand(r).is_empty() {
                    return Some(r.clone());
                }
            } else {
                let n = r.to_lowercase();
                let n = n.strip_suffix(".exe").unwrap_or(&n);
                if self.processes.contains(n) {
                    return Some(r.clone());
                }
            }
        }
        None
    }
}

fn has_wildcard(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// Glob only up to the last segment with a wildcard, then append the
/// literal rest and check it exists. Faster, and it reaches folders that
/// can be entered but not listed (`/Volumes/X/.Trashes/<uid>`).
pub fn glob_fast(pattern: &str) -> Vec<PathBuf> {
    let norm = pattern.replace('\\', "/");
    let segs: Vec<&str> = norm.split('/').collect();
    let Some(last) = segs.iter().rposition(|s| has_wildcard(s)) else {
        let p = PathBuf::from(pattern);
        return if std::fs::symlink_metadata(&p).is_ok() {
            vec![p]
        } else {
            Vec::new()
        };
    };
    let head = segs[..=last].join("/");
    let tail: Vec<&str> = segs[last + 1..]
        .iter()
        .copied()
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = Vec::new();
    for base in fsops::glob(&head) {
        if tail.is_empty() {
            out.push(base);
            continue;
        }
        let Ok(m) = std::fs::symlink_metadata(&base) else {
            continue;
        };
        if !m.is_dir() || fsops::is_link(&m) {
            continue;
        }
        let p = tail.iter().fold(base, |acc, s| acc.join(s));
        if std::fs::symlink_metadata(&p).is_ok() {
            out.push(p);
        }
    }
    out
}

/// Case-insensitive lexical containment on macOS and Windows.
fn inside(child: &str, parent: &str) -> bool {
    let fold = |s: &str| {
        let s = s.replace('\\', "/");
        let s = s.trim_end_matches('/').to_string();
        if cfg!(any(windows, target_os = "macos")) {
            s.to_lowercase()
        } else {
            s
        }
    };
    let (c, p) = (fold(child), fold(parent));
    !p.is_empty() && (c == p || c.starts_with(&format!("{p}/")))
}

fn overlaps(a: &str, b: &str) -> bool {
    inside(a, b) || inside(b, a)
}

// ── Include / exclude ────────────────────────────────────────────────

enum Pat {
    Wild(String),
    Re(Regex),
    PathRe(Regex),
}

impl Pat {
    fn hit(&self, path: &Path, name: &str) -> bool {
        match self {
            Pat::Wild(w) => fsops::wildcard(w, name),
            Pat::Re(r) => r.is_match(name),
            Pat::PathRe(r) => r.is_match(&path.to_string_lossy()),
        }
    }
}

struct Matcher {
    kind: Option<char>,
    names: Vec<Pat>,
    ex: Vec<Pat>,
}

fn regex(s: &str) -> Result<Regex, String> {
    RegexBuilder::new(s)
        .case_insensitive(cfg!(windows))
        .build()
        .map_err(|e| e.to_string())
}

fn pat(s: &str) -> Result<Option<Pat>, String> {
    Ok(Some(if let Some(r) = s.strip_prefix("path-re:") {
        Pat::PathRe(regex(r)?)
    } else if let Some(r) = s.strip_prefix("re:") {
        Pat::Re(regex(r)?)
    } else if s.starts_with("type:") {
        return Ok(None);
    } else {
        Pat::Wild(s.to_string())
    }))
}

impl Matcher {
    fn new(rule: &Rule) -> Result<Matcher, String> {
        let mut m = Matcher {
            kind: None,
            names: Vec::new(),
            ex: Vec::new(),
        };
        for i in &rule.include {
            if let Some(t) = i.strip_prefix("type:") {
                m.kind = t.chars().next();
            } else if let Some(p) = pat(i)? {
                m.names.push(p);
            }
        }
        for e in &rule.exclude {
            if let Some(p) = pat(e)? {
                m.ex.push(p);
            }
        }
        Ok(m)
    }

    /// An `include` asks for a file-by-file listing (recursive).
    fn filtered(&self) -> bool {
        self.kind.is_some() || !self.names.is_empty()
    }

    fn excluded(&self, path: &Path, name: &str) -> bool {
        self.ex.iter().any(|p| p.hit(path, name))
    }

    fn included(&self, path: &Path, name: &str, is_dir: bool) -> bool {
        match self.kind {
            Some('f') if is_dir => return false,
            Some('d') if !is_dir => return false,
            _ => {}
        }
        self.names.is_empty() || self.names.iter().any(|p| p.hit(path, name))
    }
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ── Scan ─────────────────────────────────────────────────────────────

/// One thing a rule found, ready to become a plan item.
#[derive(Debug, Clone)]
pub struct Found {
    /// Absolute path, or `<kind>:<name>` for things that are not a path.
    pub path: String,
    /// Replaces the rule label when set (a snap revision, a runtime).
    pub label: Option<String>,
    pub bytes: u64,
    pub files: u64,
    pub op: Op,
    pub admin: bool,
}

/// A scan result item.
#[derive(Debug, Clone)]
pub struct Item {
    pub rule_id: String,
    pub category: String,
    pub path: String,
    pub files: u64,
    pub destination: Destination,
    pub running: Option<String>,
    pub selected_by_default: bool,
    pub plan: PlanItem,
}

impl Item {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.plan.id,
            "rule_id": self.rule_id,
            "label": self.plan.label,
            "category": self.category,
            "path": self.path,
            "bytes": self.plan.bytes,
            "files": self.files,
            "risk": self.plan.risk,
            "admin": self.plan.admin,
            "destination": self.destination,
            "running": self.running,
            "selected_by_default": self.selected_by_default,
        })
    }
}

#[derive(Debug, Default)]
pub struct Scan {
    pub items: Vec<Item>,
    pub skipped_rules: usize,
    pub needs_full_disk_access: bool,
    /// Paths a rule matched that the protection refused.
    pub protected: usize,
}

pub struct Filter {
    pub categories: Vec<String>,
    pub max_risk: Risk,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            categories: Vec::new(),
            max_risk: Risk::Extreme,
        }
    }
}

type Probe = fn(&Rule, &Env) -> Vec<Found>;

/// Rules whose findings come from a system tool rather than a path.
fn probe_for(id: &str) -> Option<Probe> {
    Some(match id {
        "macos.simctl_unavailable" => super::rules_macos::probe_simctl_unavailable,
        "macos.simulator_runtimes" => super::rules_macos::probe_simulator_runtimes,
        "macos.tm_snapshots" => super::rules_macos::probe_tm_snapshots,
        "linux.journal" => super::rules_linux::probe_journal,
        "linux.snap_revisions" => super::rules_linux::probe_snaps,
        "ours.docker_prune" => super::rules_ours::probe_docker,
        _ => return None,
    })
}

/// The module action behind a native command that needs root.
pub fn admin_action(rule: &Rule) -> Option<(String, Value)> {
    let Action::Command { program, args, .. } = &rule.action else {
        return None;
    };
    match program.as_str() {
        "journalctl" => {
            let mb = args
                .iter()
                .find_map(|a| a.strip_prefix("--vacuum-size="))
                .and_then(|s| s.trim_end_matches('M').parse::<u64>().ok())?;
            Some(("journal_vacuum".into(), json!({ "max_mb": mb })))
        }
        "tmutil" => Some((
            "tmutil_thin".into(),
            json!({ "bytes": args.get(2).and_then(|b| b.parse::<u64>().ok())? }),
        )),
        "apt-get" | "dnf" | "paccache" | "pacman" | "zypper" => {
            Some(("pkg_cache".into(), json!({ "manager": program })))
        }
        _ => None,
    }
}

/// Scan `rules` on `env`. The caller turns the items into a plan.
pub fn scan(rules: &[&Rule], env: &Env, filter: &Filter) -> Scan {
    let mut out = Scan::default();
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut claimed: Vec<String> = Vec::new();
    let total = rules.len() as u64;

    for (i, rule) in rules.iter().enumerate() {
        super::super::progress("junk_scan", "rule", i as u64, total, rule.label.clone());
        if !filter.categories.is_empty() && !filter.categories.contains(&rule.category) {
            continue;
        }
        if rule.risk > filter.max_risk {
            continue;
        }
        let mut admin = false;
        let mut ok = true;
        for req in &rule.requires {
            match req {
                Requirement::PathExists { path } => ok &= !env.expand(path).is_empty(),
                Requirement::Program { name } => ok &= env.has_program(name),
                Requirement::FullDiskAccess => {
                    if !env.full_disk_access {
                        out.needs_full_disk_access = true;
                        ok = false;
                    }
                }
                Requirement::Admin => admin = true,
            }
        }
        if let Action::Command { program, .. } = &rule.action {
            ok &= NATIVE_PROGRAMS.contains(&program.as_str());
        }
        if !ok {
            out.skipped_rules += 1;
            continue;
        }
        let found = if let Some(probe) = probe_for(&rule.id) {
            if env.probes {
                probe(rule, env)
            } else {
                Vec::new()
            }
        } else {
            match collect(rule, env, admin, &mut seen, &mut out.protected) {
                Ok(f) => f,
                Err(_) => {
                    out.skipped_rules += 1;
                    continue;
                }
            }
        };
        let found = wrap_maintenance(rule, found);
        if found.is_empty() {
            continue;
        }
        let running = env.running(rule);
        let selected = catalog::default_selected(rule) && running.is_none();
        for f in found {
            let claim: Vec<String> = match &f.op {
                Op::Command { measure, .. } => measure
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
                _ if f.path.starts_with('/')
                    || f.path.get(1..3) == Some(":/")
                    || f.path.get(1..3) == Some(":\\") =>
                {
                    vec![f.path.clone()]
                }
                _ => Vec::new(),
            };
            if claim.iter().any(|c| claimed.iter().any(|k| overlaps(c, k))) {
                continue;
            }
            claimed.extend(claim);
            let id = format!("cleanup:{}:{}", rule.id, f.path);
            if out.items.iter().any(|it| it.plan.id == id) {
                continue;
            }
            out.items.push(Item {
                rule_id: rule.id.clone(),
                category: rule.category.clone(),
                path: f.path.clone(),
                files: f.files,
                destination: rule.destination,
                running: running.clone(),
                selected_by_default: selected,
                plan: PlanItem {
                    id,
                    label: f.label.unwrap_or_else(|| rule.label.clone()),
                    detail: Some(f.path),
                    group: rule.category.clone(),
                    bytes: f.bytes,
                    risk: rule.risk,
                    admin: admin || f.admin,
                    op: f.op,
                },
            });
        }
    }
    let n = out.items.len() as u64;
    super::super::progress("junk_scan", "done", total, total, format!("{n} items"));
    out
}

/// Windows rules that need a service stopped first, or whose files only
/// the system tool may remove (`rules_windows::maintenance_for`): their
/// items become administrator module items carrying the maintenance id.
fn wrap_maintenance(rule: &Rule, found: Vec<Found>) -> Vec<Found> {
    use super::rules_windows::{maintenance_for, Stage};
    let Some((id, stage)) = maintenance_for(&rule.id) else {
        return found;
    };
    match stage {
        Stage::Instead => {
            let Some(first) = found.first() else { return found };
            vec![Found {
                path: first.path.clone(),
                label: None,
                bytes: found.iter().map(|f| f.bytes).sum(),
                files: found.iter().map(|f| f.files).sum(),
                op: Op::Module { module: "clean".into(), action: "maintenance".into(), args: json!({ "id": id }) },
                admin: true,
            }]
        }
        Stage::Before => found
            .into_iter()
            .map(|f| match f.op {
                Op::RemovePath { path, fp, root, contents_only, .. } => Found {
                    op: Op::Module {
                        module: "clean".into(),
                        action: "remove_after_maintenance".into(),
                        args: json!({ "id": id, "path": path, "fp": fp, "root": root, "contents_only": contents_only }),
                    },
                    admin: true,
                    ..f
                },
                op => Found { op, ..f },
            })
            .collect(),
    }
}

/// The literal folder a pattern starts from (before the first wildcard).
fn literal_root(pattern: &Path) -> PathBuf {
    let s = pattern.to_string_lossy().replace('\\', "/");
    let mut parts = Vec::new();
    for seg in s.split('/') {
        if has_wildcard(seg) {
            break;
        }
        parts.push(seg);
    }
    let joined = parts.join("/");
    PathBuf::from(if joined.is_empty() {
        "/".to_string()
    } else {
        joined
    })
}

/// Days since the newest modification inside a tree (the tree itself for a file).
fn newest_age_days(path: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !m.is_dir() || fsops::is_link(&m) {
        return fsops::age_days(&m);
    }
    // Files only: a folder's own mtime moves whenever a child comes or goes.
    let mut newest: Option<u64> = None;
    for e in walkdir::WalkDir::new(path)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        if e.file_type().is_dir() {
            continue;
        }
        if let Ok(m) = e.metadata() {
            let a = fsops::age_days(&m);
            newest = Some(newest.map_or(a, |n| n.min(a)));
            if a == 0 {
                break;
            }
        }
    }
    newest.unwrap_or_else(|| fsops::age_days(&m))
}

/// `X-wal`, `X-shm`, `X-journal` whose database `X` stays: never removed.
fn orphans_live_database(path: &Path, taken: &HashSet<PathBuf>) -> bool {
    let name = name_of(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        if let Some(base) = name.strip_suffix(suffix) {
            let db = path.with_file_name(base);
            return db.exists() && !taken.contains(&db);
        }
    }
    false
}

fn remove_op(path: &Path, root: &Path, rule: &Rule, contents_only: bool) -> Option<Op> {
    Some(Op::RemovePath {
        path: path.to_path_buf(),
        destination: rule.destination,
        fp: fsops::fingerprint(path).ok()?,
        root: Some(root.to_path_buf()),
        guard: Guard::Rule,
        contents_only,
    })
}

fn collect(
    rule: &Rule,
    env: &Env,
    admin: bool,
    seen: &mut HashSet<(u64, u64)>,
    protected: &mut usize,
) -> Result<Vec<Found>, String> {
    let matcher = Matcher::new(rule)?;
    let mut found = Vec::new();

    if let Action::Command {
        program,
        args,
        measure,
    } = &rule.action
    {
        let paths: Vec<PathBuf> = measure.iter().flat_map(|m| env.expand(m)).collect();
        let (mut bytes, mut files) = (0, 0);
        for p in &paths {
            let (b, f) = fsops::measure_with(p, seen);
            bytes += b;
            files += f;
        }
        if !measure.is_empty() && files == 0 {
            return Ok(Vec::new());
        }
        let op = if admin {
            let Some((action, args)) = admin_action(rule) else {
                return Err("no admin action".into());
            };
            Op::Module {
                module: "clean".into(),
                action,
                args,
            }
        } else {
            Op::Command {
                program: program.clone(),
                args: args.clone(),
                measure: paths.clone(),
            }
        };
        let path = paths
            .first()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("cmd:{program}"));
        found.push(Found {
            path,
            label: None,
            bytes,
            files,
            op,
            admin,
        });
        return Ok(found);
    }

    // Whole-path candidates of this rule, with the folder each must stay in.
    let mut whole: Vec<(PathBuf, PathBuf)> = Vec::new();
    for target in &rule.targets {
        let literal = !has_wildcard(target) && !target.contains("{mount}");
        for m in env.expand(target) {
            let Ok(meta) = std::fs::symlink_metadata(&m) else {
                continue;
            };
            if fsops::is_link(&meta) {
                continue;
            }
            let name = name_of(&m);
            match &rule.action {
                Action::Delete => {
                    if matcher.excluded(&m, &name) || !matcher.included(&m, &name, meta.is_dir()) {
                        continue;
                    }
                    let mut root = literal_root(&m);
                    if root == m {
                        root = m.parent().map(Path::to_path_buf).unwrap_or(root);
                    }
                    whole.push((m, root));
                }
                Action::DeleteContents => {
                    if !meta.is_dir() {
                        continue;
                    }
                    if matcher.excluded(&m, &name) {
                        continue;
                    }
                    if matcher.filtered() {
                        if let Some(f) = per_file(rule, &matcher, &m, admin, seen, protected) {
                            found.push(f);
                        }
                    } else if rule.min_age_days > 0 || !matcher.ex.is_empty() {
                        if let Some(f) = old_children(rule, &matcher, &m, admin, seen, protected) {
                            found.push(f);
                        }
                    } else if protect::check(&m, Guard::Rule, Some(&m)).is_ok() {
                        let (bytes, files) = fsops::measure_with(&m, seen);
                        if files == 0 {
                            continue;
                        }
                        if let Some(op) = remove_op(&m, &m, rule, true) {
                            found.push(Found {
                                path: m.to_string_lossy().into_owned(),
                                label: None,
                                bytes,
                                files,
                                op,
                                admin,
                            });
                        }
                    } else if literal {
                        // A named folder that is itself off limits (a root, or
                        // named by the protection list): its children, one by one.
                        for e in std::fs::read_dir(&m).into_iter().flatten().flatten() {
                            whole.push((e.path(), m.clone()));
                        }
                    } else {
                        *protected += 1;
                    }
                }
                Action::Truncate | Action::SqliteVacuum => {
                    if !meta.is_file()
                        || matcher.excluded(&m, &name)
                        || !matcher.included(&m, &name, false)
                    {
                        continue;
                    }
                    if protect::check(&m, Guard::Rule, m.parent()).is_err() {
                        *protected += 1;
                        continue;
                    }
                    let Ok(fp) = fsops::fingerprint(&m) else {
                        continue;
                    };
                    let root = m.parent().map(Path::to_path_buf).unwrap_or_default();
                    let (action, bytes) = if matches!(rule.action, Action::Truncate) {
                        ("truncate", fsops::allocated(&meta))
                    } else {
                        ("sqlite_vacuum", super::sqlite::estimate(&m).unwrap_or(0))
                    };
                    if bytes == 0 {
                        continue;
                    }
                    found.push(Found {
                        path: m.to_string_lossy().into_owned(),
                        label: None,
                        bytes,
                        files: 1,
                        op: Op::Module {
                            module: "clean".into(),
                            action: action.into(),
                            args: json!({ "path": m, "root": root, "fp": fp, "running": rule.running }),
                        },
                        admin,
                    });
                }
                Action::Command { .. } => {}
            }
        }
    }

    let taken: HashSet<PathBuf> = whole.iter().map(|(p, _)| p.clone()).collect();
    for (path, root) in whole {
        let name = name_of(&path);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if fsops::is_link(&meta)
            || matcher.excluded(&path, &name)
            || orphans_live_database(&path, &taken)
        {
            continue;
        }
        if rule.min_age_days > 0 && newest_age_days(&path) < u64::from(rule.min_age_days) {
            continue;
        }
        if protect::check(&path, Guard::Rule, Some(&root)).is_err() {
            *protected += 1;
            continue;
        }
        let (bytes, files) = fsops::measure_with(&path, seen);
        if meta.is_dir() && files == 0 {
            continue;
        }
        if let Some(op) = remove_op(&path, &root, rule, false) {
            found.push(Found {
                path: path.to_string_lossy().into_owned(),
                label: None,
                bytes,
                files,
                op,
                admin,
            });
        }
    }
    Ok(found)
}

fn delete_files_found(
    rule: &Rule,
    root: &Path,
    files: Vec<(PathBuf, Fingerprint)>,
    bytes: u64,
    count: u64,
    admin: bool,
) -> Option<Found> {
    if files.is_empty() {
        return None;
    }
    let list: Vec<Value> = files
        .iter()
        .map(|(p, fp)| json!({ "path": p, "fp": fp }))
        .collect();
    Some(Found {
        path: root.to_string_lossy().into_owned(),
        label: None,
        bytes,
        files: count,
        op: Op::Module {
            module: "clean".into(),
            action: "delete_files".into(),
            args: json!({ "root": root, "destination": rule.destination, "files": list }),
        },
        admin,
    })
}

/// `DeleteContents` with `min_age_days` or `exclude` and no `include`: the
/// direct children not excluded whose newest file is old enough (Kudu's
/// recency rule), one item.
fn old_children(
    rule: &Rule,
    matcher: &Matcher,
    root: &Path,
    admin: bool,
    seen: &mut HashSet<(u64, u64)>,
    protected: &mut usize,
) -> Option<Found> {
    let mut files = Vec::new();
    let (mut bytes, mut count) = (0u64, 0u64);
    for e in std::fs::read_dir(root).ok()?.flatten() {
        if files.len() >= MAX_FILES_PER_ITEM {
            break;
        }
        let path = e.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if fsops::is_link(&meta) || matcher.excluded(&path, &name_of(&path)) {
            continue;
        }
        if rule.min_age_days > 0 && newest_age_days(&path) < u64::from(rule.min_age_days) {
            continue;
        }
        if protect::check(&path, Guard::Rule, Some(root)).is_err() {
            *protected += 1;
            continue;
        }
        let Ok(fp) = fsops::fingerprint(&path) else {
            continue;
        };
        let (b, f) = fsops::measure_with(&path, seen);
        if meta.is_dir() && f == 0 {
            continue;
        }
        bytes += b;
        count += f.max(1);
        files.push((path, fp));
    }
    delete_files_found(rule, root, files, bytes, count, admin)
}

/// A filtered folder: list what matches inside, one `delete_files` item.
fn per_file(
    rule: &Rule,
    matcher: &Matcher,
    root: &Path,
    admin: bool,
    seen: &mut HashSet<(u64, u64)>,
    protected: &mut usize,
) -> Option<Found> {
    let mut files: Vec<(PathBuf, Fingerprint)> = Vec::new();
    let (mut bytes, mut count) = (0u64, 0u64);
    let mut walk = walkdir::WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .into_iter();
    let taken_whole_dirs = rule.min_age_days == 0 && matcher.kind != Some('f');
    let mut all: Vec<PathBuf> = Vec::new();
    while let Some(entry) = walk.next() {
        let Ok(e) = entry else { continue };
        let path = e.path().to_path_buf();
        let name = name_of(&path);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if fsops::is_link(&meta) {
            continue;
        }
        let is_dir = meta.is_dir();
        if matcher.excluded(&path, &name) {
            if is_dir {
                walk.skip_current_dir();
            }
            continue;
        }
        if is_dir {
            if protect::check(&path, Guard::Rule, Some(root)).is_err() {
                *protected += 1;
                walk.skip_current_dir();
                continue;
            }
            if taken_whole_dirs && !matcher.names.is_empty() && matcher.included(&path, &name, true)
            {
                all.push(path);
                walk.skip_current_dir();
            }
            continue;
        }
        if !matcher.included(&path, &name, false) {
            continue;
        }
        if rule.min_age_days > 0 && fsops::age_days(&meta) < u64::from(rule.min_age_days) {
            continue;
        }
        all.push(path);
    }
    let taken: HashSet<PathBuf> = all.iter().cloned().collect();
    for path in all {
        if files.len() >= MAX_FILES_PER_ITEM {
            break;
        }
        if orphans_live_database(&path, &taken) {
            continue;
        }
        if protect::check(&path, Guard::Rule, Some(root)).is_err() {
            *protected += 1;
            continue;
        }
        let Ok(fp) = fsops::fingerprint(&path) else {
            continue;
        };
        let (b, f) = fsops::measure_with(&path, seen);
        bytes += b;
        count += f.max(1);
        files.push((path, fp));
    }
    delete_files_found(rule, root, files, bytes, count, admin)
}

// ── The real machine ─────────────────────────────────────────────────

/// Run a program (argv, no shell) and return stdout when it succeeds in time.
pub fn run_with_timeout(program: &str, args: &[&str], limit: Duration) -> Option<String> {
    use std::io::Read;
    let mut child = crate::core::process::std_command(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = reader.join().ok()?;
    status.success().then_some(text)
}

fn on_path(program: &str) -> bool {
    let path = crate::core::process::enhanced_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::split_paths(&path).any(|d| {
        exts.iter()
            .any(|e| d.join(format!("{program}{e}")).is_file())
    })
}

fn process_names() -> HashSet<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut out = HashSet::new();
    for p in sys.processes().values() {
        let n = p.name().to_string_lossy().to_lowercase();
        out.insert(n.strip_suffix(".exe").unwrap_or(&n).to_string());
        if let Some(exe) = p.exe().and_then(|e| e.file_name()) {
            let n = exe.to_string_lossy().to_lowercase();
            out.insert(n.strip_suffix(".exe").unwrap_or(&n).to_string());
        }
    }
    out
}

/// Read access to the user's TCC database proves Full Disk Access (as
/// PureMac does). Other systems have no such gate.
fn full_disk_access() -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    dirs::home_dir().is_some_and(|h| {
        std::fs::File::open(h.join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok()
    })
}

fn uid() -> Option<String> {
    #[cfg(unix)]
    {
        Some(unsafe { libc::getuid() }.to_string())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn mounts() -> Vec<PathBuf> {
    match super::super::current_os() {
        Os::Macos => std::fs::read_dir("/Volumes")
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !fsops::is_link(&m))
            })
            .collect(),
        Os::Linux => {
            parse_mounts(&std::fs::read_to_string("/proc/self/mounts").unwrap_or_default())
        }
        Os::Windows => Vec::new(),
    }
}

/// Mount points of real file systems from `/proc/self/mounts`.
pub fn parse_mounts(text: &str) -> Vec<PathBuf> {
    const PSEUDO: &[&str] = &[
        "proc",
        "sysfs",
        "tmpfs",
        "devtmpfs",
        "devpts",
        "cgroup",
        "cgroup2",
        "overlay",
        "squashfs",
        "autofs",
        "debugfs",
        "tracefs",
        "securityfs",
        "pstore",
        "bpf",
        "configfs",
        "mqueue",
        "hugetlbfs",
        "fusectl",
        "binfmt_misc",
        "nsfs",
        "ramfs",
        "efivarfs",
        "fuse.portal",
        "fuse.gvfsd-fuse",
        "rpc_pipefs",
        "selinuxfs",
    ];
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 || PSEUDO.contains(&f[2]) {
            continue;
        }
        let mount = f[1]
            .replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\134", "\\");
        let skip = [
            "/proc",
            "/sys",
            "/dev",
            "/boot",
            "/snap",
            "/var/lib/docker",
            "/var/snap",
        ]
        .iter()
        .any(|p| inside(&mount, p))
            || (inside(&mount, "/run") && !inside(&mount, "/run/media"));
        let p = PathBuf::from(mount);
        if !skip && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}
