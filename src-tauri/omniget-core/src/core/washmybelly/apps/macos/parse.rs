use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BundleInfo {
    pub bundle_id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub icon_file: Option<String>,
}

pub fn bundle_info(v: &Value) -> BundleInfo {
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    BundleInfo {
        bundle_id: s("CFBundleIdentifier"),
        name: s("CFBundleDisplayName").or_else(|| s("CFBundleName")),
        version: s("CFBundleShortVersionString").or_else(|| s("CFBundleVersion")),
        executable: s("CFBundleExecutable"),
        icon_file: s("CFBundleIconFile"),
    }
}

pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn is_uuid(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

pub fn valid_bundle_id(id: &str) -> bool {
    id.len() >= 5
        && id.len() <= 255
        && id.contains('.')
        && !id.starts_with('.')
        && !id.ends_with('.')
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub fn valid_label(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub fn valid_brew_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 128
        && t.as_bytes()[0].is_ascii_alphanumeric()
        && t.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.' | '@' | '+')
        })
}

pub fn is_apple(id: &str) -> bool {
    id.to_ascii_lowercase().starts_with("com.apple.")
}

pub fn lineage(name: &str, id: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let i = id.to_ascii_lowercase();
    n == i || (n.len() > i.len() && n.starts_with(&i) && n.as_bytes()[i.len()] == b'.')
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub identifier: Option<String>,
    pub team_id: Option<String>,
    pub developer: Option<String>,
    pub app_store: bool,
}

pub fn codesign(stderr: &str) -> Signature {
    let mut sig = Signature::default();
    for line in stderr.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "Identifier" => sig.identifier = Some(v.to_string()),
            "TeamIdentifier" if v != "not set" && !v.is_empty() => {
                sig.team_id = Some(v.to_string())
            }
            "Authority" if sig.developer.is_none() => {
                if let Some(rest) = v
                    .strip_prefix("Developer ID Application: ")
                    .or_else(|| v.strip_prefix("Apple Development: "))
                {
                    let name = match rest.rfind(" (") {
                        Some(i) if rest.ends_with(')') => &rest[..i],
                        _ => rest,
                    };
                    sig.developer = Some(name.to_string());
                } else if v.starts_with("Apple Mac OS Application Signing") {
                    sig.app_store = true;
                }
            }
            _ => {}
        }
    }
    sig
}

pub fn app_groups(entitlements: &Value) -> Vec<String> {
    entitlements
        .get("com.apple.security.application-groups")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn mdls_dates(out: &[u8], expected: usize) -> Option<Vec<Option<i64>>> {
    let text = String::from_utf8_lossy(out);
    let parts: Vec<&str> = text.split('\0').collect();
    let parts: Vec<&str> = if parts.len() == expected + 1 && parts.last() == Some(&"") {
        parts[..expected].to_vec()
    } else {
        parts
    };
    if parts.len() != expected {
        return None;
    }
    Some(parts.iter().map(|s| mdls_date(s)).collect())
}

pub fn mdls_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s == "(null)" {
        return None;
    }
    chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z")
        .ok()
        .map(|d| d.timestamp())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cask {
    pub token: String,
    pub full_token: String,
    pub name: Option<String>,
    pub installed: Option<String>,
    pub installed_time: Option<i64>,
    pub apps: Vec<String>,
    pub zap: Vec<String>,
    pub launchctl: Vec<String>,
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().flat_map(strings).collect(),
        _ => Vec::new(),
    }
}

pub fn brew_casks(json: &str) -> Result<Vec<Cask>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("brew info: {e}"))?;
    let mut out = Vec::new();
    for c in v
        .get("casks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(token) = c.get("token").and_then(Value::as_str) else {
            continue;
        };
        let mut cask = Cask {
            token: token.to_string(),
            full_token: c
                .get("full_token")
                .and_then(Value::as_str)
                .unwrap_or(token)
                .to_string(),
            name: c.get("name").and_then(|n| strings(n).into_iter().next()),
            installed: c.get("installed").and_then(Value::as_str).map(String::from),
            installed_time: c.get("installed_time").and_then(Value::as_i64),
            ..Default::default()
        };
        for a in c
            .get("artifacts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(app) = a.get("app") {
                match a.get("target").and_then(Value::as_str) {
                    Some(t) => cask.apps.push(t.to_string()),
                    None => {
                        for x in app.as_array().into_iter().flatten() {
                            match x {
                                Value::String(s) => cask.apps.push(s.clone()),
                                Value::Object(o) => cask.apps.extend(
                                    o.get("target").and_then(Value::as_str).map(String::from),
                                ),
                                _ => {}
                            }
                        }
                    }
                }
            }
            for entry in a.get("zap").and_then(Value::as_array).into_iter().flatten() {
                if let Some(o) = entry.as_object() {
                    for (k, val) in o {
                        for s in strings(val) {
                            cask.zap.push(format!("{k}: {s}"));
                        }
                    }
                }
            }
            for entry in a
                .get("uninstall")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(l) = entry.get("launchctl") {
                    cask.launchctl.extend(strings(l));
                }
            }
        }
        out.push(cask);
    }
    Ok(out)
}

pub fn brew_versions(text: &str) -> Vec<(String, Option<String>)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            Some((name.to_string(), it.last().map(String::from)))
        })
        .collect()
}

pub fn artifact_file_name(a: &str) -> &str {
    a.trim_end_matches('/').rsplit('/').next().unwrap_or(a)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PkgInfo {
    pub id: String,
    pub version: Option<String>,
    pub volume: String,
    pub location: String,
    pub installed: Option<i64>,
}

impl PkgInfo {
    pub fn prefix(&self) -> std::path::PathBuf {
        let mut p = std::path::PathBuf::from(if self.volume.is_empty() {
            "/"
        } else {
            &self.volume
        });
        let loc = self.location.trim_start_matches('/');
        if !loc.is_empty() {
            p.push(loc);
        }
        p
    }
}

pub fn pkg_info(v: &Value) -> Option<PkgInfo> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    Some(PkgInfo {
        id: s("pkgid")?,
        version: s("pkg-version"),
        volume: s("volume").unwrap_or_else(|| "/".into()),
        location: s("install-location").unwrap_or_default(),
        installed: v.get("install-time").and_then(Value::as_i64),
    })
}

pub fn third_party_pkgs(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_apple(l) && valid_label(l))
        .map(String::from)
        .collect()
}

const BUNDLE_EXT: &[&str] = &[
    "app",
    "framework",
    "kext",
    "plugin",
    "bundle",
    "component",
    "vst",
    "vst3",
    "clap",
    "prefPane",
    "appex",
    "xpc",
    "mdimporter",
    "qlgenerator",
    "saver",
    "driver",
    "systemextension",
    "dext",
    "aaxplugin",
];

fn is_bundle(component: &str) -> bool {
    component.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && BUNDLE_EXT.iter().any(|b| b.eq_ignore_ascii_case(ext))
    })
}

pub fn collapse_receipt(files: &str, is_dir: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out = BTreeSet::new();
    for raw in files.lines() {
        let p = raw.trim().trim_start_matches("./").trim_matches('/');
        if p.is_empty() || p == "." {
            continue;
        }
        let parts: Vec<&str> = p.split('/').collect();
        if let Some(i) = parts.iter().position(|c| is_bundle(c)) {
            out.insert(parts[..=i].join("/"));
        } else if !is_dir(p) {
            out.insert(p.to_string());
        }
    }
    let all: Vec<String> = out.into_iter().collect();
    all.iter()
        .filter(|p| {
            !all.iter()
                .any(|q| q != *p && p.starts_with(&format!("{q}/")))
        })
        .cloned()
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaunchJob {
    pub label: Option<String>,
    pub program: Option<String>,
    pub associated: Vec<String>,
}

pub fn launch_job(v: &Value) -> LaunchJob {
    LaunchJob {
        label: v.get("Label").and_then(Value::as_str).map(String::from),
        program: v
            .get("Program")
            .and_then(Value::as_str)
            .or_else(|| {
                v.get("ProgramArguments")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
            })
            .map(String::from),
        associated: v
            .get("AssociatedBundleIdentifiers")
            .map(strings)
            .unwrap_or_default(),
    }
}

pub fn container_owner(v: &Value) -> Option<String> {
    v.get("MCMMetadataIdentifier")
        .and_then(Value::as_str)
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/washmybelly_fixtures/apps_macos/"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FX}{name}")).unwrap()
    }

    #[test]
    fn info_plist_to_bundle_info() {
        let v = super::super::plist::parse(fixture("info_mullvad.plist").as_bytes()).unwrap();
        let b = bundle_info(&v);
        assert_eq!(b.bundle_id.as_deref(), Some("net.mullvad.vpn"));
        assert_eq!(b.name.as_deref(), Some("Mullvad VPN"));
        assert!(b.version.is_some());
        assert!(b.executable.is_some());
    }

    #[test]
    fn codesign_report() {
        let s = codesign(&fixture("codesign_mullvad.txt"));
        assert_eq!(s.team_id.as_deref(), Some("CKG9MXH72F"));
        assert_eq!(s.developer.as_deref(), Some("Mullvad VPN AB"));
        assert_eq!(s.identifier.as_deref(), Some("net.mullvad.vpn"));
        assert!(!s.app_store);
        let apple = codesign(
            "Identifier=com.apple.calculator\nAuthority=Software Signing\nTeamIdentifier=not set\n",
        );
        assert_eq!(apple.team_id, None);
        let mas =
            codesign("Authority=Apple Mac OS Application Signing\nTeamIdentifier=ABCDE12345\n");
        assert!(mas.app_store);
    }

    #[test]
    fn brew_info_json() {
        let casks = brew_casks(&fixture("brew_info_casks.json")).unwrap();
        let ff = casks.iter().find(|c| c.token == "firefox").unwrap();
        assert_eq!(ff.apps, vec!["/Applications/Firefox.app".to_string()]);
        assert!(ff
            .zap
            .iter()
            .any(|z| z == "trash: ~/Library/Caches/Firefox"));
        assert!(ff.zap.iter().any(|z| z.starts_with("rmdir: ")));
        let code = casks
            .iter()
            .find(|c| c.token == "visual-studio-code")
            .unwrap();
        assert_eq!(
            code.launchctl,
            vec!["com.microsoft.VSCode.ShipIt".to_string()]
        );
        let az = casks
            .iter()
            .find(|c| c.token == "azure-cli-preview")
            .unwrap();
        assert!(az.apps.is_empty());
        assert_eq!(az.installed.as_deref(), Some("2.87.0"));
        assert_eq!(az.zap, vec!["trash: ~/.azure".to_string()]);
        assert!(brew_casks("not json").is_err());
    }

    #[test]
    fn brew_list_versions() {
        let v = brew_versions("aom 3.14.1 3.15.0\napr 1.7.6\n\n");
        assert_eq!(
            v,
            vec![
                ("aom".into(), Some("3.15.0".into())),
                ("apr".into(), Some("1.7.6".into()))
            ]
        );
    }

    #[test]
    fn pkg_info_plist() {
        let v = super::super::plist::parse(fixture("pkg_info_mullvad.plist").as_bytes()).unwrap();
        let p = pkg_info(&v).unwrap();
        assert_eq!(p.id, "net.mullvad.vpn");
        assert_eq!(p.prefix(), std::path::PathBuf::from("/Applications"));
        assert_eq!(p.installed, Some(1786637508));
        assert_eq!(
            third_party_pkgs("com.apple.pkg.X\nnet.mullvad.vpn\n$(rm)\n"),
            vec!["net.mullvad.vpn".to_string()]
        );
    }

    #[test]
    fn receipts_collapse_to_bundles_and_files() {
        let files = "Mullvad VPN.app\nMullvad VPN.app/Contents\nMullvad VPN.app/Contents/Info.plist\nLibrary\nLibrary/LaunchDaemons\nLibrary/LaunchDaemons/net.mullvad.daemon.plist\nusr/local/bin/mullvad\nLibrary/Audio/Plug-Ins/Components/X.component/Contents/MacOS/X\n";
        let dirs = [
            "Library",
            "Library/LaunchDaemons",
            "usr",
            "usr/local",
            "usr/local/bin",
        ];
        let out = collapse_receipt(files, &|p| dirs.contains(&p));
        assert_eq!(
            out,
            vec![
                "Library/Audio/Plug-Ins/Components/X.component".to_string(),
                "Library/LaunchDaemons/net.mullvad.daemon.plist".to_string(),
                "Mullvad VPN.app".to_string(),
                "usr/local/bin/mullvad".to_string(),
            ]
        );
    }

    #[test]
    fn mdls_values_align_or_nothing() {
        let out = b"2026-07-15 16:02:16 +0000\0(null)\0";
        let v = mdls_dates(out, 2).unwrap();
        assert_eq!(v[0], Some(1784131336));
        assert_eq!(v[1], None);
        assert!(mdls_dates(b"x: could not find x.\n", 2).is_none());
    }

    #[test]
    fn ids_and_lineage() {
        assert!(lineage("com.foo.App.plist", "com.foo.app"));
        assert!(lineage("com.foo.app", "com.foo.app"));
        assert!(!lineage("com.foo.apple", "com.foo.app"));
        assert!(!lineage("com.foo", "com.foo.app"));
        assert!(is_uuid("0A1B2C3D-0000-4000-8000-00000000ABCD"));
        assert!(!is_uuid("com.foo.app"));
        assert!(valid_bundle_id("net.mullvad.vpn"));
        assert!(!valid_bundle_id("a b.c"));
        assert!(!valid_label("-rf"));
        assert!(!valid_label("a;b"));
        assert!(valid_brew_token("python@3.14"));
        assert!(!valid_brew_token("../x"));
        assert_eq!(normalize("Visual Studio Code 2"), "visualstudiocode2");
    }

    #[test]
    fn launchd_and_container_plists() {
        let v = super::super::plist::parse_xml(
            r#"<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>com.foo.helper</string><key>ProgramArguments</key><array><string>/Applications/Foo.app/Contents/MacOS/helper</string><string>--x</string></array><key>AssociatedBundleIdentifiers</key><string>com.foo.app</string></dict></plist>"#,
        )
        .unwrap();
        let j = launch_job(&v);
        assert_eq!(j.label.as_deref(), Some("com.foo.helper"));
        assert_eq!(
            j.program.as_deref(),
            Some("/Applications/Foo.app/Contents/MacOS/helper")
        );
        assert_eq!(j.associated, vec!["com.foo.app".to_string()]);
        let m = super::super::plist::parse_xml(&fixture("container_metadata.plist")).unwrap();
        assert_eq!(
            container_owner(&m).as_deref(),
            Some("com.example.sandboxed")
        );
    }
}
