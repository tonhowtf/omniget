//! Windows: power plans through `powercfg` (argv, GUIDs validated), the
//! "power mode" overlay from the registry, and `PowerSetRequest` for the
//! hold.

use std::path::Path;
use std::sync::mpsc;

use super::hold::Backend;
use super::parse;

fn powercfg(args: &[&str]) -> Result<String, String> {
    let out = crate::core::process::std_command("powercfg")
        .args(args)
        .output()
        .map_err(|e| format!("powercfg: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("powercfg {}: {}", args.first().unwrap_or(&""), if err.trim().is_empty() { text.trim() } else { err.trim() }));
    }
    Ok(text)
}

pub fn active_scheme() -> Result<Option<parse::Scheme>, String> {
    Ok(parse::powercfg_active(&powercfg(&["/getactivescheme"])?))
}

pub fn schemes() -> Result<Vec<parse::Scheme>, String> {
    Ok(parse::powercfg_list(&powercfg(&["/list"])?))
}

/// `(ac, dc)` overlay names of the Settings "power mode" slider.
pub fn overlay() -> (Option<&'static str>, Option<&'static str>) {
    let out = crate::core::process::std_command("reg")
        .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes"])
        .output();
    let Ok(out) = out else { return (None, None) };
    let vals = super::super::parse::reg_query(&String::from_utf8_lossy(&out.stdout));
    let get = |n: &str| vals.iter().find(|v| v.name == n).map(|v| parse::overlay_name(&v.data));
    (get("ActiveOverlayAcPowerScheme"), get("ActiveOverlayDcPowerScheme"))
}

/// `powercfg /requests` needs an administrator; `Err` says so.
pub fn requests() -> Result<Vec<parse::SleepBlocker>, String> {
    Ok(parse::powercfg_requests(&powercfg(&["/requests"])?))
}

fn guid(g: &str) -> Result<&str, String> {
    if parse::is_guid(g) {
        Ok(g)
    } else {
        Err(format!("not a plan GUID: {g:?}"))
    }
}

pub fn set_active(g: &str) -> Result<(), String> {
    powercfg(&["/setactive", guid(g)?]).map(|_| ())
}

/// Copy the hidden Ultimate Performance template; returns the new GUID.
/// Creating a plan does not need an administrator.
pub fn duplicate_ultimate() -> Result<String, String> {
    let out = powercfg(&["-duplicatescheme", parse::ULTIMATE_TEMPLATE])?;
    parse::powercfg_active(&out)
        .map(|s| s.guid)
        .ok_or_else(|| "powercfg did not print the new plan".into())
}

pub fn delete_scheme(g: &str) -> Result<(), String> {
    powercfg(&["/delete", guid(g)?]).map(|_| ())
}

/// `powercfg /batteryreport` as HTML into our own folder (path in argv).
pub fn battery_report(out: &Path) -> Result<(), String> {
    let status = crate::core::process::std_command("powercfg")
        .arg("/batteryreport")
        .arg("/output")
        .arg(out)
        .output()
        .map_err(|e| format!("powercfg: {e}"))?;
    if status.status.success() && out.exists() {
        Ok(())
    } else {
        Err("powercfg could not write the battery report".into())
    }
}

/// A power request owned by one thread: created, set, and cleared there.
pub struct PowerRequestBackend;

impl Backend for PowerRequestBackend {
    fn name(&self) -> &'static str {
        "PowerSetRequest"
    }

    fn acquire(&self, reason: &str, max_secs: Option<u64>) -> Result<Box<dyn Send>, String> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Power::{PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest};
        use windows_sys::Win32::System::Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT};

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let text = if reason.is_empty() { "OmniGet is working".to_string() } else { format!("OmniGet: {reason}") };
        std::thread::Builder::new()
            .name("wmb-power-hold-win".into())
            .spawn(move || {
                let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let mut ctx: REASON_CONTEXT = unsafe { std::mem::zeroed() };
                ctx.Version = 0; // POWER_REQUEST_CONTEXT_VERSION
                ctx.Flags = POWER_REQUEST_CONTEXT_SIMPLE_STRING;
                ctx.Reason.SimpleReasonString = wide.as_mut_ptr();
                let h = unsafe { PowerCreateRequest(&ctx) };
                if h.is_null() || h == INVALID_HANDLE_VALUE {
                    let _ = ready_tx.send(Err("PowerCreateRequest failed".into()));
                    return;
                }
                if unsafe { PowerSetRequest(h, PowerRequestSystemRequired) } == 0 {
                    unsafe { CloseHandle(h) };
                    let _ = ready_tx.send(Err("PowerSetRequest failed".into()));
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
                unsafe {
                    PowerClearRequest(h, PowerRequestSystemRequired);
                    CloseHandle(h);
                }
                drop(wide);
            })
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "the power request did not answer".to_string())??;
        Ok(Box::new(stop_tx))
    }
}
