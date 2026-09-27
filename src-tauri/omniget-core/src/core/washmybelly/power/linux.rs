//! Linux: power-profiles-daemon (and tlp-pd / tuned-ppd, same D-Bus API),
//! systemd units of the other managers, logind inhibitors, sysfs knobs.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;

use serde_json::{json, Map, Value};
use zbus::zvariant::{OwnedFd, OwnedValue, Value as ZValue};

use super::arbiter::Manager;
use super::hold::Backend;
use super::parse;

const PP_NAME: &str = "org.freedesktop.UPower.PowerProfiles";
const PP_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
const PP_LEGACY: &str = "net.hadess.PowerProfiles";
const PP_LEGACY_PATH: &str = "/net/hadess/PowerProfiles";

fn conv(v: &ZValue) -> Value {
    match v {
        ZValue::Bool(b) => json!(b),
        ZValue::U8(n) => json!(n),
        ZValue::I16(n) => json!(n),
        ZValue::U16(n) => json!(n),
        ZValue::I32(n) => json!(n),
        ZValue::U32(n) => json!(n),
        ZValue::I64(n) => json!(n),
        ZValue::U64(n) => json!(n),
        ZValue::F64(n) => json!(n),
        ZValue::Str(s) => json!(s.as_str()),
        ZValue::ObjectPath(p) => json!(p.as_str()),
        ZValue::Value(b) => conv(b),
        ZValue::Array(a) => Value::Array(a.inner().iter().map(conv).collect()),
        ZValue::Dict(d) => {
            let mut m = Map::new();
            for (k, v) in d.iter() {
                if let ZValue::Str(k) = k {
                    m.insert(k.as_str().to_string(), conv(v));
                }
            }
            Value::Object(m)
        }
        _ => Value::Null,
    }
}

fn system() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::Connection::system().map_err(|e| format!("system bus: {e}"))
}

/// Which name answers: the current one or the legacy one.
fn pp_endpoint(conn: &zbus::blocking::Connection) -> Option<(&'static str, &'static str)> {
    for (name, path) in [(PP_NAME, PP_PATH), (PP_LEGACY, PP_LEGACY_PATH)] {
        let has: Result<bool, _> = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(name,),
            )
            .and_then(|m| m.body().deserialize::<bool>());
        if has.unwrap_or(false) {
            return Some((name, path));
        }
    }
    None
}

/// `ActiveProfile`, `Profiles`, `ActiveProfileHolds`, `Version` and the
/// process that owns the name.
pub fn profiles() -> Result<Option<Value>, String> {
    let conn = system()?;
    let Some((name, path)) = pp_endpoint(&conn) else {
        return Ok(None);
    };
    let props: HashMap<String, OwnedValue> = conn
        .call_method(
            Some(name),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "GetAll",
            &(name,),
        )
        .map_err(|e| format!("PowerProfiles: {e}"))?
        .body()
        .deserialize()
        .map_err(|e| format!("PowerProfiles reply: {e}"))?;
    let get = |k: &str| props.get(k).map(|v| conv(v)).unwrap_or(Value::Null);
    let owner = conn
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionUnixProcessID",
            &(name,),
        )
        .ok()
        .and_then(|m| m.body().deserialize::<u32>().ok())
        .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
        .map(|s| s.trim().to_string());
    let version = get("Version");
    let daemon = super::sysfs::owner_manager(owner.as_deref(), version.as_str());
    let list: Vec<String> = get("Profiles")
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p["Profile"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let holds: Vec<Value> = get("ActiveProfileHolds")
        .as_array()
        .map(|a| {
            a.iter()
                .map(|h| json!({ "profile": h["Profile"], "reason": h["Reason"], "app": h["ApplicationId"] }))
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(json!({
        "daemon": daemon.map(|m| m.id()),
        "process": owner,
        "version": version,
        "active": get("ActiveProfile"),
        "profiles": list,
        "holds": holds,
        "degraded": get("PerformanceDegraded"),
        "bus_name": name,
    })))
}

/// Switch the profile through the daemon (polkit lets the active session).
pub fn set_profile(profile: &str) -> Result<(), String> {
    let conn = system()?;
    let (name, path) = pp_endpoint(&conn).ok_or("no power profile daemon is running")?;
    conn.call_method(
        Some(name),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &(name, "ActiveProfile", ZValue::from(profile)),
    )
    .map_err(|e| format!("ActiveProfile: {e}"))?;
    Ok(())
}

fn systemd() -> bool {
    Path::new("/run/systemd/system").exists()
}

fn unit_active(unit: &str) -> bool {
    crate::core::process::std_command("systemctl")
        .args(["is-active", "--quiet", "--", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Process names under `/proc/*/comm` (15 chars at most).
fn comms() -> Vec<String> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .chars()
                .all(|c| c.is_ascii_digit())
        })
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .map(|s| s.trim().to_string())
        .collect()
}

/// Managers running now: systemd units, or processes without systemd.
pub fn running_managers() -> Vec<Manager> {
    if systemd() {
        return Manager::ALL
            .into_iter()
            .filter(|m| unit_active(m.unit()))
            .collect();
    }
    let c = comms();
    let has = |p: &str| c.iter().any(|x| x.starts_with(p));
    Manager::ALL
        .into_iter()
        .filter(|m| match m {
            Manager::PowerProfilesDaemon => has("power-profiles"),
            Manager::TlpPd => has("tlp-pd"),
            Manager::TunedPpd => has("tuned-ppd"),
            Manager::Tuned => c.iter().any(|x| x == "tuned"),
            Manager::AutoCpufreq => has("auto-cpufreq"),
            // TLP is not a daemon: without systemd, look for its config run.
            Manager::Tlp => Path::new("/run/tlp").exists(),
        })
        .collect()
}

/// `tlp-stat -s` when it answers without root (older versions refuse).
pub fn tlp_stat() -> Option<parse::TlpStat> {
    let out = crate::core::process::std_command("tlp-stat")
        .arg("-s")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = parse::tlp_stat(&String::from_utf8_lossy(&out.stdout));
    s.enabled.is_some().then_some(s)
}

/// logind inhibitors that block sleep or idle.
pub fn sleep_blockers() -> Result<Vec<parse::SleepBlocker>, String> {
    let conn = system()?;
    let list: Vec<(String, String, String, String, u32, u32)> = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "ListInhibitors",
            &(),
        )
        .map_err(|e| format!("logind: {e}"))?
        .body()
        .deserialize()
        .map_err(|e| format!("logind reply: {e}"))?;
    Ok(list
        .into_iter()
        .filter(|(what, _, _, mode, _, _)| {
            mode == "block" && (what.contains("sleep") || what.contains("idle"))
        })
        .map(|(what, who, why, _, _, pid)| parse::SleepBlocker {
            process: who,
            pid: Some(pid),
            display: what.contains("idle"),
            kind: what,
            reason: why,
        })
        .collect())
}

// ── Hold: HoldProfile + a logind idle inhibitor, owned by one thread ──

pub struct DbusBackend;

impl Backend for DbusBackend {
    fn name(&self) -> &'static str {
        "power-profiles + logind"
    }

    fn acquire(&self, reason: &str, max_secs: Option<u64>) -> Result<Box<dyn Send>, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let reason = reason.to_string();
        std::thread::Builder::new()
            .name("wmb-power-hold-dbus".into())
            .spawn(move || {
                let conn = match system() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let why = if reason.is_empty() {
                    "OmniGet is working".to_string()
                } else {
                    reason
                };
                // The daemon drops the hold when this connection closes, so
                // the thread keeps it open until release or the deadline.
                let cookie: Option<(&str, &str, u32)> =
                    pp_endpoint(&conn).and_then(|(name, path)| {
                        conn.call_method(
                            Some(name),
                            path,
                            Some(name),
                            "HoldProfile",
                            &("performance", why.as_str(), "OmniGet"),
                        )
                        .ok()
                        .and_then(|m| m.body().deserialize::<u32>().ok())
                        .map(|c| (name, path, c))
                    });
                let fd: Option<OwnedFd> = conn
                    .call_method(
                        Some("org.freedesktop.login1"),
                        "/org/freedesktop/login1",
                        Some("org.freedesktop.login1.Manager"),
                        "Inhibit",
                        &("idle:sleep", "OmniGet", why.as_str(), "block"),
                    )
                    .ok()
                    .and_then(|m| m.body().deserialize::<OwnedFd>().ok());
                if cookie.is_none() && fd.is_none() {
                    let _ = ready_tx.send(Err(
                        "neither a power profile daemon nor logind answered".into(),
                    ));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                match max_secs {
                    Some(s) => {
                        let _ = stop_rx.recv_timeout(std::time::Duration::from_secs(s + 5));
                    }
                    None => {
                        let _ = stop_rx.recv();
                    }
                }
                if let Some((name, path, c)) = cookie {
                    let _ = conn.call_method(Some(name), path, Some(name), "ReleaseProfile", &(c,));
                }
                drop(fd);
            })
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "the power hold did not answer".to_string())??;
        Ok(Box::new(stop_tx))
    }
}

// ── Administrator side (runs in omniget-elevate as root) ────────────

/// Stop, disable and mask one manager unit, or undo it.
pub fn manager_unit(unit: &str, on: bool) -> Result<String, String> {
    Manager::from_unit(unit).ok_or_else(|| format!("{unit} is not a power manager unit"))?;
    let steps: Vec<Vec<&str>> = if on {
        vec![
            vec!["unmask", "--", unit],
            vec!["enable", "--now", "--", unit],
        ]
    } else {
        vec![
            vec!["disable", "--now", "--", unit],
            vec!["mask", "--", unit],
        ]
    };
    for args in steps {
        let out = crate::core::process::std_command("systemctl")
            .args(&args)
            .output()
            .map_err(|e| format!("systemctl: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "systemctl {}: {}",
                args[0],
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(format!(
        "{unit}: {}",
        if on { "enabled" } else { "stopped and masked" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_unit_refuses_other_units() {
        assert!(manager_unit("sshd.service", false).is_err());
    }
}
