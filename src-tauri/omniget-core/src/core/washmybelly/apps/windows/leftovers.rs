//! Ported design of Bulk Crap Uninstaller's junk finder
//! (Copyright (c) 2017 Marcin Szeniak, Apache License 2.0, see

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::parse::{lnk_mentions, name_match, norm_path, path_inside, NameMatch};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reason {
    pub delta: i32,
    pub text: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub uninstall_key: bool,
    pub explicit: bool,
    pub name: Option<NameMatch>,
    pub publisher_parent: bool,
    pub is_publisher_name: bool,
    pub depth: u32,
    pub empty: bool,
    pub no_subdirs: bool,
    pub many_files: bool,
    pub questionable_name: bool,
    pub has_exe: bool,
    pub name_used_by_other: bool,
    pub publisher_used_by_other: bool,
    pub used_by_other_program: bool,
}

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

pub fn confidence_word(score: i32) -> &'static str {
    match level(score) {
        "very_high" | "high" => "high",
        "medium" => "medium",
        _ => "low",
    }
}

pub fn ticked(score: i32) -> bool {
    score >= 2
}

#[derive(Debug, Clone, Serialize)]
pub struct Found {
    pub kind: &'static str,
    pub path: String,
    pub bytes: u64,
    pub score: i32,
    pub reasons: Vec<Reason>,
    pub admin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
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

#[derive(Debug, Clone, Default)]
pub struct Target {
    pub name: String,
    pub publisher: Option<String>,
    pub location: Option<String>,
    pub extra_dirs: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Other {
    pub name: String,
    pub publisher: Option<String>,
    pub location: Option<String>,
}

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
