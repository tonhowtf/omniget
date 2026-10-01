use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Env, Sys, MODULE};
use crate::core::washmybelly::elevate::{self, ElevatedOp};
use crate::core::washmybelly::plan::Outcome;
use crate::core::washmybelly::receipt::Undo;
use crate::core::washmybelly::store::Ctx;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    LaunchdUser {
        label: String,
        plist: PathBuf,
        enable: bool,
    },
    LaunchdDaemon {
        label: String,
        plist: PathBuf,
    },
    LoginItem {
        name: String,
        #[serde(default)]
        path: Option<String>,
        enable: bool,
    },
    WinApproved {
        hive: String,
        subkey: String,
        name: String,
        enable: bool,
    },
    WinTask {
        path: String,
        enable: bool,
        #[serde(default)]
        admin: bool,
    },
    WinService {
        name: String,
        mode: String,
    },
    XdgAutostart {
        file: String,
        enable: bool,
    },
    SystemdUser {
        unit: String,
        enable: bool,
        #[serde(default)]
        now: bool,
    },
}

impl Action {
    pub fn split(&self) -> (String, Value) {
        let mut v = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        let name = v
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(o) = v.as_object_mut() {
            o.remove("action");
        }
        (name, v)
    }

    pub fn join(action: &str, args: &Value) -> Result<Action, String> {
        let mut v = if args.is_object() {
            args.clone()
        } else {
            json!({})
        };
        v["action"] = json!(action);
        serde_json::from_value(v)
            .map_err(|e| format!("startup: unknown or malformed action {action}: {e}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Snapshot {
    Apply {
        action: Action,
    },
    Elevated {
        ops: Vec<ElevatedOp>,
    },
    File {
        path: PathBuf,
        previous: Option<String>,
    },
    RegValue {
        key: String,
        name: String,
        previous: Option<String>,
    },
}

fn snapshot(ctx: &Ctx, receipt_id: &str, key: &str, s: &Snapshot) -> Result<Undo, String> {
    let id = ctx.snapshot_put(receipt_id, MODULE, key, s)?;
    Ok(Undo::Module {
        module: MODULE.into(),
        action: "restore_snapshot".into(),
        args: json!({ "snapshot": id }),
    })
}

fn check_label(label: &str) -> Result<(), String> {
    if super::valid_label(label) && !label.starts_with("com.apple.") {
        Ok(())
    } else {
        Err(format!("startup: refused launchd label {label:?}"))
    }
}

fn check_plist(env: &Env, plist: &Path, daemon: bool) -> Result<(), String> {
    let parent = plist.parent().unwrap_or(Path::new(""));
    let allowed: Vec<PathBuf> = if daemon {
        vec![PathBuf::from("/Library/LaunchDaemons")]
    } else {
        vec![
            env.home.join("Library/LaunchAgents"),
            PathBuf::from("/Library/LaunchAgents"),
        ]
    };
    let ok = plist.extension().is_some_and(|e| e == "plist")
        && allowed.iter().any(|a| a == parent)
        && !std::fs::symlink_metadata(plist)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(format!("startup: refused plist {}", plist.display()))
    }
}

fn check_login_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('-')
        || name.chars().any(|c| c.is_control())
    {
        Err(format!("startup: refused login item name {name:?}"))
    } else {
        Ok(())
    }
}

pub fn check_desktop_file(file: &str) -> Result<(), String> {
    let ok = file.ends_with(".desktop")
        && file.len() > ".desktop".len()
        && file.len() <= 255
        && !file.starts_with('.')
        && !file.contains(['/', '\\', '\0']);
    if ok {
        Ok(())
    } else {
        Err(format!("startup: refused autostart file {file:?}"))
    }
}

fn check_unit(unit: &str) -> Result<(), String> {
    let kinds = [".service", ".timer", ".socket", ".path"];
    if super::valid_service_name(unit)
        && kinds.iter().any(|k| unit.ends_with(k))
        && !unit.contains("@.")
    {
        Ok(())
    } else {
        Err(format!("startup: refused unit {unit:?}"))
    }
}

pub const APPROVED_BASE: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

pub fn approved_key(hive: &str, subkey: &str) -> Result<String, String> {
    if !matches!(hive, "HKCU" | "HKLM") || !matches!(subkey, "Run" | "Run32" | "StartupFolder") {
        return Err(format!("startup: refused key {hive}\\{subkey}"));
    }
    Ok(format!("{hive}\\{APPROVED_BASE}\\{subkey}"))
}

fn check_value_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 260
        || name.starts_with(['/', '-'])
        || name.chars().any(|c| c.is_control())
    {
        Err(format!("startup: refused value name {name:?}"))
    } else {
        Ok(())
    }
}

pub fn launchd_user_argv(
    uid: u32,
    label: &str,
    plist: &Path,
    enable: bool,
) -> Vec<(Vec<String>, bool)> {
    let target = format!("gui/{uid}/{label}");
    if enable {
        vec![
            (argv!["launchctl", "enable", target], true),
            (
                argv![
                    "launchctl",
                    "bootstrap",
                    format!("gui/{uid}"),
                    plist.to_string_lossy()
                ],
                false,
            ),
        ]
    } else {
        vec![
            (argv!["launchctl", "disable", target.clone()], true),
            (argv!["launchctl", "bootout", target], false),
        ]
    }
}

pub fn login_item_argv(name: &str, path: Option<&str>, enable: bool) -> Vec<String> {
    let (script, arg) = if enable {
        (
            "tell application \"System Events\" to make login item at end with properties {path:(item 1 of argv), hidden:false}",
            path.unwrap_or_default(),
        )
    } else {
        (
            "tell application \"System Events\" to delete login item (item 1 of argv)",
            name,
        )
    };
    argv![
        "osascript",
        "-e",
        "on run argv",
        "-e",
        script,
        "-e",
        "end run",
        arg
    ]
}

pub fn login_item_path_argv(name: &str) -> Vec<String> {
    argv![
        "osascript",
        "-e",
        "on run argv",
        "-e",
        "tell application \"System Events\" to get the path of login item (item 1 of argv)",
        "-e",
        "end run",
        name
    ]
}

fn filetime_hex(unix_secs: i64) -> String {
    let ft = (unix_secs.max(0) as u64 + 11_644_473_600) * 10_000_000;
    ft.to_le_bytes()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

pub fn approved_value(enable: bool, unix_secs: i64) -> String {
    if enable {
        "020000000000000000000000".into()
    } else {
        format!("03000000{}", filetime_hex(unix_secs))
    }
}

pub fn reg_add_argv(key: &str, name: &str, data: &str) -> Vec<String> {
    argv![
        "reg",
        "add",
        key,
        "/v",
        name,
        "/t",
        "REG_BINARY",
        "/d",
        data,
        "/f"
    ]
}

pub fn reg_delete_argv(key: &str, name: &str) -> Vec<String> {
    argv!["reg", "delete", key, "/v", name, "/f"]
}

pub fn systemd_user_argv(unit: &str, enable: bool, now: bool) -> Vec<String> {
    let mut v = argv![
        "systemctl",
        "--user",
        if enable { "enable" } else { "disable" }
    ];
    if now {
        v.push("--now".into());
    }
    v.push("--".into());
    v.push(unit.into());
    v
}

pub fn set_hidden(content: &str, hidden: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_entry = false;
    let mut placed = false;
    let mut saw_entry = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_entry && hidden && !placed {
                trim_trailing_blank(&mut out);
                out.push("Hidden=true".into());
                out.push(String::new());
                placed = true;
            }
            in_entry = t == "[Desktop Entry]";
            saw_entry |= in_entry;
            out.push(line.to_string());
            continue;
        }
        if in_entry {
            let key = t.split('=').next().unwrap_or("").trim();
            if key == "Hidden" || key == "X-GNOME-Autostart-enabled" {
                continue;
            }
        }
        out.push(line.to_string());
    }
    if hidden && !placed {
        if !saw_entry {
            out.insert(0, "[Desktop Entry]".into());
        }
        trim_trailing_blank(&mut out);
        out.push("Hidden=true".into());
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn trim_trailing_blank(v: &mut Vec<String>) {
    while v.last().is_some_and(|l| l.trim().is_empty()) {
        v.pop();
    }
}

fn launchd_disabled(sys: &dyn Sys, uid: u32, label: &str) -> bool {
    sys.run(&argv_array![
        "launchctl",
        "print-disabled",
        format!("gui/{uid}")
    ])
    .ok()
    .map(|o| super::macos_disabled_lookup(&o.stdout, label))
    .unwrap_or(false)
}

fn reg_value(sys: &dyn Sys, key: &str, name: &str) -> Option<String> {
    let o = sys
        .run(&argv_array!["reg", "query", key, "/v", name])
        .ok()?;
    if !o.ok {
        return None;
    }
    crate::core::washmybelly::parse::reg_query(&o.stdout)
        .into_iter()
        .find(|v| v.name.eq_ignore_ascii_case(name))
        .map(|v| v.data)
}

fn autostart_source(env: &Env, file: &str) -> Option<PathBuf> {
    let user = env.autostart_user.join(file);
    if user.is_file() {
        return Some(user);
    }
    env.autostart_system
        .iter()
        .map(|d| d.join(file))
        .find(|p| p.is_file())
}

fn no_link(p: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => {
            Err(format!("startup: {} is a link; not touched", p.display()))
        }
        _ => Ok(()),
    }
}

pub fn execute(
    ctx: &Ctx,
    sys: &dyn Sys,
    env: &Env,
    receipt_id: &str,
    a: &Action,
) -> Result<Outcome, String> {
    let (snap, key) = match a {
        Action::LaunchdUser { label, plist, .. } => {
            check_label(label)?;
            check_plist(env, plist, false)?;
            let was_on = !launchd_disabled(sys, env.uid, label);
            (
                Snapshot::Apply {
                    action: Action::LaunchdUser {
                        label: label.clone(),
                        plist: plist.clone(),
                        enable: was_on,
                    },
                },
                label.clone(),
            )
        }
        Action::LoginItem { name, path, enable } => {
            check_login_name(name)?;
            let path = match path {
                Some(p) if !p.is_empty() => Some(p.clone()),
                _ if !*enable => sys
                    .run_ok(&login_item_path_argv(name))
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| s.starts_with('/')),
                _ => None,
            };
            if path.is_none() {
                return Err("NO_PATH: this login item has no app path to bring it back; use System Settings".into());
            }
            let undo = Action::LoginItem {
                name: name.clone(),
                path: path.clone(),
                enable: !*enable,
            };
            let snap = Snapshot::Apply { action: undo };
            let u = snapshot(ctx, receipt_id, name, &snap)?;
            let resolved = Action::LoginItem {
                name: name.clone(),
                path,
                enable: *enable,
            };
            apply(sys, env, &resolved)?;
            remember_turned_off(ctx, &resolved);
            return Ok(done(a, Some(u)));
        }
        Action::WinApproved {
            hive, subkey, name, ..
        } => {
            if hive != "HKCU" {
                return Err("startup: this item needs the administrator".into());
            }
            let key = approved_key(hive, subkey)?;
            check_value_name(name)?;
            let previous = reg_value(sys, &key, name);
            (
                Snapshot::RegValue {
                    key: key.clone(),
                    name: name.clone(),
                    previous,
                },
                format!("{key}\\{name}"),
            )
        }
        Action::WinTask {
            path,
            enable,
            admin,
        } => {
            if *admin {
                return Err("startup: this task needs the administrator".into());
            }
            super::task_argv(path, *enable)?;
            (
                Snapshot::Apply {
                    action: Action::WinTask {
                        path: path.clone(),
                        enable: !*enable,
                        admin: false,
                    },
                },
                path.clone(),
            )
        }
        Action::XdgAutostart { file, .. } => {
            check_desktop_file(file)?;
            let target = env.autostart_user.join(file);
            no_link(&target)?;
            let previous = if target.is_file() {
                Some(std::fs::read_to_string(&target).map_err(|e| e.to_string())?)
            } else {
                None
            };
            (
                Snapshot::File {
                    path: target,
                    previous,
                },
                file.clone(),
            )
        }
        Action::SystemdUser { unit, enable, .. } => {
            check_unit(unit)?;
            let active = sys
                .run(&argv_array![
                    "systemctl",
                    "--user",
                    "is-active",
                    "--",
                    unit.as_str()
                ])
                .map(|o| o.stdout.trim() == "active")
                .unwrap_or(false);
            let undo = Action::SystemdUser {
                unit: unit.clone(),
                enable: !*enable,
                now: if *enable { !active } else { active },
            };
            (Snapshot::Apply { action: undo }, unit.clone())
        }
        Action::LaunchdDaemon { .. } | Action::WinService { .. } => {
            return Err("startup: this item needs the administrator".into());
        }
    };
    let u = snapshot(ctx, receipt_id, &key, &snap)?;
    apply(sys, env, a)?;
    remember_turned_off(ctx, a);
    Ok(done(a, Some(u)))
}

fn done(a: &Action, undo: Option<Undo>) -> Outcome {
    let on = match a {
        Action::LaunchdUser { enable, .. }
        | Action::LoginItem { enable, .. }
        | Action::WinApproved { enable, .. }
        | Action::WinTask { enable, .. }
        | Action::XdgAutostart { enable, .. }
        | Action::SystemdUser { enable, .. } => *enable,
        Action::LaunchdDaemon { .. } => false,
        Action::WinService { mode, .. } => mode != "manual" && mode != "disabled",
    };
    Outcome {
        detail: if on {
            "turned on".into()
        } else {
            "turned off (nothing deleted)".into()
        },
        bytes: 0,
        undo,
    }
}

pub fn apply(sys: &dyn Sys, env: &Env, a: &Action) -> Result<String, String> {
    match a {
        Action::LaunchdUser {
            label,
            plist,
            enable,
        } => {
            check_label(label)?;
            check_plist(env, plist, false)?;
            for (cmd, must) in launchd_user_argv(env.uid, label, plist, *enable) {
                let o = sys.run(&cmd)?;
                if must && !o.ok {
                    return Err(format!("launchctl: {}", o.stderr.trim()));
                }
            }
            Ok(format!("{label} {}", if *enable { "on" } else { "off" }))
        }
        Action::LoginItem { name, path, enable } => {
            check_login_name(name)?;
            if *enable {
                let p = path
                    .as_deref()
                    .filter(|p| p.starts_with('/') && Path::new(p).exists())
                    .ok_or("NO_PATH: the app is not there anymore")?;
                sys.run_ok(&login_item_argv(name, Some(p), true))?;
            } else {
                sys.run_ok(&login_item_argv(name, None, false))?;
            }
            Ok(name.clone())
        }
        Action::WinApproved {
            hive,
            subkey,
            name,
            enable,
        } => {
            if hive != "HKCU" {
                return Err("startup: this item needs the administrator".into());
            }
            let key = approved_key(hive, subkey)?;
            check_value_name(name)?;
            sys.run_ok(&reg_add_argv(
                &key,
                name,
                &approved_value(*enable, crate::core::washmybelly::store::now()),
            ))?;
            Ok(name.clone())
        }
        Action::WinTask {
            path,
            enable,
            admin,
        } => {
            if *admin {
                return Err("startup: this task needs the administrator".into());
            }
            sys.run_ok(&super::task_argv(path, *enable)?)?;
            Ok(path.clone())
        }
        Action::XdgAutostart { file, enable } => {
            check_desktop_file(file)?;
            let src = autostart_source(env, file).ok_or_else(|| format!("{file} is gone"))?;
            no_link(&src)?;
            let text = std::fs::read_to_string(&src).map_err(|e| e.to_string())?;
            let target = env.autostart_user.join(file);
            no_link(&target)?;
            std::fs::create_dir_all(&env.autostart_user).map_err(|e| e.to_string())?;
            std::fs::write(&target, set_hidden(&text, !*enable)).map_err(|e| e.to_string())?;
            Ok(file.clone())
        }
        Action::SystemdUser { unit, enable, now } => {
            check_unit(unit)?;
            sys.run_ok(&systemd_user_argv(unit, *enable, *now))?;
            Ok(unit.clone())
        }
        Action::LaunchdDaemon { .. } | Action::WinService { .. } => {
            Err("startup: this item needs the administrator".into())
        }
    }
}

pub fn elevated(
    ctx: &Ctx,
    sys: &dyn Sys,
    receipt_id: &str,
    a: &Action,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match a {
        Action::LaunchdDaemon { label, plist } => {
            check_label(label)?;
            let env = Env {
                uid: 0,
                home: PathBuf::new(),
                autostart_user: PathBuf::new(),
                autostart_system: Vec::new(),
            };
            check_plist(&env, plist, true)?;
            super::bootout_argv("system", label)?;
            Ok((
                vec![
                    ElevatedOp::LaunchctlState {
                        domain: "system".into(),
                        label: label.clone(),
                        enabled: false,
                    },
                    ElevatedOp::LaunchctlBootout {
                        domain: "system".into(),
                        label: label.clone(),
                    },
                ],
                None,
            ))
        }
        Action::WinApproved {
            hive,
            subkey,
            name,
            enable,
        } => {
            let key = approved_key(hive, subkey)?;
            check_value_name(name)?;
            let previous = reg_value(sys, &key, name);
            let back = match previous {
                Some(data) => ElevatedOp::RegSet {
                    key: key.clone(),
                    name: name.clone(),
                    kind: "REG_BINARY".into(),
                    data,
                },
                None => ElevatedOp::RegDelete {
                    key: key.clone(),
                    name: Some(name.clone()),
                },
            };
            let u = snapshot(
                ctx,
                receipt_id,
                &format!("{key}\\{name}"),
                &Snapshot::Elevated { ops: vec![back] },
            )?;
            let data = approved_value(*enable, crate::core::washmybelly::store::now());
            Ok((
                vec![ElevatedOp::RegSet {
                    key,
                    name: name.clone(),
                    kind: "REG_BINARY".into(),
                    data,
                }],
                Some(u),
            ))
        }
        Action::WinTask { path, enable, .. } => {
            super::task_argv(path, *enable)?;
            let u = snapshot(
                ctx,
                receipt_id,
                path,
                &Snapshot::Elevated {
                    ops: vec![ElevatedOp::TaskState {
                        path: path.clone(),
                        enabled: !*enable,
                    }],
                },
            )?;
            Ok((
                vec![ElevatedOp::TaskState {
                    path: path.clone(),
                    enabled: *enable,
                }],
                Some(u),
            ))
        }
        Action::WinService { name, mode } => {
            super::service_argv(name, mode, true)?;
            let previous = sys
                .run(&argv_array!["sc.exe", "qc", name.as_str()])
                .ok()
                .and_then(|o| super::windows_start_mode(&o.stdout))
                .ok_or_else(|| format!("{name}: could not read its start type"))?;
            let u = snapshot(
                ctx,
                receipt_id,
                name,
                &Snapshot::Elevated {
                    ops: vec![ElevatedOp::ServiceStartup {
                        name: name.clone(),
                        mode: previous.clone(),
                    }],
                },
            )?;
            remember_turned_off(
                ctx,
                &Action::WinService {
                    name: name.clone(),
                    mode: previous,
                },
            );
            Ok((
                vec![ElevatedOp::ServiceStartup {
                    name: name.clone(),
                    mode: mode.clone(),
                }],
                Some(u),
            ))
        }
        _ => Err("startup: this item has no administrator form".into()),
    }
}

pub fn restore(ctx: &Ctx, sys: &dyn Sys, env: &Env, snap: &Snapshot) -> Result<String, String> {
    match snap {
        Snapshot::Apply { action } => {
            let r = apply(sys, env, action)?;
            remember_turned_off(ctx, action);
            Ok(format!("{r}: back as it was"))
        }
        Snapshot::Elevated { ops } => {
            let results = elevate::run(ctx, ops)?;
            match results.iter().find(|r| !r.ok) {
                Some(bad) => Err(bad.detail.clone()),
                None => {
                    for op in ops {
                        if let ElevatedOp::ServiceStartup { name, .. } = op {
                            forget_turned_off(ctx, &format!("startup:service:{name}"));
                        }
                    }
                    Ok("back as it was (administrator)".into())
                }
            }
        }
        Snapshot::File { path, previous } => {
            if path.parent() != Some(env.autostart_user.as_path()) {
                return Err(format!("startup: refused to restore {}", path.display()));
            }
            no_link(path)?;
            match previous {
                Some(text) => std::fs::write(path, text).map_err(|e| e.to_string())?,
                None if path.exists() => std::fs::remove_file(path).map_err(|e| e.to_string())?,
                None => {}
            }
            Ok(format!(
                "{}: back as it was",
                path.file_name()
                    .map(|f| f.to_string_lossy())
                    .unwrap_or_default()
            ))
        }
        Snapshot::RegValue {
            key,
            name,
            previous,
        } => {
            if !key.starts_with("HKCU\\") {
                return Err(format!("startup: refused to restore {key}"));
            }
            match previous {
                Some(data) => sys.run_ok(&reg_add_argv(key, name, data))?,
                None => sys.run(&reg_delete_argv(key, name))?.stdout,
            };
            Ok(format!("{name}: back as it was"))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnedOff {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub on: Action,
    pub admin: bool,
}

const TURNED_OFF_KEY: &str = "startup_turned_off";

fn turned_off_entry(a: &Action) -> Option<(TurnedOff, bool)> {
    match a {
        Action::LoginItem { name, path, enable } => Some((
            TurnedOff {
                id: format!("startup:login_item:{name}"),
                name: name.clone(),
                kind: "login_item".into(),
                on: Action::LoginItem {
                    name: name.clone(),
                    path: path.clone(),
                    enable: true,
                },
                admin: false,
            },
            !enable,
        )),
        Action::SystemdUser { unit, enable, .. } => Some((
            TurnedOff {
                id: format!("startup:user_service:{unit}"),
                name: unit.trim_end_matches(".service").to_string(),
                kind: "user_service".into(),
                on: Action::SystemdUser {
                    unit: unit.clone(),
                    enable: true,
                    now: true,
                },
                admin: false,
            },
            !enable,
        )),
        Action::WinService { name, mode } => Some((
            TurnedOff {
                id: format!("startup:service:{name}"),
                name: name.clone(),
                kind: "service".into(),
                on: Action::WinService {
                    name: name.clone(),
                    mode: mode.clone(),
                },
                admin: true,
            },
            true,
        )),
        _ => None,
    }
}

pub fn turned_off(ctx: &Ctx) -> Vec<TurnedOff> {
    ctx.kv_get(TURNED_OFF_KEY).unwrap_or_default()
}

pub fn set_turned_off(ctx: &Ctx, list: &[TurnedOff]) {
    let _ = ctx.kv_set(TURNED_OFF_KEY, &list);
}

fn remember_turned_off(ctx: &Ctx, a: &Action) {
    let Some((entry, off)) = turned_off_entry(a) else {
        return;
    };
    let mut list = turned_off(ctx);
    list.retain(|t| t.id != entry.id);
    if off {
        list.push(entry);
    }
    set_turned_off(ctx, &list);
}

fn forget_turned_off(ctx: &Ctx, id: &str) {
    let mut list = turned_off(ctx);
    let before = list.len();
    list.retain(|t| t.id != id);
    if list.len() != before {
        set_turned_off(ctx, &list);
    }
}
