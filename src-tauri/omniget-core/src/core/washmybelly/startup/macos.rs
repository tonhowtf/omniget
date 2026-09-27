//! macOS: LaunchAgents, LaunchDaemons, Login Items and the Background Task
//! Management database. Parsers are pure and compiled in every test build.

use std::collections::HashMap;

/// `launchctl print <domain>/<label>`: what matters for the list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaunchdState {
    pub running: bool,
    pub pid: Option<u32>,
    pub path: Option<String>,
    pub program: Option<String>,
}

pub fn parse_print(text: &str) -> LaunchdState {
    let mut s = LaunchdState::default();
    // Only the top level of the service block (one tab deep).
    for line in text.lines() {
        let Some(rest) = line.strip_prefix('\t') else {
            continue;
        };
        if rest.starts_with('\t') {
            continue;
        }
        let Some((k, v)) = rest.split_once(" = ") else {
            continue;
        };
        match k.trim() {
            "state" => s.running = v.trim() == "running",
            "pid" => s.pid = v.trim().parse().ok(),
            "path" => s.path = Some(v.trim().to_string()),
            "program" => s.program = Some(v.trim().to_string()),
            _ => {}
        }
    }
    s
}

/// One record of `sfltool dumpbtm`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BtmItem {
    pub uid: i64,
    pub name: String,
    pub developer: Option<String>,
    pub team_id: Option<String>,
    /// `app`, `login item`, `agent`, `daemon`, `legacy agent`, `legacy daemon`,
    /// `developer`, `quicklook`, `spotlight`, `dock tile`...
    pub kind: String,
    pub enabled: bool,
    pub allowed: bool,
    pub identifier: String,
    pub url: Option<String>,
    pub executable: Option<String>,
    pub parent: Option<String>,
    pub bundle_id: Option<String>,
}

fn null_to_none(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty() && v != "(null)").then(|| v.to_string())
}

pub fn parse_btm(text: &str) -> Vec<BtmItem> {
    let mut out = Vec::new();
    let mut uid: i64 = 0;
    let mut cur: Option<BtmItem> = None;
    let flush = |cur: &mut Option<BtmItem>, out: &mut Vec<BtmItem>| {
        if let Some(it) = cur.take() {
            if !it.identifier.is_empty() || !it.name.is_empty() {
                out.push(it);
            }
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Records for UID ") {
            flush(&mut cur, &mut out);
            uid = rest
                .split(':')
                .next()
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or(0);
            continue;
        }
        // Item headers are ` #N:` one space in; embedded identifiers are deeper.
        if line.starts_with(" #") && t.ends_with(':') {
            flush(&mut cur, &mut out);
            cur = Some(BtmItem {
                uid,
                ..Default::default()
            });
            continue;
        }
        let Some(it) = cur.as_mut() else { continue };
        let Some((k, v)) = t.split_once(": ") else {
            continue;
        };
        match k {
            "Name" => it.name = v.trim().to_string(),
            "Developer Name" => it.developer = null_to_none(v),
            "Team Identifier" => it.team_id = null_to_none(v),
            "Type" => it.kind = v.split(" (").next().unwrap_or(v).trim().to_string(),
            "Disposition" => {
                let inside = v.split(']').next().unwrap_or("").trim_start_matches('[');
                let words: Vec<&str> = inside.split(',').map(str::trim).collect();
                it.enabled = words.contains(&"enabled");
                it.allowed = words.contains(&"allowed");
            }
            "Identifier" => it.identifier = v.trim().to_string(),
            "URL" => it.url = null_to_none(v),
            "Executable Path" => it.executable = null_to_none(v),
            "Parent Identifier" => it.parent = null_to_none(v),
            "Bundle Identifier" => it.bundle_id = null_to_none(v),
            _ => {}
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// `file:///Applications/Bar%20Pro.app/` → `/Applications/Bar Pro.app`.
pub fn file_url_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8(out).ok()?;
    let s = s.trim_end_matches('/').to_string();
    s.starts_with('/').then_some(s)
}

/// `codesign -dv --verbose=2` (it writes to stderr).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signature {
    pub signed: bool,
    /// First authority, without the certificate kind and team suffix.
    pub signer: Option<String>,
    pub team_id: Option<String>,
    /// Chained to Apple's root (Developer ID or Apple itself).
    pub apple_chain: bool,
    pub apple: bool,
}

pub fn parse_codesign(text: &str) -> Signature {
    if text.contains("not signed at all") {
        return Signature::default();
    }
    let mut s = Signature::default();
    let mut authorities = Vec::new();
    for line in text.lines() {
        if let Some(a) = line.strip_prefix("Authority=") {
            authorities.push(a.trim().to_string());
        } else if let Some(t) = line.strip_prefix("TeamIdentifier=") {
            let t = t.trim();
            if t != "not set" && !t.is_empty() {
                s.team_id = Some(t.to_string());
            }
        } else if line.starts_with("Identifier=") || line.starts_with("CodeDirectory") {
            s.signed = true;
        }
    }
    if let Some(first) = authorities.first() {
        s.signed = true;
        s.apple = first == "Software Signing" || first.starts_with("Apple ");
        let name = first
            .strip_prefix("Developer ID Application: ")
            .or_else(|| first.strip_prefix("Apple Development: "))
            .or_else(|| first.strip_prefix("Apple Distribution: "))
            .unwrap_or(first);
        let name = match name.rfind(" (") {
            Some(i) if name.ends_with(')') => &name[..i],
            _ => name,
        };
        s.signer = Some(if s.apple {
            "Apple".to_string()
        } else {
            name.to_string()
        });
    }
    s.apple_chain = authorities.iter().any(|a| a == "Apple Root CA");
    s
}

/// What the list needs from a launchd plist (`plutil -convert xml1`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlistInfo {
    pub label: Option<String>,
    pub program: Option<String>,
    pub arguments: Vec<String>,
    pub disabled: bool,
    pub run_at_load: bool,
    pub keep_alive: bool,
}

pub fn parse_plist_xml(text: &str) -> Option<PlistInfo> {
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let dict = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("dict"))?;
    let mut info = PlistInfo::default();
    let mut key: Option<String> = None;
    for n in dict.children().filter(|n| n.is_element()) {
        if n.has_tag_name("key") {
            key = n.text().map(str::to_string);
            continue;
        }
        let Some(k) = key.take() else { continue };
        match k.as_str() {
            "Label" => info.label = n.text().map(str::to_string),
            "Program" => info.program = n.text().map(str::to_string),
            "ProgramArguments" => {
                info.arguments = n
                    .children()
                    .filter(|c| c.has_tag_name("string"))
                    .map(|c| c.text().unwrap_or("").to_string())
                    .collect()
            }
            "Disabled" => info.disabled = n.has_tag_name("true"),
            "RunAtLoad" => info.run_at_load = n.has_tag_name("true"),
            "KeepAlive" => info.keep_alive = n.has_tag_name("true") || n.has_tag_name("dict"),
            _ => {}
        }
    }
    Some(info)
}

impl PlistInfo {
    pub fn executable(&self) -> Option<&str> {
        self.program
            .as_deref()
            .or(self.arguments.first().map(String::as_str))
    }

    pub fn command(&self) -> String {
        if self.arguments.is_empty() {
            self.program.clone().unwrap_or_default()
        } else {
            self.arguments.join(" ")
        }
    }
}

/// Our System Events script prints `name<TAB>path` per line.
pub fn parse_login_items(text: &str) -> Vec<(String, Option<String>)> {
    text.lines()
        .filter_map(|l| {
            let (name, path) = l.split_once('\t').unwrap_or((l, ""));
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            let path = path.trim();
            let path = (path.starts_with('/')).then(|| path.trim_end_matches('/').to_string());
            Some((name.to_string(), path))
        })
        .collect()
}

/// The fixed System Events script behind [`parse_login_items`].
pub const LOGIN_ITEMS_SCRIPT: &str = "tell application \"System Events\"
set out to \"\"
repeat with li in every login item
set p to \"\"
try
set p to (path of li) as text
end try
set out to out & (name of li) & tab & p & linefeed
end repeat
return out
end tell";

pub fn btm_by_url(btm: &[BtmItem]) -> HashMap<String, &BtmItem> {
    btm.iter()
        .filter_map(|b| b.url.as_deref().and_then(file_url_path).map(|p| (p, b)))
        .collect()
}

#[cfg(target_os = "macos")]
pub use live::collect;

#[cfg(target_os = "macos")]
mod live {
    use std::path::{Path, PathBuf};

    use super::super::actions::Action;
    use super::super::{
        file_time, looks_like_updater, print_disabled, Collected, Env, StartupItem, Sys,
    };
    use super::*;

    fn plist_info(sys: &dyn Sys, path: &Path) -> Option<PlistInfo> {
        let out = sys
            .run(&argv![
                "plutil",
                "-convert",
                "xml1",
                "-o",
                "-",
                "--",
                path.to_string_lossy()
            ])
            .ok()?;
        out.ok.then(|| parse_plist_xml(&out.stdout)).flatten()
    }

    fn signature(sys: &dyn Sys, program: &str) -> Signature {
        if !program.starts_with('/') {
            return Signature::default();
        }
        match sys.run(&argv!["codesign", "-dv", "--verbose=2", "--", program]) {
            Ok(o) => parse_codesign(&format!("{}{}", o.stderr, o.stdout)),
            Err(_) => Signature::default(),
        }
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

    fn stem(p: &Path) -> String {
        p.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn plist_items(
        sys: &dyn Sys,
        env: &Env,
        disabled_user: &HashMap<String, bool>,
        disabled_system: &HashMap<String, bool>,
    ) -> Vec<StartupItem> {
        let mut items: Vec<StartupItem> = Vec::new();
        let dirs = [
            (env.home.join("Library/LaunchAgents"), "agent", "user"),
            (PathBuf::from("/Library/LaunchAgents"), "agent", "system"),
            (PathBuf::from("/Library/LaunchDaemons"), "daemon", "system"),
        ];
        for (dir, kind, scope) in &dirs {
            for p in plists(dir) {
                let info = plist_info(sys, &p).unwrap_or_default();
                let label = info.label.clone().unwrap_or_else(|| stem(&p));
                let daemon = *kind == "daemon";
                let overrides = if daemon {
                    disabled_system
                } else {
                    disabled_user
                };
                let enabled = match overrides.get(&label) {
                    Some(off) => !off,
                    None => !info.disabled,
                };
                let apple = label.starts_with("com.apple.");
                let mut it = StartupItem {
                    id: format!("startup:{kind}:{}", p.display()),
                    name: label.clone(),
                    kind: kind.to_string(),
                    scope: scope.to_string(),
                    location: p.to_string_lossy().into_owned(),
                    command: info.command(),
                    enabled,
                    installed: file_time(&p),
                    third_party: !apple,
                    recommended: !apple && looks_like_updater(&label),
                    ..Default::default()
                };
                let program = info.executable().unwrap_or_default().to_string();
                if apple {
                    it.reason = Some("system_item".into());
                } else if daemon {
                    it.needs_admin = true;
                    it.can_disable = true;
                    it.off = Some(Action::LaunchdDaemon {
                        label: label.clone(),
                        plist: p.clone(),
                    });
                    it.reason = Some("daemon_until_restart".into());
                } else {
                    it.can_disable = true;
                    it.can_enable = true;
                    it.off = Some(Action::LaunchdUser {
                        label: label.clone(),
                        plist: p.clone(),
                        enable: false,
                    });
                    it.on = Some(Action::LaunchdUser {
                        label: label.clone(),
                        plist: p.clone(),
                        enable: true,
                    });
                }
                items.push(it);
                // Remember the program for the signer pass.
                if let Some(last) = items.last_mut() {
                    last.command = if last.command.is_empty() {
                        program
                    } else {
                        last.command.clone()
                    };
                }
            }
        }
        items
    }

    pub fn collect(sys: &dyn Sys, env: &Env, include_system: bool) -> Collected {
        let mut notes = Vec::new();
        let user_domain = format!("gui/{}", env.uid);
        let disabled_user = sys
            .run(&argv!["launchctl", "print-disabled", user_domain.clone()])
            .map(|o| print_disabled(&o.stdout))
            .unwrap_or_default();
        let disabled_system = sys
            .run(&argv!["launchctl", "print-disabled", "system"])
            .map(|o| print_disabled(&o.stdout))
            .unwrap_or_default();
        // `sfltool dumpbtm` takes seconds: it runs while the plists are read.
        let (btm_out, mut items) = std::thread::scope(|s| {
            let h = s.spawn(|| sys.run(&argv!["sfltool", "dumpbtm"]));
            let items = plist_items(sys, env, &disabled_user, &disabled_system);
            (
                h.join().unwrap_or_else(|_| Err("sfltool stopped".into())),
                items,
            )
        });
        let btm = match btm_out {
            Ok(o) if o.ok && o.stdout.contains("Records for UID") => parse_btm(&o.stdout),
            Ok(o) => {
                notes.push(format!("btm_unavailable: {}", o.stderr.trim()));
                Vec::new()
            }
            Err(e) => {
                notes.push(format!("btm_unavailable: {e}"));
                Vec::new()
            }
        };
        let by_url = btm_by_url(&btm);

        for it in items.iter_mut() {
            if let Some(b) = by_url.get(&it.location) {
                it.team_id = b.team_id.clone();
                it.signer = b.developer.clone();
                // BTM names the app ("Logitech G HUB"); keep the label when it
                // only names the program file ("node").
                let program = Path::new(&it.command)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !b.name.is_empty()
                    && b.name != program
                    && !it
                        .command
                        .split(' ')
                        .any(|w| w.ends_with(&format!("/{}", b.name)))
                {
                    it.name = b.name.clone();
                }
            }
        }

        // Running state and signer, in parallel (one launchctl + one codesign each).
        std::thread::scope(|s| {
            let chunks = items.chunks_mut(4).collect::<Vec<_>>();
            for chunk in chunks {
                let user_domain = user_domain.clone();
                s.spawn(move || {
                    for it in chunk.iter_mut() {
                        if !it.third_party {
                            continue;
                        }
                        let domain = if it.kind == "daemon" {
                            "system".to_string()
                        } else {
                            user_domain.clone()
                        };
                        if let Ok(o) = sys.run(&argv![
                            "launchctl",
                            "print",
                            format!("{domain}/{}", label_of(it))
                        ]) {
                            it.running = Some(o.ok && parse_print(&o.stdout).running);
                        }
                        let program = program_of(&it.command);
                        let sig = signature(sys, &program);
                        if sig.signed {
                            it.signer = sig.signer.or(it.signer.take());
                            it.team_id = sig.team_id.or(it.team_id.take());
                            it.signer_verified = sig.apple_chain;
                        }
                    }
                });
            }
        });

        // Apple's own agents and daemons, read only.
        if include_system {
            for (dir, kind) in [
                ("/System/Library/LaunchAgents", "agent"),
                ("/System/Library/LaunchDaemons", "daemon"),
            ] {
                let overrides = if kind == "daemon" {
                    &disabled_system
                } else {
                    &disabled_user
                };
                for p in plists(Path::new(dir)) {
                    let label = stem(&p);
                    items.push(StartupItem {
                        id: format!("startup:{kind}:{}", p.display()),
                        name: label.clone(),
                        kind: kind.into(),
                        scope: "system".into(),
                        location: p.to_string_lossy().into_owned(),
                        enabled: !overrides.get(&label).copied().unwrap_or(false),
                        signer: Some("Apple".into()),
                        signer_verified: true,
                        reason: Some("system_item".into()),
                        ..Default::default()
                    });
                }
            }
        }

        // Legacy login items (System Events).
        let mut login_names: Vec<String> = Vec::new();
        match sys.run(&argv!["osascript", "-e", LOGIN_ITEMS_SCRIPT]) {
            Ok(o) if o.ok => {
                for (name, path) in parse_login_items(&o.stdout) {
                    // A path System Events does not know: take it from BTM.
                    let path = path.or_else(|| {
                        btm.iter()
                            .find(|b| b.kind == "app" && b.name == name)
                            .and_then(|b| b.url.as_deref().and_then(file_url_path))
                    });
                    let bt = btm.iter().find(|b| b.kind == "app" && b.name == name);
                    let sig = path
                        .as_deref()
                        .map(|p| signature(sys, p))
                        .unwrap_or_default();
                    let mut it = StartupItem {
                        id: format!("startup:login_item:{name}"),
                        name: name.clone(),
                        kind: "login_item".into(),
                        scope: "user".into(),
                        location: path.clone().unwrap_or_default(),
                        command: path.clone().unwrap_or_default(),
                        enabled: true,
                        signer: sig.signer.or_else(|| bt.and_then(|b| b.developer.clone())),
                        signer_verified: sig.apple_chain,
                        team_id: sig.team_id.or_else(|| bt.and_then(|b| b.team_id.clone())),
                        installed: path.as_deref().and_then(|p| file_time(Path::new(p))),
                        third_party: !sig.apple,
                        ..Default::default()
                    };
                    if path.is_some() {
                        it.can_disable = true;
                        it.can_enable = true;
                        it.off = Some(Action::LoginItem {
                            name: name.clone(),
                            path: path.clone(),
                            enable: false,
                        });
                        it.on = Some(Action::LoginItem {
                            name: name.clone(),
                            path,
                            enable: true,
                        });
                    } else {
                        it.reason = Some("settings_only".into());
                    }
                    login_names.push(name);
                    items.push(it);
                }
            }
            Ok(o) => notes.push(format!("login_items_unavailable: {}", o.stderr.trim())),
            Err(e) => notes.push(format!("login_items_unavailable: {e}")),
        }

        // Background items registered through ServiceManagement that the
        // plists and System Events do not show: switched in System Settings.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for b in &btm {
            let kind = match b.kind.as_str() {
                "login item" => "login_item",
                "agent" => "agent",
                "daemon" => "daemon",
                // An app that opens at login through SMAppService.mainApp.
                "app" if b.enabled => "login_item",
                _ => continue,
            };
            let current_user = b.uid == env.uid as i64 || (kind == "daemon" && b.uid <= 0);
            if !current_user || login_names.contains(&b.name) || !seen.insert(b.identifier.clone())
            {
                continue;
            }
            let apple =
                b.identifier.contains("com.apple.") || b.team_id.is_none() && b.developer.is_none();
            if apple && !include_system {
                continue;
            }
            let location = b
                .url
                .as_deref()
                .and_then(file_url_path)
                .or_else(|| b.parent.clone())
                .or_else(|| b.url.clone())
                .unwrap_or_default();
            items.push(StartupItem {
                id: format!("startup:btm:{}", b.identifier),
                name: b.name.clone(),
                kind: kind.into(),
                scope: if kind == "daemon" {
                    "system".into()
                } else {
                    "user".into()
                },
                location,
                command: b.executable.clone().unwrap_or_default(),
                enabled: b.enabled,
                signer: b.developer.clone(),
                team_id: b.team_id.clone(),
                third_party: !apple,
                reason: Some("settings_only".into()),
                ..Default::default()
            });
        }
        Collected { items, notes }
    }

    fn label_of(it: &StartupItem) -> String {
        match &it.off {
            Some(Action::LaunchdUser { label, .. }) | Some(Action::LaunchdDaemon { label, .. }) => {
                label.clone()
            }
            _ => Path::new(&it.location)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    /// The program of a command line whose arguments were joined with spaces:
    /// the longest leading part that exists on disk.
    fn program_of(command: &str) -> String {
        let words: Vec<&str> = command.split(' ').collect();
        for n in (1..=words.len()).rev() {
            let cand = words[..n].join(" ");
            if Path::new(&cand).exists() {
                return cand;
            }
        }
        words.first().map(|s| s.to_string()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/washmybelly_fixtures/startup")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn print_reads_state_pid_and_path() {
        let s = parse_print(&fx("launchctl_print_running.txt"));
        assert!(s.running);
        assert_eq!(s.pid, Some(84873));
        assert!(s
            .path
            .unwrap()
            .ends_with("com.setapp.DesktopClient.SetappAgent.plist"));
        let idle = parse_print(&fx("launchctl_print_idle.txt"));
        assert!(!idle.running);
        assert_eq!(idle.pid, None);
        assert!(idle.program.unwrap().ends_with("steamclean"));
    }

    #[test]
    fn print_disabled_reads_overrides() {
        let m = super::super::print_disabled(&fx("launchctl_print_disabled.txt"));
        assert_eq!(
            m.get("com.apple.ManagedClientAgent.enrollagent"),
            Some(&true)
        );
        assert_eq!(m.get("com.logi.ghub"), Some(&false));
        assert!(super::super::macos_disabled_lookup(
            &fx("launchctl_print_disabled.txt"),
            "com.apple.ManagedClientAgent.enrollagent"
        ));
        assert!(!super::super::macos_disabled_lookup(
            &fx("launchctl_print_disabled.txt"),
            "not.there"
        ));
    }

    #[test]
    fn btm_reads_records_per_uid() {
        let items = parse_btm(&fx("sfltool_dumpbtm.txt"));
        let daemon = items
            .iter()
            .find(|i| i.identifier == "16.com.bjango.istatmenus.installer")
            .unwrap();
        assert_eq!(daemon.uid, -2);
        assert_eq!(daemon.kind, "legacy daemon");
        assert_eq!(daemon.team_id.as_deref(), Some("Y93TK974AT"));
        assert!(daemon.enabled);
        assert_eq!(
            file_url_path(daemon.url.as_deref().unwrap()).unwrap(),
            "/Library/LaunchDaemons/com.bjango.istatmenus.installer.plist"
        );
        let dev = items.iter().find(|i| i.kind == "developer").unwrap();
        assert!(!dev.enabled);
        let bartender = items.iter().find(|i| i.name == "Bartender Pro").unwrap();
        assert_eq!(bartender.uid, 501);
        assert_eq!(bartender.kind, "app");
        assert!(bartender.enabled);
        assert_eq!(
            file_url_path(bartender.url.as_deref().unwrap()).unwrap(),
            "/Applications/Setapp/Bartender Pro.app"
        );
        let betterzip = items.iter().find(|i| i.name == "BetterZip").unwrap();
        assert!(!betterzip.enabled);
        let helper = items
            .iter()
            .find(|i| i.name == "CleanMyMac Health Monitor")
            .unwrap();
        assert_eq!(helper.kind, "login item");
        assert_eq!(
            helper.parent.as_deref(),
            Some("2.com.macpaw.CleanMyMac-setapp")
        );
        let teams = items.iter().find(|i| i.kind == "agent").unwrap();
        assert_eq!(teams.developer, None, "(null) is none");
        assert_eq!(teams.team_id.as_deref(), Some("UBF8T346G9"));
        let legacy = items.iter().filter(|i| i.kind == "legacy agent").count();
        assert_eq!(legacy, 2);
        let by = btm_by_url(&items);
        assert!(
            by.contains_key("/Users/alex/Library/LaunchAgents/com.google.GoogleUpdater.wake.plist")
        );
    }

    #[test]
    fn codesign_names_the_signer() {
        let s = parse_codesign(&fx("codesign_dv.txt"));
        assert!(s.signed && s.apple_chain && !s.apple);
        assert_eq!(s.signer.as_deref(), Some("Setapp Limited"));
        assert_eq!(s.team_id.as_deref(), Some("MEHY5QF425"));
        let a = parse_codesign(&fx("codesign_dv_apple.txt"));
        assert!(a.apple && a.apple_chain);
        assert_eq!(a.signer.as_deref(), Some("Apple"));
        assert_eq!(a.team_id, None);
        let u = parse_codesign(&fx("codesign_unsigned.txt"));
        assert!(!u.signed && u.signer.is_none());
    }

    #[test]
    fn plist_reads_label_and_program() {
        let p = parse_plist_xml(&fx("launchagent.plist.xml")).unwrap();
        assert_eq!(p.label.as_deref(), Some("com.valvesoftware.steamclean"));
        assert!(p.run_at_load);
        assert!(!p.disabled);
        assert_eq!(p.arguments.len(), 2);
        assert!(p.command().ends_with("steamclean Public"));
        assert!(p.executable().unwrap().ends_with("SteamApps/steamclean"));
    }

    #[test]
    fn login_items_lines() {
        let v = parse_login_items("Bartender Pro\t\nPaste\t/Applications/Setapp/Paste.app/\nA, B\t/Applications/A, B.app\n\n");
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], ("Bartender Pro".into(), None));
        assert_eq!(v[1].1.as_deref(), Some("/Applications/Setapp/Paste.app"));
        assert_eq!(v[2].0, "A, B", "a comma in a name does not split it");
    }
}
