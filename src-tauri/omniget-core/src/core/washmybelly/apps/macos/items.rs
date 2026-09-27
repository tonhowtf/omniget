//! From an [`App`] and what the system said about it to plan items.
//!
//! Pure over the file system (fingerprints and sizes of real paths), so the
//! tests build a fake home in a temporary folder and check the plan.
//!
//! Screen contract: the app itself is group `app`; every leftover item's id
//! ends in `:<path>` with the same path as its [`Leftover`]; the Homebrew
//! zap alternative is its own item in group `zap`; `extra.cask` is the
//! token; `extra.notes` are plain English strings; "reset app" is the
//! leftover ids without the app item.
//!
//! The items a person would expect ticked (the app, high-confidence data
//! in `~/Library`, the receipt of a bare `.pkg`) go back as the plan. The
//! rest (name-only matches, admin leftovers of an app bundle, everything of
//! an open app, the zap alternative) come back separately with a reason, so
//! the caller registers them as candidates outside the default plan.

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

/// One `.pkg` receipt tied to the app: absolute paths, already collapsed.
#[derive(Debug, Clone, Default)]
pub struct Receipt {
    pub id: String,
    pub paths: Vec<PathBuf>,
}

/// Everything the builder needs from the system, gathered by the caller.
#[derive(Debug, Clone)]
pub struct Env {
    pub roots: Roots,
    pub others: Vec<Other>,
    pub running: bool,
    pub app_groups: Vec<String>,
    pub receipts: Vec<Receipt>,
    /// Files other third-party receipts also list: never removed.
    pub shared: HashSet<PathBuf>,
    pub uid: u32,
}

/// A candidate left out of the default plan, and why.
#[derive(Debug, Clone)]
pub struct Unticked {
    pub item: PlanItem,
    /// `open`, `admin`, `medium`, `low`, `alternative`
    /// (i18n key `washmybelly.apps.mac.why.<why>`).
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
    // The helper removes for good; the Trash and our quarantine can undo.
    if admin {
        Risk::Destructive
    } else {
        Risk::Reversible
    }
}

/// A plain removal. `None` when the path is gone or protected.
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

fn module_item(
    id: String,
    label: String,
    detail: String,
    group: &str,
    path: &Path,
    root: &Path,
    admin: bool,
    action: &str,
    mut args: Value,
) -> Option<PlanItem> {
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
            &f.path,
            &f.root,
            f.admin,
            action,
            json!({ "label": job, "domain": domain }),
        );
    }
    // The app's own preferences file: removed through cfprefsd, so the
    // cached copy goes too (see `actions.rs`).
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
                &f.path,
                &f.root,
                false,
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

/// The subject of a leftover search, from what the inventory knows.
pub fn subject(app: &App) -> Subject {
    // A bare id (the app is gone): `app:<bundle id>` from an old listing, or
    // `removed:<name or bundle id>` from the screen.
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
    // A bundle id is not a name.
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

    // Homebrew formulae and casks without an app bundle: brew does the work.
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

    // The app bundle itself.
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

    // Leftovers.
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

    // Installer receipts.
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

    // The Homebrew zap, never in the default plan.
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

/// The `extra.unticked` rows: candidates outside the default plan, pickable
/// by id through `wmb_plan`.
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

#[cfg(test)]
mod tests {
    use super::super::leftovers::tests::fake_home;
    use super::*;

    fn env(roots: Roots) -> Env {
        Env {
            roots,
            others: vec![Other {
                name: "Example Editor Pro".into(),
                path: "/Applications/Example Editor Pro.app".into(),
                bundle_id: Some("com.example.editor.pro".into()),
                team_id: Some("TEAM123456".into()),
            }],
            running: false,
            app_groups: vec![],
            receipts: vec![],
            shared: HashSet::new(),
            uid: 501,
        }
    }

    fn fake_app(dir: &Path) -> App {
        let app = dir.join("Applications/Example Editor.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/editor"), vec![0u8; 8192]).unwrap();
        App {
            id: "app:com.example.editor".into(),
            name: "Example Editor".into(),
            source: "app_bundle".into(),
            path: Some(app.display().to_string()),
            native_id: Some("com.example.editor".into()),
            removable: true,
            extra: json!({ "team_id": "TEAM123456", "executable": "editor",
                "cask": { "token": "example-editor", "zap": ["trash: ~/Library/Caches/com.example.editor"] } }),
            ..Default::default()
        }
    }

    fn ids(v: &[PlanItem]) -> Vec<String> {
        v.iter().map(|i| i.id.clone()).collect()
    }

    #[test]
    fn plan_with_app_leftovers_and_zap() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let app = fake_app(dir.path());
        let b = build(&app, &env(roots.clone()));
        let t = ids(&b.ticked);
        assert!(
            t[0].ends_with("Example Editor.app"),
            "the app comes first: {t:?}"
        );
        assert!(t
            .iter()
            .all(|i| i.starts_with("uninstall:app:com.example.editor:")));
        let app_item = &b.ticked[0];
        assert_eq!(app_item.group, "app");
        assert!(
            matches!(&app_item.op, Op::RemovePath { destination: Destination::Trash, guard: Guard::Picked, root: Some(r), .. } if r.ends_with("Applications"))
        );
        assert_eq!(app_item.risk, Risk::Reversible);
        // Every leftover id ends with its Leftover path.
        let lo = b.extra["leftovers"].as_array().unwrap();
        for l in lo {
            let p = l["path"].as_str().unwrap();
            assert!(
                t.iter()
                    .chain(b.unticked.iter().map(|u| &u.item.id))
                    .any(|i| i.ends_with(&format!(":{p}"))),
                "{p}"
            );
        }
        assert!(lo.iter().any(|l| l["confidence"] == "low"));
        // User LaunchAgent and the preferences file: module items, ticked.
        let agent = b
            .ticked
            .iter()
            .find(|i| i.id.ends_with("com.example.updater.plist"))
            .unwrap();
        assert!(matches!(&agent.op, Op::Module { action, .. } if action == "mac.bootout_agent"));
        let prefs = b
            .ticked
            .iter()
            .find(|i| i.id.ends_with("Preferences/com.example.editor.plist"))
            .unwrap();
        assert!(
            matches!(&prefs.op, Op::Module { action, args, .. } if action == "mac.remove_prefs" && args["bundle_id"] == "com.example.editor")
        );
        // Daemon and /Library data: admin, unticked.
        let daemon = b
            .unticked
            .iter()
            .find(|u| u.item.id.ends_with("com.example.editor.helper.plist"))
            .unwrap();
        assert!(daemon.item.admin && daemon.why == "admin");
        assert!(
            matches!(&daemon.item.op, Op::Module { action, args, .. } if action == "mac.remove_daemon" && args["domain"] == "system")
        );
        // Name-only: unticked, low.
        assert!(b.unticked.iter().any(|u| u
            .item
            .id
            .ends_with("Application Support/Example Editor")
            && u.why == "low"));
        // Zap alternative.
        let zap = b
            .unticked
            .iter()
            .find(|u| u.item.id.ends_with(":zap"))
            .unwrap();
        assert_eq!(zap.item.group, "zap");
        assert!(
            matches!(&zap.item.op, Op::Command { program, args, .. } if program == "brew" && args.join(" ") == "uninstall --zap --cask example-editor")
        );
        assert!(zap
            .item
            .detail
            .as_deref()
            .unwrap()
            .contains("~/Library/Caches/com.example.editor"));
        assert_eq!(b.extra["cask"], "example-editor");
        assert!(b.extra["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(Value::is_string));
        assert_eq!(b.extra["running"], false);
        // Unique and stable ids.
        let mut all: Vec<String> = t.clone();
        all.extend(b.unticked.iter().map(|u| u.item.id.clone()));
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(n, all.len());
        assert_eq!(ids(&build(&app, &env(roots)).ticked), t);
    }

    #[test]
    fn an_open_app_ticks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let app = fake_app(dir.path());
        let mut e = env(roots);
        e.running = true;
        let b = build(&app, &e);
        assert!(b.ticked.is_empty());
        assert!(b
            .unticked
            .iter()
            .filter(|u| u.why != "alternative")
            .all(|u| u.why == "open"));
        assert_eq!(b.extra["running"], true);
    }

    #[test]
    fn a_removed_app_is_searched_by_bundle_id_or_name() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let by_id = App {
            id: "removed:com.example.editor".into(),
            name: "com.example.editor".into(),
            source: "removed".into(),
            ..Default::default()
        };
        let b = build(&by_id, &env(roots.clone()));
        assert!(b
            .ticked
            .iter()
            .any(|i| i.id.ends_with("Caches/com.example.editor")));
        assert!(b.ticked.iter().all(|i| i.group != "app"));
        let by_name = App {
            id: "removed:Example Editor".into(),
            name: "Example Editor".into(),
            source: "removed".into(),
            ..Default::default()
        };
        let b = build(&by_name, &env(roots));
        assert!(b.ticked.is_empty(), "name matches are never ticked");
        assert!(b.unticked.iter().any(|u| u
            .item
            .id
            .ends_with("Application Support/Example Editor")
            && u.why == "low"));
    }

    #[test]
    fn a_bare_pkg_plans_its_files_and_the_forget() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let lib = dir.path().join("opt/foo");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("tool"), b"x").unwrap();
        std::fs::write(lib.join("shared.dylib"), b"x").unwrap();
        let app = App {
            id: "pkg:com.foo.tool".into(),
            name: "com.foo.tool".into(),
            source: "pkg".into(),
            native_id: Some("com.foo.tool".into()),
            removable: true,
            ..Default::default()
        };
        let mut e = env(roots);
        e.receipts = vec![Receipt {
            id: "com.foo.tool".into(),
            paths: vec![lib.join("tool"), lib.join("shared.dylib"), lib.join("gone")],
        }];
        e.shared.insert(lib.join("shared.dylib"));
        let b = build(&app, &e);
        let t = ids(&b.ticked);
        assert_eq!(t.len(), 2, "{t:?}");
        assert!(b.ticked.iter().all(|i| i.admin));
        assert!(t[1].ends_with("pkg-forget:com.foo.tool"));
        assert!(b.extra["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("another package")));
    }

    #[test]
    fn a_formula_goes_through_brew() {
        let app = App {
            id: "brew:wget".into(),
            name: "wget".into(),
            source: "homebrew_formula".into(),
            native_id: Some("wget".into()),
            removable: true,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let b = build(
            &app,
            &env(Roots {
                home: dir.path().to_path_buf(),
                system: None,
            }),
        );
        assert_eq!(b.ticked.len(), 1);
        assert!(
            matches!(&b.ticked[0].op, Op::Command { args, .. } if args.join(" ") == "uninstall --formula wget")
        );
    }

    #[test]
    fn items_survive_plan_validation() {
        let dir = tempfile::tempdir().unwrap();
        let roots = fake_home(dir.path());
        let app = fake_app(dir.path());
        let b = build(&app, &env(roots));
        let ctx = super::super::super::super::store::Ctx::at(&dir.path().join("state"));
        let plan = super::super::super::super::plan::offer(
            &ctx,
            "uninstall",
            b.ticked.clone(),
            super::super::super::super::plan::Source::Ui,
        )
        .unwrap();
        assert_eq!(plan.items.len(), b.ticked.len());
    }
}
