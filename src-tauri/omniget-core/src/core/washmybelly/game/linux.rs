//! Linux: gamemode over D-Bus (session bus), the portal inside Flatpak, and
//! a fallback OmniGet undoes by itself when gamemode is not there.
//!
//! The D-Bus calls are built in [`super::dbus`] (tested on every system);
//! this file only sends them. Connections that hold something (the power
//! profile hold, the screen saver inhibit) live in a thread local of the
//! watcher thread, so they close with it: even a crash releases them.

use std::cell::RefCell;

use serde_json::{json, Value};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use super::children::own_children;
use super::dbus::{self, Arg, Bus, Call, Verb};
use super::{Active, Applied, ApplyFn, Exec, ExitWaiter, PollWaiter, Target, UndoStep, WaiterFn};

thread_local! {
    static KEPT: RefCell<Vec<(Bus, Connection)>> = const { RefCell::new(Vec::new()) };
}

fn connect(bus: Bus) -> Result<Connection, String> {
    match bus {
        Bus::Session => Connection::session().map_err(|e| format!("session bus: {e}")),
        Bus::System => Connection::system().map_err(|e| format!("system bus: {e}")),
    }
}

/// Send one [`Call`]. Only the argument shapes the module builds.
fn send(conn: &Connection, c: &Call) -> Result<zbus::message::Message, String> {
    let (d, p, i, m) = (Some(c.dest), c.path, Some(c.iface), c.method);
    let r = match c.args.as_slice() {
        [] => conn.call_method(d, p, i, m, &()),
        [Arg::I32(a)] => conn.call_method(d, p, i, m, &(*a,)),
        [Arg::I32(a), Arg::I32(b)] => conn.call_method(d, p, i, m, &(*a, *b)),
        [Arg::U32(a)] => conn.call_method(d, p, i, m, &(*a,)),
        [Arg::Str(a), Arg::Str(b)] => conn.call_method(d, p, i, m, &(a.as_str(), b.as_str())),
        [Arg::Str(a), Arg::Str(b), Arg::Str(x)] => {
            conn.call_method(d, p, i, m, &(a.as_str(), b.as_str(), x.as_str()))
        }
        _ => {
            return Err(format!(
                "{}: argument shape {} not supported",
                c.method,
                c.signature()
            ))
        }
    };
    r.map_err(|e| format!("{}.{}: {e}", c.iface, c.method))
}

fn send_i32(conn: &Connection, c: &Call) -> Result<i32, String> {
    send(conn, c)?
        .body()
        .deserialize::<i32>()
        .map_err(|e| format!("{}: {e}", c.method))
}

fn send_u32(conn: &Connection, c: &Call) -> Result<u32, String> {
    send(conn, c)?
        .body()
        .deserialize::<u32>()
        .map_err(|e| format!("{}: {e}", c.method))
}

fn get_prop(
    conn: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
) -> Result<OwnedValue, String> {
    conn.call_method(
        Some(dest),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(iface, name),
    )
    .map_err(|e| format!("{iface}.{name}: {e}"))?
    .body()
    .deserialize::<OwnedValue>()
    .map_err(|e| e.to_string())
}

fn name_has_owner(conn: &Connection, name: &str) -> bool {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(name,),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

fn name_activatable(conn: &Connection, name: &str) -> bool {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListActivatableNames",
        &(),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<Vec<String>>().ok())
    .is_some_and(|v| v.iter().any(|n| n == name))
}

fn in_flatpak() -> bool {
    crate::core::dependencies::is_flatpak()
}

fn self_pid() -> i32 {
    std::process::id() as i32
}

// ── Priority of OmniGet's own children ───────────────────────────────────

const IOPRIO_WHO_PROCESS: libc::c_int = 1;
const IOPRIO_CLASS_SHIFT: i32 = 13;
const IOPRIO_IDLE: i32 = 3 << IOPRIO_CLASS_SHIFT;
const CHILD_NICE: i32 = 10;

/// Threads of a process: nice and I/O priority are per thread on Linux.
fn tasks(pid: u32) -> Vec<u32> {
    let mut v: Vec<u32> = std::fs::read_dir(format!("/proc/{pid}/task"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    if v.is_empty() {
        v.push(pid);
    }
    v
}

fn get_nice(pid: u32) -> Option<i32> {
    unsafe {
        *libc::__errno_location() = 0;
        let r = libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t);
        if r == -1 && *libc::__errno_location() != 0 {
            return None;
        }
        Some(r)
    }
}

fn set_nice(pid: u32, v: i32) -> Result<(), String> {
    let mut last = Ok(());
    for t in tasks(pid) {
        if unsafe { libc::setpriority(libc::PRIO_PROCESS, t as libc::id_t, v) } != 0 {
            last = Err(std::io::Error::last_os_error());
        }
    }
    last.map_err(|e| format!("nice {pid}: {e}"))
}

fn get_ioprio(pid: u32) -> Option<i32> {
    let r = unsafe { libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, pid as libc::c_int) };
    (r >= 0).then_some(r as i32)
}

fn set_ioprio(pid: u32, v: i32) -> Result<(), String> {
    let mut last = Ok(());
    for t in tasks(pid) {
        if unsafe {
            libc::syscall(
                libc::SYS_ioprio_set,
                IOPRIO_WHO_PROCESS,
                t as libc::c_int,
                v,
            )
        } != 0
        {
            last = Err(std::io::Error::last_os_error());
        }
    }
    last.map_err(|e| format!("ioprio {pid}: {e}"))
}

fn lower_children(game: u32, known: &[u32]) -> Vec<UndoStep> {
    let mut out = Vec::new();
    for c in own_children(game)
        .into_iter()
        .filter(|c| !known.contains(&c.pid))
    {
        if let Some(prev) = get_ioprio(c.pid) {
            if prev != IOPRIO_IDLE && set_ioprio(c.pid, IOPRIO_IDLE).is_ok() {
                out.push(UndoStep::IoPrio {
                    pid: c.pid,
                    started: c.started,
                    prev,
                });
            }
        }
        if let Some(prev) = get_nice(c.pid) {
            if prev < CHILD_NICE && set_nice(c.pid, CHILD_NICE).is_ok() {
                out.push(UndoStep::Nice {
                    pid: c.pid,
                    started: c.started,
                    prev,
                });
            }
        }
    }
    out
}

// ── Executor ─────────────────────────────────────────────────────────────

pub struct Sys;

fn with_kept<T>(bus: Bus, f: impl FnOnce(&Connection) -> T) -> Option<T> {
    KEPT.with(|k| {
        k.borrow()
            .iter()
            .find(|(b, _)| *b == bus)
            .map(|(_, c)| f(c))
    })
}

impl Exec for Sys {
    fn undo(&self, step: &UndoStep) -> Result<String, String> {
        match step {
            UndoStep::GamemodeUnregister {
                portal,
                caller,
                game,
            } => {
                let call = if *portal {
                    dbus::portal(Verb::Unregister, *game)
                } else {
                    dbus::daemon_by_pid(Verb::Unregister, *caller, *game)
                };
                let conn = connect(Bus::Session)?;
                let code = send_i32(&conn, &call)?;
                // -1 after the game exited: the daemon already dropped it.
                Ok(format!(
                    "gamemode: {}",
                    dbus::reply_meaning(Verb::Unregister, code)
                ))
            }
            UndoStep::ReleaseProfile { cookie } => match with_kept(Bus::System, |c| {
                send(c, &dbus::release_profile(*cookie)).map(|_| ())
            }) {
                Some(r) => r.map(|_| "power profile released".into()),
                None => Ok("released when OmniGet's connection closed".into()),
            },
            UndoStep::UnInhibit { cookie } => match with_kept(Bus::Session, |c| {
                send(c, &dbus::screensaver_uninhibit(*cookie)).map(|_| ())
            }) {
                Some(r) => r.map(|_| "screen saver allowed again".into()),
                None => Ok("released when OmniGet's connection closed".into()),
            },
            UndoStep::IoPrio { pid, prev, .. } => {
                set_ioprio(*pid, *prev).map(|_| "I/O priority restored".into())
            }
            UndoStep::Nice { pid, prev, .. } => set_nice(*pid, *prev)
                .map(|_| "priority restored".into())
                .map_err(|e| {
                    // A normal user may not raise priority back (RLIMIT_NICE).
                    format!("KEPT_LOWER: {e}; it stays lower until that task ends")
                }),
            other => Err(format!("not a Linux step: {other:?}")),
        }
    }

    fn tick(&self, active: &Active) -> Vec<UndoStep> {
        if active.method != "native" {
            return Vec::new();
        }
        let known: Vec<u32> = active.undo.iter().filter_map(UndoStep::child_pid).collect();
        lower_children(active.pid, &known)
    }
}

// ── Apply ────────────────────────────────────────────────────────────────

fn try_gamemode(target: &Target, notes: &mut Vec<String>) -> Option<Applied> {
    let conn = match connect(Bus::Session) {
        Ok(c) => c,
        Err(e) => {
            notes.push(format!("no_session_bus: {e}"));
            return None;
        }
    };
    let flatpak = in_flatpak();
    let game = target.pid as i32;
    let call = if flatpak {
        dbus::portal(Verb::Register, game)
    } else {
        dbus::daemon_by_pid(Verb::Register, self_pid(), game)
    };
    match send_i32(&conn, &call) {
        Ok(0) => Some(Applied {
            method: if flatpak { "portal" } else { "feral_gamemode" }.into(),
            applied: vec!["gamemode".into()],
            notes: notes.clone(),
            undo: vec![UndoStep::GamemodeUnregister {
                portal: flatpak,
                caller: self_pid(),
                game,
            }],
        }),
        Ok(code) => {
            notes.push(format!(
                "gamemode_{}",
                dbus::reply_meaning(Verb::Register, code)
            ));
            None
        }
        Err(e) => {
            notes.push(format!("gamemode_unavailable: {e}"));
            None
        }
    }
}

fn fallback(target: &Target, mut notes: Vec<String>) -> Applied {
    let mut out = Applied {
        method: "native".into(),
        ..Default::default()
    };
    match connect(Bus::System).and_then(|c| send_u32(&c, &dbus::hold_profile()).map(|k| (c, k))) {
        Ok((conn, cookie)) => {
            KEPT.with(|k| k.borrow_mut().push((Bus::System, conn)));
            out.applied.push("power_profile".into());
            out.undo.push(UndoStep::ReleaseProfile { cookie });
        }
        Err(e) => notes.push(format!("power_profile_unavailable: {e}")),
    }
    match connect(Bus::Session)
        .and_then(|c| send_u32(&c, &dbus::screensaver_inhibit()).map(|k| (c, k)))
    {
        Ok((conn, cookie)) => {
            KEPT.with(|k| k.borrow_mut().push((Bus::Session, conn)));
            out.applied.push("screensaver_inhibit".into());
            out.undo.push(UndoStep::UnInhibit { cookie });
        }
        Err(e) => notes.push(format!("screensaver_unavailable: {e}")),
    }
    let kids = lower_children(target.pid, &[]);
    if !kids.is_empty() {
        out.applied.push("children_lowered".into());
    }
    out.undo.extend(kids);
    out.applied.push("watch_children".into());
    out.notes = notes;
    out
}

fn apply(target: &Target) -> Result<Applied, String> {
    let mut notes = Vec::new();
    if let Some(a) = try_gamemode(target, &mut notes) {
        return Ok(a);
    }
    Ok(fallback(target, notes))
}

pub fn plan(target: &Target) -> (ApplyFn, WaiterFn) {
    let t = target.clone();
    let (pid, started) = (target.pid, target.started);
    (
        Box::new(move || apply(&t)),
        Box::new(move || Box::new(PollWaiter { pid, started }) as Box<dyn ExitWaiter>),
    )
}

// ── Status ───────────────────────────────────────────────────────────────

fn install_hint() -> Value {
    let caps = super::super::caps::cached(false);
    let managers: Vec<String> = caps["linux"]["package_managers"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match super::parse::install_command(&managers) {
        Some((m, argv)) => json!({ "manager": m, "argv": argv }),
        None => Value::Null,
    }
}

pub fn status(pid: Option<u32>) -> Value {
    let conn = Connection::session().ok();
    if in_flatpak() {
        let version = conn
            .as_ref()
            .and_then(|c| {
                get_prop(
                    c,
                    dbus::PORTAL_NAME,
                    dbus::PORTAL_PATH,
                    dbus::PORTAL_GAMEMODE_IFACE,
                    "version",
                )
                .ok()
            })
            .and_then(|v| u32::try_from(v).ok());
        let active = conn
            .as_ref()
            .and_then(|c| {
                get_prop(
                    c,
                    dbus::PORTAL_NAME,
                    dbus::PORTAL_PATH,
                    dbus::PORTAL_GAMEMODE_IFACE,
                    "Active",
                )
                .ok()
            })
            .and_then(|v| bool::try_from(v).ok());
        let mut v = json!({
            "backend": if version.is_some() { "portal" } else { "native" },
            "available": true,
            "can_force": version.is_some(),
            "reason": if version.is_some() { Value::Null } else { json!("portal_missing") },
            "system": { "flatpak": true, "portal_version": version, "gamemode_active": active },
            "install": Value::Null,
        });
        if let (Some(p), Some(c)) = (pid, conn.as_ref()) {
            v["pid_gamemode"] = send_i32(c, &dbus::portal(Verb::Query, p as i32))
                .map(|r| json!(dbus::reply_meaning(Verb::Query, r)))
                .unwrap_or(Value::Null);
        }
        return v;
    }
    let running = conn
        .as_ref()
        .is_some_and(|c| name_has_owner(c, dbus::GAMEMODE_NAME));
    let activatable = conn
        .as_ref()
        .is_some_and(|c| name_activatable(c, dbus::GAMEMODE_NAME));
    let installed = running
        || activatable
        || super::super::caps::cached(false)["linux"]["gamemode"]
            .as_bool()
            .unwrap_or(false);
    let (client_count, games) = match (running, conn.as_ref()) {
        (true, Some(c)) => {
            let n = get_prop(
                c,
                dbus::GAMEMODE_NAME,
                dbus::GAMEMODE_PATH,
                dbus::GAMEMODE_IFACE,
                "ClientCount",
            )
            .ok()
            .and_then(|v| i32::try_from(v).ok());
            let g: Vec<Value> = send(c, &dbus::list_games())
                .ok()
                .and_then(|m| m.body().deserialize::<Vec<(i32, OwnedObjectPath)>>().ok())
                .unwrap_or_default()
                .into_iter()
                .map(|(pid, _)| json!({ "pid": pid }))
                .collect();
            (n, g)
        }
        _ => (None, Vec::new()),
    };
    let mut v = json!({
        "backend": if installed { "feral_gamemode" } else { "native" },
        "available": true,
        "can_force": installed,
        "reason": if installed { Value::Null } else { json!("gamemode_missing") },
        "system": {
            "flatpak": false,
            "installed": installed,
            "daemon_running": running,
            "client_count": client_count,
            "games": games,
        },
        "install": if installed { Value::Null } else { install_hint() },
        "fallback": ["power_profile", "screensaver_inhibit", "children_lowered"],
    });
    if let (Some(p), Some(c), true) = (pid, conn.as_ref(), installed) {
        v["pid_gamemode"] = send_i32(c, &dbus::daemon_single(Verb::Query, p as i32))
            .map(|r| json!(dbus::reply_meaning(Verb::Query, r)))
            .unwrap_or(Value::Null);
    }
    v
}
