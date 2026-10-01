//! Windows: Run/RunOnce keys, the Startup folders, `StartupApproved`,
//! logon and boot tasks, third-party automatic services. Parsers are pure
//! and compiled in every test build.

use std::collections::HashMap;

use crate::core::washmybelly::parse::RegValue;

/// `StartupApproved` values by lower-case entry name: `true` = on.
pub fn approved_map(values: &[RegValue]) -> HashMap<String, bool> {
    values
        .iter()
        .filter_map(|v| {
            super::windows_approved_enabled(&v.data).map(|on| (v.name.to_ascii_lowercase(), on))
        })
        .collect()
}

/// A tiny CSV reader: quoted fields, `""` escapes, commas inside quotes.
pub fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut row = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    cur.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => row.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
        row.push(cur);
        rows.push(row);
    }
    rows
}

/// One task of `schtasks /query /fo csv /v` (triggers merged).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Task {
    pub path: String,
    pub enabled: bool,
    pub author: String,
    pub run: String,
    pub run_as: String,
    pub comment: String,
    /// Starts at logon or at boot.
    pub at_logon: bool,
    pub at_boot: bool,
}

const DISABLED_WORDS: &[&str] = &[
    "disabled",
    "desabilitado",
    "deshabilitado",
    "deaktiviert",
    "désactivé",
    "disattivato",
    "uitgeschakeld",
    "wyłączone",
    "отключено",
    "無効",
    "已禁用",
    "禁用",
];
const LOGON_WORDS: &[&str] = &[
    "logon",
    "log on",
    "logon time",
    "logon",
    "início de sessão",
    "inicio de sesión",
    "anmeldung",
    "ouverture de session",
    "accesso",
    "aanmelding",
    "logowania",
    "входе в систему",
    "ログオン",
    "登录",
];
const BOOT_WORDS: &[&str] = &[
    "system start",
    "start up",
    "startup",
    "inicialização do sistema",
    "inicio del sistema",
    "systemstart",
    "démarrage",
    "avvio",
    "opstarten",
    "uruchomieniu",
    "запуске",
    "起動",
    "启动",
];

fn has_any(text: &str, words: &[&str]) -> bool {
    let t = text.to_lowercase();
    words.iter().any(|w| t.contains(w))
}

/// Tasks with a logon or boot trigger, outside `\Microsoft\`. Columns are
/// read by position (the header is localized); a header row is skipped.
pub fn parse_schtasks(csv: &str) -> Vec<Task> {
    let mut by_path: Vec<Task> = Vec::new();
    for row in csv_rows(csv) {
        if row.len() < 19 || row[1] == "TaskName" || !row[1].starts_with('\\') {
            continue;
        }
        let path = row[1].clone();
        let schedule = row[18].trim();
        let at_logon = has_any(schedule, LOGON_WORDS);
        let at_boot = !at_logon && has_any(schedule, BOOT_WORDS);
        let disabled =
            has_any(row[11].trim(), DISABLED_WORDS) || has_any(row[3].trim(), DISABLED_WORDS);
        match by_path.iter_mut().find(|t| t.path == path) {
            Some(t) => {
                t.at_logon |= at_logon;
                t.at_boot |= at_boot;
            }
            None => by_path.push(Task {
                path,
                enabled: !disabled,
                author: row[7].clone(),
                run: row[8].clone(),
                run_as: row[14].clone(),
                comment: if row[10] == "N/A" {
                    String::new()
                } else {
                    row[10].clone()
                },
                at_logon,
                at_boot,
            }),
        }
    }
    by_path
        .into_iter()
        .filter(|t| {
            (t.at_logon || t.at_boot) && !t.path.to_ascii_lowercase().starts_with("\\microsoft\\")
        })
        .collect()
}

/// `sc.exe query`: service name → (display name, running).
pub fn parse_sc_query(text: &str) -> HashMap<String, (String, bool)> {
    let mut out = HashMap::new();
    let mut name = String::new();
    let mut display = String::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(n) = t.strip_prefix("SERVICE_NAME:") {
            name = n.trim().to_string();
            display.clear();
        } else if let Some(d) = t.strip_prefix("DISPLAY_NAME:") {
            display = d.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("STATE") {
            let code = rest
                .trim_start_matches([' ', ':'])
                .split_whitespace()
                .next()
                .unwrap_or("");
            if !name.is_empty() {
                out.insert(name.clone(), (display.clone(), code == "4"));
            }
        }
    }
    out
}

/// A service from `reg query HKLM\SYSTEM\CurrentControlSet\Services /s`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Service {
    pub name: String,
    pub display: String,
    pub start: u32,
    pub delayed: bool,
    pub image: String,
    pub kind: u32,
}

fn dword(s: &str) -> u32 {
    let s = s.trim();
    s.strip_prefix("0x")
        .map(|h| u32::from_str_radix(h, 16).unwrap_or(0))
        .unwrap_or_else(|| s.parse().unwrap_or(0))
}

/// Services (not drivers) from one or more `reg query` outputs of the
/// Services key; values of the same service are merged.
pub fn parse_services(texts: &[&str]) -> Vec<Service> {
    let mut map: Vec<Service> = Vec::new();
    for text in texts {
        for v in crate::core::washmybelly::parse::reg_query(text) {
            let lower = v.key.to_ascii_lowercase();
            let Some(i) = lower.find("\\services\\") else {
                continue;
            };
            let name = &v.key[i + "\\services\\".len()..];
            if name.is_empty() || name.contains('\\') {
                continue;
            }
            let idx = match map.iter().position(|s| s.name == name) {
                Some(i) => i,
                None => {
                    map.push(Service {
                        name: name.to_string(),
                        ..Default::default()
                    });
                    map.len() - 1
                }
            };
            let s = &mut map[idx];
            match v.name.as_str() {
                "Start" => s.start = dword(&v.data),
                "Type" => s.kind = dword(&v.data),
                "DelayedAutostart" => s.delayed = dword(&v.data) == 1,
                "ImagePath" => s.image = v.data.clone(),
                "DisplayName" => s.display = v.data.clone(),
                _ => {}
            }
        }
    }
    // 0x10 own process, 0x20 shared process (drivers are 0x1/0x2).
    map.into_iter().filter(|s| s.kind & 0x30 != 0).collect()
}

/// Lives in the Windows folder (a component of the system).
pub fn is_system_path(path: &str) -> bool {
    let p = path.trim().trim_start_matches('"').to_ascii_lowercase();
    p.starts_with("%systemroot%")
        || p.starts_with("\\systemroot\\")
        || p.starts_with("system32\\")
        || p.starts_with("%windir%")
        || p.get(1..).is_some_and(|r| r.starts_with(":\\windows\\"))
}

/// The program of a command line: the quoted part, or up to `.exe`.
pub fn program_path(command: &str) -> String {
    let c = command.trim();
    if let Some(rest) = c.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("").to_string();
    }
    let lower = c.to_ascii_lowercase();
    if let Some(i) = lower.find(".exe") {
        return c[..i + 4].to_string();
    }
    c.split_whitespace().next().unwrap_or("").to_string()
}

/// Our Authenticode script prints `status<TAB>subject<TAB>path` per file.
pub fn parse_authenticode(text: &str) -> HashMap<String, (bool, Option<String>)> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            let status = parts.next()?.trim();
            let subject = parts.next()?.trim();
            let path = parts.next()?.trim();
            if path.is_empty() {
                return None;
            }
            let cn = subject
                .split(", ")
                .find_map(|p| p.strip_prefix("CN="))
                .map(|s| s.trim_matches('"').to_string());
            Some((path.to_ascii_lowercase(), (status == "Valid", cn)))
        })
        .collect()
}

/// Fixed script; the paths come in `OMNIGET_WMB_PATHS`, one per line.
pub const AUTHENTICODE_SCRIPT: &str = "$ErrorActionPreference='SilentlyContinue'; foreach ($p in ($env:OMNIGET_WMB_PATHS -split \"`n\")) { if ($p) { $s = Get-AuthenticodeSignature -LiteralPath $p; \"{0}`t{1}`t{2}\" -f $s.Status, $s.SignerCertificate.Subject, $p } }";

#[cfg(windows)]
pub use live::collect;

#[cfg(windows)]
mod live {
    use std::path::{Path, PathBuf};

    use super::super::actions::Action;
    use super::super::{file_time, looks_like_updater, Collected, Env, StartupItem, Sys};
    use super::*;
    use crate::core::washmybelly::parse::reg_query;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
    const WOW_RUN: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
    const WOW_RUN_ONCE: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\RunOnce";

    fn query(sys: &dyn Sys, key: &str) -> Vec<RegValue> {
        match sys.run(&argv_array!["reg", "query", key]) {
            Ok(o) if o.ok => reg_query(&o.stdout),
            _ => Vec::new(),
        }
    }

    fn approved(sys: &dyn Sys, hive: &str, sub: &str) -> HashMap<String, bool> {
        approved_map(&query(
            sys,
            &format!("{hive}\\{}\\{sub}", super::super::actions::APPROVED_BASE),
        ))
    }

    pub fn collect(sys: &dyn Sys, env: &Env, include_system: bool) -> Collected {
        let _ = env;
        let mut notes = Vec::new();
        let mut items: Vec<StartupItem> = Vec::new();

        // Run and RunOnce.
        let keys = [
            ("HKCU", RUN, "Run", "run"),
            ("HKCU", RUN_ONCE, "Run", "run_once"),
            ("HKLM", RUN, "Run", "run"),
            ("HKLM", WOW_RUN, "Run32", "run"),
            ("HKLM", RUN_ONCE, "Run", "run_once"),
            ("HKLM", WOW_RUN_ONCE, "Run32", "run_once"),
        ];
        let mut approved_cache: HashMap<(String, String), HashMap<String, bool>> = HashMap::new();
        for (hive, key, sub, kind) in keys {
            let full = format!("{hive}\\{key}");
            for v in query(sys, &full) {
                if v.name.is_empty() {
                    continue;
                }
                let map = approved_cache
                    .entry((hive.into(), sub.into()))
                    .or_insert_with(|| approved(sys, hive, sub));
                let enabled = map
                    .get(&v.name.to_ascii_lowercase())
                    .copied()
                    .unwrap_or(true);
                let program = program_path(&v.data);
                let system = is_system_path(&program);
                if system && !include_system {
                    continue;
                }
                let mut it = StartupItem {
                    id: format!("startup:{kind}:{full}\\{}", v.name),
                    name: v.name.clone(),
                    kind: kind.into(),
                    scope: if hive == "HKCU" {
                        "user".into()
                    } else {
                        "system".into()
                    },
                    location: full.clone(),
                    command: v.data.clone(),
                    enabled,
                    installed: file_time(Path::new(&expand(&program))),
                    third_party: !system,
                    recommended: !system && looks_like_updater(&format!("{} {}", v.name, v.data)),
                    needs_admin: hive == "HKLM",
                    ..Default::default()
                };
                if kind == "run_once" {
                    it.reason = Some("run_once".into());
                } else if system {
                    it.reason = Some("system_item".into());
                } else {
                    it.can_disable = true;
                    it.can_enable = true;
                    let mk = |enable| Action::WinApproved {
                        hive: hive.into(),
                        subkey: sub.into(),
                        name: v.name.clone(),
                        enable,
                    };
                    it.off = Some(mk(false));
                    it.on = Some(mk(true));
                }
                items.push(it);
            }
        }

        // Startup folders.
        let folders = [
            (
                std::env::var_os("APPDATA").map(|a| {
                    PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\Startup")
                }),
                "HKCU",
            ),
            (
                std::env::var_os("ProgramData").map(|a| {
                    PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\StartUp")
                }),
                "HKLM",
            ),
        ];
        for (dir, hive) in folders {
            let Some(dir) = dir else { continue };
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let map = approved(sys, hive, "StartupFolder");
            for e in rd.flatten() {
                let file = e.file_name().to_string_lossy().into_owned();
                if file.eq_ignore_ascii_case("desktop.ini") {
                    continue;
                }
                let path = e.path();
                let name = Path::new(&file)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or(file.clone());
                items.push(StartupItem {
                    id: format!("startup:startup_folder:{}", path.display()),
                    name,
                    kind: "startup_folder".into(),
                    scope: if hive == "HKCU" {
                        "user".into()
                    } else {
                        "system".into()
                    },
                    location: path.to_string_lossy().into_owned(),
                    command: path.to_string_lossy().into_owned(),
                    enabled: map.get(&file.to_ascii_lowercase()).copied().unwrap_or(true),
                    installed: file_time(&path),
                    third_party: true,
                    needs_admin: hive == "HKLM",
                    can_disable: true,
                    can_enable: true,
                    off: Some(Action::WinApproved {
                        hive: hive.into(),
                        subkey: "StartupFolder".into(),
                        name: file.clone(),
                        enable: false,
                    }),
                    on: Some(Action::WinApproved {
                        hive: hive.into(),
                        subkey: "StartupFolder".into(),
                        name: file.clone(),
                        enable: true,
                    }),
                    ..Default::default()
                });
            }
        }

        // Logon and boot tasks.
        let me = std::env::var("USERNAME")
            .unwrap_or_default()
            .to_ascii_lowercase();
        match sys.run(&argv_array![
            "schtasks", "/query", "/fo", "csv", "/v", "/nh"
        ]) {
            Ok(o) if o.ok => {
                for t in parse_schtasks(&o.stdout) {
                    let run_as = t.run_as.to_ascii_lowercase();
                    let mine =
                        !me.is_empty() && (run_as == me || run_as.ends_with(&format!("\\{me}")));
                    let name = t.path.rsplit('\\').next().unwrap_or(&t.path).to_string();
                    let valid = super::super::valid_task_path(&t.path);
                    items.push(StartupItem {
                        id: format!("startup:task:{}", t.path),
                        name,
                        kind: "task".into(),
                        scope: if mine { "user".into() } else { "system".into() },
                        location: t.path.clone(),
                        command: t.run.clone(),
                        enabled: t.enabled,
                        signer: (!t.author.is_empty() && t.author != "N/A")
                            .then(|| t.author.clone()),
                        third_party: true,
                        recommended: looks_like_updater(&format!("{} {}", t.path, t.run)),
                        needs_admin: !mine,
                        can_disable: valid,
                        can_enable: valid,
                        reason: (!valid).then(|| "unsupported_name".into()),
                        off: valid.then(|| Action::WinTask {
                            path: t.path.clone(),
                            enable: false,
                            admin: !mine,
                        }),
                        on: valid.then(|| Action::WinTask {
                            path: t.path.clone(),
                            enable: true,
                            admin: !mine,
                        }),
                        ..Default::default()
                    });
                }
            }
            Ok(o) => notes.push(format!("tasks_unavailable: {}", o.stderr.trim())),
            Err(e) => notes.push(format!("tasks_unavailable: {e}")),
        }

        // Third-party automatic services: listed, and "set to manual" only.
        let svc_key = r"HKLM\SYSTEM\CurrentControlSet\Services";
        let texts: Vec<String> = [
            "Start",
            "Type",
            "ImagePath",
            "DisplayName",
            "DelayedAutostart",
        ]
        .iter()
        .filter_map(|v| {
            sys.run(&argv_array!["reg", "query", svc_key, "/s", "/v", *v])
                .ok()
                .map(|o| o.stdout)
        })
        .collect();
        let running = sys
            .run(&argv_array![
                "sc.exe", "query", "type=", "service", "state=", "all"
            ])
            .map(|o| parse_sc_query(&o.stdout))
            .unwrap_or_default();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        for s in parse_services(&refs) {
            if s.start != 2 || is_system_path(&s.image) || s.image.is_empty() {
                continue;
            }
            let protected = super::super::service_argv(&s.name, "manual", true).is_err();
            let display = if s.display.is_empty() || s.display.starts_with('@') {
                s.name.clone()
            } else {
                s.display.clone()
            };
            let program = program_path(&s.image);
            items.push(StartupItem {
                id: format!("startup:service:{}", s.name),
                name: display,
                kind: "service".into(),
                scope: "system".into(),
                location: format!("{svc_key}\\{}", s.name),
                command: s.image.clone(),
                enabled: true,
                running: running.get(&s.name).map(|r| r.1),
                installed: file_time(Path::new(&expand(&program))),
                third_party: true,
                recommended: false,
                needs_admin: true,
                can_disable: !protected,
                reason: Some(if protected {
                    "protected".into()
                } else {
                    "service_manual".into()
                }),
                off: (!protected).then(|| Action::WinService {
                    name: s.name.clone(),
                    mode: "manual".into(),
                }),
                ..Default::default()
            });
        }

        sign(sys, &mut items);
        Collected { items, notes }
    }

    fn expand(p: &str) -> String {
        let mut out = p.to_string();
        for var in [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
            "LOCALAPPDATA",
            "APPDATA",
            "USERPROFILE",
        ] {
            if let Ok(v) = std::env::var(var) {
                let pat = format!("%{var}%");
                if let Some(i) = out.to_ascii_lowercase().find(&pat.to_ascii_lowercase()) {
                    out.replace_range(i..i + pat.len(), &v);
                }
            }
        }
        out
    }

    /// One PowerShell call for every program's Authenticode signature.
    fn sign(sys: &dyn Sys, items: &mut [StartupItem]) {
        let paths: Vec<String> = items
            .iter()
            .filter(|i| i.third_party)
            .map(|i| expand(&program_path(&i.command)))
            .filter(|p| Path::new(p).is_file() && !p.contains('\n'))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if paths.is_empty() {
            return;
        }
        let Ok(o) = sys.run_env(
            &argv_array![
                "powershell",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                AUTHENTICODE_SCRIPT
            ],
            &[("OMNIGET_WMB_PATHS".into(), paths.join("\n"))],
        ) else {
            return;
        };
        let sigs = parse_authenticode(&o.stdout);
        for it in items.iter_mut() {
            let key = expand(&program_path(&it.command)).to_ascii_lowercase();
            if let Some((valid, cn)) = sigs.get(&key) {
                it.signer_verified = *valid;
                if cn.is_some() {
                    it.signer = cn.clone();
                }
            }
        }
    }
}
