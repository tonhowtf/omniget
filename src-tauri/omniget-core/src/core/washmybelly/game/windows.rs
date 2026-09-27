//! Windows: while the game runs, a performance power plan (the previous one
//! is remembered), `AboveNormal` priority and EcoQoS off for the game only,
//! and the display kept awake by the watcher thread. The watcher waits on
//! the process handle and undoes everything when the game exits.
//!
//! Windows' own Game Mode is a tweak (another module); here it is only read
//! (`HKCU\Software\Microsoft\GameBar\AutoGameModeEnabled`).

use std::time::Duration;

use serde_json::{json, Value};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Power::{PowerGetActiveScheme, PowerSetActiveScheme, SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};
use windows_sys::Win32::System::Threading::{
    GetPriorityClass, GetProcessInformation, OpenProcess, ProcessPowerThrottling, SetPriorityClass, SetProcessInformation, WaitForSingleObject, ABOVE_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SYNCHRONIZE,
};

use super::parse::{guid_string, parse_guid, SCHEME_HIGH_PERFORMANCE, SCHEME_ULTIMATE};
use super::{Applied, ApplyFn, Exec, ExitWaiter, PollWaiter, Target, UndoStep, WaiterFn};

fn to_guid(s: &str) -> Option<GUID> {
    let (data1, data2, data3, data4) = parse_guid(s)?;
    Some(GUID { data1, data2, data3, data4 })
}

fn active_scheme() -> Option<String> {
    let mut p: *mut GUID = std::ptr::null_mut();
    let r = unsafe { PowerGetActiveScheme(std::ptr::null_mut(), &mut p) };
    if r != 0 || p.is_null() {
        return None;
    }
    let g = unsafe { *p };
    unsafe { LocalFree(p as _) };
    Some(guid_string(g.data1, g.data2, g.data3, g.data4))
}

fn set_scheme(s: &str) -> Result<(), String> {
    let g = to_guid(s).ok_or("bad scheme id")?;
    let r = unsafe { PowerSetActiveScheme(std::ptr::null_mut(), &g) };
    if r == 0 {
        Ok(())
    } else {
        Err(format!("power plan {s}: error {r}"))
    }
}

/// A process handle that closes itself.
struct Proc(HANDLE);

impl Proc {
    fn open(pid: u32, access: u32) -> Result<Proc, String> {
        let h = unsafe { OpenProcess(access, 0, pid) };
        if h.is_null() {
            Err(format!("open process {pid}: {}", std::io::Error::last_os_error()))
        } else {
            Ok(Proc(h))
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn get_throttling(p: &Proc) -> (u32, u32) {
    let mut st = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: 0,
        StateMask: 0,
    };
    let ok = unsafe { GetProcessInformation(p.0, ProcessPowerThrottling, &mut st as *mut _ as *mut _, std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32) };
    // Not readable: treat as "the system decides" (both masks 0).
    if ok == 0 {
        (0, 0)
    } else {
        (st.ControlMask, st.StateMask)
    }
}

fn set_throttling(p: &Proc, control: u32, state: u32) -> Result<(), String> {
    let st = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: control,
        StateMask: state,
    };
    let ok = unsafe { SetProcessInformation(p.0, ProcessPowerThrottling, &st as *const _ as *const _, std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32) };
    if ok == 0 {
        Err(format!("EcoQoS: {}", std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

pub struct Sys;

impl Exec for Sys {
    fn undo(&self, step: &UndoStep) -> Result<String, String> {
        match step {
            UndoStep::PowerScheme { prev, set } => {
                // Someone picked another plan meanwhile: leave theirs.
                if active_scheme().as_deref() != Some(set.as_str()) {
                    return Ok("power plan was changed since; left as is".into());
                }
                set_scheme(prev).map(|_| "power plan restored".into())
            }
            UndoStep::PriorityClass { pid, prev, .. } => {
                let p = Proc::open(*pid, PROCESS_SET_INFORMATION)?;
                if unsafe { SetPriorityClass(p.0, *prev) } == 0 {
                    return Err(format!("priority: {}", std::io::Error::last_os_error()));
                }
                Ok("priority restored".into())
            }
            UndoStep::PowerThrottling { pid, control, state, .. } => {
                let p = Proc::open(*pid, PROCESS_SET_INFORMATION)?;
                set_throttling(&p, *control, *state).map(|_| "EcoQoS restored".into())
            }
            UndoStep::ExecutionState => {
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
                Ok("display may sleep again".into())
            }
            other => Err(format!("not a Windows step: {other:?}")),
        }
    }
}

/// Runs on the watcher thread, so the execution state belongs to it.
fn apply(target: &Target) -> Result<Applied, String> {
    let mut out = Applied {
        method: "windows".into(),
        ..Default::default()
    };
    // Power plan.
    match active_scheme() {
        Some(prev) if prev == SCHEME_HIGH_PERFORMANCE || prev == SCHEME_ULTIMATE => out.notes.push("power_plan_already_performance".into()),
        Some(prev) => match set_scheme(SCHEME_HIGH_PERFORMANCE) {
            Ok(()) => {
                out.applied.push("power_plan".into());
                out.undo.push(UndoStep::PowerScheme {
                    prev,
                    set: SCHEME_HIGH_PERFORMANCE.into(),
                });
            }
            Err(_) => out.notes.push("no_performance_plan".into()),
        },
        None => out.notes.push("power_plan_unreadable".into()),
    }
    // Priority and EcoQoS of the game only.
    match Proc::open(target.pid, PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION) {
        Ok(p) => {
            let prev = unsafe { GetPriorityClass(p.0) };
            if prev == NORMAL_PRIORITY_CLASS {
                // Only from Normal: a class someone chose on purpose (high,
                // below normal...) is left alone.
                if unsafe { SetPriorityClass(p.0, ABOVE_NORMAL_PRIORITY_CLASS) } != 0 {
                    out.applied.push("priority".into());
                    out.undo.push(UndoStep::PriorityClass {
                        pid: target.pid,
                        started: target.started,
                        prev,
                    });
                }
            } else if prev == 0 {
                out.notes.push("priority_unreadable".into());
            } else {
                out.notes.push("priority_left_as_chosen".into());
            }
            let (control, state) = get_throttling(&p);
            if set_throttling(&p, PROCESS_POWER_THROTTLING_EXECUTION_SPEED, 0).is_ok() {
                out.applied.push("ecoqos_off".into());
                out.undo.push(UndoStep::PowerThrottling {
                    pid: target.pid,
                    started: target.started,
                    control,
                    state,
                });
            }
        }
        Err(_) => out.notes.push("game_not_accessible".into()),
    }
    // Display awake, owned by this thread.
    if unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) } != 0 {
        out.applied.push("display_awake".into());
        out.undo.push(UndoStep::ExecutionState);
    }
    if out.applied.is_empty() {
        let _ = super::undo_all(&Sys, &out.undo);
        return Err(format!("NOTHING_APPLIED: {}", out.notes.join(", ")));
    }
    Ok(out)
}

/// `WaitForSingleObject` on the game's handle.
struct HandleWaiter {
    h: Option<Proc>,
    fallback: PollWaiter,
}

// The handle is only used by the watcher thread that owns it.
unsafe impl Send for HandleWaiter {}

impl ExitWaiter for HandleWaiter {
    fn wait(&mut self, timeout: Duration) -> bool {
        match &self.h {
            Some(p) => unsafe { WaitForSingleObject(p.0, timeout.as_millis().min(u32::MAX as u128) as u32) == WAIT_OBJECT_0 },
            None => self.fallback.wait(timeout),
        }
    }
}

pub fn plan(target: &Target) -> (ApplyFn, WaiterFn) {
    let t = target.clone();
    let (pid, started) = (target.pid, target.started);
    (
        Box::new(move || apply(&t)),
        Box::new(move || {
            Box::new(HandleWaiter {
                h: Proc::open(pid, PROCESS_SYNCHRONIZE).ok(),
                fallback: PollWaiter { pid, started },
            }) as Box<dyn ExitWaiter>
        }),
    )
}

pub fn status(_pid: Option<u32>) -> Value {
    let reg = crate::core::process::std_command("reg")
        .args(["query", r"HKCU\Software\Microsoft\GameBar", "/v", "AutoGameModeEnabled"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let auto = super::parse::auto_game_mode(&reg);
    let scheme = active_scheme();
    json!({
        "backend": "windows",
        "available": true,
        "can_force": true,
        "reason": Value::Null,
        "system": {
            // null: the value is absent and Windows uses its default (on).
            "auto_game_mode": auto,
            "active_power_plan": scheme,
            "performance_plan_active": scheme.as_deref().is_some_and(|s| s == SCHEME_HIGH_PERFORMANCE || s == SCHEME_ULTIMATE),
        },
        "offers": ["power_plan", "priority", "ecoqos_off", "display_awake"],
        "install": Value::Null,
    })
}
