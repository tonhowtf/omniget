//! Windows readings without admin: GPU load from the PDH `GPU Engine`
//! counters (what Task Manager shows), battery from `GetSystemPowerStatus`
//! and `powercfg /batteryreport /xml`, process priority by exact PID.

use std::sync::Mutex;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::Threading::{
    OpenProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS, PROCESS_SET_INFORMATION,
};

use super::parse::{self, Battery, Gpu};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct GpuQuery {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    primed: bool,
}

// PDH handles are used only under the mutex.
unsafe impl Send for GpuQuery {}

static GPU: Mutex<Option<Result<GpuQuery, String>>> = Mutex::new(None);

fn open_query() -> Result<GpuQuery, String> {
    unsafe {
        let mut query: PDH_HQUERY = std::ptr::null_mut();
        let rc = PdhOpenQueryW(std::ptr::null(), 0, &mut query);
        if rc != 0 {
            return Err(format!("PdhOpenQuery = {rc:#x}"));
        }
        let path = wide("\\GPU Engine(*)\\Utilization Percentage");
        let mut counter: PDH_HCOUNTER = std::ptr::null_mut();
        let rc = PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter);
        if rc != 0 {
            return Err(format!("GPU Engine counter unavailable ({rc:#x})"));
        }
        Ok(GpuQuery {
            query,
            counter,
            primed: false,
        })
    }
}

/// GPU load per adapter. PDH needs two samples: the first call primes the
/// query and returns an empty list; later calls give the load since the
/// previous one.
pub fn gpus() -> Result<Vec<Gpu>, String> {
    let mut g = GPU.lock().map_err(|_| "gpu lock")?;
    if g.is_none() {
        *g = Some(open_query());
    }
    let q = match g.as_mut() {
        Some(Ok(q)) => q,
        Some(Err(e)) => return Err(e.clone()),
        None => return Err("gpu".into()),
    };
    unsafe {
        if PdhCollectQueryData(q.query) != 0 {
            return Err("PdhCollectQueryData failed".into());
        }
        if !q.primed {
            q.primed = true;
            std::thread::sleep(std::time::Duration::from_millis(250));
            if PdhCollectQueryData(q.query) != 0 {
                return Err("PdhCollectQueryData failed".into());
            }
        }
        let mut size: u32 = 0;
        let mut count: u32 = 0;
        let rc = PdhGetFormattedCounterArrayW(
            q.counter,
            PDH_FMT_DOUBLE,
            &mut size,
            &mut count,
            std::ptr::null_mut(),
        );
        if rc != PDH_MORE_DATA || size == 0 {
            return Ok(Vec::new());
        }
        // The buffer holds the item array followed by the instance names.
        let words = (size as usize).div_ceil(std::mem::size_of::<u64>());
        let mut buf: Vec<u64> = vec![0; words];
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let rc =
            PdhGetFormattedCounterArrayW(q.counter, PDH_FMT_DOUBLE, &mut size, &mut count, items);
        if rc != 0 {
            return Ok(Vec::new());
        }
        let mut list = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let it = &*items.add(i);
            if it.FmtValue.CStatus != 0 || it.szName.is_null() {
                continue;
            }
            let mut len = 0;
            while *it.szName.add(len) != 0 {
                len += 1;
            }
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len));
            list.push((name, it.FmtValue.Anonymous.doubleValue));
        }
        Ok(parse::gpu_util_from_pdh(&list)
            .into_iter()
            .enumerate()
            .map(|(i, (_, pct))| Gpu {
                name: format!("GPU {i}"),
                vendor: "unknown",
                utilization_percent: Some(pct),
                source: "pdh",
                ..Default::default()
            })
            .collect())
    }
}

fn power_status() -> Option<SYSTEM_POWER_STATUS> {
    let mut s = SYSTEM_POWER_STATUS {
        ACLineStatus: 0,
        BatteryFlag: 0,
        BatteryLifePercent: 0,
        SystemStatusFlag: 0,
        BatteryLifeTime: 0,
        BatteryFullLifeTime: 0,
    };
    (unsafe { GetSystemPowerStatus(&mut s) } != 0).then_some(s)
}

/// Charge state from `GetSystemPowerStatus`, capacities and cycles from the
/// battery report (written to a temporary file of ours, path in argv).
pub fn battery() -> Result<Battery, String> {
    let st = power_status();
    let no_battery = st.map(|s| s.BatteryFlag == 128).unwrap_or(false);
    if no_battery {
        return Ok(Battery {
            present: false,
            state: "unknown",
            on_ac: Some(true),
            source: "GetSystemPowerStatus",
            ..Default::default()
        });
    }
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("battery.xml");
    let out = crate::core::process::std_command("powercfg")
        .arg("/batteryreport")
        .arg("/xml")
        .arg("/output")
        .arg(&path)
        .output()
        .map_err(|e| format!("powercfg: {e}"))?;
    let mut b = if out.status.success() {
        let xml = std::fs::read_to_string(&path).map_err(|e| format!("battery report: {e}"))?;
        parse::battery_from_powercfg(&xml)?
    } else {
        None
    }
    .unwrap_or(Battery {
        present: st.is_some_and(|s| s.BatteryFlag != 255),
        state: "unknown",
        source: "GetSystemPowerStatus",
        ..Default::default()
    });
    if let Some(s) = st {
        b.on_ac = match s.ACLineStatus {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        if s.BatteryLifePercent <= 100 {
            b.charge_percent = Some(s.BatteryLifePercent as f64);
        }
        b.state = if s.BatteryFlag & 8 != 0 {
            "charging"
        } else if b.on_ac == Some(false) {
            "discharging"
        } else if s.BatteryLifePercent == 100 {
            "full"
        } else if b.on_ac == Some(true) {
            "not_charging"
        } else {
            "unknown"
        };
        if s.BatteryLifeTime != u32::MAX && b.on_ac == Some(false) {
            b.time_remaining_min = Some(s.BatteryLifeTime as i64 / 60);
        }
    }
    Ok(b)
}

/// Priority class of an exact PID.
pub fn set_priority(pid: u32, level: &str) -> Result<(), String> {
    let class = match level {
        "low" => BELOW_NORMAL_PRIORITY_CLASS,
        "normal" => NORMAL_PRIORITY_CLASS,
        "high" => ABOVE_NORMAL_PRIORITY_CLASS,
        _ => return Err(format!("unknown priority {level}")),
    };
    unsafe {
        let h = OpenProcess(PROCESS_SET_INFORMATION, 0, pid);
        if h.is_null() {
            return Err("could not open the process (access denied or gone)".into());
        }
        let ok = SetPriorityClass(h, class) != 0;
        CloseHandle(h);
        if ok {
            Ok(())
        } else {
            Err("SetPriorityClass failed".into())
        }
    }
}
