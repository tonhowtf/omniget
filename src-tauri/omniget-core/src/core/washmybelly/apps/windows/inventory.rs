//! Ported design of Bulk Crap Uninstaller (Copyright (c) 2017 Marcin
//! Szeniak, Apache License 2.0, see `data/washmybelly/bcu/NOTICE`): the

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::App;
use super::catalog;
use super::parse::{
    self, detect_kind, install_date, is_guid, norm_path, normalize_name, path_inside, AppxPkg,
    Kind, Probe, RegEntry, Root, SteamApp, WingetRow,
};

pub struct DiskProbe;

impl Probe for DiskProbe {
    fn exists(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn nullsoft(&self, path: &str) -> bool {
        use std::io::Read;
        let Ok(f) = std::fs::File::open(path) else {
            return false;
        };
        let mut buf = Vec::new();
        if f.take(4 << 20).read_to_end(&mut buf).is_err() {
            return false;
        }
        parse::has_nullsoft(&buf)
    }
}

pub struct ListProbe;

impl Probe for ListProbe {
    fn exists(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn nullsoft(&self, _path: &str) -> bool {
        false
    }
}

pub fn x<'a>(a: &'a App, key: &str) -> Option<&'a str> {
    a.extra
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn xb(a: &App, key: &str) -> bool {
    a.extra.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn clean_location(s: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let s = parse::expand_env(s.trim().trim_matches('"'), env);
    let s = s.trim().trim_end_matches(['\\', '/']).to_string();
    (s.len() > 3).then_some(s)
}

pub fn entry_to_app(
    e: &RegEntry,
    probe: &dyn Probe,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<App> {
    let name = e.get("DisplayName")?.to_string();
    let uninstall = e.get_prefixed("UninstallString").map(str::to_string);
    let quiet = e.get("QuietUninstallString").map(str::to_string);
    let kind = detect_kind(e, probe);
    let steam_appid = e
        .key_name
        .strip_prefix("Steam App ")
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string);
    let product_code = match kind {
        Kind::Msi | Kind::Wix => e.product_code(),
        _ => None,
    };
    let id = if kind == Kind::Msi && is_guid(&e.key_name) {
        format!("msi:{}", e.key_name.to_ascii_uppercase())
    } else {
        format!("reg:{}:{}", e.root.as_str(), e.key_name)
    };
    let path = e
        .get("InstallLocation")
        .or_else(|| e.get("Inno Setup: App Path"))
        .and_then(|l| clean_location(l, env));
    let bytes = e
        .get("EstimatedSize")
        .and_then(parse::dword)
        .filter(|kb| *kb > 0)
        .map(|kb| kb * 1024);
    let mut removable = uninstall.is_some() || quiet.is_some();
    let mut reason = None;
    if !removable {
        reason = Some("no uninstaller is registered for it".to_string());
    }
    if e.flag("NoRemove") {
        removable = false;
        reason = Some("the vendor marked it as not removable".into());
    }
    if let Some(r) = catalog::registry_never(&name, e.get("Publisher"), None) {
        removable = false;
        reason = Some(r.into());
    }
    Some(App {
        id,
        name,
        version: e.get("DisplayVersion").map(str::to_string),
        publisher: e.get("Publisher").map(str::to_string),
        source: if kind == Kind::Msi {
            "msi".into()
        } else {
            "registry".into()
        },
        path,
        bytes,
        native_id: product_code.clone().or_else(|| Some(e.key_name.clone())),
        installed: e.get("InstallDate").and_then(install_date),
        last_used: None,
        removable,
        reason,
        needs_admin: e.root.machine() && steam_appid.is_none(),
        extra: json!({
            "uninstall_string": uninstall,
            "quiet_uninstall_string": quiet,
            "installer_kind": kind.as_str(),
            "product_code": product_code,
            "winget_id": null,
            "appx": null,
            "system_component": e.flag("SystemComponent"),
            "is_update": e.is_update(),
            "reg_root": e.root.as_str(),
            "reg_key": e.key_name,
            "arch": if e.root.x86() { "x86" } else { "x64" },
            "steam_appid": steam_appid,
            "sources": ["registry"],
        }),
    })
}

#[derive(Debug, Clone, Default)]
pub struct MsiProduct {
    pub code: String,
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    pub location: Option<String>,
    pub machine: bool,
}

pub fn msi_to_app(p: &MsiProduct) -> App {
    App {
        id: format!("msi:{}", p.code.to_ascii_uppercase()),
        name: p.name.clone(),
        version: p.version.clone(),
        publisher: p.publisher.clone(),
        source: "msi".into(),
        path: p
            .location
            .clone()
            .filter(|l| l.trim().len() > 3)
            .map(|l| l.trim_end_matches('\\').to_string()),
        native_id: Some(p.code.to_ascii_uppercase()),
        removable: true,
        needs_admin: p.machine,
        extra: json!({
            "installer_kind": "msi",
            "product_code": p.code.to_ascii_uppercase(),
            "winget_id": null,
            "appx": null,
            "system_component": false,
            "is_update": false,
            "reg_root": if p.machine { "hklm" } else { "hkcu" },
            "reg_key": p.code.to_ascii_uppercase(),
            "sources": ["msi"],
        }),
        ..Default::default()
    }
}

pub fn steam_to_app(lib: &str, s: &SteamApp) -> App {
    let path = (!s.installdir.is_empty()).then(|| {
        PathBuf::from(lib)
            .join("steamapps")
            .join("common")
            .join(&s.installdir)
            .to_string_lossy()
            .to_string()
    });
    App {
        id: format!("steam:{}", s.appid),
        name: s.name.clone(),
        publisher: None,
        source: "steam".into(),
        path,
        bytes: s.size,
        native_id: Some(s.appid.clone()),
        last_used: s.last_updated,
        removable: true,
        needs_admin: false,
        extra: json!({ "installer_kind": "other", "steam_appid": s.appid, "winget_id": null, "appx": null, "system_component": false, "sources": ["steam"] }),
        ..Default::default()
    }
}

pub fn steam_apps(libraries: &[String]) -> Vec<App> {
    let mut out = Vec::new();
    for lib in libraries {
        let dir = Path::new(lib).join("steamapps");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                n.starts_with("appmanifest_") && n.ends_with(".acf")
            })
            .collect();
        files.sort();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            if let Some(app) = parse::steam_manifest(&text) {
                if app.appid != "228980" {
                    out.push(steam_to_app(lib, &app));
                }
            }
        }
    }
    out
}

pub fn scoop_apps(root: &Path, global: bool) -> Vec<App> {
    let Ok(rd) = std::fs::read_dir(root.join("apps")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let name = d
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.eq_ignore_ascii_case("scoop") || !parse::scoop_name_ok(&name) {
            continue;
        }
        let manifest: Value = std::fs::read_to_string(d.join("current").join("manifest.json"))
            .ok()
            .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
            .unwrap_or(Value::Null);
        out.push(App {
            id: format!("scoop:{name}"),
            name: name.clone(),
            version: manifest.get("version").and_then(Value::as_str).map(str::to_string),
            source: "scoop".into(),
            path: Some(d.to_string_lossy().to_string()),
            native_id: Some(name.clone()),
            removable: true,
            needs_admin: global,
            extra: json!({ "installer_kind": "other", "scoop_name": name, "scoop_global": global, "winget_id": null, "appx": null, "system_component": false, "sources": ["scoop"] }),
            ..Default::default()
        });
    }
    out
}

pub fn appx_to_app(p: &AppxPkg) -> App {
    let class = catalog::classify_appx(&p.name, &p.signature, p.non_removable);
    let removable = class.classification != "never";
    App {
        id: format!("appx:{}", p.name),
        name: class
            .friendly_name
            .clone()
            .unwrap_or_else(|| p.name.clone()),
        version: p.version.clone(),
        publisher: p.publisher.clone(),
        source: "appx".into(),
        path: p.install_location.clone(),
        native_id: Some(p.family.clone()),
        removable,
        reason: class.reason.map(str::to_string),
        needs_admin: false,
        extra: json!({
            "installer_kind": "other",
            "winget_id": null,
            "appx": {
                "name": p.name,
                "family": p.family,
                "full_name": p.full_name,
                "classification": class.classification,
                "classified_by": class.source,
                "store_id": class.store_id,
                "suggested": class.suggested,
                "description": class.description,
                "signature": p.signature,
            },
            "system_component": false,
            "sources": ["appx"],
        }),
        ..Default::default()
    }
}

fn same_text(a: Option<&str>, b: Option<&str>) -> Option<bool> {
    match (a, b) {
        (Some(a), Some(b)) if !a.trim().is_empty() && !b.trim().is_empty() => {
            Some(normalize_name(a) == normalize_name(b))
        }
        _ => None,
    }
}

pub fn related(a: &App, b: &App) -> i32 {
    let eq = |k: &str| match (x(a, k), x(b, k)) {
        (Some(p), Some(q)) => p.eq_ignore_ascii_case(q),
        _ => false,
    };
    if eq("product_code") || eq("steam_appid") {
        return 100;
    }
    if let (Some(p), Some(q)) = (a.path.as_deref(), b.path.as_deref()) {
        if norm_path(p) == norm_path(q) {
            return 100;
        }
    }
    if eq("uninstall_string") {
        return 100;
    }
    let mut s = 0;
    s += match same_text(Some(&a.name), Some(&b.name)) {
        Some(true) => 8,
        Some(false) => -5,
        None => 0,
    };
    s += match same_text(a.publisher.as_deref(), b.publisher.as_deref()) {
        Some(true) => 5,
        Some(false) => -5,
        None => 0,
    };
    s += match (a.version.as_deref(), b.version.as_deref()) {
        (Some(p), Some(q)) if p == q => 5,
        (Some(_), Some(_)) => -2,
        _ => 0,
    };
    s += match (x(a, "arch"), x(b, "arch")) {
        (Some(p), Some(q)) if p == q => 3,
        (Some(_), Some(_)) => -5,
        _ => 0,
    };
    if let (Some(p), Some(q)) = (a.path.as_deref(), b.path.as_deref()) {
        if path_inside(p, q) || path_inside(q, p) {
            s += 5;
        } else {
            s -= 3;
        }
    }
    s
}

pub const MERGE_AT: i32 = 13;

fn fill(dst: &mut App, src: &App) {
    dst.version = dst.version.take().or_else(|| src.version.clone());
    dst.publisher = dst.publisher.take().or_else(|| src.publisher.clone());
    dst.path = dst.path.take().or_else(|| src.path.clone());
    dst.bytes = dst.bytes.or(src.bytes);
    dst.installed = dst.installed.or(src.installed);
    dst.last_used = dst.last_used.or(src.last_used);
    if let (Some(d), Some(s)) = (dst.extra.as_object_mut(), src.extra.as_object()) {
        for (k, v) in s {
            if k == "sources" {
                continue;
            }
            let empty = d
                .get(k)
                .map(|x| x.is_null() || x == &json!("") || x == &json!(false))
                .unwrap_or(true);
            if empty && !v.is_null() {
                d.insert(k.clone(), v.clone());
            }
        }
        let mut sources: Vec<Value> = d
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for v in s
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if !sources.contains(&v) {
                sources.push(v);
            }
        }
        d.insert("sources".into(), Value::Array(sources));
    }
}

pub fn merge_into(list: &mut Vec<App>, new: App) {
    let best = list
        .iter()
        .enumerate()
        .map(|(i, a)| (related(a, &new), i))
        .filter(|(s, _)| *s >= MERGE_AT)
        .max_by_key(|(s, _)| *s);
    match best {
        Some((_, i)) => fill(&mut list[i], &new),
        None => list.push(new),
    }
}

pub fn attach_winget(list: &mut Vec<App>, rows: &[WingetRow]) {
    for row in rows {
        if let Some((root, key)) = row.arp() {
            if let Some(a) = list.iter_mut().find(|a| {
                x(a, "reg_root") == Some(root.as_str())
                    && x(a, "reg_key").is_some_and(|k| k.eq_ignore_ascii_case(&key))
            }) {
                if let Some(o) = a.extra.as_object_mut() {
                    o.insert("winget_arp".into(), json!(row.id));
                }
            }
            continue;
        }
        let Some(pid) = row.package_id() else {
            continue;
        };
        let rn = normalize_name(row.name.trim_end_matches('\u{2026}'));
        if rn.is_empty() {
            continue;
        }
        let candidates: Vec<usize> = list
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                matches!(a.source.as_str(), "registry" | "msi") && x(a, "winget_id").is_none()
            })
            .filter(|(_, a)| {
                let an = normalize_name(&a.name);
                an == rn || (row.truncated && an.starts_with(&rn))
            })
            .map(|(i, _)| i)
            .collect();
        let pick = match candidates.len() {
            0 => None,
            1 => Some(candidates[0]),
            _ => candidates
                .iter()
                .copied()
                .find(|i| list[*i].version.as_deref() == Some(row.version.as_str()))
                .or(Some(candidates[0])),
        };
        match pick {
            Some(i) => {
                let never =
                    catalog::registry_never(&list[i].name, list[i].publisher.as_deref(), Some(pid));
                let a = &mut list[i];
                if let Some(o) = a.extra.as_object_mut() {
                    o.insert("winget_id".into(), json!(pid));
                    o.insert("winget_source".into(), json!(row.source));
                    if let Some(av) = &row.available {
                        o.insert("winget_available".into(), json!(av));
                    }
                }
                if let Some(r) = never {
                    a.removable = false;
                    a.reason = Some(r.into());
                }
            }
            None if row.source.as_deref() == Some("winget") => {
                let never = catalog::registry_never(&row.name, None, Some(pid));
                list.push(App {
                    id: format!("winget:{pid}"),
                    name: row.name.clone(),
                    version: Some(row.version.clone()).filter(|v| !v.is_empty() && v != "Unknown"),
                    source: "winget".into(),
                    native_id: Some(pid.to_string()),
                    removable: never.is_none(),
                    reason: never.map(str::to_string),
                    needs_admin: false,
                    extra: json!({ "installer_kind": "other", "winget_id": pid, "winget_source": row.source, "winget_available": row.available, "appx": null, "system_component": false, "sources": ["winget"] }),
                    ..Default::default()
                });
            }
            None => {}
        }
    }
}

pub fn finalize(list: Vec<App>) -> Vec<App> {
    let mut out: Vec<App> = list
        .into_iter()
        .filter(|a| !xb(a, "system_component") && !xb(a, "is_update"))
        .collect();
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    out
}

pub fn orphan_dirs(bases: &[PathBuf], apps: &[App]) -> Vec<App> {
    let mut out = Vec::new();
    for base in bases {
        let Ok(rd) = std::fs::read_dir(base) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for d in dirs {
            let name = d
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let lname = name.to_lowercase();
            if super::leftovers::DIR_BLACKLIST.contains(&lname.as_str())
                || lname.starts_with("windows")
                || lname.starts_with("microsoft")
            {
                continue;
            }
            let ds = d.to_string_lossy().to_string();
            let claimed = apps.iter().any(|a| {
                a.path
                    .as_deref()
                    .is_some_and(|p| path_inside(p, &ds) || path_inside(&ds, p))
                    || x(a, "uninstall_string").is_some_and(|u| {
                        norm_path(u.trim_start_matches('"'))
                            .starts_with(&format!("{}\\", norm_path(&ds)))
                    })
                    || parse::name_match(&name, &a.name, a.publisher.as_deref())
                        != parse::NameMatch::None
                    || a.publisher.as_deref().is_some_and(|p| {
                        parse::name_match(&name, p, None) == parse::NameMatch::Perfect
                    })
            });
            if claimed {
                continue;
            }
            let exes = walkdir::WalkDir::new(&d)
                .max_depth(2)
                .follow_links(false)
                .into_iter()
                .flatten()
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path()
                            .extension()
                            .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
                })
                .take(41)
                .count();
            if exes == 0 || exes > 40 {
                continue;
            }
            out.push(App {
                id: format!("dir:{ds}"),
                name: name.clone(),
                source: "orphan_dir".into(),
                path: Some(ds.clone()),
                removable: true,
                reason: None,
                needs_admin: true,
                extra: json!({ "installer_kind": "other", "confidence": "low", "orphan": true, "winget_id": null, "appx": null, "system_component": false, "sources": ["orphan_dir"] }),
                ..Default::default()
            });
        }
    }
    out
}

pub fn reg_location(a: &App) -> Option<(Root, String)> {
    let root = x(a, "reg_root").and_then(Root::parse)?;
    let key = x(a, "reg_key")?.to_string();
    Some((root, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_classifies_an_uninstaller_without_reading_it() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("uninstall.exe");
        std::fs::write(&exe, b"MZ Nullsoft Install System").unwrap();
        let e = RegEntry {
            root: Root::Hkcu,
            key_name: "Demo".into(),
            values: [
                ("DisplayName".to_string(), "Demo".to_string()),
                (
                    "UninstallString".to_string(),
                    format!("\"{}\"", exe.display()),
                ),
            ]
            .into_iter()
            .collect(),
        };
        let env = |_: &str| -> Option<String> { None };
        let listed = entry_to_app(&e, &ListProbe, &env).unwrap();
        let deep = entry_to_app(&e, &DiskProbe, &env).unwrap();
        assert_eq!(x(&listed, "installer_kind"), Some("other"));
        assert_eq!(x(&deep, "installer_kind"), Some("nsis"));
        assert_eq!(listed.id, deep.id);
        assert!(listed.removable);
    }
}
