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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Prior {
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

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Run(Vec<String>),
    Write { path: PathBuf, content: String },
    Remove(PathBuf),
    Elevated(ElevatedOp),
}

fn s(x: &str) -> String {
    x.to_string()
}

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

pub fn run_user_steps(sys: &dyn Sys, steps: &[Step]) -> Result<usize, String> {
    let mut n = 0;
    for st in steps {
        match st {
            Step::Run(argv) => {
                if argv.first().map(String::as_str) == Some("false") {
                    return Err(argv.get(1).cloned().unwrap_or_default());
                }
                let o = sys.run(argv)?;
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
