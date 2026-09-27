//! A pretend Windows for the tests: an in-memory registry that answers
//! `reg query/add/delete` the way `reg.exe` prints, plus `sc.exe`,
//! `schtasks`, `powercfg` and `whoami`. Every call is recorded.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use super::super::startup::{Output, Sys};

pub const SID: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";

#[derive(Default)]
pub struct State {
    /// Upper-case canonical key → (name upper → (display name, kind, data)).
    pub keys: BTreeMap<String, BTreeMap<String, (String, String, String)>>,
    pub denied: BTreeSet<String>,
    pub services: BTreeMap<String, String>,
    pub tasks: BTreeMap<String, bool>,
    pub active_scheme: String,
    pub schemes: BTreeSet<String>,
    pub calls: Vec<Vec<String>>,
    pub fail_on: Option<String>,
}

pub struct FakeSys(pub Mutex<State>);

fn ok(stdout: String) -> Output {
    Output {
        ok: true,
        code: Some(0),
        stdout,
        stderr: String::new(),
    }
}

fn err(stderr: &str) -> Output {
    Output {
        ok: false,
        code: Some(1),
        stdout: String::new(),
        stderr: stderr.to_string(),
    }
}

/// `HKCU\x`, `HKEY_USERS\<SID>\x` … → `HKEY_CURRENT_USER\X` style canonical key.
fn canon(key: &str) -> (String, String) {
    let (root, rest) = key.split_once('\\').unwrap_or((key, ""));
    let (long, rest) = match root.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => ("HKEY_CURRENT_USER", rest.to_string()),
        "HKLM" | "HKEY_LOCAL_MACHINE" => ("HKEY_LOCAL_MACHINE", rest.to_string()),
        "HKU" | "HKEY_USERS" => match rest.split_once('\\') {
            Some((sid, tail)) if sid == SID => ("HKEY_CURRENT_USER", tail.to_string()),
            _ => ("HKEY_USERS", rest.to_string()),
        },
        other => panic!("fake reg: hive {other}"),
    };
    let display = if rest.is_empty() {
        long.to_string()
    } else {
        format!("{long}\\{rest}")
    };
    (display.to_ascii_uppercase(), display)
}

/// How reg.exe would print this key back (the form the caller used).
fn shown(query: &str) -> String {
    let (root, rest) = query.split_once('\\').unwrap_or((query, ""));
    let long = match root.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => "HKEY_CURRENT_USER",
        "HKLM" | "HKEY_LOCAL_MACHINE" => "HKEY_LOCAL_MACHINE",
        _ => "HKEY_USERS",
    };
    format!("{long}\\{rest}")
}

impl FakeSys {
    pub fn new() -> FakeSys {
        let s = State {
            active_scheme: super::state::BALANCED.into(),
            ..State::default()
        };
        FakeSys(Mutex::new(s))
    }

    pub fn set(&self, key: &str, name: &str, kind: &str, data: &str) {
        let mut s = self.0.lock().unwrap();
        Self::add_key(&mut s, key);
        let (id, _) = canon(key);
        s.keys.get_mut(&id).unwrap().insert(
            name.to_ascii_uppercase(),
            (name.to_string(), kind.to_string(), data.to_string()),
        );
    }

    pub fn get(&self, key: &str, name: &str) -> Option<(String, String)> {
        let s = self.0.lock().unwrap();
        let (id, _) = canon(key);
        s.keys
            .get(&id)?
            .get(&name.to_ascii_uppercase())
            .map(|(_, k, d)| (k.clone(), d.clone()))
    }

    pub fn has_key(&self, key: &str) -> bool {
        self.0.lock().unwrap().keys.contains_key(&canon(key).0)
    }

    pub fn add_key(s: &mut State, key: &str) {
        let (_, display) = canon(key);
        let parts: Vec<&str> = display.split('\\').collect();
        for i in 2..=parts.len() {
            let k = parts[..i].join("\\").to_ascii_uppercase();
            s.keys.entry(k).or_default();
        }
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        self.0.lock().unwrap().calls.clone()
    }

    fn reg(s: &mut State, a: &[String]) -> Output {
        let verb = a[1].as_str();
        let key = a[2].as_str();
        let (id, _) = canon(key);
        if s.denied.contains(&id) {
            return err("ERROR: Access is denied.");
        }
        let flag = |f: &str| a.iter().any(|x| x.eq_ignore_ascii_case(f));
        let arg = |f: &str| {
            a.iter()
                .position(|x| x.eq_ignore_ascii_case(f))
                .and_then(|i| a.get(i + 1))
                .cloned()
        };
        let name = if flag("/ve") {
            Some(String::new())
        } else {
            arg("/v")
        };
        match verb {
            "query" => {
                let Some(vals) = s.keys.get(&id) else {
                    return err(
                        "ERROR: The system was unable to find the specified registry key or value.",
                    );
                };
                let print = |k: &str,
                             vals: &BTreeMap<String, (String, String, String)>,
                             only: Option<&str>| {
                    let mut out = format!("\r\n{k}\r\n");
                    for (up, (n, kind, d)) in vals {
                        if only.is_some_and(|o| o.to_ascii_uppercase() != *up) {
                            continue;
                        }
                        let shown_name = if n.is_empty() {
                            "(Default)"
                        } else {
                            n.as_str()
                        };
                        let shown_data = if kind == "REG_DWORD" || kind == "REG_QWORD" {
                            format!("0x{:x}", d.parse::<u64>().unwrap())
                        } else {
                            d.clone()
                        };
                        out.push_str(&format!("    {shown_name}    {kind}    {shown_data}\r\n"));
                    }
                    out
                };
                if let Some(n) = &name {
                    if !vals.contains_key(&n.to_ascii_uppercase()) {
                        return err("ERROR: The system was unable to find the specified registry key or value.");
                    }
                    return ok(print(&shown(key), vals, Some(n)));
                }
                let mut out = print(&shown(key), vals, None);
                let prefix = format!("{id}\\");
                let subs: Vec<String> = s
                    .keys
                    .keys()
                    .filter(|k| k.starts_with(&prefix))
                    .cloned()
                    .collect();
                for sub in subs {
                    let rel = &sub[prefix.len()..];
                    let full = format!("{}\\{}", shown(key), rel);
                    if flag("/s") {
                        out.push_str(&print(&full, &s.keys[&sub], None));
                    } else if !rel.contains('\\') {
                        out.push_str(&format!("\r\n{full}\r\n"));
                    }
                }
                ok(out)
            }
            "add" => {
                if s.fail_on
                    .as_deref()
                    .is_some_and(|f| name.as_deref() == Some(f))
                {
                    return err("ERROR: pretend failure");
                }
                Self::add_key(s, key);
                if let Some(n) = name {
                    let kind = arg("/t").unwrap_or_else(|| "REG_SZ".into());
                    let data = arg("/d").unwrap_or_default();
                    s.keys
                        .get_mut(&id)
                        .unwrap()
                        .insert(n.to_ascii_uppercase(), (n, kind, data));
                }
                ok("The operation completed successfully.\r\n".into())
            }
            "delete" => {
                if !s.keys.contains_key(&id) {
                    return err(
                        "ERROR: The system was unable to find the specified registry key or value.",
                    );
                }
                match name {
                    Some(n) => {
                        if s.keys
                            .get_mut(&id)
                            .unwrap()
                            .remove(&n.to_ascii_uppercase())
                            .is_none()
                        {
                            return err("ERROR: The system was unable to find the specified registry key or value.");
                        }
                    }
                    None => {
                        let prefix = format!("{id}\\");
                        s.keys.retain(|k, _| k != &id && !k.starts_with(&prefix));
                    }
                }
                ok("The operation completed successfully.\r\n".into())
            }
            v => panic!("fake reg: verb {v}"),
        }
    }
}

impl Sys for FakeSys {
    fn run(&self, argv: &[String]) -> Result<Output, String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(argv.to_vec());
        let a = argv;
        Ok(match a[0].as_str() {
            "reg" => Self::reg(&mut s, a),
            "sc.exe" if a[1] == "qc" => match s.services.get(&a[2]) {
                Some(m) => {
                    let code = match m.as_str() {
                        "automatic" | "delayed" => "2   AUTO_START",
                        "manual" => "3   DEMAND_START",
                        _ => "4   DISABLED",
                    };
                    ok(format!("[SC] QueryServiceConfig SUCCESS\r\n\r\nSERVICE_NAME: {}\r\n        START_TYPE         : {code}\r\n", a[2]))
                }
                None => Output {
                    ok: false,
                    code: Some(1060),
                    stdout: "[SC] OpenService FAILED 1060:\r\n".into(),
                    stderr: String::new(),
                },
            },
            "sc.exe" if a[1] == "config" => {
                let mode = match a[4].as_str() {
                    "auto" => "automatic",
                    "delayed-auto" => "delayed",
                    "demand" => "manual",
                    other => other,
                };
                s.services.insert(a[2].clone(), mode.into());
                ok("[SC] ChangeServiceConfig SUCCESS\r\n".into())
            }
            "schtasks" if a[1] == "/query" => match s.tasks.get(&a[3]) {
                Some(e) => ok(format!(
                    "<Task><Settings><Enabled>{e}</Enabled></Settings></Task>"
                )),
                None => err("ERROR: The system cannot find the file specified."),
            },
            "schtasks" if a[1] == "/Change" => {
                s.tasks.insert(a[3].clone(), a[4] == "/ENABLE");
                ok("SUCCESS".into())
            }
            "powercfg" => match a[1].as_str() {
                "/getactivescheme" => ok(format!(
                    "Power Scheme GUID: {}  (Plan)\r\n",
                    s.active_scheme
                )),
                "/setactive" => {
                    s.active_scheme = a[2].clone();
                    ok(String::new())
                }
                "/duplicatescheme" => {
                    let g = "5b2cf1a0-1a2b-4c3d-8e9f-0a1b2c3d4e5f".to_string();
                    s.schemes.insert(g.clone());
                    ok(format!(
                        "Power Scheme GUID: {g}  (Ultimate Performance)\r\n"
                    ))
                }
                "/delete" => {
                    s.schemes.remove(&a[2]);
                    ok(String::new())
                }
                other => panic!("fake powercfg {other}"),
            },
            "whoami" => ok(format!("\"desktop-1\\ana\",\"{SID}\"\r\n")),
            other => err(&format!("fake: {other} is not installed")),
        })
    }
}
