//! What a Windows app leaves behind, with a confidence score.
//!
//! Ported design of Bulk Crap Uninstaller's junk finder
//! (Copyright (c) 2017 Marcin Szeniak, Apache License 2.0, see
//! `data/washmybelly/bcu/NOTICE`): the confidence weights
//! (`Junk/Confidence/ConfidenceRecords.cs`), the four levels
//! (`ConfidenceCollection.cs`), the folder blacklist and questionable names
//! (`UninstallToolsGlobalConfig.cs`) and the "folder still used by another
//! program" rule (`JunkCreatorBase.cs`). Rewritten in Rust and modified for
//! OmniGet: the scanners here read folders, registry key names, shortcuts,
//! services and scheduled tasks handed in by the caller, so they are tested
//! on every OS with temporary folders and fake key lists.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::parse::{lnk_mentions, name_match, norm_path, path_inside, NameMatch};

/// One piece of evidence and its weight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reason {
    pub delta: i32,
    pub text: &'static str,
}

/// What we know about one candidate.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// The app's own Uninstall registry entry.
    pub uninstall_key: bool,
    /// The app points here (install folder, uninstaller folder, a shortcut
    /// or service whose target is inside the install folder).
    pub explicit: bool,
    pub name: Option<NameMatch>,
    /// The parent folder or key is named after the publisher.
    pub publisher_parent: bool,
    /// The name is the company's, not the product's.
    pub is_publisher_name: bool,
    /// Levels below the base folder (1 = direct child).
    pub depth: u32,
    pub empty: bool,
    pub no_subdirs: bool,
    pub many_files: bool,
    pub questionable_name: bool,
    pub has_exe: bool,
    pub name_used_by_other: bool,
    pub publisher_used_by_other: bool,
    /// Another installed program lives in (or under) this folder.
    pub used_by_other_program: bool,
}

/// Sum the evidence, BCU weights.
pub fn score(f: &Facts) -> (i32, Vec<Reason>) {
    let mut r = Vec::new();
    let mut add = |on: bool, delta: i32, text: &'static str| {
        if on {
            r.push(Reason { delta, text });
        }
    };
    add(f.uninstall_key, 20, "it is the app's own uninstall entry");
    add(f.explicit, 4, "the app points here");
    add(
        f.name == Some(NameMatch::Perfect),
        2,
        "the name matches the app",
    );
    add(
        f.name == Some(NameMatch::None),
        -2,
        "the name only loosely matches the app",
    );
    add(
        f.publisher_parent,
        4,
        "it sits under the publisher's folder",
    );
    add(
        f.is_publisher_name,
        -2,
        "the name is the company's, not the app's",
    );
    add(
        f.depth > 1,
        -2 * (f.depth as i32 - 1),
        "it is deep inside the folder tree",
    );
    add(f.empty, 4, "the folder is empty");
    add(f.no_subdirs && !f.empty, 2, "the folder has no subfolders");
    add(f.many_files, -2, "the folder holds many files");
    add(f.questionable_name, -3, "the name is generic");
    add(f.has_exe, -4, "programs are still inside");
    add(
        f.name_used_by_other,
        -4,
        "another installed app has this name",
    );
    add(
        f.publisher_used_by_other,
        -4,
        "another app of this publisher is installed",
    );
    add(
        f.used_by_other_program,
        -7,
        "another installed program still uses this folder",
    );
    let total = r.iter().map(|x| x.delta).sum();
    (total, r)
}

/// Four levels (BCU: bad / questionable / good / very good).
pub fn level(score: i32) -> &'static str {
    if score >= 5 {
        "very_high"
    } else if score >= 2 {
        "high"
    } else if score >= 0 {
        "medium"
    } else {
        "low"
    }
}

/// The level in the three words of `Leftover.confidence`.
pub fn confidence_word(score: i32) -> &'static str {
    match level(score) {
        "very_high" | "high" => "high",
        "medium" => "medium",
        _ => "low",
    }
}

/// Ticked by default: only the two top levels.
pub fn ticked(score: i32) -> bool {
    score >= 2
}

/// One leftover found.
#[derive(Debug, Clone, Serialize)]
pub struct Found {
    /// `folder`, `file`, `registry`, `service`, `task`.
    pub kind: &'static str,
    /// Folder or file path, registry key, service key or task path.
    pub path: String,
    pub bytes: u64,
    pub score: i32,
    pub reasons: Vec<Reason>,
    /// Only an administrator can remove it.
    pub admin: bool,
    /// Service name (for `service`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// The base folder it was found under (the plan's `root`).
    #[serde(skip)]
    pub base: Option<PathBuf>,
}

impl Found {
    fn new(kind: &'static str, path: String, facts: &Facts, admin: bool) -> Found {
        let (score, reasons) = score(facts);
        Found {
            kind,
            path,
            bytes: 0,
            score,
            reasons,
            admin,
            service: None,
            base: None,
        }
    }
}

/// The app whose leftovers we look for.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub name: String,
    pub publisher: Option<String>,
    pub location: Option<String>,
    /// Other folders the app points to (its uninstaller's folder).
    pub extra_dirs: Vec<String>,
}

/// Another installed app (for "still used" checks).
#[derive(Debug, Clone, Default)]
pub struct Other {
    pub name: String,
    pub publisher: Option<String>,
    pub location: Option<String>,
}

/// Folder names that are never a leftover (BCU's list plus Windows own).
pub const DIR_BLACKLIST: &[&str] = &[
    "microsoft",
    "microsoft games",
    "temp",
    "tmp",
    "programs",
    "common",
    "common files",
    "clients",
    "downloads",
    "desktop",
    "documents",
    "internet explorer",
    "windows",
    "windows nt",
    "windows photo viewer",
    "windows mail",
    "windows defender",
    "windows defender advanced threat protection",
    "windows media player",
    "windows security",
    "windows sidebar",
    "windowspowershell",
    "uninstall information",
    "reference assemblies",
    "installshield installation information",
    "installer",
    "winsxs",
    "windowsapps",
    "directx",
    "directxredist",
    "packages",
    "package cache",
    "modifiablewindowsapps",
    "msbuild",
    "microsoft.net",
    "dotnet",
    "crashdumps",
    "d3dscache",
    "connecteddevicesplatform",
    "comms",
    "history",
    "virtualstore",
    "publishers",
    "application data",
    "start menu",
    "startup",
    "ssh",
    "nvidia corporation",
    "intel",
    "amd",
    "realtek",
    "ssl",
    "usoshared",
    "usoprivate",
    "regid.1991-06.com.microsoft",
    "package",
    "cache",
    "logs",
    "log",
    "google",
    "mozilla",
    "apple",
    "adobe",
];

/// BCU's questionable names: too generic to trust.
const QUESTIONABLE: &[&str] = &[
    "install",
    "settings",
    "config",
    "configuration",
    "users",
    "data",
    "bin",
    "app",
    "update",
    "updater",
];

/// Registry key names under `Software` that are never a leftover.
pub const KEY_BLACKLIST: &[&str] = &[
    "microsoft",
    "classes",
    "clients",
    "policies",
    "registeredapplications",
    "wow6432node",
    "odbc",
    "windows",
    "intel",
    "amd",
    "nvidia corporation",
    "realtek",
    "google",
    "mozilla",
    "apple inc.",
    "apple computer, inc.",
    "adobe",
    "partner",
    "defaultusereenvironment",
    "appdatalow",
    "system",
    "setup",
    "wow64",
    "khronos",
    "valve",
];

fn blacklisted(name: &str, list: &[&str]) -> bool {
    let n = name.trim().to_lowercase();
    n.is_empty() || n.starts_with('.') || n.starts_with('{') || list.contains(&n.as_str())
}

fn is_publisher(name: &str, t: &Target) -> bool {
    t.publisher
        .as_deref()
        .map(|p| name_match(name, p, None) == NameMatch::Perfect || first_word_match(name, p))
        .unwrap_or(false)
}

/// `Foo Software GmbH` publishes into `Foo`.
fn first_word_match(name: &str, publisher: &str) -> bool {
    let first = super::parse::normalize_name(publisher)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    first.len() >= 4 && super::parse::normalize_name(name) == first
}

fn name_used_by_other(name: &str, t: &Target, others: &[Other]) -> bool {
    others
        .iter()
        .filter(|o| super::parse::normalize_name(&o.name) != super::parse::normalize_name(&t.name))
        .any(|o| name_match(name, &o.name, o.publisher.as_deref()) == NameMatch::Perfect)
}

fn publisher_used_by_other(t: &Target, others: &[Other]) -> bool {
    let Some(p) = t.publisher.as_deref() else {
        return false;
    };
    let np = super::parse::normalize_name(p);
    others.iter().any(|o| {
        o.publisher
            .as_deref()
            .map(super::parse::normalize_name)
            .as_deref()
            == Some(np.as_str())
            && super::parse::normalize_name(&o.name) != super::parse::normalize_name(&t.name)
    })
}

fn used_by_other_program(path: &str, others: &[Other]) -> bool {
    others
        .iter()
        .filter_map(|o| o.location.as_deref())
        .filter(|l| !l.trim().is_empty())
        .any(|l| path_inside(l, path) || path_inside(path, l))
}

fn explicit(path: &str, t: &Target) -> bool {
    let p = norm_path(path);
    t.location
        .as_deref()
        .map(|l| norm_path(l) == p)
        .unwrap_or(false)
        || t.extra_dirs.iter().any(|d| norm_path(d) == p)
}

/// Files, executables and subfolders of a folder, stopping early.
fn folder_facts(dir: &Path, f: &mut Facts) {
    let mut files = 0usize;
    let mut subdirs = 0usize;
    for e in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if e.file_type().is_dir() {
            if e.depth() == 1 {
                subdirs += 1;
            }
            continue;
        }
        files += 1;
        let ext = e
            .path()
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "exe" {
            f.has_exe = true;
        }
        if files > 100 {
            f.many_files = true;
            break;
        }
    }
    f.empty = files == 0 && subdirs == 0;
    f.no_subdirs = subdirs == 0;
}

fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| crate::core::washmybelly::fsops::is_link(&m))
        .unwrap_or(true)
}

fn child_dirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .filter(|(_, p)| !is_link(p))
        .collect();
    v.sort();
    v
}

/// Folders in the given bases (Program Files, ProgramData, AppData...)
/// named after the app, its publisher's folder with the app inside, and the
/// install folder itself. `bool` = only an administrator can remove there.
pub fn scan_dirs(bases: &[(PathBuf, bool)], t: &Target, others: &[Other]) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let push = |path: &Path,
                base: &Path,
                depth: u32,
                publisher_parent: bool,
                admin: bool,
                out: &mut Vec<Found>| {
        let ps = path.to_string_lossy().to_string();
        if out.iter().any(|f| norm_path(&f.path) == norm_path(&ps)) {
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let m = name_match(&name, &t.name, t.publisher.as_deref());
        let mut f = Facts {
            explicit: explicit(&ps, t),
            name: Some(m),
            publisher_parent,
            is_publisher_name: m != NameMatch::Perfect && is_publisher(&name, t),
            depth,
            questionable_name: QUESTIONABLE.contains(&name.to_lowercase().as_str()),
            name_used_by_other: name_used_by_other(&name, t, others),
            used_by_other_program: used_by_other_program(&ps, others),
            ..Default::default()
        };
        folder_facts(path, &mut f);
        // BCU only keeps a candidate with at least one positive piece.
        if !f.explicit && m == NameMatch::None && !f.publisher_parent {
            return;
        }
        let mut found = Found::new("folder", ps, &f, admin);
        found.base = Some(base.to_path_buf());
        found.bytes = crate::core::washmybelly::fsops::measure(path).0;
        out.push(found);
    };
    for (base, admin) in bases {
        for (name, path) in child_dirs(base) {
            if blacklisted(&name, DIR_BLACKLIST) && !explicit(&path.to_string_lossy(), t) {
                continue;
            }
            let ps = path.to_string_lossy().to_string();
            let m = name_match(&name, &t.name, t.publisher.as_deref());
            if explicit(&ps, t) || m != NameMatch::None {
                push(&path, base, 1, false, *admin, &mut out);
                continue;
            }
            if is_publisher(&name, t) {
                for (child, cpath) in child_dirs(&path) {
                    let cps = cpath.to_string_lossy().to_string();
                    let m2 = name_match(&child, &t.name, t.publisher.as_deref());
                    if explicit(&cps, t) || m2 != NameMatch::None {
                        push(&cpath, base, 2, true, *admin, &mut out);
                    }
                }
            }
        }
    }
    // The install folder may live outside every base (`D:\Games\X`).
    if let Some(loc) = t.location.as_deref().filter(|l| !l.trim().is_empty()) {
        let p = PathBuf::from(loc.trim().trim_end_matches(['\\', '/']));
        if p.is_absolute()
            && p.is_dir()
            && !is_link(&p)
            && !out.iter().any(|f| norm_path(&f.path) == norm_path(loc))
        {
            let base = p
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| p.clone());
            let admin = bases
                .iter()
                .find(|(b, _)| path_inside(loc, &b.to_string_lossy()))
                .map(|(_, a)| *a)
                .unwrap_or(false);
            push(&p, &base, 1, false, admin, &mut out);
        }
    }
    out
}

/// Registry keys under `Software` named after the app, or after its
/// publisher with the app inside. `children(key)` lists subkey names.
pub fn scan_keys(
    children: &dyn Fn(&str) -> Vec<String>,
    bases: &[(&str, bool)],
    t: &Target,
    others: &[Other],
) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let pub_used = publisher_used_by_other(t, others);
    for (base, admin) in bases {
        for name in children(base) {
            if blacklisted(&name, KEY_BLACKLIST) {
                continue;
            }
            let key = format!(r"{base}\{name}");
            let m = name_match(&name, &t.name, t.publisher.as_deref());
            if m != NameMatch::None {
                let f = Facts {
                    name: Some(m),
                    depth: 1,
                    is_publisher_name: m != NameMatch::Perfect && is_publisher(&name, t),
                    name_used_by_other: name_used_by_other(&name, t, others),
                    ..Default::default()
                };
                out.push(Found::new("registry", key, &f, *admin));
                continue;
            }
            if !is_publisher(&name, t) {
                continue;
            }
            let kids = children(&key);
            for child in &kids {
                let m2 = name_match(child, &t.name, t.publisher.as_deref());
                if m2 == NameMatch::None {
                    continue;
                }
                let f = Facts {
                    name: Some(m2),
                    publisher_parent: true,
                    depth: 2,
                    name_used_by_other: name_used_by_other(child, t, others),
                    ..Default::default()
                };
                out.push(Found::new(
                    "registry",
                    format!(r"{key}\{child}"),
                    &f,
                    *admin,
                ));
            }
            // The whole publisher key, when the app is the only thing in it
            // and no other app of this publisher is installed.
            if !pub_used
                && !kids.is_empty()
                && kids
                    .iter()
                    .all(|c| name_match(c, &t.name, t.publisher.as_deref()) != NameMatch::None)
            {
                let f = Facts {
                    publisher_parent: false,
                    is_publisher_name: true,
                    depth: 1,
                    explicit: false,
                    name: Some(NameMatch::Partial),
                    ..Default::default()
                };
                let mut found = Found::new("registry", key.clone(), &f, *admin);
                found.reasons.push(Reason {
                    delta: 4,
                    text: "everything inside belongs to the app",
                });
                found.score += 4;
                out.push(found);
            }
        }
    }
    out
}

/// Start Menu shortcuts (and their folders) for the app.
pub fn scan_shortcuts(dirs: &[(PathBuf, bool)], t: &Target, others: &[Other]) -> Vec<Found> {
    let mut out = Vec::new();
    for (dir, admin) in dirs {
        for e in walkdir::WalkDir::new(dir)
            .min_depth(1)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            let p = e.path();
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if e.file_type().is_dir() {
                if blacklisted(&name, DIR_BLACKLIST) {
                    continue;
                }
                let m = name_match(&name, &t.name, t.publisher.as_deref());
                if m == NameMatch::Perfect || (m == NameMatch::Partial && is_publisher(&name, t)) {
                    let mut f = Facts {
                        name: Some(m),
                        depth: e.depth() as u32,
                        name_used_by_other: name_used_by_other(&name, t, others),
                        ..Default::default()
                    };
                    folder_facts(p, &mut f);
                    f.has_exe = false;
                    let mut found =
                        Found::new("folder", p.to_string_lossy().to_string(), &f, *admin);
                    found.base = Some(dir.clone());
                    found.bytes = crate::core::washmybelly::fsops::measure(p).0;
                    out.push(found);
                }
                continue;
            }
            if !p
                .extension()
                .map(|x| x.eq_ignore_ascii_case("lnk"))
                .unwrap_or(false)
            {
                continue;
            }
            // Inside a folder we already offer: the folder carries it.
            if out
                .iter()
                .any(|f: &Found| f.kind == "folder" && path_inside(&p.to_string_lossy(), &f.path))
            {
                continue;
            }
            let bytes = std::fs::read(p).unwrap_or_default();
            let points = t
                .location
                .as_deref()
                .map(|l| lnk_mentions(&bytes, l))
                .unwrap_or(false);
            let other = others
                .iter()
                .filter_map(|o| o.location.as_deref())
                .any(|l| lnk_mentions(&bytes, l));
            let m = name_match(&name, &t.name, t.publisher.as_deref());
            if !points && m != NameMatch::Perfect {
                continue;
            }
            let f = Facts {
                explicit: points,
                name: Some(m),
                used_by_other_program: other && !points,
                name_used_by_other: name_used_by_other(&name, t, others),
                ..Default::default()
            };
            let mut found = Found::new("file", p.to_string_lossy().to_string(), &f, *admin);
            found.base = Some(dir.clone());
            found.bytes = crate::core::washmybelly::fsops::measure(p).0;
            out.push(found);
        }
    }
    out
}

/// Services whose program lives in the app's folder. `services` holds
/// `(name, ImagePath)` pairs read from the registry.
pub fn scan_services(services: &[(String, String)], t: &Target) -> Vec<Found> {
    let Some(loc) = t.location.as_deref().filter(|l| l.trim().len() > 3) else {
        return Vec::new();
    };
    services
        .iter()
        .filter(|(_, image)| {
            let img = image.trim().trim_start_matches('"');
            path_inside(img, loc) || norm_path(img).starts_with(&format!("{}\\", norm_path(loc)))
        })
        .map(|(name, _)| {
            let m = name_match(name, &t.name, t.publisher.as_deref());
            let f = Facts {
                explicit: true,
                name: Some(if m == NameMatch::None {
                    NameMatch::Partial
                } else {
                    m
                }),
                ..Default::default()
            };
            let mut found = Found::new(
                "service",
                format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{name}"),
                &f,
                true,
            );
            found.service = Some(name.clone());
            found
        })
        .collect()
}

/// Scheduled tasks whose action runs something in the app's folder.
pub fn scan_tasks(tasks: &[(String, String)], t: &Target) -> Vec<Found> {
    let Some(loc) = t.location.as_deref().filter(|l| l.trim().len() > 3) else {
        return Vec::new();
    };
    let nloc = norm_path(loc);
    tasks
        .iter()
        .filter(|(path, run)| {
            !path.to_ascii_lowercase().starts_with(r"\microsoft\")
                && norm_path(run.trim_start_matches('"')).starts_with(&format!("{nloc}\\"))
        })
        .map(|(path, _)| {
            let leaf = super::parse::win_file_name(path);
            let m = name_match(leaf, &t.name, t.publisher.as_deref());
            let f = Facts {
                explicit: true,
                name: Some(if m == NameMatch::None {
                    NameMatch::Partial
                } else {
                    m
                }),
                ..Default::default()
            };
            Found::new("task", path.clone(), &f, true)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Target {
        Target {
            name: "Foo Tool 2.1".into(),
            publisher: Some("Foo Software".into()),
            location: None,
            extra_dirs: vec![],
        }
    }

    #[test]
    fn weights_and_levels() {
        let (s, r) = score(&Facts {
            uninstall_key: true,
            ..Default::default()
        });
        assert_eq!(s, 20);
        assert_eq!(r.len(), 1);
        assert_eq!(level(s), "very_high");
        let (s, _) = score(&Facts {
            explicit: true,
            name: Some(NameMatch::Perfect),
            has_exe: true,
            depth: 1,
            ..Default::default()
        });
        assert_eq!(s, 2);
        assert_eq!(level(s), "high");
        let (s, _) = score(&Facts {
            name: Some(NameMatch::Partial),
            depth: 1,
            ..Default::default()
        });
        assert_eq!(level(s), "medium");
        assert!(!ticked(s));
        let (s, _) = score(&Facts {
            name: Some(NameMatch::Perfect),
            used_by_other_program: true,
            depth: 1,
            ..Default::default()
        });
        assert_eq!(s, -5);
        assert_eq!(level(s), "low");
        assert_eq!(confidence_word(20), "high");
        assert_eq!(confidence_word(-1), "low");
        let (s, _) = score(&Facts {
            name: Some(NameMatch::Perfect),
            publisher_parent: true,
            depth: 2,
            empty: true,
            ..Default::default()
        });
        assert_eq!(s, 2 + 4 - 2 + 4);
    }

    #[test]
    fn folders_are_scored_and_blacklist_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("Program Files");
        let appdata = tmp.path().join("AppData");
        std::fs::create_dir_all(pf.join("Foo Tool")).unwrap();
        std::fs::write(pf.join("Foo Tool").join("foo.exe"), b"x").unwrap();
        std::fs::create_dir_all(pf.join("Microsoft")).unwrap();
        std::fs::create_dir_all(appdata.join("Foo Software").join("Foo Tool").join("cache"))
            .unwrap();
        std::fs::write(
            appdata
                .join("Foo Software")
                .join("Foo Tool")
                .join("cache")
                .join("a.bin"),
            b"123",
        )
        .unwrap();
        std::fs::create_dir_all(appdata.join("Unrelated")).unwrap();
        std::fs::create_dir_all(appdata.join("Foo Tool Data")).unwrap();
        std::fs::write(appdata.join("Foo Tool Data").join("x.json"), b"{}").unwrap();
        let mut target = t();
        target.location = Some(pf.join("Foo Tool").to_string_lossy().to_string());
        let found = scan_dirs(
            &[(pf.clone(), true), (appdata.clone(), false)],
            &target,
            &[],
        );
        let paths: Vec<String> = found.iter().map(|f| f.path.clone()).collect();
        assert!(!paths.iter().any(|p| p.ends_with("Microsoft")));
        assert!(!paths.iter().any(|p| p.ends_with("Unrelated")));
        let install = found
            .iter()
            .find(|f| f.path.ends_with("Foo Tool") && f.admin)
            .unwrap();
        assert!(install
            .reasons
            .iter()
            .any(|r| r.text == "the app points here"));
        assert!(
            install.reasons.iter().any(|r| r.delta == -4),
            "an exe is still inside"
        );
        let nested = found
            .iter()
            .find(|f| f.path.contains("Foo Software"))
            .unwrap();
        assert!(nested.reasons.iter().any(|r| r.text.contains("publisher")));
        assert!(!nested.admin);
        assert!(nested.bytes > 0);
        let data = found
            .iter()
            .find(|f| f.path.ends_with("Foo Tool Data"))
            .unwrap();
        assert!(data.score < nested.score);
    }

    #[test]
    fn a_folder_used_by_another_program_loses_points() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("pd");
        std::fs::create_dir_all(base.join("Foo Tool").join("Plugin")).unwrap();
        let others = vec![Other {
            name: "Foo Plugin".into(),
            publisher: None,
            location: Some(
                base.join("Foo Tool")
                    .join("Plugin")
                    .to_string_lossy()
                    .to_string(),
            ),
        }];
        let found = scan_dirs(&[(base.clone(), false)], &t(), &others);
        let f = found.iter().find(|f| f.path.ends_with("Foo Tool")).unwrap();
        assert!(f.reasons.iter().any(|r| r.delta == -7));
        assert!(!ticked(f.score));
    }

    #[test]
    fn install_folder_outside_the_bases_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let loc = tmp.path().join("Games").join("Whatever");
        std::fs::create_dir_all(&loc).unwrap();
        let mut target = t();
        target.location = Some(loc.to_string_lossy().to_string());
        let found = scan_dirs(&[], &target, &[]);
        assert_eq!(found.len(), 1);
        assert!(found[0]
            .reasons
            .iter()
            .any(|r| r.text == "the app points here"));
    }

    #[test]
    fn keys_by_name_and_publisher() {
        let children = |k: &str| -> Vec<String> {
            match k {
                r"HKCU\Software" => vec![
                    "Microsoft".into(),
                    "Foo Software".into(),
                    "Foo Tool".into(),
                    "Other".into(),
                ],
                r"HKCU\Software\Foo Software" => vec!["Foo Tool".into()],
                _ => vec![],
            }
        };
        let found = scan_keys(&children, &[(r"HKCU\Software", false)], &t(), &[]);
        let paths: Vec<&str> = found.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&r"HKCU\Software\Foo Tool"));
        assert!(paths.contains(&r"HKCU\Software\Foo Software\Foo Tool"));
        assert!(
            paths.contains(&r"HKCU\Software\Foo Software"),
            "publisher key with only the app inside"
        );
        assert!(!paths.iter().any(|p| p.contains("Microsoft")));
        // Another app of the publisher keeps the publisher key.
        let others = vec![Other {
            name: "Foo Viewer".into(),
            publisher: Some("Foo Software".into()),
            location: None,
        }];
        let found = scan_keys(&children, &[(r"HKCU\Software", false)], &t(), &others);
        assert!(!found
            .iter()
            .any(|f| f.path == r"HKCU\Software\Foo Software"));
    }

    #[test]
    fn shortcuts_by_target_and_name() {
        let tmp = tempfile::tempdir().unwrap();
        let sm = tmp.path().join("Programs");
        std::fs::create_dir_all(sm.join("Foo Tool")).unwrap();
        std::fs::write(sm.join("Foo Tool").join("Foo Tool.lnk"), b"x").unwrap();
        std::fs::write(sm.join("Launcher.lnk"), b"..C:\\Apps\\FooTool\\foo.exe..").unwrap();
        std::fs::write(sm.join("Else.lnk"), b"..C:\\Apps\\Else\\e.exe..").unwrap();
        let mut target = t();
        target.location = Some(r"C:\Apps\FooTool".into());
        let found = scan_shortcuts(&[(sm.clone(), false)], &target, &[]);
        assert!(found
            .iter()
            .any(|f| f.kind == "folder" && f.path.ends_with("Foo Tool")));
        assert!(
            !found.iter().any(|f| f.path.ends_with("Foo Tool.lnk")),
            "carried by its folder"
        );
        let l = found
            .iter()
            .find(|f| f.path.ends_with("Launcher.lnk"))
            .unwrap();
        assert!(l.reasons.iter().any(|r| r.text == "the app points here"));
        assert!(!found.iter().any(|f| f.path.ends_with("Else.lnk")));
    }

    #[test]
    fn services_and_tasks_inside_the_install_folder() {
        let mut target = t();
        target.location = Some(r"C:\Program Files\Foo Tool\".into());
        let services = vec![
            (
                "FooUpdate".to_string(),
                r#""C:\Program Files\Foo Tool\upd.exe" -service"#.to_string(),
            ),
            (
                "Spooler".to_string(),
                r"C:\Windows\System32\spoolsv.exe".to_string(),
            ),
        ];
        let s = scan_services(&services, &target);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].service.as_deref(), Some("FooUpdate"));
        assert!(s[0].admin && ticked(s[0].score));
        let tasks = vec![
            (
                r"\Foo\Update".to_string(),
                r#""C:\Program Files\Foo Tool\upd.exe" /c"#.to_string(),
            ),
            (
                r"\Microsoft\Windows\X".to_string(),
                r"C:\Program Files\Foo Tool\x.exe".to_string(),
            ),
        ];
        let tk = scan_tasks(&tasks, &target);
        assert_eq!(tk.len(), 1);
        assert_eq!(tk[0].path, r"\Foo\Update");
    }
}
