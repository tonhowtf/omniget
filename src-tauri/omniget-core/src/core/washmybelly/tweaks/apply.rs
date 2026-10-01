use serde::{Deserialize, Serialize};

use super::super::elevate::ElevatedOp;
use super::super::startup::Sys;
use super::catalog::{Hive, TweakOp};
use super::guard;
use super::state::{self, full_key, Tree, ValState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    SetValue {
        full: String,
        name: String,
        kind: String,
        data: String,
    },
    DeleteValue {
        full: String,
        name: String,
    },
    CreateKey {
        full: String,
    },
    DeleteKey {
        full: String,
    },
    Service {
        name: String,
        mode: String,
    },
    Task {
        path: String,
        enabled: bool,
    },
    PowerActive {
        guid: String,
    },
    PowerDuplicate,
    PowerDelete {
        guid: String,
    },
    Feature {
        name: String,
        enabled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Prev {
    Value {
        full: String,
        name: String,
        prev: ValState,
        #[serde(default)]
        created_root: Option<String>,
    },
    Key {
        full: String,
        tree: Option<Tree>,
    },
    Service {
        name: String,
        prev: Option<String>,
    },
    Task {
        path: String,
        prev: Option<bool>,
    },
    Power {
        prev: Option<String>,
        #[serde(default)]
        created: Option<String>,
    },
    Feature {
        name: String,
        prev: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub tweak: String,
    pub action: String,
    pub elevated: bool,
    pub entries: Vec<Prev>,
}

pub struct KeySets {
    pub catalog: std::collections::HashSet<String>,
    pub whole: std::collections::HashSet<String>,
}

fn name_args(name: &str) -> Vec<String> {
    if name.is_empty() {
        vec!["/ve".into()]
    } else {
        vec!["/v".into(), name.to_string()]
    }
}

pub fn step_argv(step: &Step) -> Result<Vec<String>, String> {
    let reg = |verb: &str, full: &str| vec!["reg".to_string(), verb.to_string(), full.to_string()];
    Ok(match step {
        Step::SetValue {
            full,
            name,
            kind,
            data,
        } => {
            let mut v = reg("add", full);
            v.extend(name_args(name));
            v.extend([
                "/t".into(),
                kind.clone(),
                "/d".into(),
                data.clone(),
                "/f".into(),
                "/reg:64".into(),
            ]);
            v
        }
        Step::DeleteValue { full, name } => {
            let mut v = reg("delete", full);
            v.extend(name_args(name));
            v.extend(["/f".into(), "/reg:64".into()]);
            v
        }
        Step::CreateKey { full } => {
            let mut v = reg("add", full);
            v.extend(["/f".into(), "/reg:64".into()]);
            v
        }
        Step::DeleteKey { full } => {
            let mut v = reg("delete", full);
            v.extend(["/f".into(), "/reg:64".into()]);
            v
        }
        Step::Service { name, mode } => super::super::startup::service_argv(name, mode, true)?,
        Step::Task { path, enabled } => super::super::startup::task_argv(path, *enabled)?,
        Step::PowerActive { guid } => vec!["powercfg".into(), "/setactive".into(), guid.clone()],
        Step::PowerDelete { guid } => vec!["powercfg".into(), "/delete".into(), guid.clone()],
        Step::PowerDuplicate => vec![
            "powercfg".into(),
            "/duplicatescheme".into(),
            state::ULTIMATE_TEMPLATE.into(),
        ],
        Step::Feature { name, enabled } => {
            if !super::catalog::FEATURES.contains(&name.as_str()) {
                return Err(format!("optional feature {name} is not in the list"));
            }
            let mut v = vec!["dism".to_string(), "/online".into()];
            if *enabled {
                v.extend([
                    "/enable-feature".into(),
                    format!("/featurename:{name}"),
                    "/all".into(),
                ]);
            } else {
                v.extend(["/disable-feature".into(), format!("/featurename:{name}")]);
            }
            v.push("/norestart".into());
            v
        }
    })
}

pub fn step_elevated(step: &Step) -> Result<ElevatedOp, String> {
    Ok(match step {
        Step::SetValue {
            full,
            name,
            kind,
            data,
        } => ElevatedOp::RegSet {
            key: full.clone(),
            name: name.clone(),
            kind: kind.clone(),
            data: data.clone(),
        },
        Step::CreateKey { full } => ElevatedOp::RegSet {
            key: full.clone(),
            name: String::new(),
            kind: KEY_KIND.into(),
            data: String::new(),
        },
        Step::DeleteValue { full, name } => ElevatedOp::RegDelete {
            key: full.clone(),
            name: Some(name.clone()),
        },
        Step::DeleteKey { full } => ElevatedOp::RegDelete {
            key: full.clone(),
            name: None,
        },
        Step::Service { name, mode } => ElevatedOp::ServiceStartup {
            name: name.clone(),
            mode: mode.clone(),
        },
        Step::Task { path, enabled } => ElevatedOp::TaskState {
            path: path.clone(),
            enabled: *enabled,
        },
        Step::Feature { .. } => {
            return Err(
                "NEEDS_HELPER_UPDATE: optional features are not in omniget-elevate yet".into(),
            )
        }
        Step::PowerActive { .. } | Step::PowerDelete { .. } | Step::PowerDuplicate => {
            return Err("a power plan never needs the administrator".into())
        }
    })
}

pub const KEY_KIND: &str = "KEY";

pub fn run_step(sys: &dyn Sys, step: &Step) -> Result<Option<String>, String> {
    let argv = step_argv(step)?;
    let out = sys.run_ok(&argv)?;
    match step {
        Step::PowerDuplicate => {
            let guid =
                state::parse_guid(&out).ok_or("powercfg did not say which plan it created")?;
            sys.run_ok(&step_argv(&Step::PowerActive { guid: guid.clone() })?)?;
            Ok(Some(guid))
        }
        _ => Ok(None),
    }
}

fn created_root(sys: &dyn Sys, full: &str, keys: &KeySets) -> Option<String> {
    let mut root = None;
    let mut cur = full.to_string();
    loop {
        if state::key_exists(sys, &cur) != Some(false) {
            break;
        }
        let norm = guard::normalize(&cur).ok()?;
        if !guard::created_key_ok(&norm, &keys.catalog) {
            break;
        }
        root = Some(cur.clone());
        match cur.rsplit_once('\\') {
            Some((parent, _)) => cur = parent.to_string(),
            None => break,
        }
    }
    root
}

pub fn capture(
    sys: &dyn Sys,
    op: &TweakOp,
    sid: Option<&str>,
    keys: &KeySets,
) -> Result<(Prev, Vec<Step>), String> {
    Ok(match op {
        TweakOp::RegSet {
            hive,
            key,
            name,
            kind,
            data,
            ..
        } => {
            let full = full_key(*hive, key, sid);
            let prev = state::read_value(sys, &full, name);
            if prev == ValState::Unknown {
                return Err(format!("could not read {full}; nothing was changed"));
            }
            let root = if prev == ValState::Absent {
                created_root(sys, &full, keys)
            } else {
                None
            };
            let data = state::norm_data(kind, data).ok_or("bad data")?;
            let step = Step::SetValue {
                full: full.clone(),
                name: name.clone(),
                kind: kind.clone(),
                data,
            };
            (
                Prev::Value {
                    full,
                    name: name.clone(),
                    prev,
                    created_root: root,
                },
                vec![step],
            )
        }
        TweakOp::RegDelete {
            hive,
            key,
            name: Some(n),
        } => {
            let full = full_key(*hive, key, sid);
            let prev = state::read_value(sys, &full, n);
            let steps = match &prev {
                ValState::Unknown => {
                    return Err(format!("could not read {full}; nothing was changed"))
                }
                ValState::Absent => vec![],
                ValState::Present { .. } => vec![Step::DeleteValue {
                    full: full.clone(),
                    name: n.clone(),
                }],
            };
            (
                Prev::Value {
                    full,
                    name: n.clone(),
                    prev,
                    created_root: None,
                },
                steps,
            )
        }
        TweakOp::RegDelete {
            hive,
            key,
            name: None,
        } => {
            let full = full_key(*hive, key, sid);
            let tree = state::read_tree(sys, &full)
                .ok_or_else(|| format!("could not read {full}; nothing was changed"))?;
            let steps = if tree.is_some() {
                vec![Step::DeleteKey { full: full.clone() }]
            } else {
                vec![]
            };
            (Prev::Key { full, tree }, steps)
        }
        TweakOp::ServiceStartup { name, mode } => {
            let prev = state::service_mode(sys, name)
                .ok_or_else(|| format!("could not read service {name}"))?;
            let steps = match &prev {
                Some(m) if m != mode => vec![Step::Service {
                    name: name.clone(),
                    mode: mode.clone(),
                }],
                _ => vec![],
            };
            (
                Prev::Service {
                    name: name.clone(),
                    prev,
                },
                steps,
            )
        }
        TweakOp::TaskState { path, enabled } => {
            let prev = state::task_enabled(sys, path)
                .ok_or_else(|| format!("could not read task {path}"))?;
            let steps = match prev {
                Some(e) if e != *enabled => vec![Step::Task {
                    path: path.clone(),
                    enabled: *enabled,
                }],
                _ => vec![],
            };
            (
                Prev::Task {
                    path: path.clone(),
                    prev,
                },
                steps,
            )
        }
        TweakOp::PowerSetting { scheme } => {
            let prev = state::active_scheme(sys);
            if prev.is_none() {
                return Err("could not read the active power plan".into());
            }
            let steps = match scheme.as_str() {
                "balanced" => vec![Step::PowerActive {
                    guid: state::BALANCED.into(),
                }],
                "high_performance" => vec![Step::PowerActive {
                    guid: state::HIGH_PERFORMANCE.into(),
                }],
                "ultimate" => vec![Step::PowerDuplicate],
                other => return Err(format!("unknown power plan {other}")),
            };
            (
                Prev::Power {
                    prev,
                    created: None,
                },
                steps,
            )
        }
        TweakOp::OptionalFeature { name, enabled } => {
            let prev = state::feature_enabled(sys, name)
                .ok_or_else(|| format!("could not read feature {name}"))?;
            let steps = match prev {
                Some(e) if e == *enabled => vec![],
                None => vec![],
                _ => vec![Step::Feature {
                    name: name.clone(),
                    enabled: *enabled,
                }],
            };
            (
                Prev::Feature {
                    name: name.clone(),
                    prev,
                },
                steps,
            )
        }
    })
}

pub fn restore_steps(p: &Prev) -> Vec<Step> {
    match p {
        Prev::Value {
            full,
            name,
            prev: ValState::Present { kind, data },
            ..
        } => {
            vec![Step::SetValue {
                full: full.clone(),
                name: name.clone(),
                kind: kind.clone(),
                data: data.clone(),
            }]
        }
        Prev::Value {
            created_root: Some(root),
            prev: ValState::Absent,
            ..
        } => vec![Step::DeleteKey { full: root.clone() }],
        Prev::Value {
            full,
            name,
            prev: ValState::Absent,
            ..
        } => vec![Step::DeleteValue {
            full: full.clone(),
            name: name.clone(),
        }],
        Prev::Value {
            prev: ValState::Unknown,
            ..
        } => vec![],
        Prev::Key { tree: None, .. } => vec![],
        Prev::Key { tree: Some(t), .. } => t
            .keys
            .iter()
            .map(|k| Step::CreateKey { full: k.clone() })
            .chain(t.values.iter().map(|(k, n, kind, d)| Step::SetValue {
                full: k.clone(),
                name: n.clone(),
                kind: kind.clone(),
                data: d.clone(),
            }))
            .collect(),
        Prev::Service {
            name,
            prev: Some(m),
        } => vec![Step::Service {
            name: name.clone(),
            mode: m.clone(),
        }],
        Prev::Task {
            path,
            prev: Some(e),
        } => vec![Step::Task {
            path: path.clone(),
            enabled: *e,
        }],
        Prev::Power {
            prev: Some(g),
            created,
        } => {
            let mut v = vec![Step::PowerActive { guid: g.clone() }];
            if let Some(c) = created {
                if c != g {
                    v.push(Step::PowerDelete { guid: c.clone() });
                }
            }
            v
        }
        Prev::Feature {
            name,
            prev: Some(e),
        } => vec![Step::Feature {
            name: name.clone(),
            enabled: *e,
        }],
        Prev::Service { prev: None, .. }
        | Prev::Task { prev: None, .. }
        | Prev::Power { prev: None, .. }
        | Prev::Feature { prev: None, .. } => vec![],
    }
}

pub fn undo_steps(s: &Snapshot) -> Vec<Step> {
    s.entries.iter().rev().flat_map(restore_steps).collect()
}

pub fn apply_direct(sys: &dyn Sys, ops: &[TweakOp], keys: &KeySets) -> Result<Vec<Prev>, String> {
    let mut done: Vec<Prev> = Vec::new();
    for op in ops {
        if op.needs_admin() {
            return Err("this change needs the administrator".into());
        }
        let r = capture(sys, op, None, keys).and_then(|(mut prev, steps)| {
            for step in &steps {
                if let Some(created) = run_step(sys, step)? {
                    if let Prev::Power { created: c, .. } = &mut prev {
                        *c = Some(created);
                    }
                }
            }
            Ok(prev)
        });
        match r {
            Ok(p) => done.push(p),
            Err(e) => {
                let snap = Snapshot {
                    tweak: String::new(),
                    action: String::new(),
                    elevated: false,
                    entries: done,
                };
                let back = run_restore(sys, &snap);
                return Err(match back {
                    Ok(_) => format!("{e} (the earlier changes were put back)"),
                    Err(b) => format!("{e}; putting the earlier changes back also failed: {b}"),
                });
            }
        }
    }
    Ok(done)
}

pub fn run_restore(sys: &dyn Sys, s: &Snapshot) -> Result<usize, String> {
    let mut n = 0;
    let mut errors = Vec::new();
    for step in undo_steps(s) {
        let gone = match &step {
            Step::DeleteValue { full, name } => {
                state::read_value(sys, full, name) == ValState::Absent
            }
            Step::DeleteKey { full } => state::key_exists(sys, full) == Some(false),
            _ => false,
        };
        if gone {
            continue;
        }
        match run_step(sys, &step) {
            Ok(_) => n += 1,
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(n)
    } else {
        Err(errors.join("; "))
    }
}

pub fn plan_for_helper(
    sys: &dyn Sys,
    ops: &[TweakOp],
    sid: Option<&str>,
    keys: &KeySets,
) -> Result<(Vec<Prev>, Vec<ElevatedOp>), String> {
    let mut prevs = Vec::new();
    let mut out = Vec::new();
    for op in ops {
        if matches!(
            op,
            TweakOp::RegSet {
                hive: Hive::Hkcu,
                ..
            } | TweakOp::RegDelete {
                hive: Hive::Hkcu,
                ..
            }
        ) && sid.is_none()
        {
            return Err("could not tell who is signed in (whoami)".into());
        }
        let (prev, steps) = capture(sys, op, sid, keys)?;
        for s in &steps {
            out.push(step_elevated(s)?);
        }
        prevs.push(prev);
    }
    Ok((prevs, out))
}

pub fn restore_for_helper(s: &Snapshot) -> Result<Vec<ElevatedOp>, String> {
    undo_steps(s).iter().map(step_elevated).collect()
}

pub fn helper_reg_set(
    sys: &dyn Sys,
    key: &str,
    name: &str,
    kind: &str,
    data: &str,
) -> Result<String, String> {
    if kind == KEY_KIND {
        guard::value_write_ok(key, "", false)?;
        sys.run_ok(&step_argv(&Step::CreateKey {
            full: key.to_string(),
        })?)?;
        return Ok(format!("{key} created"));
    }
    guard::value_write_ok(key, name, false)?;
    if name.contains(['\0', '\n', '\r']) {
        return Err(format!("PROTECTED: bad value name {name:?}"));
    }
    let data = state::norm_data(kind, data)
        .ok_or_else(|| format!("reg_set: {kind} data does not parse"))?;
    sys.run_ok(&step_argv(&Step::SetValue {
        full: key.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        data,
    })?)?;
    Ok(format!("{key}\\{name} set"))
}

pub fn helper_reg_delete(
    sys: &dyn Sys,
    key: &str,
    name: Option<&str>,
    keys: &KeySets,
) -> Result<String, String> {
    match name {
        Some(n) => {
            guard::value_write_ok(key, n, true)?;
            if state::read_value(sys, key, n) == ValState::Absent {
                return Ok(format!("{key}\\{n} was already gone"));
            }
            sys.run_ok(&step_argv(&Step::DeleteValue {
                full: key.to_string(),
                name: n.to_string(),
            })?)?;
            Ok(format!("{key}\\{n} deleted"))
        }
        None => {
            guard::key_delete_ok(key, &keys.catalog, &keys.whole)?;
            if state::key_exists(sys, key) == Some(false) {
                return Ok(format!("{key} was already gone"));
            }
            sys.run_ok(&step_argv(&Step::DeleteKey {
                full: key.to_string(),
            })?)?;
            Ok(format!("{key} deleted"))
        }
    }
}
