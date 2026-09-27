//! Running one privacy item, never the whole script: each fact becomes a
//! typed step (a `reg`/`defaults`/`tccutil` argv run as the person, a file
//! write, or an [`ElevatedOp`] for the administrator helper). Before any
//! step runs, the real previous state (value and type) is read into a
//! [`Snapshot`]; undo puts that back. privacy.sexy's revert code is a
//! separate action, "back to the default" ([`Action::RevertDefault`]).
//!
//! Building steps ([`steps`], [`restore_steps`]) is apart from running
//! them ([`run_user_steps`]), so tests check the exact argv with a fake
//! [`Sys`] and never touch the registry or `defaults`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::catalog::{Fact, Hive, Item, Part, RegRevert};
use super::state::{self, Env, FactState, Reader, SERVICES_KEY};
use crate::core::washmybelly::elevate::ElevatedOp;
use crate::core::washmybelly::startup::Sys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Apply,
    RevertDefault,
}

impl Action {
    pub fn parse(s: &str) -> Result<Action, String> {
        match s {
            "apply" => Ok(Action::Apply),
            "revert_default" => Ok(Action::RevertDefault),
            other => Err(format!("privacy: unknown action {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Apply => "apply",
            Action::RevertDefault => "revert_default",
        }
    }
}

/// The state one step is about to change, as it really was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Prior {
    /// `value` is `(kind, data)`, `None` when the value did not exist.
    Reg {
        key: String,
        name: String,
        value: Option<(String, String)>,
    },
    Service {
        name: String,
        mode: String,
    },
    Task {
        path: String,
        enabled: bool,
    },
    /// `value` is `(type from read-type, printed value)`.
    Defaults {
        domain: String,
        key: String,
        value: Option<(String, String)>,
    },
    Systemd {
        unit: String,
        enabled: bool,
    },
    File {
        path: PathBuf,
        content: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub item_id: String,
    pub action: String,
    pub priors: Vec<Prior>,
}

/// One unit of work.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A command run as the person (argv).
    Run(Vec<String>),
    /// Replace a file of the person's (Firefox `user.js`).
    Write {
        path: PathBuf,
        content: String,
    },
    Remove(PathBuf),
    /// Work for the administrator helper.
    Elevated(ElevatedOp),
}

fn s(x: &str) -> String {
    x.to_string()
}

/// `reg add` (`/ve` for the default value).
pub fn reg_add_argv(key: &str, name: &str, kind: &str, data: &str) -> Vec<String> {
    let mut v = vec![s("reg"), s("add"), s(key)];
    if name.is_empty() || name.eq_ignore_ascii_case("(default)") {
        v.push(s("/ve"));
    } else {
        v.extend([s("/v"), s(name)]);
    }
    v.extend([s("/t"), s(kind), s("/d"), s(data), s("/f")]);
    v
}

pub fn reg_delete_argv(key: &str, name: &str) -> Vec<String> {
    let mut v = vec![s("reg"), s("delete"), s(key)];
    if name.is_empty() || name.eq_ignore_ascii_case("(default)") {
        v.push(s("/ve"));
    } else {
        v.extend([s("/v"), s(name)]);
    }
    v.push(s("/f"));
    v
}

fn reg_set_step(key: &str, name: &str, kind: &str, data: &str) -> Step {
    if super::catalog::hive(key) == Some(Hive::User) {
        Step::Run(reg_add_argv(key, name, kind, data))
    } else {
        Step::Elevated(ElevatedOp::RegSet {
            key: key.into(),
            name: name.into(),
            kind: kind.into(),
            data: data.into(),
        })
    }
}

fn reg_delete_step(key: &str, name: &str) -> Step {
    if super::catalog::hive(key) == Some(Hive::User) {
        Step::Run(reg_delete_argv(key, name))
    } else {
        Step::Elevated(ElevatedOp::RegDelete {
            key: key.into(),
            name: Some(name.into()),
        })
    }
}

/// `defaults write domain key [-type] value`.
pub fn defaults_write_argv(
    env: &Env,
    domain: &str,
    key: &str,
    kind: Option<&str>,
    value: &str,
) -> Vec<String> {
    let mut v = vec![s("defaults"), s("write"), env.expand(domain), s(key)];
    if let Some(k) = kind {
        v.push(format!("-{k}"));
    }
    v.push(s(value));
    v
}

pub fn defaults_delete_argv(env: &Env, domain: &str, key: &str) -> Vec<String> {
    vec![s("defaults"), s("delete"), env.expand(domain), s(key)]
}

/// privacy.sexy's `defaultStartupMode` as a `service_startup` mode.
pub fn default_mode(m: &str) -> Option<&'static str> {
    match m.trim().to_ascii_lowercase().as_str() {
        "automatic" => Some("automatic"),
        "manual" => Some("manual"),
        "disabled" => Some("disabled"),
        _ => None,
    }
}

fn start_value(mode: &str) -> Option<&'static str> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "boot" => Some("0"),
        "system" => Some("1"),
        "automatic" => Some("2"),
        "manual" => Some("3"),
        "disabled" => Some("4"),
        _ => None,
    }
}

fn reg_prior(r: &Reader, key: &str, name: &str) -> Result<Prior, String> {
    let value = r
        .key(key)?
        .and_then(|v| state::reg_value(&v, name).map(|x| (x.kind.clone(), x.data.clone())));
    Ok(Prior::Reg {
        key: key.into(),
        name: name.into(),
        value,
    })
}

fn defaults_prior(r: &Reader, domain: &str, key: &str) -> Result<Prior, String> {
    let value = match state::defaults_read(r.sys, r.env, domain, key)? {
        None => None,
        Some(v) => {
            let o = r
                .sys
                .run(&[s("defaults"), s("read-type"), r.env.expand(domain), s(key)])?;
            let kind = o
                .stdout
                .trim()
                .strip_prefix("Type is ")
                .unwrap_or("")
                .trim()
                .to_string();
            Some((kind, v))
        }
    };
    Ok(Prior::Defaults {
        domain: domain.into(),
        key: key.into(),
        value,
    })
}

fn with_line(content: &str, line: &str, prefix: &str) -> String {
    let mut lines: Vec<&str> = content.lines().filter(|l| !l.starts_with(prefix)).collect();
    lines.push(line);
    lines.join("\n") + "\n"
}

fn without_line(content: &str, prefix: &str) -> String {
    let lines: Vec<&str> = content.lines().filter(|l| !l.starts_with(prefix)).collect();
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

/// Steps of an item for an action, with the state they will change.
/// Things already in the wanted state are left alone.
pub fn steps(r: &Reader, it: &Item, action: Action) -> Result<(Vec<Step>, Vec<Prior>), String> {
    if !it.executable() {
        return Err(format!(
            "CODE_ONLY: {} runs free-form code; copy it instead",
            it.name
        ));
    }
    let parts: Vec<Part> = match action {
        Action::Apply => it.parts.clone(),
        Action::RevertDefault => it
            .default_parts()
            .ok_or_else(|| format!("{} has no default to go back to", it.name))?,
    };
    // macOS revert scripts are plain `defaults` lines: they apply as written.
    let reverting = action == Action::RevertDefault && it.revert_parts.is_none();
    let mut out = Vec::new();
    let mut priors = Vec::new();
    for p in parts
        .iter()
        .filter(|p| r.build.is_none_or(|b| p.builds.contains(b)))
    {
        match &p.fact {
            Fact::CommandExists { command } => {
                if !r.env.has_command(command) {
                    return Err(format!("{command} is not installed"));
                }
            }
            Fact::RegSet {
                key,
                name,
                kind,
                data,
                revert,
            } => {
                let target = if reverting {
                    match revert {
                        RegRevert::None => continue,
                        RegRevert::Delete => None,
                        RegRevert::Set { kind, data } => Some((kind.clone(), data.clone())),
                    }
                } else {
                    Some((kind.clone(), data.clone()))
                };
                let prior = reg_prior(r, key, name)?;
                let Prior::Reg { value: cur, .. } = &prior else {
                    unreachable!()
                };
                match (target, cur) {
                    (None, None) => {}
                    (None, Some(_)) => {
                        out.push(reg_delete_step(key, name));
                        priors.push(prior);
                    }
                    (Some((k, d)), cur) => {
                        if cur.as_ref().is_some_and(|(ck, cd)| {
                            ck.eq_ignore_ascii_case(&k) && state::reg_data_equal(&k, cd, &d)
                        }) {
                            continue;
                        }
                        out.push(reg_set_step(key, name, &k, &d));
                        priors.push(prior);
                    }
                }
            }
            Fact::RegDelete { key, name, revert } => {
                let prior = reg_prior(r, key, name)?;
                let Prior::Reg { value: cur, .. } = &prior else {
                    unreachable!()
                };
                if reverting {
                    if let RegRevert::Set { kind, data } = revert {
                        out.push(reg_set_step(key, name, kind, data));
                        priors.push(prior);
                    }
                } else if cur.is_some() {
                    out.push(reg_delete_step(key, name));
                    priors.push(prior);
                }
            }
            Fact::Service {
                name,
                default_mode: dm,
            } => {
                let Some(values) = r.key(&format!("{SERVICES_KEY}\\{name}"))? else {
                    continue;
                };
                let mode =
                    state::service_mode(&values).ok_or_else(|| format!("{name}: no start type"))?;
                let want = if reverting {
                    default_mode(dm).ok_or_else(|| format!("{name}: cannot go back to {dm}"))?
                } else {
                    "disabled"
                };
                if mode == want {
                    continue;
                }
                if matches!(mode.as_str(), "boot" | "system") {
                    return Err(format!("{name} starts with the system; not changed"));
                }
                out.push(Step::Elevated(ElevatedOp::ServiceStartup {
                    name: name.clone(),
                    mode: want.into(),
                }));
                priors.push(Prior::Service {
                    name: name.clone(),
                    mode,
                });
            }
            Fact::ServiceReg {
                name,
                default_mode: dm,
            } => {
                let want = if reverting {
                    start_value(dm).ok_or_else(|| format!("{name}: cannot go back to {dm}"))?
                } else {
                    "4"
                };
                for inst in r.service_instances(name)? {
                    let key = format!("{SERVICES_KEY}\\{inst}");
                    let prior = reg_prior(r, &key, "Start")?;
                    let Prior::Reg {
                        value: Some((_, cur)),
                        ..
                    } = &prior
                    else {
                        continue;
                    };
                    if state::reg_number(cur) == state::reg_number(want) {
                        continue;
                    }
                    out.push(Step::Elevated(ElevatedOp::RegSet {
                        key,
                        name: "Start".into(),
                        kind: "REG_DWORD".into(),
                        data: want.into(),
                    }));
                    priors.push(prior);
                }
            }
            Fact::Task {
                path,
                name,
                enable_on_revert,
            } => {
                if reverting && !enable_on_revert {
                    continue;
                }
                let want_enabled = reverting;
                let tasks = state::read_tasks(r.sys)?;
                for (tp, tn, enabled) in state::matching_tasks(&tasks, path, name) {
                    if *enabled == want_enabled {
                        continue;
                    }
                    let full = format!("{tp}{tn}");
                    out.push(Step::Elevated(ElevatedOp::TaskState {
                        path: full.clone(),
                        enabled: want_enabled,
                    }));
                    priors.push(Prior::Task {
                        path: full,
                        enabled: *enabled,
                    });
                }
            }
            Fact::Defaults {
                domain,
                key,
                kind,
                value,
                ..
            } => {
                let prior = defaults_prior(r, domain, key)?;
                if let Prior::Defaults {
                    value: Some((_, cur)),
                    ..
                } = &prior
                {
                    if state::defaults_equal(kind.as_deref(), cur, value) {
                        continue;
                    }
                }
                out.push(Step::Run(defaults_write_argv(
                    r.env,
                    domain,
                    key,
                    kind.as_deref(),
                    value,
                )));
                priors.push(prior);
            }
            Fact::DefaultsDelete { domain, key, .. } => {
                let prior = defaults_prior(r, domain, key)?;
                if matches!(prior, Prior::Defaults { value: None, .. }) {
                    continue;
                }
                out.push(Step::Run(defaults_delete_argv(r.env, domain, key)));
                priors.push(prior);
            }
            Fact::TccReset { service } => {
                out.push(Step::Run(vec![s("tccutil"), s("reset"), service.clone()]))
            }
            Fact::Systemd { unit } => {
                let o = r
                    .sys
                    .run(&[s("systemctl"), s("is-enabled"), s("--"), unit.clone()])?;
                let enabled = match state::systemd_state(&o.stdout, &o.stderr) {
                    FactState::Absent => continue,
                    FactState::Applied => false,
                    FactState::NotApplied => true,
                    FactState::Unknown(e) => return Err(e),
                };
                if enabled == reverting {
                    continue;
                }
                let mode = if reverting { "enable" } else { "disable" };
                out.push(Step::Elevated(ElevatedOp::ServiceStartup {
                    name: unit.clone(),
                    mode: mode.into(),
                }));
                priors.push(Prior::Systemd {
                    unit: unit.clone(),
                    enabled,
                });
            }
            Fact::FirefoxPref { name, value } => {
                let line = state::firefox_pref_line(name, value);
                let prefix = format!("user_pref(\"{name}\",");
                for prof in state::firefox_profiles(&r.env.home) {
                    let path = prof.join("user.js");
                    let cur = std::fs::read_to_string(&path).ok();
                    let text = cur.clone().unwrap_or_default();
                    let new = if reverting {
                        without_line(&text, &prefix)
                    } else {
                        with_line(&text, &line, &prefix)
                    };
                    if cur.as_deref() == Some(new.as_str()) || (cur.is_none() && reverting) {
                        continue;
                    }
                    out.push(Step::Write {
                        path: path.clone(),
                        content: new,
                    });
                    priors.push(Prior::File { path, content: cur });
                }
            }
            other => return Err(format!("no typed operation for {other:?}")),
        }
    }
    Ok((out, priors))
}

/// Steps that bring back the state a snapshot saved, last change first.
pub fn restore_steps(env: &Env, snap: &Snapshot) -> Vec<Step> {
    snap.priors
        .iter()
        .rev()
        .map(|p| match p {
            Prior::Reg {
                key,
                name,
                value: Some((kind, data)),
            } => reg_set_step(key, name, kind, data),
            Prior::Reg {
                key,
                name,
                value: None,
            } => reg_delete_step(key, name),
            Prior::Service { name, mode } => Step::Elevated(ElevatedOp::ServiceStartup {
                name: name.clone(),
                mode: mode.clone(),
            }),
            Prior::Task { path, enabled } => Step::Elevated(ElevatedOp::TaskState {
                path: path.clone(),
                enabled: *enabled,
            }),
            Prior::Defaults {
                domain,
                key,
                value: None,
            } => Step::Run(defaults_delete_argv(env, domain, key)),
            Prior::Defaults {
                domain,
                key,
                value: Some((kind, v)),
            } => {
                let (flag, v) = match kind.as_str() {
                    "boolean" => (
                        "bool",
                        if v.trim() == "1" {
                            "true".to_string()
                        } else {
                            "false".to_string()
                        },
                    ),
                    "integer" => ("int", v.clone()),
                    "float" => ("float", v.clone()),
                    "string" => ("string", v.clone()),
                    // Arrays, dictionaries, dates and data cannot travel as argv.
                    _ => {
                        return Step::Run(vec![
                            s("false"),
                            format!("cannot restore {domain} {key} of type {kind}"),
                        ])
                    }
                };
                Step::Run(defaults_write_argv(env, domain, key, Some(flag), &v))
            }
            Prior::Systemd { unit, enabled } => Step::Elevated(ElevatedOp::ServiceStartup {
                name: unit.clone(),
                mode: if *enabled { "enable" } else { "disable" }.into(),
            }),
            Prior::File {
                path,
                content: Some(c),
            } => Step::Write {
                path: path.clone(),
                content: c.clone(),
            },
            Prior::File {
                path,
                content: None,
            } => Step::Remove(path.clone()),
        })
        .collect()
}

/// Run the person's steps in order; stop at the first failure.
pub fn run_user_steps(sys: &dyn Sys, steps: &[Step]) -> Result<usize, String> {
    let mut n = 0;
    for st in steps {
        match st {
            Step::Run(argv) => {
                if argv.first().map(String::as_str) == Some("false") {
                    return Err(argv.get(1).cloned().unwrap_or_default());
                }
                let o = sys.run(argv)?;
                // `tccutil` exits 70 for a service this macOS does not have.
                let tcc_unsupported = argv[0] == "tccutil" && o.code == Some(70);
                if !o.ok && !tcc_unsupported {
                    let msg = if o.stderr.trim().is_empty() {
                        o.stdout.trim()
                    } else {
                        o.stderr.trim()
                    };
                    return Err(format!("{}: {msg}", argv[0]));
                }
            }
            Step::Write { path, content } => {
                std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Step::Remove(path) => match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            },
            Step::Elevated(_) => continue,
        }
        n += 1;
    }
    Ok(n)
}

pub fn elevated_ops(steps: &[Step]) -> Vec<ElevatedOp> {
    steps
        .iter()
        .filter_map(|s| match s {
            Step::Elevated(op) => Some(op.clone()),
            _ => None,
        })
        .collect()
}

/// A step as a line the person can read (the "copy command" view).
pub fn describe(st: &Step) -> String {
    match st {
        Step::Run(argv) => argv
            .iter()
            .map(|a| {
                if a.contains(' ') || a.is_empty() {
                    format!("'{}'", a.replace('\'', "'\\''"))
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        Step::Write { path, .. } => format!("# write {}", path.display()),
        Step::Remove(path) => format!("# remove {}", path.display()),
        Step::Elevated(op) => {
            crate::core::washmybelly::elevate::copy_command(std::slice::from_ref(op)).join(" ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::washmybelly::privacy::catalog::{Builds, Item};
    use crate::core::washmybelly::privacy::compiler::{Docs, ScriptData};
    use crate::core::washmybelly::privacy::test_support::{fixture, FakeSys};
    use crate::core::washmybelly::Os;

    fn item(os: Os, facts: Vec<Fact>) -> Item {
        Item {
            id: "privacy:test:x".into(),
            os,
            name: "x".into(),
            path: vec!["Configure programs".into()],
            docs: String::new(),
            recommend: Some("standard".into()),
            parts: facts
                .into_iter()
                .map(|fact| Part {
                    fact,
                    builds: Builds::default(),
                })
                .collect(),
            revert_parts: None,
            restart: vec![],
            cleanup: false,
            opaque_reason: None,
            has_revert: true,
            script: ScriptData {
                name: "x".into(),
                code: None,
                revert_code: None,
                call: None,
                recommend: None,
                docs: Docs::None,
            },
        }
    }

    fn env(os: Os) -> Env {
        Env {
            os,
            home: PathBuf::from("/h"),
            root: PathBuf::from("/r"),
            path_dirs: vec![],
            user: "u".into(),
            system_root: PathBuf::from("/w"),
        }
    }

    #[test]
    fn registry_apply_snapshots_the_real_previous_value() {
        let sys = FakeSys::default()
            .with(
                &["reg", "query", "HKCU\\Software\\Test"],
                true,
                "HKEY_CURRENT_USER\\Software\\Test\r\n    Tracking    REG_DWORD    0x1\r\n",
            )
            .with_any_ok();
        let e = env(Os::Windows);
        let r = Reader::new(&sys, &e);
        let it = item(
            Os::Windows,
            vec![
                Fact::RegSet {
                    key: "HKCU\\Software\\Test".into(),
                    name: "Tracking".into(),
                    kind: "REG_DWORD".into(),
                    data: "0".into(),
                    revert: RegRevert::Delete,
                },
                Fact::RegSet {
                    key: "HKLM\\SOFTWARE\\Policies\\T".into(),
                    name: "Off".into(),
                    kind: "REG_DWORD".into(),
                    data: "1".into(),
                    revert: RegRevert::Set {
                        kind: "REG_DWORD".into(),
                        data: "0".into(),
                    },
                },
            ],
        );
        let (st, priors) = steps(&r, &it, Action::Apply).unwrap();
        assert_eq!(
            st[0],
            Step::Run(vec![
                "reg".into(),
                "add".into(),
                "HKCU\\Software\\Test".into(),
                "/v".into(),
                "Tracking".into(),
                "/t".into(),
                "REG_DWORD".into(),
                "/d".into(),
                "0".into(),
                "/f".into()
            ])
        );
        assert!(
            matches!(&st[1], Step::Elevated(ElevatedOp::RegSet { key, data, .. }) if key == "HKLM\\SOFTWARE\\Policies\\T" && data == "1")
        );
        assert_eq!(
            priors[0],
            Prior::Reg {
                key: "HKCU\\Software\\Test".into(),
                name: "Tracking".into(),
                value: Some(("REG_DWORD".into(), "0x1".into()))
            }
        );
        assert_eq!(
            priors[1],
            Prior::Reg {
                key: "HKLM\\SOFTWARE\\Policies\\T".into(),
                name: "Off".into(),
                value: None
            }
        );

        // Undo goes back to the real previous state, not to privacy.sexy's default.
        let snap = Snapshot {
            item_id: it.id.clone(),
            action: "apply".into(),
            priors,
        };
        let back = restore_steps(&e, &snap);
        assert!(
            matches!(&back[0], Step::Elevated(ElevatedOp::RegDelete { key, name: Some(n) }) if key == "HKLM\\SOFTWARE\\Policies\\T" && n == "Off")
        );
        assert_eq!(
            back[1],
            Step::Run(reg_add_argv(
                "HKCU\\Software\\Test",
                "Tracking",
                "REG_DWORD",
                "0x1"
            ))
        );

        // "Back to the default" uses the revert data instead.
        let (d, _) = steps(&r, &it, Action::RevertDefault).unwrap();
        assert_eq!(
            d[0],
            Step::Run(reg_delete_argv("HKCU\\Software\\Test", "Tracking"))
        );
        assert!(matches!(&d[1], Step::Elevated(ElevatedOp::RegSet { data, .. }) if data == "0"));
    }

    #[test]
    fn an_applied_value_is_left_alone() {
        let sys = FakeSys::default().with(
            &["reg", "query", "HKCU\\Software\\Test"],
            true,
            "HKEY_CURRENT_USER\\Software\\Test\r\n    Tracking    REG_DWORD    0x0\r\n",
        );
        let e = env(Os::Windows);
        let r = Reader::new(&sys, &e);
        let it = item(
            Os::Windows,
            vec![Fact::RegSet {
                key: "HKCU\\Software\\Test".into(),
                name: "Tracking".into(),
                kind: "REG_DWORD".into(),
                data: "0".into(),
                revert: RegRevert::None,
            }],
        );
        let (st, priors) = steps(&r, &it, Action::Apply).unwrap();
        assert!(st.is_empty() && priors.is_empty());
    }

    #[test]
    fn services_and_tasks_become_helper_operations() {
        let sys = FakeSys::default()
            .with(
                &[
                    "reg",
                    "query",
                    "HKLM\\SYSTEM\\CurrentControlSet\\Services\\DiagTrack",
                ],
                true,
                &fixture("reg_query_service_diagtrack.txt"),
            )
            .with_argv(state::task_list_argv(), true, &fixture("tasks.txt"));
        let e = env(Os::Windows);
        let r = Reader::new(&sys, &e);
        let it = item(
            Os::Windows,
            vec![
                Fact::Service {
                    name: "DiagTrack".into(),
                    default_mode: "Automatic".into(),
                },
                Fact::Task {
                    path: "\\Microsoft\\Windows\\Customer Experience Improvement Program\\".into(),
                    name: "*".into(),
                    enable_on_revert: true,
                },
            ],
        );
        assert!(it.admin());
        let (st, priors) = steps(&r, &it, Action::Apply).unwrap();
        assert_eq!(
            elevated_ops(&st),
            vec![
                ElevatedOp::ServiceStartup {
                    name: "DiagTrack".into(),
                    mode: "disabled".into()
                },
                ElevatedOp::TaskState {
                    path: "\\Microsoft\\Windows\\Customer Experience Improvement Program\\UsbCeip"
                        .into(),
                    enabled: false
                },
            ]
        );
        assert_eq!(
            priors[0],
            Prior::Service {
                name: "DiagTrack".into(),
                mode: "automatic".into()
            }
        );
        let back = restore_steps(
            &e,
            &Snapshot {
                item_id: "x".into(),
                action: "apply".into(),
                priors,
            },
        );
        assert_eq!(
            elevated_ops(&back),
            vec![
                ElevatedOp::TaskState {
                    path: "\\Microsoft\\Windows\\Customer Experience Improvement Program\\UsbCeip"
                        .into(),
                    enabled: true
                },
                ElevatedOp::ServiceStartup {
                    name: "DiagTrack".into(),
                    mode: "automatic".into()
                },
            ]
        );
    }

    #[test]
    fn macos_defaults_snapshot_type_and_value() {
        let sys = FakeSys::default()
            .with(
                &[
                    "defaults",
                    "read",
                    "com.apple.AdLib",
                    "allowIdentifierForAdvertising",
                ],
                true,
                "1\n",
            )
            .with(
                &[
                    "defaults",
                    "read-type",
                    "com.apple.AdLib",
                    "allowIdentifierForAdvertising",
                ],
                true,
                "Type is boolean\n",
            )
            .with_any_ok();
        let e = env(Os::Macos);
        let r = Reader::new(&sys, &e);
        let it = item(
            Os::Macos,
            vec![Fact::Defaults {
                domain: "com.apple.AdLib".into(),
                key: "allowIdentifierForAdvertising".into(),
                kind: Some("bool".into()),
                value: "false".into(),
                sudo: false,
            }],
        );
        let (st, priors) = steps(&r, &it, Action::Apply).unwrap();
        assert_eq!(
            st,
            vec![Step::Run(vec![
                "defaults".into(),
                "write".into(),
                "com.apple.AdLib".into(),
                "allowIdentifierForAdvertising".into(),
                "-bool".into(),
                "false".into()
            ])]
        );
        run_user_steps(&sys, &st).unwrap();
        assert!(sys.calls().iter().any(|c| c.join(" ")
            == "defaults write com.apple.AdLib allowIdentifierForAdvertising -bool false"));
        let back = restore_steps(
            &e,
            &Snapshot {
                item_id: "x".into(),
                action: "apply".into(),
                priors,
            },
        );
        assert_eq!(
            back,
            vec![Step::Run(defaults_write_argv(
                &e,
                "com.apple.AdLib",
                "allowIdentifierForAdvertising",
                Some("bool"),
                "true"
            ))]
        );
    }

    #[test]
    fn firefox_prefs_are_written_and_restored_from_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = env(Os::Linux);
        e.home = dir.path().to_path_buf();
        let prof = dir.path().join(".mozilla/firefox/abcd1234.default");
        std::fs::create_dir_all(&prof).unwrap();
        std::fs::write(prof.join("user.js"), "user_pref(\"a\", 1);\nuser_pref(\"datareporting.healthreport.uploadEnabled\", true);\n").unwrap();
        let sys = FakeSys::default();
        let r = Reader::new(&sys, &e);
        let it = item(
            Os::Linux,
            vec![Fact::FirefoxPref {
                name: "datareporting.healthreport.uploadEnabled".into(),
                value: "false".into(),
            }],
        );
        let (st, priors) = steps(&r, &it, Action::Apply).unwrap();
        run_user_steps(&sys, &st).unwrap();
        let now = std::fs::read_to_string(prof.join("user.js")).unwrap();
        assert_eq!(now, "user_pref(\"a\", 1);\nuser_pref(\"datareporting.healthreport.uploadEnabled\", false);\n");
        assert_eq!(r.fact(&it.parts[0].fact), FactState::Applied);
        run_user_steps(
            &sys,
            &restore_steps(
                &e,
                &Snapshot {
                    item_id: "x".into(),
                    action: "apply".into(),
                    priors,
                },
            ),
        )
        .unwrap();
        assert!(std::fs::read_to_string(prof.join("user.js"))
            .unwrap()
            .contains("uploadEnabled\", true"));
    }

    #[test]
    fn code_only_items_never_build_steps() {
        let sys = FakeSys::default();
        let e = env(Os::Windows);
        let r = Reader::new(&sys, &e);
        let it = item(
            Os::Windows,
            vec![Fact::Opaque {
                what: "RunPowerShell".into(),
            }],
        );
        assert!(steps(&r, &it, Action::Apply)
            .unwrap_err()
            .starts_with("CODE_ONLY"));
        assert!(
            sys.calls().iter().all(|c| c[0] == "reg"),
            "only the build number was read"
        );
    }

    #[test]
    fn systemd_units_disable_through_the_helper() {
        let sys = FakeSys::default().with(
            &["systemctl", "is-enabled", "--", "whoopsie"],
            true,
            "enabled\n",
        );
        let e = env(Os::Linux);
        let r = Reader::new(&sys, &e);
        let it = item(
            Os::Linux,
            vec![Fact::Systemd {
                unit: "whoopsie".into(),
            }],
        );
        let (st, priors) = steps(&r, &it, Action::Apply).unwrap();
        assert_eq!(
            elevated_ops(&st),
            vec![ElevatedOp::ServiceStartup {
                name: "whoopsie".into(),
                mode: "disable".into()
            }]
        );
        assert_eq!(
            priors,
            vec![Prior::Systemd {
                unit: "whoopsie".into(),
                enabled: true
            }]
        );
    }
}
