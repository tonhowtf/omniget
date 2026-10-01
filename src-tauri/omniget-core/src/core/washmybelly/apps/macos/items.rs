use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::super::fsops;
use super::super::super::plan::{Op, PlanItem};
use super::super::super::protect::{self, Guard};
use super::super::super::{Destination, Risk};
use super::super::{App, Leftover};
use super::leftovers::{self, Found, Launchd, Note, Other, Roots, Subject};
use super::parse;

#[derive(Debug, Clone, Default)]
pub struct Receipt {
    pub id: String,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Env {
    pub roots: Roots,
    pub others: Vec<Other>,
    pub running: bool,
    pub app_groups: Vec<String>,
    pub receipts: Vec<Receipt>,
    pub shared: HashSet<PathBuf>,
    pub uid: u32,
}

#[derive(Debug, Clone)]
pub struct Unticked {
    pub item: PlanItem,
    pub why: &'static str,
}

pub struct Built {
    pub ticked: Vec<PlanItem>,
    pub unticked: Vec<Unticked>,
    pub extra: Value,
}

fn item_id(app: &App, path: &Path) -> String {
    format!("uninstall:{}:{}", app.id, path.display())
}

fn tilde(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn risk(admin: bool) -> Risk {
    if admin {
        Risk::Destructive
    } else {
        Risk::Reversible
    }
}

fn remove_item(
    id: String,
    label: String,
    detail: String,
    group: &str,
    path: &Path,
    root: &Path,
    admin: bool,
) -> Option<PlanItem> {
    protect::check(path, Guard::Picked, Some(root)).ok()?;
    let fp = fsops::fingerprint(path).ok()?;
    Some(PlanItem {
        id,
        label,
        detail: Some(detail),
        group: group.to_string(),
        bytes: fsops::measure(path).0,
        risk: risk(admin),
        admin,
        op: Op::RemovePath {
            path: path.to_path_buf(),
            destination: Destination::Trash,
            fp,
            root: Some(root.to_path_buf()),
            guard: Guard::Picked,
            contents_only: false,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn module_item(
    id: String,
    label: String,
    detail: String,
    group: &str,
    f: &Found,
    action: &str,
    mut args: Value,
) -> Option<PlanItem> {
    let (path, root, admin) = (f.path.as_path(), f.root.as_path(), f.admin);
    protect::check(path, Guard::Picked, Some(root)).ok()?;
    let fp = fsops::fingerprint(path).ok()?;
    args["plist"] = json!(path);
    args["root"] = json!(root);
    args["fp"] = json!(fp);
    Some(PlanItem {
        id,
        label,
        detail: Some(detail),
        group: group.to_string(),
        bytes: fsops::measure(path).0,
        risk: risk(admin),
        admin,
        op: Op::Module {
            module: "apps".into(),
            action: action.into(),
            args,
        },
    })
}

fn found_item(app: &App, subj: &Subject, f: &Found, home: &Path, uid: u32) -> Option<PlanItem> {
    let label = tilde(&f.path, home);
    let id = item_id(app, &f.path);
    if let Some((kind, job)) = &f.launchd {
        let (action, domain, detail) = match kind {
            Launchd::UserAgent => (
                "mac.bootout_agent",
                format!("gui/{uid}"),
                format!(
                    "{}. Stops the job {job}, then keeps its file in OmniGet's quarantine.",
                    f.evidence
                ),
            ),
            Launchd::SystemAgent => (
                "mac.remove_daemon",
                format!("gui/{uid}"),
                format!(
                    "{}. Stops the job {job} and removes its file (administrator).",
                    f.evidence
                ),
            ),
            Launchd::Daemon => (
                "mac.remove_daemon",
                "system".to_string(),
                format!(
                    "{}. Stops the system job {job} and removes its file (administrator).",
                    f.evidence
                ),
            ),
        };
        return module_item(
            id,
            label,
            detail,
            "launch",
            f,
            action,
            json!({ "label": job, "domain": domain }),
        );
    }
    if let Some(b) = subj.bundle_id.as_deref() {
        if !f.admin && f.path == home.join("Library/Preferences").join(format!("{b}.plist")) {
            let detail = format!(
                "{}. Kept in OmniGet's quarantine; macOS forgets its cached copy.",
                f.evidence
            );
            let args = json!({
                "bundle_id": b,
                "app_path": subj.app_path,
                "executable": app.extra.get("executable"),
            });
            return module_item(
                id,
                label,
                detail,
                "preferences",
                f,
                "mac.remove_prefs",
                args,
            );
        }
    }
    remove_item(
        id,
        label,
        f.evidence.clone(),
        f.group,
        &f.path,
        &f.root,
        f.admin,
    )
}

pub fn subject(app: &App) -> Subject {
    let bare = app
        .id
        .strip_prefix("app:")
        .or_else(|| app.id.strip_prefix("removed:"))
        .map(|r| r.split('@').next().unwrap_or(r).trim().to_string());
    let bundle_id = app
        .native_id
        .clone()
        .filter(|_| matches!(app.source.as_str(), "app_bundle" | "removed" | "app"))
        .or_else(|| {
            bare.clone()
                .filter(|b| parse::valid_bundle_id(b) && !b.contains(' '))
        });
    let mut names = vec![app.name.clone()];
    if let Some(b) = bare.filter(|b| !parse::valid_bundle_id(b)) {
        names.push(b);
    }
    if let Some(p) = &app.path {
        if let Some(stem) = Path::new(p).file_stem() {
            names.push(stem.to_string_lossy().into_owned());
        }
    }
    if let Some(exe) = app.extra.get("executable").and_then(Value::as_str) {
        names.push(exe.to_string());
    }
    names.retain(|n| Some(n.as_str()) != bundle_id.as_deref());
    names.dedup();
    Subject {
        bundle_id,
        team_id: app
            .extra
            .get("team_id")
            .and_then(Value::as_str)
            .map(String::from),
        names,
        app_path: app
            .path
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.extension().is_some_and(|e| e == "app")),
        app_groups: Vec::new(),
    }
}

pub fn build(app: &App, env: &Env) -> Built {
    let home = &env.roots.home;
    let mut ticked: Vec<PlanItem> = Vec::new();
    let mut unticked: Vec<Unticked> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut leftovers_out: Vec<Leftover> = Vec::new();
    let cask = app.extra.get("cask").filter(|c| c.is_object()).cloned();
    let token = cask
        .as_ref()
        .and_then(|c| c.get("token"))
        .and_then(Value::as_str)
        .map(String::from);

    let put = |item: PlanItem,
               tick: bool,
               why: &'static str,
               ticked: &mut Vec<PlanItem>,
               unticked: &mut Vec<Unticked>| {
        if tick && !env.running {
            ticked.push(item);
        } else {
            unticked.push(Unticked {
                item,
                why: if env.running { "open" } else { why },
            });
        }
    };

    if env.running {
        notes.push(format!(
            "{} is open. Quit it first; its items stay unticked until then.",
            app.name
        ));
    }

    if app.source == "homebrew_formula" || app.source == "homebrew_cask" {
        let name = app.native_id.clone().unwrap_or_default();
        if parse::valid_brew_token(&name) {
            let kind = if app.source == "homebrew_formula" {
                "--formula"
            } else {
                "--cask"
            };
            let measure: Vec<PathBuf> = app.path.iter().map(PathBuf::from).collect();
            put(
                PlanItem {
                    id: format!("uninstall:{}:brew", app.id),
                    label: format!("brew uninstall {kind} {name}"),
                    detail: Some("Homebrew removes the package. There is no undo; install it again with brew.".into()),
                    group: "app".into(),
                    bytes: app.bytes.unwrap_or(0),
                    risk: Risk::Destructive,
                    admin: false,
                    op: Op::Command { program: "brew".into(), args: vec!["uninstall".into(), kind.into(), name.clone()], measure: measure.clone() },
                },
                true,
                "",
                &mut ticked,
                &mut unticked,
            );
            if app.source == "homebrew_cask" {
                if let Some(z) = zap_item(app, &name, cask.as_ref(), measure) {
                    unticked.push(Unticked {
                        item: z,
                        why: "alternative",
                    });
                }
            }
        }
        return Built {
            ticked,
            unticked,
            extra: extra(Vec::new(), notes, token, env.running),
        };
    }

    let mut subj = subject(app);
    subj.app_groups = env.app_groups.clone();

    let app_path = subj.app_path.clone();
    if let Some(p) = &app_path {
        if app.removable && std::fs::symlink_metadata(p).is_ok() {
            let root = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            let detail = if app.needs_admin {
                "The app belongs to the administrator: it is removed for good, with your password."
                    .to_string()
            } else {
                "Moves the app to the Trash (Finder can put it back).".to_string()
            };
            if let Some(it) = remove_item(
                item_id(app, p),
                app.name.clone(),
                detail,
                "app",
                p,
                &root,
                app.needs_admin,
            ) {
                put(it, true, "", &mut ticked, &mut unticked);
            }
        } else if !app.removable {
            notes.push(format!("{} is part of macOS and stays.", app.name));
        }
    }

    let (found, search_notes) = leftovers::find(&env.roots, &subj, &env.others);
    let shared_bid = search_notes
        .iter()
        .any(|n| matches!(n, Note::SharedBundleId(_)));
    for n in &search_notes {
        notes.push(match n {
            Note::Apple => "Apple's own data is never touched; only the app itself can go.".to_string(),
            Note::SharedBundleId(paths) => format!(
                "Another copy uses the same bundle id ({}); its data, launch jobs and the Homebrew zap are left alone.",
                paths.join(", ")
            ),
            Note::NoBundleId => "No bundle id to go by: only folders with the same name were found, and they stay unticked.".to_string(),
            Note::UnreadableContainers(n) => {
                format!("{n} sandbox container(s) could not be read. Give OmniGet Full Disk Access to check them too.")
            }
        });
    }
    for f in &found {
        let Some(it) = found_item(app, &subj, f, home, env.uid) else {
            continue;
        };
        leftovers_out.push(Leftover {
            path: f.path.display().to_string(),
            bytes: it.bytes,
            confidence: f.confidence.to_string(),
            evidence: f.evidence.clone(),
            kind: f.kind.to_string(),
        });
        let (tick, why) = match (f.confidence, f.admin) {
            ("high", false) => (true, ""),
            ("high", true) => (false, "admin"),
            ("medium", _) => (false, "medium"),
            _ => (false, "low"),
        };
        put(it, tick, why, &mut ticked, &mut unticked);
    }

    let is_pkg = app.source == "pkg";
    let already: HashSet<PathBuf> = ticked
        .iter()
        .chain(unticked.iter().map(|u| &u.item))
        .filter_map(|i| match &i.op {
            Op::RemovePath { path, .. } => Some(path.clone()),
            Op::Module { args, .. } => args.get("plist").and_then(Value::as_str).map(PathBuf::from),
            _ => None,
        })
        .collect();
    let mut skipped_shared = 0usize;
    for r in &env.receipts {
        if !parse::valid_label(&r.id) || parse::is_apple(&r.id) {
            continue;
        }
        for p in &r.paths {
            if already.contains(p) || app_path.as_deref().is_some_and(|a| p.starts_with(a)) {
                continue;
            }
            if env.shared.contains(p) {
                skipped_shared += 1;
                continue;
            }
            let root = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            let evidence = format!("listed in the installer receipt {}", r.id);
            let Some(it) = remove_item(
                item_id(app, p),
                p.display().to_string(),
                evidence.clone(),
                "receipt",
                p,
                &root,
                true,
            ) else {
                continue;
            };
            leftovers_out.push(Leftover {
                path: p.display().to_string(),
                bytes: it.bytes,
                confidence: "high".into(),
                evidence,
                kind: if p.is_dir() {
                    "folder".into()
                } else {
                    "file".into()
                },
            });
            put(it, is_pkg, "admin", &mut ticked, &mut unticked);
        }
        put(
            PlanItem {
                id: format!("uninstall:{}:pkg-forget:{}", app.id, r.id),
                label: format!("Forget the installer receipt {}", r.id),
                detail: Some("macOS stops listing the package (pkgutil --forget). Runs after its files are gone.".into()),
                group: "receipt".into(),
                bytes: 0,
                risk: Risk::Destructive,
                admin: true,
                op: Op::Module { module: "apps".into(), action: "mac.pkg_forget".into(), args: json!({ "id": r.id }) },
            },
            is_pkg,
            "admin",
            &mut ticked,
            &mut unticked,
        );
    }
    if skipped_shared > 0 {
        notes.push(format!("{skipped_shared} file(s) of the installer receipt are also used by another package and stay."));
    }

    if let (Some(t), false) = (&token, shared_bid) {
        let measure: Vec<PathBuf> = app_path.iter().cloned().collect();
        if let Some(z) = zap_item(app, t, cask.as_ref(), measure) {
            unticked.push(Unticked {
                item: z,
                why: "alternative",
            });
            notes.push(format!(
                "Homebrew installed {}. The Trash leaves brew still listing it; the zap alternative removes it through brew.",
                app.name
            ));
        }
    }

    Built {
        ticked,
        unticked,
        extra: extra(leftovers_out, notes, token, env.running),
    }
}

fn zap_item(
    app: &App,
    token: &str,
    cask: Option<&Value>,
    measure: Vec<PathBuf>,
) -> Option<PlanItem> {
    if !parse::valid_brew_token(token) {
        return None;
    }
    let zap: Vec<String> = cask
        .and_then(|c| c.get("zap"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let detail = if zap.is_empty() {
        "Homebrew removes the app. This cask lists nothing extra to zap. No undo.".to_string()
    } else {
        let shown: Vec<&str> = zap.iter().take(12).map(String::as_str).collect();
        let more = zap.len().saturating_sub(shown.len());
        format!(
            "Homebrew removes the app and everything its zap lists, for good (no Trash):\n{}{}",
            shown.join("\n"),
            if more > 0 {
                format!("\n…and {more} more")
            } else {
                String::new()
            }
        )
    };
    Some(PlanItem {
        id: format!("uninstall:{}:zap", app.id),
        label: format!("brew uninstall --zap --cask {token}"),
        detail: Some(detail),
        group: "zap".into(),
        bytes: app.bytes.unwrap_or(0),
        risk: Risk::Destructive,
        admin: false,
        op: Op::Command {
            program: "brew".into(),
            args: vec![
                "uninstall".into(),
                "--zap".into(),
                "--cask".into(),
                token.into(),
            ],
            measure,
        },
    })
}

fn extra(
    leftovers: Vec<Leftover>,
    notes: Vec<String>,
    cask: Option<String>,
    running: bool,
) -> Value {
    json!({ "leftovers": leftovers, "notes": notes, "cask": cask, "running": running })
}

pub fn unticked_json(u: &[Unticked]) -> Value {
    Value::Array(
        u.iter()
            .map(|x| {
                json!({
                    "id": x.item.id, "label": x.item.label, "detail": x.item.detail, "group": x.item.group,
                    "bytes": x.item.bytes, "risk": x.item.risk, "admin": x.item.admin,
                    "why": x.why,
                })
            })
            .collect(),
    )
}
