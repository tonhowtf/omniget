use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use super::catalog::{Builds, Fact, Item};
use crate::core::washmybelly::parse::{self, RegValue};
use crate::core::washmybelly::startup::Sys;
use crate::core::washmybelly::Os;

#[derive(Debug, Clone)]
pub struct Env {
    pub os: Os,
    pub home: PathBuf,
    pub root: PathBuf,
    pub path_dirs: Vec<PathBuf>,
    pub user: String,
    pub system_root: PathBuf,
}

impl Env {
    pub fn real() -> Env {
        let home = dirs::home_dir().unwrap_or_default();
        let path_dirs = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        Env {
            os: super::super::current_os(),
            home,
            root: PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }),
            path_dirs,
            user,
            system_root: PathBuf::from(
                std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()),
            ),
        }
    }

    pub fn sys_path(&self, p: &str) -> PathBuf {
        self.root.join(p.trim_start_matches('/'))
    }

    pub fn expand(&self, s: &str) -> String {
        let home = self.home.to_string_lossy();
        let s = if s == "~" {
            home.to_string()
        } else if let Some(rest) = s.strip_prefix("~/") {
            format!("{home}/{rest}")
        } else {
            s.to_string()
        };
        s.replace("$HOME", &home).replace("$USER", &self.user)
    }

    pub fn has_command(&self, cmd: &str) -> bool {
        if cmd.contains('/') {
            return Path::new(cmd).is_file();
        }
        let exts: &[&str] = if cfg!(windows) {
            &["", ".exe", ".cmd", ".bat"]
        } else {
            &[""]
        };
        self.path_dirs
            .iter()
            .any(|d| exts.iter().any(|e| d.join(format!("{cmd}{e}")).is_file()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactState {
    Applied,
    NotApplied,
    Absent,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum ItemState {
    Applied,
    NotApplied,
    Unknown(String),
    NotPresent,
}

impl ItemState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemState::Applied => "applied",
            ItemState::NotApplied => "not_applied",
            ItemState::Unknown(_) => "unknown",
            ItemState::NotPresent => "not_present",
        }
    }
}

pub fn readable(it: &Item) -> bool {
    !it.cleanup
        && it
            .parts
            .iter()
            .any(|p| !matches!(p.fact, Fact::CommandExists { .. }))
        && it
            .parts
            .iter()
            .all(|p| !matches!(p.fact, Fact::Opaque { .. } | Fact::TccReset { .. }))
}

pub fn reg_subkeys(text: &str) -> Vec<String> {
    let mut lines = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with(' '));
    let _parent = lines.next();
    lines
        .filter_map(|l| l.rsplit('\\').next().map(str::to_string))
        .collect()
}

pub fn reg_data_equal(kind: &str, shown: &str, wanted: &str) -> bool {
    match kind.to_ascii_uppercase().as_str() {
        "REG_DWORD" | "REG_QWORD" => match (reg_number(shown), reg_number(wanted)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
        "REG_BINARY" => shown.eq_ignore_ascii_case(wanted),
        "REG_MULTI_SZ" => {
            shown == wanted || shown.replace("\\0", "\n") == wanted.replace("\\0", "\n")
        }
        _ => shown == wanted,
    }
}

pub fn reg_number(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s
            .parse::<i64>()
            .ok()
            .map(|n| n as u64 & if n < 0 { 0xffff_ffff } else { u64::MAX }),
    }
}

pub fn service_mode(values: &[RegValue]) -> Option<String> {
    let get = |n: &str| {
        values
            .iter()
            .find(|v| v.name.eq_ignore_ascii_case(n))
            .and_then(|v| reg_number(&v.data))
    };
    Some(
        match get("Start")? {
            0 => "boot",
            1 => "system",
            2 if get("DelayedAutostart") == Some(1) => "delayed",
            2 => "automatic",
            3 => "manual",
            4 => "disabled",
            _ => return None,
        }
        .into(),
    )
}

pub fn parse_tasks(text: &str) -> Vec<(String, String, bool)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.trim().splitn(3, '|');
            let (path, name, state) = (it.next()?, it.next()?, it.next()?);
            (!name.is_empty()).then(|| {
                (
                    path.to_string(),
                    name.to_string(),
                    !state.trim().eq_ignore_ascii_case("Disabled"),
                )
            })
        })
        .collect()
}

pub fn matching_tasks<'a>(
    tasks: &'a [(String, String, bool)],
    path: &str,
    name: &str,
) -> Vec<&'a (String, String, bool)> {
    let (p, n) = (path.to_lowercase(), name.to_lowercase());
    tasks
        .iter()
        .filter(|(tp, tn, _)| {
            crate::core::washmybelly::fsops::wildcard(&p, &tp.to_lowercase())
                && crate::core::washmybelly::fsops::wildcard(&n, &tn.to_lowercase())
        })
        .collect()
}

pub fn hosts_blocks(text: &str, domain: &str) -> bool {
    text.lines().any(|l| {
        let l = l.split('#').next().unwrap_or("");
        let mut w = l.split_whitespace();
        matches!(w.next(), Some("0.0.0.0" | "::" | "127.0.0.1"))
            && w.any(|d| d.eq_ignore_ascii_case(domain))
    })
}

pub fn defaults_equal(kind: Option<&str>, shown: &str, wanted: &str) -> bool {
    let shown = shown.trim();
    match kind {
        Some("bool" | "boolean") => {
            let want = matches!(wanted.to_ascii_lowercase().as_str(), "true" | "yes" | "1");
            let have = matches!(shown.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
            want == have
                && matches!(
                    shown,
                    "0" | "1" | "true" | "false" | "YES" | "NO" | "yes" | "no"
                )
        }
        Some("int" | "integer") => {
            shown.parse::<i64>().ok() == wanted.parse::<i64>().ok() && shown.parse::<i64>().is_ok()
        }
        Some("float") => match (shown.parse::<f64>(), wanted.parse::<f64>()) {
            (Ok(a), Ok(b)) => (a - b).abs() < 1e-9,
            _ => false,
        },
        _ => shown == wanted,
    }
}

pub fn systemd_state(stdout: &str, stderr: &str) -> FactState {
    let s = stdout.lines().next().unwrap_or("").trim();
    match s {
        "disabled" | "masked" | "masked-runtime" => FactState::Applied,
        "enabled" | "enabled-runtime" | "static" | "indirect" | "generated" | "alias"
        | "linked" | "linked-runtime" => FactState::NotApplied,
        "not-found" => FactState::Absent,
        _ if stderr.contains("No such file")
            || stderr.contains("not found")
            || stderr.contains("does not exist") =>
        {
            FactState::Absent
        }
        other => FactState::Unknown(format!("systemctl says {other:?}")),
    }
}

pub fn firefox_pref_line(name: &str, value: &str) -> String {
    format!("user_pref(\"{name}\", {value});")
}

pub fn firefox_profiles(home: &Path) -> Vec<PathBuf> {
    let roots = [
        ".mozilla/firefox",
        ".var/app/org.mozilla.firefox/.mozilla/firefox",
        "snap/firefox/common/.mozilla/firefox",
    ];
    let mut out = Vec::new();
    for r in roots {
        let Ok(rd) = std::fs::read_dir(home.join(r)) else {
            continue;
        };
        let mut found: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| is_profile_name(&e.file_name().to_string_lossy()))
            .map(|e| e.path())
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

fn is_profile_name(n: &str) -> bool {
    let b = n.as_bytes();
    b.len() > 9
        && b[..8]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && b[8] == b'.'
}

pub fn jsonc_parse(text: &str) -> Option<serde_json::Value> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_str = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    static TRAILING: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r",(\s*[}\]])").unwrap());
    serde_json::from_str(&TRAILING.replace_all(&out, "$1")).ok()
}

pub fn par_map<T: Sync, R: Send>(inputs: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = 8.min(inputs.len().max(1));
    let chunk = inputs.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = inputs
            .chunks(chunk)
            .map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

pub fn reg_read_key(sys: &dyn Sys, key: &str) -> Result<Option<Vec<RegValue>>, String> {
    let o = sys.run(&["reg".to_string(), "query".to_string(), key.to_string()])?;
    if o.ok {
        return Ok(Some(
            parse::reg_query(&o.stdout)
                .into_iter()
                .filter(|v| v.key.eq_ignore_ascii_case(key) || same_key(&v.key, key))
                .collect(),
        ));
    }
    let err = o.stderr.to_lowercase();
    if err.contains("denied") || err.contains("negado") || err.contains("refusé") {
        return Err(format!("cannot read {key}: access denied"));
    }
    Ok(None)
}

pub fn same_key(a: &str, b: &str) -> bool {
    long_key(a).eq_ignore_ascii_case(&long_key(b))
}

pub fn long_key(k: &str) -> String {
    let (root, rest) = k.split_once('\\').unwrap_or((k, ""));
    let root = match root.to_ascii_uppercase().as_str() {
        "HKLM" => "HKEY_LOCAL_MACHINE".to_string(),
        "HKCU" => "HKEY_CURRENT_USER".to_string(),
        "HKU" => "HKEY_USERS".to_string(),
        "HKCR" => "HKEY_CLASSES_ROOT".to_string(),
        other => other.to_string(),
    };
    if rest.is_empty() {
        root
    } else {
        format!("{root}\\{}", rest.trim_end_matches('\\'))
    }
}

pub fn reg_value<'a>(values: &'a [RegValue], name: &str) -> Option<&'a RegValue> {
    let default = name.is_empty() || name.eq_ignore_ascii_case("(default)");
    values.iter().find(|v| {
        if default {
            v.name.is_empty()
        } else {
            v.name.eq_ignore_ascii_case(name)
        }
    })
}

pub const SERVICES_KEY: &str = "HKLM\\SYSTEM\\CurrentControlSet\\Services";
pub const DISALLOW_RUN_KEY: &str =
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer\\DisallowRun";

pub fn store_apps_argv() -> Vec<String> {
    vec![
        "powershell".into(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        "Get-AppxPackage | ForEach-Object { $_.Name }".into(),
    ]
}

pub fn task_list_argv() -> Vec<String> {
    vec![
        "powershell".into(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        "Get-ScheduledTask | ForEach-Object { $_.TaskPath + '|' + $_.TaskName + '|' + [string]$_.State }".into(),
    ]
}

pub fn read_tasks(sys: &dyn Sys) -> Result<Vec<(String, String, bool)>, String> {
    let o = sys.run(&task_list_argv())?;
    if !o.ok {
        return Err(format!("Get-ScheduledTask failed: {}", o.stderr.trim()));
    }
    Ok(parse_tasks(&o.stdout))
}

pub fn defaults_read(
    sys: &dyn Sys,
    env: &Env,
    domain: &str,
    key: &str,
) -> Result<Option<String>, String> {
    let o = sys.run(&[
        "defaults".into(),
        "read".into(),
        env.expand(domain),
        key.to_string(),
    ])?;
    if o.ok {
        Ok(Some(o.stdout.trim_end_matches('\n').to_string()))
    } else if o.stderr.contains("does not exist") {
        Ok(None)
    } else {
        Err(format!("defaults read {domain} {key}: {}", o.stderr.trim()))
    }
}

pub type KeyRead = Result<Option<Vec<RegValue>>, String>;
pub type Task = (String, String, bool);

pub struct Reader<'a> {
    pub sys: &'a dyn Sys,
    pub env: &'a Env,
    reg: Mutex<HashMap<String, KeyRead>>,
    tasks: Mutex<Option<Result<Vec<Task>, String>>>,
    services: Mutex<Option<Result<Vec<String>, String>>>,
    apps: Mutex<Option<Result<Vec<String>, String>>>,
    pub build: Option<u32>,
}

impl<'a> Reader<'a> {
    pub fn new(sys: &'a dyn Sys, env: &'a Env) -> Reader<'a> {
        let mut r = Reader {
            sys,
            env,
            reg: Mutex::new(HashMap::new()),
            tasks: Mutex::new(None),
            services: Mutex::new(None),
            apps: Mutex::new(None),
            build: None,
        };
        if env.os == Os::Windows {
            r.build = r
                .key("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
                .ok()
                .flatten()
                .and_then(|v| {
                    reg_value(&v, "CurrentBuild").and_then(|x| x.data.trim().parse().ok())
                });
        }
        r
    }

    pub fn prefetch(&self, items: &[&Item]) {
        let mut keys: Vec<String> = Vec::new();
        for it in items {
            for p in &it.parts {
                match &p.fact {
                    Fact::RegSet { key, .. }
                    | Fact::RegDelete { key, .. }
                    | Fact::RegKeyGone { key } => keys.push(key.clone()),
                    Fact::Service { name, .. } => keys.push(format!("{SERVICES_KEY}\\{name}")),
                    Fact::ServiceReg { name, .. } if !name.contains('*') => {
                        keys.push(format!("{SERVICES_KEY}\\{name}"))
                    }
                    _ => {}
                }
            }
        }
        keys.sort_by_key(|k| k.to_ascii_lowercase());
        keys.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let got = par_map(&keys, |k| {
            (k.to_ascii_lowercase(), reg_read_key(self.sys, k))
        });
        if let Ok(mut m) = self.reg.lock() {
            m.extend(got);
        }
    }

    pub fn key(&self, key: &str) -> Result<Option<Vec<RegValue>>, String> {
        let k = key.to_ascii_lowercase();
        if let Some(v) = self.reg.lock().ok().and_then(|m| m.get(&k).cloned()) {
            return v;
        }
        let v = reg_read_key(self.sys, key);
        if let Ok(mut m) = self.reg.lock() {
            m.insert(k, v.clone());
        }
        v
    }

    fn tasks(&self) -> Result<Vec<(String, String, bool)>, String> {
        let mut g = self.tasks.lock().map_err(|_| "lock")?;
        g.get_or_insert_with(|| read_tasks(self.sys)).clone()
    }

    fn service_names(&self) -> Result<Vec<String>, String> {
        let mut g = self.services.lock().map_err(|_| "lock")?;
        g.get_or_insert_with(|| {
            let o = self
                .sys
                .run(&["reg".into(), "query".into(), SERVICES_KEY.into()])?;
            if o.ok {
                Ok(reg_subkeys(&o.stdout))
            } else {
                Err("cannot list services".into())
            }
        })
        .clone()
    }

    fn store_apps(&self) -> Result<Vec<String>, String> {
        let mut g = self.apps.lock().map_err(|_| "lock")?;
        g.get_or_insert_with(|| {
            let o = self.sys.run(&store_apps_argv())?;
            if o.ok {
                Ok(o.stdout
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect())
            } else {
                Err(format!("Get-AppxPackage failed: {}", o.stderr.trim()))
            }
        })
        .clone()
    }

    pub fn service_instances(&self, name: &str) -> Result<Vec<String>, String> {
        if !name.contains('*') {
            return Ok(vec![name.to_string()]);
        }
        let pat = name.to_lowercase();
        Ok(self
            .service_names()?
            .into_iter()
            .filter(|s| crate::core::washmybelly::fsops::wildcard(&pat, &s.to_lowercase()))
            .collect())
    }

    pub fn fact(&self, f: &Fact) -> FactState {
        match self.fact_inner(f) {
            Ok(s) => s,
            Err(e) => FactState::Unknown(e),
        }
    }

    fn fact_inner(&self, f: &Fact) -> Result<FactState, String> {
        let yes = |b: bool| {
            if b {
                FactState::Applied
            } else {
                FactState::NotApplied
            }
        };
        Ok(match f {
            Fact::RegSet {
                key,
                name,
                kind,
                data,
                ..
            } => match self.key(key)? {
                None => FactState::NotApplied,
                Some(v) => {
                    yes(reg_value(&v, name).is_some_and(|x| reg_data_equal(kind, &x.data, data)))
                }
            },
            Fact::RegDelete { key, name, .. } => match self.key(key)? {
                None => FactState::Applied,
                Some(v) => yes(reg_value(&v, name).is_none()),
            },
            Fact::RegKeyGone { key } => yes(self.key(key)?.is_none()),
            Fact::Service { name, .. } => match self.key(&format!("{SERVICES_KEY}\\{name}"))? {
                None => FactState::Absent,
                Some(v) => match service_mode(&v) {
                    Some(m) => yes(m == "disabled"),
                    None => FactState::Unknown(format!("{name}: no start type")),
                },
            },
            Fact::ServiceReg { name, .. } => {
                let names = self.service_instances(name)?;
                let mut states = Vec::new();
                for n in names {
                    if let Some(v) = self.key(&format!("{SERVICES_KEY}\\{n}"))? {
                        states.push(service_mode(&v).as_deref() == Some("disabled"));
                    }
                }
                if states.is_empty() {
                    FactState::Absent
                } else {
                    yes(states.iter().all(|s| *s))
                }
            }
            Fact::Task { path, name, .. } => {
                let tasks = self.tasks()?;
                let m = matching_tasks(&tasks, path, name);
                if m.is_empty() {
                    FactState::Absent
                } else {
                    yes(m.iter().all(|t| !t.2))
                }
            }
            Fact::HostsBlock { domain } => {
                let p = self
                    .env
                    .system_root
                    .join("System32")
                    .join("drivers")
                    .join("etc")
                    .join("hosts");
                let text = std::fs::read_to_string(&p).map_err(|e| format!("hosts: {e}"))?;
                yes(hosts_blocks(&text, domain))
            }
            Fact::StoreApp { package } => yes(!self
                .store_apps()?
                .iter()
                .any(|a| a.eq_ignore_ascii_case(package))),
            Fact::DisallowRun { exe } => match self.key(DISALLOW_RUN_KEY)? {
                None => FactState::NotApplied,
                Some(v) => yes(v.iter().any(|x| x.data.eq_ignore_ascii_case(exe))),
            },
            Fact::Defaults {
                domain,
                key,
                kind,
                value,
                ..
            } => match defaults_read(self.sys, self.env, domain, key)? {
                None => FactState::NotApplied,
                Some(shown) => yes(defaults_equal(kind.as_deref(), &shown, value)),
            },
            Fact::DefaultsDelete { domain, key, .. } => {
                yes(defaults_read(self.sys, self.env, domain, key)?.is_none())
            }
            Fact::Systemd { unit } => {
                let o = self.sys.run(&[
                    "systemctl".into(),
                    "is-enabled".into(),
                    "--".into(),
                    unit.clone(),
                ])?;
                systemd_state(&o.stdout, &o.stderr)
            }
            Fact::Package { manager, name } => {
                let argv: Vec<String> = match manager.as_str() {
                    "apt" => vec![
                        "dpkg-query".into(),
                        "-W".into(),
                        "--showformat=${db:Status-Status}".into(),
                        "--".into(),
                        name.clone(),
                    ],
                    "pacman" => vec!["pacman".into(), "-Q".into(), "--".into(), name.clone()],
                    _ => vec!["rpm".into(), "-q".into(), "--".into(), name.clone()],
                };
                let o = self.sys.run(&argv)?;
                let installed = o.ok && (manager != "apt" || o.stdout.trim() == "installed");
                yes(!installed)
            }
            Fact::FirefoxPref { name, value } => {
                let profiles = firefox_profiles(&self.env.home);
                if profiles.is_empty() {
                    FactState::Absent
                } else {
                    let line = firefox_pref_line(name, value);
                    yes(profiles.iter().all(|p| {
                        std::fs::read_to_string(p.join("user.js"))
                            .is_ok_and(|t| t.lines().any(|l| l == line))
                    }))
                }
            }
            Fact::VsCodeSetting { name, json } => {
                let files = [
                    ".config/Code/User/settings.json",
                    ".var/app/com.visualstudio.code/config/Code/User/settings.json",
                ];
                let want: serde_json::Value =
                    serde_json::from_str(json).map_err(|e| format!("{name}: {e}"))?;
                let mut found = false;
                let mut all = true;
                for f in files {
                    let Ok(text) = std::fs::read_to_string(self.env.home.join(f)) else {
                        continue;
                    };
                    found = true;
                    let v = jsonc_parse(&text).ok_or_else(|| format!("{f}: not readable JSON"))?;
                    all &= v.get(name) == Some(&want);
                }
                if found {
                    yes(all)
                } else {
                    FactState::Absent
                }
            }
            Fact::ConfigLine { path, on, .. } => {
                match std::fs::read_to_string(self.env.sys_path(path)) {
                    Err(_) => FactState::Absent,
                    Ok(t) => yes(t.lines().any(|l| l.trim() == on)),
                }
            }
            Fact::CommandExists { command } => {
                if self.env.has_command(command) {
                    FactState::Applied
                } else {
                    FactState::Absent
                }
            }
            Fact::TccReset { .. } => {
                FactState::Unknown("a one-time reset: there is no state to read".into())
            }
            Fact::Opaque { what } => {
                FactState::Unknown(format!("free-form code ({what}) cannot be read"))
            }
        })
    }

    fn in_range(&self, b: &Builds) -> bool {
        self.build.is_none_or(|n| b.contains(n))
    }

    pub fn item(&self, it: &Item) -> ItemState {
        if it.os != self.env.os {
            return ItemState::Unknown("another system".into());
        }
        let mut states = Vec::new();
        for p in it.parts.iter().filter(|p| self.in_range(&p.builds)) {
            let s = self.fact(&p.fact);
            if matches!(p.fact, Fact::CommandExists { .. }) {
                if s == FactState::Absent {
                    return ItemState::NotPresent;
                }
                continue;
            }
            states.push(s);
        }
        let considered = it
            .parts
            .iter()
            .any(|p| !matches!(p.fact, Fact::CommandExists { .. }) && self.in_range(&p.builds));
        if !considered && !it.parts.is_empty() {
            return ItemState::NotPresent;
        }
        if it.cleanup {
            return ItemState::Unknown("a one-time cleanup: there is no state to read".into());
        }
        if let Some(FactState::Unknown(r)) =
            states.iter().find(|s| matches!(s, FactState::Unknown(_)))
        {
            return ItemState::Unknown(r.clone());
        }
        let present: Vec<&FactState> = states.iter().filter(|s| **s != FactState::Absent).collect();
        if present.is_empty() {
            return ItemState::NotPresent;
        }
        if present.iter().all(|s| **s == FactState::Applied) {
            ItemState::Applied
        } else {
            ItemState::NotApplied
        }
    }
}
