//! The macOS inventory (read-only) and the system facts a leftover search
//! needs. Only compiled on macOS; the parsing it relies on lives in
//! `parse.rs` and `plist.rs`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::super::super::fsops;
use super::super::App;
use super::items::{Env, Receipt};
use super::leftovers::{Other, Roots};
use super::parse::{self, Cask, PkgInfo};
use super::plist;

const CACHE_TTL: Duration = Duration::from_secs(30);
const WORKERS: usize = 8;

#[derive(Clone, Default)]
struct Snapshot {
    apps: Vec<App>,
    /// Third-party receipts: id → absolute paths (collapsed).
    receipts: HashMap<String, Vec<PathBuf>>,
}

static CACHE: Mutex<Option<(Instant, Snapshot)>> = Mutex::new(None);

fn snapshot(force: bool) -> Result<Snapshot, String> {
    if !force {
        if let Ok(g) = CACHE.lock() {
            if let Some((at, s)) = g.as_ref() {
                if at.elapsed() < CACHE_TTL {
                    return Ok(s.clone());
                }
            }
        }
    }
    let s = scan()?;
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((Instant::now(), s.clone()));
    }
    Ok(s)
}

pub fn list_cached(force: bool) -> Result<Vec<App>, String> {
    Ok(snapshot(force)?.apps)
}

fn progress(done: u64, total: u64, msg: impl Into<String>) {
    super::super::super::progress("apps_list", "scan", done, total, msg);
}

/// Run `f` over `items` on a few threads, keeping the order.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..WORKERS.min(items.len().max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                if let Ok(mut o) = out.lock() {
                    o[i] = Some(r);
                }
            });
        }
    });
    out.into_inner()
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect()
}

fn run(prog: &str, args: &[&str]) -> Option<std::process::Output> {
    crate::core::process::std_command(prog)
        .args(args)
        .output()
        .ok()
}

fn run_ok(prog: &str, args: &[&str]) -> Option<String> {
    let o = run(prog, args)?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

// ── App bundles ──────────────────────────────────────────────────────

struct Bundle {
    path: PathBuf,
    system: bool,
    link: bool,
}

fn bundles(home: Option<&Path>) -> Vec<Bundle> {
    let mut roots = vec![(PathBuf::from("/Applications"), false)];
    if let Some(h) = home {
        roots.push((h.join("Applications"), false));
    }
    roots.push((PathBuf::from("/System/Applications"), true));
    let mut out = Vec::new();
    for (root, system) in roots {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            let link = fsops::is_link(&m);
            if p.extension().is_some_and(|e| e == "app") {
                out.push(Bundle {
                    path: p,
                    system,
                    link,
                });
            } else if m.is_dir() && !link {
                // One folder deeper: `Utilities`, `Setapp`, vendor folders.
                let Ok(rd) = std::fs::read_dir(&p) else {
                    continue;
                };
                let mut inner: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|q| q.extension().is_some_and(|e| e == "app"))
                    .collect();
                inner.sort();
                for q in inner {
                    let link = std::fs::symlink_metadata(&q)
                        .map(|m| fsops::is_link(&m))
                        .unwrap_or(false);
                    out.push(Bundle {
                        path: q,
                        system,
                        link,
                    });
                }
            }
        }
    }
    out
}

/// Whether this user can move the bundle without a password: write access
/// to the bundle (a folder's `..` changes) and to its parent.
fn user_can_move(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let ok = |q: &Path| {
        std::ffi::CString::new(q.as_os_str().as_bytes())
            .map(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
            .unwrap_or(false)
    };
    ok(p) && p.parent().is_some_and(ok)
}

struct BundleFacts {
    info: parse::BundleInfo,
    bytes: u64,
    sig: parse::Signature,
    app_store: bool,
    can_move: bool,
}

fn facts(b: &Bundle) -> BundleFacts {
    let info = plist::read(&b.path.join("Contents/Info.plist"))
        .map(|v| parse::bundle_info(&v))
        .unwrap_or_default();
    if b.link {
        return BundleFacts {
            info,
            bytes: 0,
            sig: Default::default(),
            app_store: false,
            can_move: false,
        };
    }
    let bytes = fsops::measure(&b.path).0;
    let sig = if b.system {
        parse::Signature::default()
    } else {
        let path = b.path.to_string_lossy().to_string();
        run("/usr/bin/codesign", &["-dv", "--verbose=2", "--", &path])
            .map(|o| parse::codesign(&String::from_utf8_lossy(&o.stderr)))
            .unwrap_or_default()
    };
    BundleFacts {
        app_store: b.path.join("Contents/_MASReceipt").exists() || sig.app_store,
        can_move: !b.system && user_can_move(&b.path),
        info,
        bytes,
        sig,
    }
}

/// `kMDItemLastUsedDate` and `kMDItemDateAdded`, one attribute per `mdls`
/// call over a batch of paths.
fn dates(paths: &[&Path], attr: &str) -> Vec<Option<i64>> {
    let mut out = Vec::with_capacity(paths.len());
    for chunk in paths.chunks(64) {
        let mut args: Vec<String> = vec!["-raw".into(), "-name".into(), attr.into(), "--".into()];
        args.extend(chunk.iter().map(|p| p.to_string_lossy().into_owned()));
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let got =
            run("/usr/bin/mdls", &argv).and_then(|o| parse::mdls_dates(&o.stdout, chunk.len()));
        out.extend(got.unwrap_or_else(|| vec![None; chunk.len()]));
    }
    out
}

// ── Homebrew ─────────────────────────────────────────────────────────

fn brew_bin() -> Option<PathBuf> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

fn brew(bin: &Path, args: &[&str]) -> Option<String> {
    let o = crate::core::process::std_command(bin)
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn casks(bin: &Path) -> Vec<Cask> {
    if let Some(json) = brew(bin, &["info", "--cask", "--json=v2", "--installed"]) {
        if let Ok(c) = parse::brew_casks(&json) {
            return c;
        }
    }
    // Fallback: tokens and versions only.
    brew(bin, &["list", "--cask", "--versions"])
        .map(|t| {
            parse::brew_versions(&t)
                .into_iter()
                .map(|(token, v)| Cask {
                    full_token: token.clone(),
                    token,
                    installed: v,
                    ..Default::default()
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── Receipts ─────────────────────────────────────────────────────────

fn receipt(id: &str) -> Option<(PkgInfo, Vec<PathBuf>)> {
    let info = run_ok("/usr/sbin/pkgutil", &["--pkg-info-plist", id])
        .and_then(|t| plist::parse_xml(&t).ok())
        .and_then(|v| parse::pkg_info(&v))?;
    let files = run_ok("/usr/sbin/pkgutil", &["--files", id]).unwrap_or_default();
    let dirs: HashSet<String> = run_ok("/usr/sbin/pkgutil", &["--only-dirs", "--files", id])
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().trim_matches('/').to_string())
        .collect();
    let prefix = info.prefix();
    let paths = parse::collapse_receipt(&files, &|p| dirs.contains(p))
        .into_iter()
        .map(|r| prefix.join(r))
        .collect();
    Some((info, paths))
}

// ── The scan ─────────────────────────────────────────────────────────

fn scan() -> Result<Snapshot, String> {
    let home = dirs::home_dir();
    let found = bundles(home.as_deref());
    let total = found.len() as u64 + 3;
    progress(0, total, "apps");
    let facts = par_map(&found, facts);
    progress(found.len() as u64, total, "dates");
    let live: Vec<&Path> = found
        .iter()
        .filter(|b| !b.link)
        .map(|b| b.path.as_path())
        .collect();
    let used: HashMap<&Path, Option<i64>> = live
        .iter()
        .copied()
        .zip(dates(&live, "kMDItemLastUsedDate"))
        .collect();
    let added: HashMap<&Path, Option<i64>> = live
        .iter()
        .copied()
        .zip(dates(&live, "kMDItemDateAdded"))
        .collect();

    let mut apps: Vec<App> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for (b, f) in found.iter().zip(facts) {
        let path_s = b.path.to_string_lossy().to_string();
        let stem = b
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bid = f.info.bundle_id.clone();
        let base = match &bid {
            Some(id) => format!("app:{id}"),
            None => format!("app:{path_s}"),
        };
        let id = if seen_ids.insert(base.clone()) {
            base
        } else {
            format!("{base}@{path_s}")
        };
        seen_ids.insert(id.clone());
        let apple = bid.as_deref().is_some_and(parse::is_apple);
        let (removable, reason) = if b.system {
            (false, Some("washmybelly.apps.mac.reason.system"))
        } else if b.link {
            (false, Some("washmybelly.apps.mac.reason.link"))
        } else if apple && !f.app_store {
            (false, Some("washmybelly.apps.mac.reason.system"))
        } else {
            (true, None)
        };
        let origin = if b.system || (apple && !f.app_store) {
            "system"
        } else if f.app_store {
            "app_store"
        } else {
            "direct"
        };
        apps.push(App {
            id,
            name: f.info.name.clone().unwrap_or(stem),
            version: f.info.version.clone(),
            publisher: if apple {
                Some("Apple".into())
            } else {
                f.sig.developer.clone()
            },
            source: "app_bundle".into(),
            path: Some(path_s),
            bytes: (!b.link).then_some(f.bytes),
            native_id: bid,
            installed: added.get(b.path.as_path()).copied().flatten(),
            last_used: used.get(b.path.as_path()).copied().flatten(),
            removable,
            reason: reason.map(String::from),
            needs_admin: removable && !f.can_move,
            extra: json!({
                "team_id": f.sig.team_id,
                "developer": f.sig.developer,
                "app_store": f.app_store,
                "apple": apple,
                "executable": f.info.executable,
                "origin": origin,
                "cask": null,
                "receipts": [],
            }),
        });
    }

    // Homebrew.
    progress(found.len() as u64 + 1, total, "Homebrew");
    if let Some(bin) = brew_bin() {
        let prefix = bin
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        for c in casks(&bin) {
            let owner = apps.iter_mut().find(|a| {
                a.source == "app_bundle"
                    && c.apps.iter().any(|t| {
                        let p = a.path.as_deref().unwrap_or("");
                        p == t
                            || (!t.starts_with('/')
                                && Path::new(p).file_name().is_some_and(|n| {
                                    n.to_string_lossy() == parse::artifact_file_name(t)
                                }))
                    })
            });
            let cask_json = json!({ "token": c.token, "full_token": c.full_token, "installed": c.installed, "zap": c.zap, "launchctl": c.launchctl });
            match owner {
                Some(a) => {
                    a.extra["cask"] = cask_json;
                    a.extra["origin"] = json!("homebrew");
                }
                None => {
                    let room = prefix.join("Caskroom").join(&c.token);
                    apps.push(App {
                        id: format!("cask:{}", c.token),
                        name: c.name.clone().unwrap_or_else(|| c.token.clone()),
                        version: c.installed.clone(),
                        source: "homebrew_cask".into(),
                        bytes: Some(fsops::measure(&room).0),
                        path: Some(room.to_string_lossy().into_owned()),
                        native_id: Some(c.token.clone()),
                        installed: c.installed_time,
                        removable: true,
                        extra: json!({ "cask": cask_json, "origin": "homebrew" }),
                        ..Default::default()
                    });
                }
            }
        }
        let formulae = brew(&bin, &["list", "--formula", "--versions"])
            .map(|t| parse::brew_versions(&t))
            .unwrap_or_default();
        let cellar = prefix.join("Cellar");
        let sizes = par_map(&formulae, |(name, _)| fsops::measure(&cellar.join(name)).0);
        for ((name, version), bytes) in formulae.into_iter().zip(sizes) {
            apps.push(App {
                id: format!("brew:{name}"),
                name: name.clone(),
                version,
                source: "homebrew_formula".into(),
                path: Some(cellar.join(&name).to_string_lossy().into_owned()),
                bytes: Some(bytes),
                native_id: Some(name),
                removable: true,
                extra: json!({ "origin": "homebrew" }),
                ..Default::default()
            });
        }
    }

    // Installer receipts.
    progress(found.len() as u64 + 2, total, "receipts");
    let ids = run_ok("/usr/sbin/pkgutil", &["--pkgs"])
        .map(|t| parse::third_party_pkgs(&t))
        .unwrap_or_default();
    let got = par_map(&ids, |id| receipt(id));
    let mut receipts = HashMap::new();
    for (info, paths) in got.into_iter().flatten() {
        let bundle = paths
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .cloned();
        let owner = bundle.as_ref().and_then(|b| {
            apps.iter_mut().find(|a| {
                a.source == "app_bundle" && a.path.as_deref() == Some(&*b.to_string_lossy())
            })
        });
        match owner {
            Some(a) => {
                if let Some(list) = a.extra["receipts"].as_array_mut() {
                    list.push(json!(info.id));
                }
                if a.extra["origin"] == "direct" {
                    a.extra["origin"] = json!("pkg");
                }
            }
            None => {
                let live: Vec<&PathBuf> = paths
                    .iter()
                    .filter(|p| std::fs::symlink_metadata(p).is_ok())
                    .collect();
                if !live.is_empty() {
                    let mut seen = HashSet::new();
                    let bytes = live
                        .iter()
                        .take(2000)
                        .map(|p| fsops::measure_with(p, &mut seen).0)
                        .sum();
                    apps.push(App {
                        id: format!("pkg:{}", info.id),
                        name: bundle.as_ref().and_then(|b| b.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| info.id.clone()),
                        version: info.version.clone(),
                        source: "pkg".into(),
                        path: None,
                        bytes: Some(bytes),
                        native_id: Some(info.id.clone()),
                        installed: info.installed,
                        removable: true,
                        needs_admin: true,
                        extra: json!({ "origin": "pkg", "files": live.len(), "prefix": info.prefix() }),
                        ..Default::default()
                    });
                }
            }
        }
        receipts.insert(info.id.clone(), paths);
    }
    progress(total, total, "done");
    Ok(Snapshot { apps, receipts })
}

// ── Facts for one uninstall ──────────────────────────────────────────

fn entitlement_groups(app: &Path) -> Vec<String> {
    let path = app.to_string_lossy().to_string();
    let Some(o) = run(
        "/usr/bin/codesign",
        &["-d", "--entitlements", "-", "--xml", "--", &path],
    ) else {
        return Vec::new();
    };
    // The XML lands on stdout or stderr depending on the macOS version.
    for stream in [&o.stdout, &o.stderr] {
        let text = String::from_utf8_lossy(stream);
        if let Some(i) = text.find("<?xml") {
            if let Ok(v) = plist::parse_xml(&text[i..]) {
                return parse::app_groups(&v);
            }
        }
    }
    Vec::new()
}

pub fn env_for(app: &App) -> Result<Env, String> {
    let home = dirs::home_dir().ok_or("no home folder")?;
    let snap = snapshot(false)?;
    let others: Vec<Other> = snap
        .apps
        .iter()
        .filter(|o| o.source == "app_bundle" && o.id != app.id && o.path != app.path)
        .map(|o| Other {
            name: o.name.clone(),
            path: o.path.clone().unwrap_or_default(),
            bundle_id: o.native_id.clone(),
            team_id: o
                .extra
                .get("team_id")
                .and_then(Value::as_str)
                .map(String::from),
        })
        .collect();
    let app_path = app
        .path
        .as_deref()
        .map(Path::new)
        .filter(|p| p.extension().is_some_and(|e| e == "app"));
    let running = app_path
        .is_some_and(|p| super::running(p, app.extra.get("executable").and_then(Value::as_str)));
    let app_groups = app_path.map(entitlement_groups).unwrap_or_default();
    let mut ids: Vec<String> = app
        .extra
        .get("receipts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if app.source == "pkg" {
        ids.extend(app.native_id.clone());
    }
    let mut receipts = Vec::new();
    for id in &ids {
        let paths = match snap.receipts.get(id) {
            Some(p) => p.clone(),
            None => receipt(id).map(|(_, p)| p).unwrap_or_default(),
        };
        receipts.push(Receipt {
            id: id.clone(),
            paths,
        });
    }
    let shared: HashSet<PathBuf> = snap
        .receipts
        .iter()
        .filter(|(id, _)| !ids.contains(id))
        .flat_map(|(_, p)| p.iter().cloned())
        .collect();
    Ok(Env {
        roots: Roots {
            home,
            system: Some(PathBuf::from("/Library")),
        },
        others,
        running,
        app_groups,
        receipts,
        shared,
        uid: unsafe { libc::getuid() },
    })
}
