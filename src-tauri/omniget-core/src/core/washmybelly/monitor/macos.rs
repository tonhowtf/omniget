//! macOS readings without root: SMC through IOKit (temperatures, fans,
//! power), the IORegistry through `ioreg` (GPU load, battery) and
//! `kern.memorystatus_level` (memory pressure).
//!
//! The SMC connection, the key list and each key's type are cached for the
//! life of the app: enumerating the keys costs a few thousand calls once.
//! Keys by chip come from the Stats table in [`super::smc_keys`] (MIT).
//! Only reads: no write selector is ever sent.

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::Mutex;

use super::parse::{self, Battery, Gpu};
use super::smc_keys::{self, SENSORS};
use super::{Fan, Temp};

// ── IOKit FFI ────────────────────────────────────────────────────────────

type KernReturn = i32;
type MachPort = u32;
type IoObject = u32;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: MachPort, matching: *mut c_void) -> IoObject;
    fn IOServiceOpen(
        service: IoObject,
        owning_task: MachPort,
        kind: u32,
        connect: *mut IoObject,
    ) -> KernReturn;
    fn IOServiceClose(connect: IoObject) -> KernReturn;
    fn IOObjectRelease(object: IoObject) -> KernReturn;
    fn IOConnectCallStructMethod(
        connection: IoObject,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> KernReturn;
}

extern "C" {
    static mach_task_self_: MachPort;
}

const KERNEL_INDEX_SMC: u32 = 2;
const SMC_READ_BYTES: u8 = 5;
const SMC_READ_INDEX: u8 = 8;
const SMC_READ_KEYINFO: u8 = 9;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Vers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PLimit {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    mem: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyInfo {
    data_size: u32,
    data_type: u32,
    attributes: u8,
}

/// `SMCKeyData_t` of AppleSMC (80 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyData {
    key: u32,
    vers: Vers,
    p_limit: PLimit,
    info: KeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

const _: () = assert!(std::mem::size_of::<KeyData>() == 80);

fn fourcc(s: &str) -> u32 {
    let b = s.as_bytes();
    if b.len() != 4 {
        return 0;
    }
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn key_str(k: u32) -> String {
    String::from_utf8_lossy(&k.to_be_bytes()).into_owned()
}

/// (key, name, group, kind, average).
type MatchedKey = (u32, String, &'static str, &'static str, bool);

struct Smc {
    conn: IoObject,
    info: HashMap<u32, Option<(u32, [u8; 4])>>,
    keys: Option<Vec<u32>>,
    /// Table entries of this chip matched against the enumerated keys:
    /// (key, name, group, kind, average).
    matched: Option<Vec<MatchedKey>>,
}

// The connection is a mach port number; the struct is only touched under the mutex.
unsafe impl Send for Smc {}

impl Smc {
    fn open() -> Result<Smc, String> {
        unsafe {
            let matching = IOServiceMatching(c"AppleSMC".as_ptr());
            if matching.is_null() {
                return Err("AppleSMC matching failed".into());
            }
            // Consumes `matching`.
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return Err("no AppleSMC service".into());
            }
            let mut conn: IoObject = 0;
            let kr = IOServiceOpen(service, mach_task_self_, 0, &mut conn);
            IOObjectRelease(service);
            if kr != 0 || conn == 0 {
                return Err(format!("IOServiceOpen(AppleSMC) = {kr:#x}"));
            }
            Ok(Smc {
                conn,
                info: HashMap::new(),
                keys: None,
                matched: None,
            })
        }
    }

    fn call(&self, input: &KeyData) -> Option<KeyData> {
        let mut out = KeyData::default();
        let mut size = std::mem::size_of::<KeyData>();
        let kr = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                KERNEL_INDEX_SMC,
                input as *const KeyData as *const c_void,
                std::mem::size_of::<KeyData>(),
                &mut out as *mut KeyData as *mut c_void,
                &mut size,
            )
        };
        (kr == 0 && out.result == 0).then_some(out)
    }

    fn key_info(&mut self, key: u32) -> Option<(u32, [u8; 4])> {
        if let Some(i) = self.info.get(&key) {
            return *i;
        }
        let input = KeyData {
            key,
            data8: SMC_READ_KEYINFO,
            ..Default::default()
        };
        let i = self
            .call(&input)
            .map(|o| (o.info.data_size, o.info.data_type.to_be_bytes()))
            .filter(|(size, _)| *size > 0 && *size <= 32);
        self.info.insert(key, i);
        i
    }

    fn read(&mut self, key: u32) -> Option<f64> {
        let (size, ty) = self.key_info(key)?;
        let mut input = KeyData {
            key,
            data8: SMC_READ_BYTES,
            ..Default::default()
        };
        input.info.data_size = size;
        let out = self.call(&input)?;
        parse::smc_decode(&ty, &out.bytes[..size as usize]).filter(|v| v.is_finite())
    }

    fn keys(&mut self) -> &[u32] {
        if self.keys.is_none() {
            let count = self.read(fourcc("#KEY")).unwrap_or(0.0) as u32;
            let mut keys = Vec::with_capacity(count as usize);
            for i in 0..count.min(10_000) {
                let input = KeyData {
                    data8: SMC_READ_INDEX,
                    data32: i,
                    ..Default::default()
                };
                if let Some(o) = self.call(&input) {
                    keys.push(o.key);
                }
            }
            self.keys = Some(keys);
        }
        self.keys.as_deref().unwrap_or(&[])
    }

    fn matched(&mut self, family: u16) -> Vec<MatchedKey> {
        if let Some(m) = &self.matched {
            return m.clone();
        }
        let keys: Vec<u32> = self.keys().to_vec();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for e in SENSORS.iter().filter(|e| e.on & family != 0) {
            for k in &keys {
                let ks = key_str(*k);
                if smc_keys::key_matches(e.key, &ks) && seen.insert(*k) {
                    out.push((*k, smc_keys::name_for(e, &ks), e.group, e.kind, e.average));
                }
            }
        }
        self.matched = Some(out.clone());
        out
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        unsafe {
            IOServiceClose(self.conn);
        }
    }
}

static SMC: Mutex<Option<Result<Smc, String>>> = Mutex::new(None);

fn with_smc<T>(f: impl FnOnce(&mut Smc) -> T) -> Result<T, String> {
    let mut g = SMC.lock().map_err(|_| "smc lock")?;
    if g.is_none() {
        *g = Some(Smc::open());
    }
    match g.as_mut() {
        Some(Ok(smc)) => Ok(f(smc)),
        Some(Err(e)) => Err(e.clone()),
        None => Err("smc".into()),
    }
}

fn family() -> u16 {
    let brand = sysctl_string("machdep.cpu.brand_string").unwrap_or_default();
    smc_keys::family(&brand)
}

// ── sysctl ───────────────────────────────────────────────────────────────

fn sysctl_string(name: &str) -> Option<String> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mut len: libc::size_t = 0;
    unsafe {
        if libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
            || len == 0
        {
            return None;
        }
        let mut buf = vec![0u8; len];
        if libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        buf.truncate(len);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf8(buf).ok()
    }
}

fn sysctl_i32(name: &str) -> Option<i32> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mut v: i32 = 0;
    let mut len: libc::size_t = std::mem::size_of::<i32>();
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            &mut v as *mut i32 as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(v)
}

/// Memory pressure 0-100 (higher is worse): the kernel's free memory level
/// (`kern.memorystatus_level`), the figure Activity Monitor's graph follows.
pub fn memory_pressure() -> Option<f64> {
    sysctl_i32("kern.memorystatus_level")
        .filter(|l| (0..=100).contains(l))
        .map(|l| (100 - l) as f64)
}

// ── Readings ─────────────────────────────────────────────────────────────

fn valid_temp(v: f64) -> bool {
    v > 0.0 && v < 110.0
}

fn avg(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| ((v.iter().sum::<f64>() / v.len() as f64) * 10.0).round() / 10.0)
}

/// CPU and GPU temperature: the chip's per-core keys averaged (Stats).
pub fn main_temps() -> (Option<f64>, Option<f64>) {
    let fam = family();
    with_smc(|smc| {
        // Intel: a single CPU diode key first.
        if fam == smc_keys::INTEL {
            for k in ["TC0D", "TC0E", "TC0F", "TC0P", "TC0H"] {
                if let Some(v) = smc.read(fourcc(k)).filter(|v| valid_temp(*v)) {
                    let gpu = ["TCGC", "TGDD", "TG0D", "TG0P"].iter().find_map(|k| {
                        smc.read(fourcc(k))
                            .filter(|v| valid_temp(*v) && *v != 128.0)
                    });
                    return (Some(v), gpu);
                }
            }
        }
        let m = smc.matched(fam);
        let mut cpu = Vec::new();
        let mut gpu = Vec::new();
        for (k, _, group, kind, average) in &m {
            if *kind != "temperature" || !average {
                continue;
            }
            if let Some(v) = smc.read(*k).filter(|v| valid_temp(*v)) {
                match *group {
                    "CPU" => cpu.push(v),
                    "GPU" => gpu.push(v),
                    _ => {}
                }
            }
        }
        (avg(&cpu), avg(&gpu))
    })
    .unwrap_or((None, None))
}

/// Temperatures, fans and power readings (name, watts).
type SmcSensors = (Vec<Temp>, Vec<Fan>, Vec<(String, f64)>);

/// Every temperature and power key of this chip, and the fans.
pub fn smc_sensors() -> Result<SmcSensors, String> {
    let fam = family();
    with_smc(|smc| {
        let mut temps = Vec::new();
        let mut power = Vec::new();
        for (k, name, group, kind, _) in smc.matched(fam) {
            let Some(v) = smc.read(k) else { continue };
            match kind {
                "temperature" if valid_temp(v) => temps.push(Temp {
                    name,
                    group: match group {
                        "CPU" => "CPU",
                        "GPU" => "GPU",
                        "system" => "system",
                        _ => "sensor",
                    },
                    celsius: (v * 10.0).round() / 10.0,
                    critical: None,
                    source: "smc",
                }),
                "power" if (0.0..1000.0).contains(&v) => {
                    power.push((name, (v * 100.0).round() / 100.0))
                }
                _ => {}
            }
        }
        let mut fans = Vec::new();
        let n = smc.read(fourcc("FNum")).unwrap_or(0.0) as u32;
        for i in 0..n.min(8) {
            let Some(rpm) = smc.read(fourcc(&format!("F{i}Ac"))) else {
                continue;
            };
            fans.push(Fan {
                name: format!("Fan {}", i + 1),
                rpm: rpm.round(),
                min_rpm: smc.read(fourcc(&format!("F{i}Mn"))).map(f64::round),
                max_rpm: smc.read(fourcc(&format!("F{i}Mx"))).map(f64::round),
            });
        }
        (temps, fans, power)
    })
}

/// GPUs from `ioreg -r -d 1 -w 0 -c IOAccelerator` (no root).
pub fn gpus() -> Result<Vec<Gpu>, String> {
    let out = crate::core::process::std_command("ioreg")
        .args(["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"])
        .output()
        .map_err(|e| format!("ioreg: {e}"))?;
    if !out.status.success() {
        return Err(format!("ioreg exited with {}", out.status));
    }
    Ok(parse::gpus_from_ioreg(&String::from_utf8_lossy(
        &out.stdout,
    )))
}

/// `ioreg -r -c AppleSmartBattery -a`.
pub fn battery() -> Result<Battery, String> {
    let out = crate::core::process::std_command("ioreg")
        .args(["-r", "-c", "AppleSmartBattery", "-a"])
        .output()
        .map_err(|e| format!("ioreg: {e}"))?;
    if !out.status.success() {
        return Err(format!("ioreg exited with {}", out.status));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        // No AppleSmartBattery at all: a desktop Mac.
        return Ok(parse::battery_from_ioreg(&serde_json::Value::Array(vec![])));
    }
    Ok(parse::battery_from_ioreg(&parse::plist(&text)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real SMC of this Mac (read-only). On a machine without
    /// AppleSMC (some CI VMs) it only checks that nothing panics.
    #[test]
    fn real_smc_reads_or_fails_cleanly() {
        match smc_sensors() {
            Ok((temps, _fans, _power)) => {
                for t in &temps {
                    assert!(t.celsius > 0.0 && t.celsius < 110.0);
                }
            }
            Err(e) => assert!(!e.is_empty()),
        }
        let _ = main_temps();
        assert!(memory_pressure().is_none_or(|p| (0.0..=100.0).contains(&p)));
    }
}
