use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::startup::macos::{file_url_path, parse_btm, parse_plist_xml, BtmItem};
use super::super::startup::Sys;
use super::parse::{self, Assess, CodeSign, Script, Verify};
use super::{Class, Item, QuarantineInfo};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[derive(Debug, Clone)]
pub struct Roots {
    pub uid: i64,
    pub user_agents: PathBuf,
    pub agents: PathBuf,
    pub daemons: PathBuf,
    pub helpers: PathBuf,
    pub extensions: PathBuf,
    pub applications: PathBuf,
    pub xprotect_rules: PathBuf,
    pub yara: Option<PathBuf>,
}

impl Roots {
    pub fn real() -> Roots {
        let home = dirs::home_dir().unwrap_or_default();
        Roots {
            uid: super::super::startup::current_uid() as i64,
            user_agents: home.join("Library/LaunchAgents"),
            agents: "/Library/LaunchAgents".into(),
            daemons: "/Library/LaunchDaemons".into(),
            helpers: "/Library/PrivilegedHelperTools".into(),
            extensions: "/Library/Extensions".into(),
            applications: "/Applications".into(),
            xprotect_rules: "/Library/Apple/System/Library/CoreServices/XProtect.bundle/Contents/Resources/XProtect.yara".into(),
            yara: find_yara(),
        }
    }
}

fn find_yara() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p).filter(|d| d.is_absolute()));
    }
    dirs.into_iter()
        .map(|d| d.join("yara"))
        .find(|p| p.is_file())
}

#[derive(Debug, Default)]
pub struct Inventory {
    pub items: Vec<Item>,
    pub notes: Vec<String>,
    pub yara: Value,
}

impl Inventory {
    pub fn to_json(&self) -> Value {
        let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
        let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
        for it in &self.items {
            *by_kind.entry(it.kind.as_str()).or_default() += 1;
            if let Some(c) = it.class {
                let k = serde_json::to_value(c)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                *by_class.entry(k).or_default() += 1;
            }
        }
        json!({
            "items": self.items,
            "count": self.items.len(),
            "third_party": self.items.iter().filter(|i| i.third_party).count(),
            "by_kind": by_kind,
            "by_class": by_class,
            "notes": self.notes,
            "xprotect_yara": self.yara,
            "plan_id": Value::Null,
            "about": "An inventory, not an antivirus: signature, notarization and quarantine as macOS reports them. Nothing is judged malware. Turn items off in the startup module (startup_id).",
        })
    }
}

#[derive(Debug, Clone, Default)]
struct Entry {
    item: Item,
    target: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Check {
    exists: bool,
    cs: Option<CodeSign>,
    verify: Option<Verify>,
    assess: Option<Assess>,
    quarantine: Option<parse::Quarantine>,
    yara: Vec<String>,
}

pub fn collect(sys: &dyn Sys, roots: &Roots) -> Inventory {
    let mut inv = Inventory::default();
    super::super::progress(
        "security",
        "list",
        0,
        0,
        "reading launchd, background tasks and extensions",
    );

    let (btm_text, mut entries) = std::thread::scope(|s| {
        let h = s.spawn(|| sys.run(&argv(&["sfltool", "dumpbtm"])));
        let mut e = launchd_entries(sys, roots);
        e.extend(helper_entries(roots));
        (
            h.join().unwrap_or_else(|_| Err("sfltool stopped".into())),
            e,
        )
    });
    let btm = match btm_text {
        Ok(o) if o.ok && o.stdout.contains("Records for UID") => parse_btm(&o.stdout),
        Ok(o) => {
            inv.notes
                .push(format!("btm_unavailable: {}", first_line(&o.stderr)));
            Vec::new()
        }
        Err(e) => {
            inv.notes.push(format!("btm_unavailable: {e}"));
            Vec::new()
        }
    };
    merge_btm(&mut entries, &btm, roots.uid);
    entries.extend(sysext_entries(sys, roots, &mut inv.notes));
    entries.extend(kext_entries(sys, roots, &mut inv.notes));
    entries.extend(profile_entries(sys, &mut inv.notes));
    entries.extend(cron_entries(sys));

    let targets: Vec<String> = {
        let mut seen = HashSet::new();
        entries
            .iter()
            .filter_map(|e| e.target.clone())
            .filter(|t| seen.insert(t.clone()))
            .collect()
    };
    let rules = roots
        .yara
        .as_ref()
        .filter(|_| roots.xprotect_rules.is_file());
    inv.yara = match (&roots.yara, roots.xprotect_rules.is_file()) {
        (Some(y), true) => json!({ "ran": true, "yara": y, "rules": roots.xprotect_rules }),
        (None, true) => {
            inv.notes
                .push("yara_unavailable: install yara to also run XProtect's own rules".into());
            json!({ "ran": false, "reason": "no_yara_binary", "rules": roots.xprotect_rules })
        }
        (_, false) => json!({ "ran": false, "reason": "no_xprotect_rules" }),
    };
    let total = targets.len() as u64;
    let done = std::sync::atomic::AtomicU64::new(0);
    let checks: HashMap<String, Check> = std::thread::scope(|s| {
        let chunk = targets.len().div_ceil(6).max(1);
        let handles: Vec<_> = targets
            .chunks(chunk)
            .map(|part| {
                let done = &done;
                s.spawn(move || {
                    part.iter()
                        .map(|t| {
                            let c = check(
                                sys,
                                t,
                                rules.map(|y| (y.as_path(), roots.xprotect_rules.as_path())),
                            );
                            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            super::super::progress("security", "check", n, total, t.clone());
                            (t.clone(), c)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });

    for mut e in entries {
        if let Some(t) = &e.target {
            if let Some(c) = checks.get(t) {
                apply(&mut e.item, t, c);
            }
        } else if e.item.class.is_none() {
            e.item.class = Some(Class::Unknown);
        }
        if e.item.summary.is_empty() {
            e.item.summary = summary(&e.item);
        }
        let apple = e.item.class == Some(Class::Apple);
        e.item.third_party = e.item.third_party && !apple;
        inv.items.push(e.item);
    }
    inv.items.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    super::super::progress(
        "security",
        "done",
        total,
        total,
        format!("{} items", inv.items.len()),
    );
    inv
}

fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn plists(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "plist"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn created(p: &Path) -> Option<i64> {
    let m = std::fs::symlink_metadata(p).ok()?;
    let t = m.created().or_else(|_| m.modified()).ok()?;
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn apple_id(id: &str) -> bool {
    id.starts_with("com.apple.") || id.contains(".com.apple.")
}

fn path_notes(path: &str) -> Vec<String> {
    let mut n = Vec::new();
    if [
        "/tmp/",
        "/private/tmp/",
        "/var/tmp/",
        "/private/var/tmp/",
        "/private/var/folders/",
        "/var/folders/",
    ]
    .iter()
    .any(|p| path.starts_with(p))
    {
        n.push("in_temp_folder".into());
    }
    if path
        .split('/')
        .any(|c| c.len() > 1 && c.starts_with('.') && c != "..")
    {
        n.push("hidden_folder".into());
    }
    n
}

fn launchd_entries(sys: &dyn Sys, roots: &Roots) -> Vec<Entry> {
    let mut out = Vec::new();
    for (dir, kind, scope) in [
        (&roots.user_agents, "launch_agent", "user"),
        (&roots.agents, "launch_agent", "system"),
        (&roots.daemons, "launch_daemon", "system"),
    ] {
        for p in plists(dir) {
            let path = p.to_string_lossy().into_owned();
            let info = sys
                .run(&argv(&[
                    "plutil", "-convert", "xml1", "-o", "-", "--", &path,
                ]))
                .ok()
                .filter(|o| o.ok)
                .and_then(|o| parse_plist_xml(&o.stdout))
                .unwrap_or_default();
            let label = info.label.clone().unwrap_or_else(|| stem(&p));
            let program = info.executable().map(str::to_string);
            let args: Vec<String> = if info.arguments.is_empty() {
                program.iter().cloned().collect()
            } else {
                info.arguments.clone()
            };
            let mut item = Item {
                id: format!("security:{kind}:{path}"),
                kind: kind.into(),
                scope: scope.into(),
                name: label.clone(),
                path: Some(path.clone()),
                program: program.clone(),
                command: (!args.is_empty()).then(|| args.join(" ")),
                installed: created(&p),
                third_party: !apple_id(&label),
                startup_id: Some(format!(
                    "startup:{}:{path}",
                    if kind == "launch_daemon" {
                        "daemon"
                    } else {
                        "agent"
                    }
                )),
                state: Some(if info.disabled { "disabled" } else { "enabled" }.into()),
                exists: true,
                ..Default::default()
            };
            let target = match parse::script_of(&args) {
                Script::File(s) => {
                    item.notes.push("runs_script".into());
                    item.script = Some(s.clone());
                    Some(s)
                }
                Script::Inline => {
                    item.notes.push("inline_command".into());
                    program.clone()
                }
                Script::None => program.clone(),
            };
            if let Some(t) = &target {
                item.notes.extend(path_notes(t));
                if !t.starts_with('/') {
                    item.class = Some(Class::Unknown);
                    item.summary = format!("Runs \"{t}\" found through the PATH; not checked");
                    out.push(Entry { item, target: None });
                    continue;
                }
            } else {
                item.class = Some(Class::Unknown);
                item.summary = "The plist names no program".into();
            }
            out.push(Entry {
                item,
                target: target.filter(|t| t.starts_with('/')),
            });
        }
    }
    out
}

fn helper_entries(roots: &Roots) -> Vec<Entry> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(&roots.helpers)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v.into_iter()
        .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
        .map(|p| {
            let path = p.to_string_lossy().into_owned();
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Entry {
                item: Item {
                    id: format!("security:privileged_helper:{path}"),
                    kind: "privileged_helper".into(),
                    scope: "system".into(),
                    third_party: !apple_id(&name),
                    name,
                    path: Some(path.clone()),
                    program: Some(path.clone()),
                    installed: created(&p),
                    exists: true,
                    notes: path_notes(&path),
                    ..Default::default()
                },
                target: Some(path),
            }
        })
        .collect()
}

fn merge_btm(entries: &mut Vec<Entry>, btm: &[BtmItem], uid: i64) {
    let by_url: HashMap<String, &BtmItem> = btm
        .iter()
        .filter_map(|b| b.url.as_deref().and_then(file_url_path).map(|p| (p, b)))
        .collect();
    for e in entries.iter_mut() {
        if let Some(b) = e.item.path.as_ref().and_then(|p| by_url.get(p)) {
            e.item.state = Some(if b.enabled { "enabled" } else { "disabled" }.into());
            if b.team_id.is_some() {
                e.item.team_id = b.team_id.clone();
            }
        }
    }
    let known: HashSet<String> = entries.iter().filter_map(|e| e.item.path.clone()).collect();
    let parents: HashMap<&str, &BtmItem> = btm.iter().map(|b| (b.identifier.as_str(), b)).collect();
    let mut seen = HashSet::new();
    for b in btm {
        if !matches!(
            b.kind.as_str(),
            "app" | "login item" | "agent" | "daemon" | "legacy agent" | "legacy daemon"
        ) {
            continue;
        }
        if !(b.uid == uid || b.uid <= 0)
            || apple_id(&b.identifier)
            || !seen.insert(b.identifier.clone())
        {
            continue;
        }
        let parent_path = b
            .parent
            .as_deref()
            .and_then(|p| parents.get(p))
            .and_then(|p| p.url.as_deref())
            .and_then(file_url_path);
        let resolve = |s: &str| -> Option<String> {
            if s.starts_with("file://") {
                file_url_path(s)
            } else if s.starts_with('/') {
                Some(s.trim_end_matches('/').to_string())
            } else {
                parent_path
                    .as_ref()
                    .map(|pp| format!("{pp}/{}", s.trim_end_matches('/')))
            }
        };
        let url = b.url.as_deref().and_then(resolve);
        if url.as_ref().is_some_and(|u| known.contains(u)) {
            continue;
        }
        let exe = b.executable.as_deref().and_then(resolve);
        let target = match b.kind.as_str() {
            "app" | "login item" => url.clone().or(exe.clone()),
            _ => exe.clone().or(url.clone()),
        };
        if b.kind == "app" && !b.enabled {
            continue;
        }
        let path = url.clone().or(exe.clone());
        let id_part = path.clone().unwrap_or_else(|| b.identifier.clone());
        entries.push(Entry {
            item: Item {
                id: format!("security:background_task:{id_part}"),
                kind: "background_task".into(),
                scope: if b.kind.contains("daemon") {
                    "system"
                } else {
                    "user"
                }
                .into(),
                name: b.name.clone(),
                path,
                program: exe,
                team_id: b.team_id.clone(),
                signer: b.developer.clone(),
                state: Some(if b.enabled { "enabled" } else { "disabled" }.into()),
                third_party: true,
                startup_id: Some(format!("startup:btm:{}", b.identifier)),
                detail: Some(format!(
                    "{} · {}",
                    b.kind,
                    b.bundle_id.clone().unwrap_or_else(|| b.identifier.clone())
                )),
                exists: true,
                ..Default::default()
            },
            target,
        });
    }
}

fn find_sysext(roots: &Roots, bundle_id: &str) -> Option<String> {
    let rd = std::fs::read_dir(&roots.applications).ok()?;
    for app in rd.flatten() {
        for ext in ["systemextension", "dext"] {
            let p = app
                .path()
                .join("Contents/Library/SystemExtensions")
                .join(format!("{bundle_id}.{ext}"));
            if p.exists() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn sysext_entries(sys: &dyn Sys, roots: &Roots, notes: &mut Vec<String>) -> Vec<Entry> {
    let text = match sys.run(&argv(&["systemextensionsctl", "list"])) {
        Ok(o) if o.ok => o.stdout,
        Ok(o) => {
            notes.push(format!(
                "system_extensions_unavailable: {}",
                first_line(&o.stderr)
            ));
            return Vec::new();
        }
        Err(e) => {
            notes.push(format!("system_extensions_unavailable: {e}"));
            return Vec::new();
        }
    };
    parse::parse_systemextensions(&text)
        .into_iter()
        .filter(|x| !apple_id(&x.bundle_id))
        .map(|x| {
            let kind = if x.category == "network_extension" { "network_extension" } else { "system_extension" };
            let target = find_sysext(roots, &x.bundle_id);
            let mut item = Item {
                id: format!("security:{kind}:{}", x.bundle_id),
                kind: kind.into(),
                scope: "system".into(),
                name: if x.name.is_empty() { x.bundle_id.clone() } else { x.name.clone() },
                path: target.clone(),
                team_id: x.team_id.clone(),
                state: Some(x.state.clone()),
                third_party: true,
                exists: true,
                detail: Some(format!(
                    "{} · {}{}",
                    x.category.replace('_', " "),
                    x.bundle_id,
                    x.version.as_deref().map(|v| format!(" ({v})")).unwrap_or_default()
                )),
                ..Default::default()
            };
            if target.is_none() {
                item.class = Some(Class::Unknown);
                item.summary = format!(
                    "Team ID {}; the app that installed it was not found, so the signature was not checked (macOS only activates notarized extensions)",
                    x.team_id.as_deref().unwrap_or("unknown")
                );
            }
            Entry { item, target }
        })
        .collect()
}

fn kext_entries(sys: &dyn Sys, roots: &Roots, notes: &mut Vec<String>) -> Vec<Entry> {
    let loaded: HashMap<String, String> =
        match sys.run(&argv(&["kmutil", "showloaded", "--list-only"])) {
            Ok(o) if o.ok => parse::parse_kmutil(&o.stdout)
                .into_iter()
                .filter(|(b, _)| !apple_id(b))
                .collect(),
            Ok(o) => {
                notes.push(format!("kexts_unavailable: {}", first_line(&o.stderr)));
                HashMap::new()
            }
            Err(e) => {
                notes.push(format!("kexts_unavailable: {e}"));
                HashMap::new()
            }
        };
    let mut out = Vec::new();
    let mut matched = HashSet::new();
    let mut kexts: Vec<PathBuf> = std::fs::read_dir(&roots.extensions)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "kext"))
                .collect()
        })
        .unwrap_or_default();
    kexts.sort();
    for k in kexts {
        let path = k.to_string_lossy().into_owned();
        let plist = k.join("Contents/Info.plist").to_string_lossy().into_owned();
        let bundle = sys
            .run(&argv(&[
                "plutil",
                "-extract",
                "CFBundleIdentifier",
                "raw",
                "-o",
                "-",
                "--",
                &plist,
            ]))
            .ok()
            .filter(|o| o.ok)
            .map(|o| o.stdout.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| stem(&k));
        if apple_id(&bundle) {
            continue;
        }
        let version = loaded.get(&bundle).cloned();
        let is_loaded = version.is_some();
        matched.insert(bundle.clone());
        out.push(Entry {
            item: Item {
                id: format!("security:kernel_extension:{path}"),
                kind: "kernel_extension".into(),
                scope: "system".into(),
                name: stem(&k),
                path: Some(path.clone()),
                installed: created(&k),
                state: Some(if is_loaded { "loaded" } else { "not_loaded" }.into()),
                third_party: true,
                exists: true,
                detail: Some(format!(
                    "{bundle}{}",
                    version.map(|v| format!(" ({v})")).unwrap_or_default()
                )),
                ..Default::default()
            },
            target: Some(path),
        });
    }
    for (bundle, version) in loaded {
        if matched.contains(&bundle) {
            continue;
        }
        out.push(Entry {
            item: Item {
                id: format!("security:kernel_extension:{bundle}"),
                kind: "kernel_extension".into(),
                scope: "system".into(),
                name: bundle.clone(),
                state: Some("loaded".into()),
                third_party: true,
                exists: true,
                class: Some(Class::Unknown),
                summary: "Loaded in the kernel; its bundle is not in /Library/Extensions, so the signature was not checked".into(),
                detail: Some(format!("{bundle} ({version})")),
                ..Default::default()
            },
            target: None,
        });
    }
    out
}

fn profile_entries(sys: &dyn Sys, notes: &mut Vec<String>) -> Vec<Entry> {
    let text = match sys.run(&argv(&["profiles", "show", "-type", "configuration"])) {
        Ok(o) if o.ok => o.stdout,
        Ok(o) => {
            notes.push(format!(
                "profiles_unavailable: {}",
                first_line(&format!("{}{}", o.stderr, o.stdout))
            ));
            return Vec::new();
        }
        Err(e) => {
            notes.push(format!("profiles_unavailable: {e}"));
            return Vec::new();
        }
    };
    if !text.contains("_computerlevel") {
        notes.push("profiles_device_need_admin: device-wide profiles are listed in System Settings > General > Device Management".into());
    }
    parse::parse_profiles(&text)
        .into_iter()
        .map(|p| {
            let org = p
                .organization
                .clone()
                .unwrap_or_else(|| "an unnamed organization".into());
            let (class, summary) = match p.verification.as_deref() {
                Some("verified") => (Class::Signed, format!("Signed profile from {org}")),
                Some("unsigned") => (Class::Unsigned, format!("Unsigned profile from {org}")),
                Some(other) => (Class::Unknown, format!("Profile from {org} ({other})")),
                None => (Class::Unknown, format!("Profile from {org}")),
            };
            let mut notes = Vec::new();
            if p.removal_disallowed {
                notes.push("removal_disallowed".to_string());
            }
            Entry {
                item: Item {
                    id: format!(
                        "security:profile:{}",
                        if p.identifier.is_empty() {
                            &p.name
                        } else {
                            &p.identifier
                        }
                    ),
                    kind: "profile".into(),
                    scope: if p.scope == "_computerlevel" {
                        "system"
                    } else {
                        "user"
                    }
                    .into(),
                    name: if p.name.is_empty() {
                        p.identifier.clone()
                    } else {
                        p.name.clone()
                    },
                    path: Some(p.identifier.clone()),
                    signer: p.organization.clone(),
                    installed: p.installed.as_deref().and_then(parse::parse_profile_date),
                    class: Some(class),
                    summary,
                    third_party: true,
                    exists: true,
                    notes,
                    detail: Some(
                        [
                            p.description.clone().unwrap_or_default(),
                            p.payloads.join(", "),
                        ]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · "),
                    )
                    .filter(|s| !s.is_empty()),
                    ..Default::default()
                },
                target: None,
            }
        })
        .collect()
}

fn cron_entries(sys: &dyn Sys) -> Vec<Entry> {
    let Ok(o) = sys.run(&argv(&["crontab", "-l"])) else {
        return Vec::new();
    };
    if !o.ok {
        return Vec::new();
    }
    parse::parse_crontab(&o.stdout)
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let args: Vec<String> = c.command.split_whitespace().map(str::to_string).collect();
            let program = args.first().cloned();
            let mut item = Item {
                id: format!("security:cron:{i}:{}", c.command),
                kind: "cron".into(),
                scope: "user".into(),
                name: program
                    .as_deref()
                    .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
                    .unwrap_or_default(),
                program: program.clone(),
                command: Some(c.command.clone()),
                detail: Some(c.schedule.clone()),
                third_party: true,
                exists: true,
                ..Default::default()
            };
            let target = match parse::script_of(&args) {
                Script::File(s) => {
                    item.notes.push("runs_script".into());
                    item.script = Some(s.clone());
                    Some(s)
                }
                Script::Inline => {
                    item.notes.push("inline_command".into());
                    program
                }
                Script::None => program,
            }
            .filter(|t| t.starts_with('/'));
            if target.is_none() {
                item.class = Some(Class::Unknown);
                item.summary = "A shell command; not checked".into();
            }
            if let Some(t) = &target {
                item.notes.extend(path_notes(t));
            }
            Entry { item, target }
        })
        .collect()
}

fn head4(p: &str) -> Vec<u8> {
    use std::io::Read;
    let mut buf = [0u8; 4];
    match std::fs::File::open(p).and_then(|mut f| f.read(&mut buf)) {
        Ok(n) => buf[..n].to_vec(),
        Err(_) => Vec::new(),
    }
}

fn check(sys: &dyn Sys, target: &str, yara: Option<(&Path, &Path)>) -> Check {
    let mut c = Check {
        exists: std::fs::symlink_metadata(target).is_ok(),
        ..Default::default()
    };
    if !c.exists {
        return c;
    }
    let cs = sys
        .run(&argv(&["codesign", "-dv", "--verbose=4", "--", target]))
        .map(|o| parse::parse_codesign(&format!("{}{}", o.stderr, o.stdout)))
        .ok();
    let bundle = Path::new(target).is_dir();
    if let Some(cs) = &cs {
        if cs.signed && !cs.adhoc && !cs.apple() {
            c.verify = sys
                .run(&argv(&[
                    "codesign",
                    "-v",
                    "-R=notarized",
                    "--check-notarization",
                    "--",
                    target,
                ]))
                .ok()
                .map(|o| parse::parse_verify(o.code, &format!("{}{}", o.stderr, o.stdout)));
            if target.ends_with(".app") {
                c.assess = sys
                    .run(&argv(&[
                        "spctl", "--assess", "--type", "execute", "-vv", "--", target,
                    ]))
                    .ok()
                    .map(|o| parse::parse_spctl(&format!("{}{}", o.stderr, o.stdout)));
            }
        } else if !cs.signed && !bundle && !parse::is_macho(&head4(target)) {
            c.assess = sys
                .run(&argv(&[
                    "spctl",
                    "-a",
                    "-t",
                    "open",
                    "--context",
                    "context:primary-signature",
                    "-vv",
                    "--",
                    target,
                ]))
                .ok()
                .map(|o| parse::parse_spctl(&format!("{}{}", o.stderr, o.stdout)));
        }
    }
    c.cs = cs;
    let q_on = parse::enclosing_app(target).unwrap_or(target);
    c.quarantine = sys
        .run(&argv(&["xattr", "-p", "com.apple.quarantine", "--", q_on]))
        .ok()
        .filter(|o| o.ok)
        .and_then(|o| parse::parse_quarantine(&o.stdout));
    if let Some((bin, rules)) = yara {
        if !bundle {
            let (b, r) = (
                bin.to_string_lossy().into_owned(),
                rules.to_string_lossy().into_owned(),
            );
            if let Ok(o) = sys.run(&[b, "-w".into(), r, target.to_string()]) {
                c.yara = parse::parse_yara(&o.stdout);
            }
        }
    }
    c
}

pub fn classify(
    exists: bool,
    cs: Option<&CodeSign>,
    verify: Option<&Verify>,
    assess: Option<&Assess>,
) -> (Class, Option<bool>) {
    if !exists {
        return (Class::Missing, None);
    }
    let Some(cs) = cs else {
        return (Class::Unknown, None);
    };
    if !cs.signed {
        return match assess {
            Some(a) if a.accepted => match a.notarized() {
                Some(true) => (Class::Notarized, Some(true)),
                _ => (Class::Signed, a.notarized()),
            },
            _ => (Class::Unsigned, None),
        };
    }
    if cs.adhoc {
        return (Class::Adhoc, Some(false));
    }
    if let Some(Verify::Invalid(_)) = verify {
        return (Class::Invalid, None);
    }
    if cs.apple() {
        return (Class::Apple, None);
    }
    if cs.app_store() || assess.and_then(|a| a.source.as_deref()) == Some("Mac App Store") {
        return (Class::AppStore, None);
    }
    let notarized = assess.and_then(Assess::notarized).or(match verify {
        Some(Verify::Notarized) => Some(true),
        Some(Verify::NotNotarized) => Some(false),
        _ => None,
    });
    match notarized {
        Some(true) => (Class::Notarized, Some(true)),
        n => (Class::Signed, n),
    }
}

fn apply(item: &mut Item, target: &str, c: &Check) {
    item.checked = Some(target.to_string());
    item.exists = c.exists;
    let (class, notarized) = classify(
        c.exists,
        c.cs.as_ref(),
        c.verify.as_ref(),
        c.assess.as_ref(),
    );
    item.class = Some(class);
    item.notarized = notarized;
    if let Some(cs) = &c.cs {
        item.signer = cs.signer().or(item.signer.take());
        item.team_id = cs.team_id.clone().or(item.team_id.take());
        item.hardened_runtime = cs.runtime;
        item.signed_at = cs.timestamp.clone();
    }
    if let Some(Verify::Invalid(r)) = &c.verify {
        item.invalid_reason = Some(r.clone());
    }
    if let Some(q) = &c.quarantine {
        item.notes.push("quarantined".into());
        item.quarantine = Some(QuarantineInfo {
            agent: q.agent.clone(),
            at: q.at,
            approved: q.approved,
        });
    }
    if item.installed.is_none() {
        item.installed = created(Path::new(target));
    }
    item.xprotect_matches = c.yara.clone();
    item.summary = summary(item);
}

pub fn summary(it: &Item) -> String {
    let who = match (&it.signer, &it.team_id) {
        (Some(s), Some(t)) => format!("{s} ({t})"),
        (Some(s), None) => s.clone(),
        (None, Some(t)) => format!("Team ID {t}"),
        (None, None) => "an unknown developer".into(),
    };
    let mut s = match it.class {
        Some(Class::Apple) => "Signed by Apple".to_string(),
        Some(Class::AppStore) => format!("From the Mac App Store ({who})"),
        Some(Class::Notarized) => format!("Signed and notarized by {who}"),
        Some(Class::Signed) => format!("Signed by {who}, not notarized"),
        Some(Class::Adhoc) => "Signed without a developer identity (ad hoc)".to_string(),
        Some(Class::Unsigned) => "Not signed".to_string(),
        Some(Class::Invalid) => format!(
            "The signature does not check out: {}",
            it.invalid_reason.as_deref().unwrap_or("unknown reason")
        ),
        Some(Class::Missing) => format!(
            "Points to a file that does not exist: {}",
            it.checked
                .as_deref()
                .or(it.program.as_deref())
                .unwrap_or("?")
        ),
        Some(Class::Unknown) | None => "Could not be checked".to_string(),
    };
    if let Some(script) = &it.script {
        if let Some(p) = &it.program {
            let name = p.rsplit('/').next().unwrap_or(p);
            s.push_str(&format!(" (script {script}, run by {name})"));
        }
    }
    if !it.xprotect_matches.is_empty() {
        s.push_str(&format!(
            "; matches XProtect rule {}",
            it.xprotect_matches.join(", ")
        ));
    }
    s
}
