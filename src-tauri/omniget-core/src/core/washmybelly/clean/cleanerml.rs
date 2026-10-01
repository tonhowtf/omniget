//! `Action.py`, `General.py`), Copyright (c) 2008-2026 Andrew Ziem,
//! GPL-3.0-or-later. The XML files are embedded unchanged from

use roxmltree::{Document, Node};
use serde::Serialize;

use super::super::{Destination, Os, Risk};
use super::rule::{Action, Requirement, Rule};

pub const ALL_OS: [Os; 3] = [Os::Macos, Os::Linux, Os::Windows];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Skip {
    pub cleaner: String,
    pub option: String,
    pub what: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Converted {
    pub rules: Vec<Rule>,
    pub skipped: Vec<Skip>,
}

pub fn os_match(attr: Option<&str>, os: Os) -> bool {
    let Some(attr) = attr.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let cur: &[&str] = match os {
        Os::Macos => &["macos", "bsd", "unix"],
        Os::Linux => &["linux", "unix"],
        Os::Windows => &["windows"],
    };
    let norm = |t: &str| {
        if t == "darwin" {
            "macos".to_string()
        } else {
            t.to_string()
        }
    };
    let tokens: Vec<&str> = attr
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    let neg: Vec<String> = tokens
        .iter()
        .filter_map(|t| t.strip_prefix('!'))
        .map(norm)
        .collect();
    let pos: Vec<String> = tokens
        .iter()
        .filter(|t| !t.starts_with('!'))
        .map(|t| norm(t))
        .collect();
    if neg.iter().any(|t| cur.contains(&t.as_str())) {
        return false;
    }
    pos.is_empty() || pos.iter().any(|t| cur.contains(&t.as_str()))
}

pub fn expand_multi_var(s: &str, vars: &[(String, Vec<String>)]) -> Vec<String> {
    if !s.contains("$$") {
        return vec![s.to_string()];
    }
    let mut out = vec![s.to_string()];
    for (name, values) in vars {
        let key = format!("$${name}$$");
        if !s.contains(&key) {
            continue;
        }
        let mut next = Vec::new();
        for cur in &out {
            for v in values {
                next.push(cur.replace(&key, v));
            }
        }
        out = next;
    }
    out
}

fn text(n: Node) -> String {
    n.text().unwrap_or("").trim().to_string()
}

fn child_text(n: Node, tag: &str) -> String {
    n.children()
        .find(|c| c.has_tag_name(tag))
        .map(text)
        .unwrap_or_default()
}

fn keep_path(p: &str, os: Os) -> bool {
    if p.contains("$$") || p.trim().is_empty() {
        return false;
    }
    match os {
        Os::Windows => !p.starts_with('/') && !p.starts_with('~') && !p.contains("$XDG_"),
        Os::Macos => !p.contains('\\') && !p.contains('%'),
        Os::Linux => !p.contains('\\') && !p.contains('%') && !p.starts_with("~/Library/"),
    }
}

fn tidy(p: &str) -> String {
    let t = p.trim();
    let t = t.trim_end_matches(['/', '\\']);
    if t.is_empty() {
        p.trim().to_string()
    } else {
        t.to_string()
    }
}

const BROWSERS: &[&str] = &[
    "brave",
    "chromium",
    "elinks",
    "epiphany",
    "firefox",
    "google_chrome",
    "internet_explorer",
    "konqueror",
    "librewolf",
    "links2",
    "microsoft_edge",
    "opera",
    "palemoon",
    "safari",
    "safari_windows",
    "seamonkey",
    "vivaldi",
    "waterfox",
    "zen",
];
const DEV: &[&str] = &[
    "android_studio",
    "claude",
    "java",
    "python",
    "vscode",
    "sqlite3",
    "vim",
    "mc",
];
const SYSTEM: &[&str] = &[
    "thumbnails",
    "x11",
    "gnome",
    "kde",
    "nautilus",
    "windows_explorer",
    "windows_defender",
    "wine",
    "winetricks",
    "bash",
    "fish",
    "zsh",
    "journald",
    "localizations",
];
const CACHE_OPTIONS: &[&str] = &[
    "cache",
    "tmp",
    "temp",
    "temporary_files",
    "thumbnails",
    "ide_cache",
    "gradle_cache",
    "package_lists",
    "index",
    "blocklists",
    "thumbs_db",
    "ds_store",
    "pycache",
    "shell_snapshots",
    "stats",
];
const LOG_OPTIONS: &[&str] = &["logs", "log", "debug_logs", "crash_reports", "memory_dump"];
const USER_FILE_OPTIONS: &[&str] = &[
    "downloads",
    "downloaded_podcasts",
    "recordings",
    "attachments",
    "installers",
    "old_versions",
    "torrents",
    "backup",
    "venv",
    "node_modules",
    "sync_conflict",
    "vim_swap_user",
    "vim_swap_root",
];

pub fn classify(cleaner: &str, option: &str, action: &Action) -> (String, Risk, Destination) {
    let opt = option.to_ascii_lowercase();
    if matches!(action, Action::SqliteVacuum) {
        return ("database".into(), Risk::Reversible, Destination::Unlink);
    }
    if CACHE_OPTIONS.contains(&opt.as_str()) {
        let cat = if BROWSERS.contains(&cleaner) {
            "browser"
        } else if DEV.contains(&cleaner) {
            "dev"
        } else if SYSTEM.contains(&cleaner) {
            "system"
        } else {
            "apps"
        };
        return (cat.into(), Risk::Destructive, Destination::Unlink);
    }
    if LOG_OPTIONS.contains(&opt.as_str()) {
        return ("logs".into(), Risk::Destructive, Destination::Unlink);
    }
    if USER_FILE_OPTIONS.contains(&opt.as_str()) {
        return ("user_files".into(), Risk::Reversible, Destination::Trash);
    }
    ("privacy".into(), Risk::Destructive, Destination::Quarantine)
}

fn skip_reason(command: &str) -> Option<&'static str> {
    Some(match command {
        "delete" | "truncate" | "sqlite.vacuum" => return None,
        "json" | "ini" | "winreg" | "office_registrymodifications" => {
            "edits a settings file or the registry (not ported yet)"
        }
        "cookie"
        | "chrome.autofill"
        | "chrome.databases_db"
        | "chrome.favicons"
        | "chrome.history"
        | "chrome.keywords"
        | "mozilla.favicons"
        | "mozilla.url.history" => {
            "partial SQL cleanup of browser data from Special.py (not ported yet)"
        }
        "apt.autoclean" | "apt.autoremove" | "apt.clean" | "dnf.autoremove" | "dnf.clean_all"
        | "yum.clean_all" | "pacman.cache" | "journald.clean" | "snap.disabled" => {
            "system package manager: covered by the Linux rules through the manager itself"
        }
        "process" => "runs a program: never taken from CleanerML",
        "win.shell.change.notify" => "Windows shell refresh: not needed",
        "macos.orphaned_framework_versions" => {
            "old framework versions inside app bundles: not ported (touches signed bundles)"
        }
        _ => "unknown command",
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GroupKey {
    kind: u8,
    include: Vec<String>,
    exclude: Vec<String>,
    admin: bool,
}

pub fn system_target(p: &str, os: Os) -> bool {
    match os {
        Os::Windows => {
            let l = p.to_ascii_lowercase();
            [
                "%windir%",
                "%systemroot%",
                "%programfiles",
                "%commonprogramfiles",
                "%systemdrive%",
                "c:\\windows",
            ]
            .iter()
            .any(|r| l.starts_with(r))
        }
        _ => p.starts_with('/'),
    }
}

pub fn convert(name: &str, xml: &str) -> Result<Converted, String> {
    if xml.contains("<!DOCTYPE") {
        return Err(format!("{name}: CleanerML with a DTD is refused"));
    }
    let doc = Document::parse(xml).map_err(|e| format!("{name}: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name("cleaner") {
        return Err(format!("{name}: not a cleaner"));
    }
    let cid = root.attribute("id").unwrap_or(name).to_string();
    let clabel = child_text(root, "label");
    let mut out = Converted::default();
    let mut skipped = std::collections::BTreeSet::new();

    if root.children().any(|c| c.has_tag_name("localizations")) {
        skipped.insert(Skip {
            cleaner: cid.clone(),
            option: String::new(),
            what: "localizations".into(),
            reason:
                "unused language files: not ported (removing them breaks app signatures on macOS)"
                    .into(),
        });
    }

    for os in ALL_OS {
        if !os_match(root.attribute("os"), os) {
            continue;
        }
        let mut vars: Vec<(String, Vec<String>)> = Vec::new();
        for v in root.children().filter(|c| c.has_tag_name("var")) {
            let vname = v.attribute("name").unwrap_or("").to_string();
            let mut values = Vec::new();
            for val in v.children().filter(|c| c.has_tag_name("value")) {
                if !os_match(val.attribute("os"), os) {
                    continue;
                }
                if val.attribute("search") == Some("winreg") {
                    skipped.insert(Skip {
                        cleaner: cid.clone(),
                        option: String::new(),
                        what: format!("var {vname}"),
                        reason: "value read from the Windows registry (not ported yet)".into(),
                    });
                    continue;
                }
                let t = tidy(&text(val));
                if keep_path(&t, os) {
                    values.push(t);
                }
            }
            if let Some(e) = vars.iter_mut().find(|(n, _)| *n == vname) {
                e.1.extend(values);
            } else {
                vars.push((vname, values));
            }
        }
        let mut running = Vec::new();
        for r in root.children().filter(|c| c.has_tag_name("running")) {
            if !os_match(r.attribute("os"), os) {
                continue;
            }
            let t = text(r);
            match r.attribute("type") {
                Some("pathname") => {
                    for p in expand_multi_var(&t, &vars) {
                        if keep_path(&p, os) && !running.contains(&p) {
                            running.push(p);
                        }
                    }
                }
                _ => {
                    if !t.is_empty() && !running.contains(&t) {
                        running.push(t);
                    }
                }
            }
        }

        for opt in root.children().filter(|c| c.has_tag_name("option")) {
            if !os_match(opt.attribute("os"), os) {
                continue;
            }
            let oid = opt.attribute("id").unwrap_or("").to_string();
            let olabel = child_text(opt, "label");
            let mut description = child_text(opt, "description");
            let warning = child_text(opt, "warning");
            if !warning.is_empty() {
                description = format!("{description} Warning: {warning}")
                    .trim()
                    .to_string();
            }
            let mut groups: Vec<(GroupKey, Vec<String>)> = Vec::new();
            for act in opt.children().filter(|c| c.has_tag_name("action")) {
                if !os_match(act.attribute("os"), os) {
                    continue;
                }
                let command = act.attribute("command").unwrap_or("");
                if let Some(reason) = skip_reason(command) {
                    skipped.insert(Skip {
                        cleaner: cid.clone(),
                        option: oid.clone(),
                        what: command.to_string(),
                        reason: reason.into(),
                    });
                    continue;
                }
                let search = act.attribute("search").unwrap_or("file");
                let mut include = Vec::new();
                let mut exclude = Vec::new();
                if let Some(t) = act.attribute("type") {
                    include.push(format!("type:{t}"));
                }
                if let Some(r) = act.attribute("regex") {
                    include.push(format!("re:{r}"));
                }
                if let Some(r) = act.attribute("wholeregex") {
                    include.push(format!("path-re:{r}"));
                }
                if let Some(r) = act.attribute("nregex") {
                    exclude.push(format!("re:{r}"));
                }
                if let Some(r) = act.attribute("nwholeregex") {
                    exclude.push(format!("path-re:{r}"));
                }
                let kind = match (command, search) {
                    (_, "deep") => {
                        skipped.insert(Skip {
                            cleaner: cid.clone(),
                            option: oid.clone(),
                            what: "search=deep".into(),
                            reason:
                                "deep scan over folders the person picks: not part of the junk scan"
                                    .into(),
                        });
                        continue;
                    }
                    ("sqlite.vacuum", _) => 2,
                    ("truncate", _) => 3,
                    (_, "walk.all") => 1,
                    (_, "walk.files") => {
                        if !include.iter().any(|i| i.starts_with("type:")) {
                            include.insert(0, "type:f".into());
                        }
                        1
                    }
                    (_, "file" | "glob" | "walk.top") => 0,
                    (_, other) => {
                        skipped.insert(Skip {
                            cleaner: cid.clone(),
                            option: oid.clone(),
                            what: format!("search={other}"),
                            reason: "unknown search".into(),
                        });
                        continue;
                    }
                };
                let Some(path) = act.attribute("path") else {
                    continue;
                };
                let targets: Vec<String> = expand_multi_var(path, &vars)
                    .into_iter()
                    .map(|p| tidy(&p))
                    .filter(|p| keep_path(p, os))
                    .collect();
                for t in targets {
                    let key = GroupKey {
                        kind,
                        include: include.clone(),
                        exclude: exclude.clone(),
                        admin: system_target(&t, os),
                    };
                    let slot = match groups.iter().position(|(k, _)| *k == key) {
                        Some(i) => i,
                        None => {
                            groups.push((key, Vec::new()));
                            groups.len() - 1
                        }
                    };
                    if !groups[slot].1.contains(&t) {
                        groups[slot].1.push(t);
                    }
                }
            }
            let mut merged: Vec<(GroupKey, Vec<String>)> = Vec::new();
            for (key, targets) in groups {
                let types = |k: &GroupKey| {
                    k.include
                        .iter()
                        .filter(|i| i.starts_with("type:"))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                let names = key.include.iter().any(|i| !i.starts_with("type:"));
                let slot = merged.iter().position(|(k, t)| {
                    names
                        && k.include.iter().any(|i| !i.starts_with("type:"))
                        && k.kind == key.kind
                        && k.admin == key.admin
                        && k.exclude == key.exclude
                        && types(k) == types(&key)
                        && *t == targets
                });
                match slot {
                    Some(i) => {
                        for inc in key.include {
                            if !merged[i].0.include.contains(&inc) {
                                merged[i].0.include.push(inc);
                            }
                        }
                    }
                    None => merged.push((key, targets)),
                }
            }
            let groups = merged;
            for (i, (key, targets)) in groups.into_iter().enumerate() {
                let action = match key.kind {
                    0 => Action::Delete,
                    1 => Action::DeleteContents,
                    2 => Action::SqliteVacuum,
                    _ => Action::Truncate,
                };
                let (category, risk, destination) = classify(&cid, &oid, &action);
                let id = if i == 0 {
                    format!("bleachbit.{cid}.{oid}")
                } else {
                    format!("bleachbit.{cid}.{oid}.{}", i + 1)
                };
                let rule = Rule {
                    id,
                    os: vec![os],
                    category,
                    label_key: None,
                    label: format!("{clabel}: {olabel}"),
                    description: description.clone(),
                    targets,
                    include: key.include,
                    exclude: key.exclude,
                    running: running.clone(),
                    min_age_days: 0,
                    action,
                    destination,
                    risk,
                    requires: if key.admin {
                        vec![Requirement::Admin]
                    } else {
                        Vec::new()
                    },
                    source: "BleachBit".into(),
                    license: "GPL-3.0".into(),
                };
                merge(&mut out.rules, rule);
            }
        }
    }
    out.skipped = skipped.into_iter().collect();
    Ok(out)
}

fn merge(rules: &mut Vec<Rule>, rule: Rule) {
    let same = |a: &Rule, b: &Rule| {
        a.id == b.id
            && a.targets == b.targets
            && a.include == b.include
            && a.exclude == b.exclude
            && a.running == b.running
            && a.action == b.action
            && a.label == b.label
            && a.description == b.description
    };
    if let Some(r) = rules.iter_mut().find(|r| same(r, &rule)) {
        for os in rule.os {
            if !r.os.contains(&os) {
                r.os.push(os);
            }
        }
    } else {
        rules.push(rule);
    }
}

pub fn all() -> Converted {
    let mut out = Converted::default();
    for (name, xml) in EMBEDDED {
        match convert(name, xml) {
            Ok(c) => {
                out.rules.extend(c.rules);
                out.skipped.extend(c.skipped);
            }
            Err(e) => out.skipped.push(Skip {
                cleaner: name.to_string(),
                option: String::new(),
                what: "file".into(),
                reason: e,
            }),
        }
    }
    out
}

pub const EMBEDDED: &[(&str, &str)] = &[
    (
        "adobe_reader",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/adobe_reader.xml"),
    ),
    (
        "amsn",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/amsn.xml"),
    ),
    (
        "amule",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/amule.xml"),
    ),
    (
        "android_studio",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/android_studio.xml"),
    ),
    (
        "apt",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/apt.xml"),
    ),
    (
        "audacious",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/audacious.xml"),
    ),
    (
        "bash",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/bash.xml"),
    ),
    (
        "beagle",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/beagle.xml"),
    ),
    (
        "brave",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/brave.xml"),
    ),
    (
        "chromium",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/chromium.xml"),
    ),
    (
        "claude",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/claude.xml"),
    ),
    (
        "d4x",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/d4x.xml"),
    ),
    (
        "deepscan",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/deepscan.xml"),
    ),
    (
        "discord",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/discord.xml"),
    ),
    (
        "dnf",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/dnf.xml"),
    ),
    (
        "easytag",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/easytag.xml"),
    ),
    (
        "elinks",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/elinks.xml"),
    ),
    (
        "emesene",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/emesene.xml"),
    ),
    (
        "epiphany",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/epiphany.xml"),
    ),
    (
        "evolution",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/evolution.xml"),
    ),
    (
        "exaile",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/exaile.xml"),
    ),
    (
        "filezilla",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/filezilla.xml"),
    ),
    (
        "firefox",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/firefox.xml"),
    ),
    (
        "fish",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/fish.xml"),
    ),
    (
        "flash",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/flash.xml"),
    ),
    (
        "geary",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/geary.xml"),
    ),
    (
        "gedit",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/gedit.xml"),
    ),
    (
        "gftp",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/gftp.xml"),
    ),
    (
        "gimp",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/gimp.xml"),
    ),
    (
        "gl-117",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/gl-117.xml"),
    ),
    (
        "gnome",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/gnome.xml"),
    ),
    (
        "google_chrome",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/google_chrome.xml"),
    ),
    (
        "google_earth",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/google_earth.xml"),
    ),
    (
        "google_toolbar",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/google_toolbar.xml"),
    ),
    (
        "gpodder",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/gpodder.xml"),
    ),
    (
        "gwenview",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/gwenview.xml"),
    ),
    (
        "hexchat",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/hexchat.xml"),
    ),
    (
        "hippo_opensim_viewer",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/hippo_opensim_viewer.xml"),
    ),
    (
        "internet_explorer",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/internet_explorer.xml"),
    ),
    (
        "java",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/java.xml"),
    ),
    (
        "journald",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/journald.xml"),
    ),
    (
        "kde",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/kde.xml"),
    ),
    (
        "konqueror",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/konqueror.xml"),
    ),
    (
        "libreoffice",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/libreoffice.xml"),
    ),
    (
        "librewolf",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/librewolf.xml"),
    ),
    (
        "liferea",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/liferea.xml"),
    ),
    (
        "links2",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/links2.xml"),
    ),
    (
        "mc",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/mc.xml"),
    ),
    (
        "microsoft_edge",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/microsoft_edge.xml"),
    ),
    (
        "microsoft_office",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/microsoft_office.xml"),
    ),
    (
        "miro",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/miro.xml"),
    ),
    (
        "nautilus",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/nautilus.xml"),
    ),
    (
        "nexuiz",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/nexuiz.xml"),
    ),
    (
        "octave",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/octave.xml"),
    ),
    (
        "openoffice",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/openoffice.xml"),
    ),
    (
        "opera",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/opera.xml"),
    ),
    (
        "pacman",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/pacman.xml"),
    ),
    (
        "paint",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/paint.xml"),
    ),
    (
        "palemoon",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/palemoon.xml"),
    ),
    (
        "pidgin",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/pidgin.xml"),
    ),
    (
        "profanity",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/profanity.xml"),
    ),
    (
        "python",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/python.xml"),
    ),
    (
        "realplayer",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/realplayer.xml"),
    ),
    (
        "recoll",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/recoll.xml"),
    ),
    (
        "rhythmbox",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/rhythmbox.xml"),
    ),
    (
        "safari",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/safari.xml"),
    ),
    (
        "safari_windows",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/safari_windows.xml"),
    ),
    (
        "screenlets",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/screenlets.xml"),
    ),
    (
        "seamonkey",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/seamonkey.xml"),
    ),
    (
        "secondlife_viewer",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/secondlife_viewer.xml"),
    ),
    (
        "silverlight",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/silverlight.xml"),
    ),
    (
        "skype",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/skype.xml"),
    ),
    (
        "slack",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/slack.xml"),
    ),
    (
        "smartftp",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/smartftp.xml"),
    ),
    (
        "snap",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/snap.xml"),
    ),
    (
        "sqlite3",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/sqlite3.xml"),
    ),
    (
        "teamviewer",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/teamviewer.xml"),
    ),
    (
        "thumbnails",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/thumbnails.xml"),
    ),
    (
        "thunderbird",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/thunderbird.xml"),
    ),
    (
        "tortoisesvn",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/tortoisesvn.xml"),
    ),
    (
        "transmission",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/transmission.xml"),
    ),
    (
        "tremulous",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/tremulous.xml"),
    ),
    (
        "vim",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/vim.xml"),
    ),
    (
        "vivaldi",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/vivaldi.xml"),
    ),
    (
        "vlc",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/vlc.xml"),
    ),
    (
        "vscode",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/vscode.xml"),
    ),
    (
        "vuze",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/vuze.xml"),
    ),
    (
        "warzone2100",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/warzone2100.xml"),
    ),
    (
        "waterfox",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/waterfox.xml"),
    ),
    (
        "winamp",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/winamp.xml"),
    ),
    (
        "windows_defender",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/windows_defender.xml"),
    ),
    (
        "windows_explorer",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/windows_explorer.xml"),
    ),
    (
        "windows_media_player",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/windows_media_player.xml"),
    ),
    (
        "wine",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/wine.xml"),
    ),
    (
        "winetricks",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/winetricks.xml"),
    ),
    (
        "winrar",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/winrar.xml"),
    ),
    (
        "winzip",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/winzip.xml"),
    ),
    (
        "wordpad",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/wordpad.xml"),
    ),
    (
        "x11",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/x11.xml"),
    ),
    (
        "xine",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/xine.xml"),
    ),
    (
        "yahoo_messenger",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/yahoo_messenger.xml"),
    ),
    (
        "yum",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/yum.xml"),
    ),
    (
        "zen",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/zen.xml"),
    ),
    (
        "zoom",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/zoom.xml"),
    ),
    (
        "zsh",
        include_str!("../../../../data/washmybelly/bleachbit/cleaners/zsh.xml"),
    ),
];
