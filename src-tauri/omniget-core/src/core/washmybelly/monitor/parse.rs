#![allow(dead_code)]

use serde::Serialize;
use serde_json::{Map, Value};

pub fn plist(xml: &str) -> Result<Value, String> {
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| format!("plist: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "plist" {
        return Err("plist: no <plist> root".into());
    }
    Ok(root
        .children()
        .find(|n| n.is_element())
        .map(plist_value)
        .unwrap_or(Value::Null))
}

fn plist_value(n: roxmltree::Node) -> Value {
    let text = || n.text().unwrap_or("").trim().to_string();
    match n.tag_name().name() {
        "dict" => {
            let mut map = Map::new();
            let mut key: Option<String> = None;
            for c in n.children().filter(|c| c.is_element()) {
                if c.tag_name().name() == "key" {
                    key = Some(c.text().unwrap_or("").to_string());
                } else if let Some(k) = key.take() {
                    map.insert(k, plist_value(c));
                }
            }
            Value::Object(map)
        }
        "array" => Value::Array(
            n.children()
                .filter(|c| c.is_element())
                .map(plist_value)
                .collect(),
        ),
        "integer" => {
            let t = text();
            if let Ok(v) = t.parse::<i64>() {
                Value::from(v)
            } else if let Ok(v) = t.parse::<u64>() {
                Value::from(v as i64)
            } else {
                Value::String(t)
            }
        }
        "real" => text()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::String(text()),
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Battery {
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycles: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design_capacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_charge_capacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_unit: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_ac: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage_v: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_remaining_min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub source: &'static str,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn health(full: f64, design: f64) -> Option<f64> {
    (design > 0.0 && full > 0.0).then(|| round1((full / design * 100.0).min(100.0)))
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

fn int(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

// WHY: on Apple Silicon MaxCapacity is a percentage; the real full charge is AppleRawMaxCapacity.
pub fn battery_from_ioreg(v: &Value) -> Battery {
    let b = match v {
        Value::Array(a) => a.first(),
        Value::Object(_) => Some(v),
        _ => None,
    };
    let Some(b) = b else {
        return Battery {
            present: false,
            state: "unknown",
            source: "AppleSmartBattery",
            ..Default::default()
        };
    };
    let data = b.get("BatteryData").unwrap_or(&Value::Null);
    let design = num(b, "DesignCapacity").or_else(|| num(data, "DesignCapacity"));
    let max = num(b, "MaxCapacity");
    let raw_max = num(b, "AppleRawMaxCapacity");
    let percent_scale = max.map(|m| m <= 100.0).unwrap_or(raw_max.is_some());
    let full = raw_max
        .or_else(|| max.filter(|_| !percent_scale))
        .or_else(|| num(b, "NominalChargeCapacity"));
    let charge = if percent_scale {
        num(b, "CurrentCapacity")
    } else {
        match (num(b, "CurrentCapacity"), full) {
            (Some(c), Some(f)) if f > 0.0 => Some(round1(c / f * 100.0)),
            _ => None,
        }
    };
    let charging = flag(b, "IsCharging").unwrap_or(false);
    let on_ac = flag(b, "ExternalConnected");
    let fully = flag(b, "FullyCharged").unwrap_or(false);
    let state = if charging {
        "charging"
    } else if fully {
        "full"
    } else if on_ac == Some(true) {
        "not_charging"
    } else if on_ac == Some(false) {
        "discharging"
    } else {
        "unknown"
    };
    let voltage = num(b, "Voltage").map(|mv| mv / 1000.0);
    let amps = int(b, "InstantAmperage")
        .filter(|a| *a != 0)
        .or_else(|| int(b, "Amperage"))
        .map(|ma| ma as f64 / 1000.0);
    let power = match (voltage, amps) {
        (Some(v), Some(a)) => Some(round1(v * a)),
        _ => None,
    };
    let time = int(b, "TimeRemaining")
        .filter(|t| *t > 0 && *t < 65535 && !charging && on_ac != Some(true));
    Battery {
        present: true,
        cycles: int(b, "CycleCount").or_else(|| int(data, "CycleCount")),
        health_percent: match (full, design) {
            (Some(f), Some(d)) => health(f, d),
            _ => None,
        },
        design_capacity: design,
        full_charge_capacity: full,
        capacity_unit: Some("mAh"),
        charge_percent: charge,
        temperature_c: num(b, "Temperature")
            .filter(|t| *t > 0.0)
            .map(|t| round1(t / 100.0)),
        state,
        on_ac,
        voltage_v: voltage.map(|v| (v * 100.0).round() / 100.0),
        power_w: power,
        time_remaining_min: time,
        name: b
            .get("DeviceName")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        source: "AppleSmartBattery",
    }
}

pub fn battery_from_powercfg(xml: &str) -> Result<Option<Battery>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("battery report: {e}"))?;
    let Some(bat) = doc.descendants().find(|n| n.tag_name().name() == "Battery") else {
        return Ok(None);
    };
    let field = |name: &str| {
        bat.children()
            .find(|c| c.tag_name().name() == name)
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
    };
    let f = |name: &str| field(name).and_then(|t| t.parse::<f64>().ok());
    let design = f("DesignCapacity").filter(|v| *v > 0.0);
    let full = f("FullChargeCapacity").filter(|v| *v > 0.0);
    Ok(Some(Battery {
        present: true,
        cycles: f("CycleCount").map(|c| c as i64).filter(|c| *c > 0),
        health_percent: match (full, design) {
            (Some(f), Some(d)) => health(f, d),
            _ => None,
        },
        design_capacity: design,
        full_charge_capacity: full,
        capacity_unit: Some("mWh"),
        state: "unknown",
        name: field("Id").filter(|s| !s.is_empty()),
        source: "powercfg",
        ..Default::default()
    }))
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub vendor: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renderer_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiler_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freq_mhz: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_freq_mhz: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cores: Option<u32>,
    pub source: &'static str,
}

fn inline_num(dict: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{key}\"=");
    let at = dict.find(&pat)? + pat.len();
    let rest = &dict[at..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse::<f64>().ok()
}

fn prop<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\" = ");
    block
        .lines()
        .find_map(|l| l.trim_start().strip_prefix(&pat))
        .map(str::trim)
}

fn vendor_from_class(class: &str) -> &'static str {
    let c = class.to_ascii_lowercase();
    if c.starts_with("agx") {
        "apple"
    } else if c.contains("nvidia") || c.starts_with("nv") || c.contains("geforce") {
        "nvidia"
    } else if c.contains("amd") || c.contains("radeon") || c.contains("ati") {
        "amd"
    } else if c.contains("intel") {
        "intel"
    } else {
        "unknown"
    }
}

pub fn gpus_from_ioreg(text: &str) -> Vec<Gpu> {
    let mut out = Vec::new();
    for block in text.split("+-o ").skip(1) {
        let class = block
            .split("<class ")
            .nth(1)
            .and_then(|s| s.split([',', '>']).next())
            .unwrap_or("")
            .trim()
            .to_string();
        let stats = prop(block, "PerformanceStatistics").unwrap_or("");
        let util = inline_num(stats, "Device Utilization %")
            .or_else(|| inline_num(stats, "GPU Activity(%)"));
        let name = prop(block, "model")
            .map(|m| {
                m.trim_matches(|c| c == '"' || c == '<' || c == '>')
                    .to_string()
            })
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| class.clone());
        out.push(Gpu {
            name,
            vendor: vendor_from_class(&class),
            utilization_percent: util,
            renderer_percent: inline_num(stats, "Renderer Utilization %"),
            tiler_percent: inline_num(stats, "Tiler Utilization %"),
            temperature_c: inline_num(stats, "Temperature(C)").filter(|t| *t > 0.0 && *t < 128.0),
            fan_percent: inline_num(stats, "Fan Speed(%)"),
            freq_mhz: inline_num(stats, "Core Clock(MHz)"),
            memory_used: inline_num(stats, "In use system memory")
                .or_else(|| inline_num(stats, "vramUsedBytes"))
                .map(|v| v as u64),
            cores: prop(block, "gpu-core-count").and_then(|s| s.parse().ok()),
            source: "IOAccelerator",
            ..Default::default()
        });
    }
    out
}

pub const NVIDIA_QUERY: &str =
    "--query-gpu=index,name,utilization.gpu,temperature.gpu,fan.speed,power.draw,clocks.gr,memory.used,memory.total";

pub fn gpus_from_nvidia_smi(text: &str) -> Vec<Gpu> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').map(str::trim).collect();
            if f.len() < 9 {
                return None;
            }
            let n = |i: usize| f[i].parse::<f64>().ok();
            let mib = |i: usize| n(i).map(|v| (v * 1024.0 * 1024.0) as u64);
            Some(Gpu {
                name: f[1].to_string(),
                vendor: "nvidia",
                utilization_percent: n(2),
                temperature_c: n(3),
                fan_percent: n(4),
                power_w: n(5),
                freq_mhz: n(6),
                memory_used: mib(7),
                memory_total: mib(8),
                source: "nvidia-smi",
                ..Default::default()
            })
        })
        .collect()
}

pub fn gpu_util_from_pdh(items: &[(String, f64)]) -> Vec<(String, f64)> {
    use std::collections::BTreeMap;
    let mut by: BTreeMap<(String, String), f64> = BTreeMap::new();
    for (name, v) in items {
        let luid = name
            .split("luid_")
            .nth(1)
            .and_then(|s| s.split("_phys").next())
            .unwrap_or("")
            .to_string();
        let eng = name.split("engtype_").nth(1).unwrap_or("").to_string();
        if luid.is_empty() || !v.is_finite() {
            continue;
        }
        *by.entry((luid, eng)).or_default() += v;
    }
    let mut per: BTreeMap<String, f64> = BTreeMap::new();
    for ((luid, _), v) in by {
        let e = per.entry(luid).or_default();
        *e = e.max(v.min(100.0));
    }
    per.into_iter().map(|(l, v)| (l, round1(v))).collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CpuTimes {
    pub busy: u64,
    pub iowait: u64,
    pub total: u64,
}

pub fn proc_stat_total(text: &str) -> Option<CpuTimes> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 4 {
        return None;
    }
    let get = |i: usize| v.get(i).copied().unwrap_or(0);
    let idle = get(3);
    let iowait = get(4);
    let total: u64 = v.iter().sum();
    Some(CpuTimes {
        busy: total - idle - iowait,
        iowait,
        total,
    })
}

pub fn iowait_percent(a: CpuTimes, b: CpuTimes) -> Option<f64> {
    let dt = b.total.checked_sub(a.total)?;
    (dt > 0).then(|| round1(b.iowait.saturating_sub(a.iowait) as f64 / dt as f64 * 100.0))
}

pub fn psi_some_avg10(text: &str) -> Option<f64> {
    let line = text.lines().find(|l| l.starts_with("some "))?;
    line.split_whitespace()
        .find_map(|f| f.strip_prefix("avg10="))
        .and_then(|v| v.parse().ok())
}

pub fn smc_decode(ty: &[u8; 4], b: &[u8]) -> Option<f64> {
    let be16 = || b.get(..2).map(|x| u16::from_be_bytes([x[0], x[1]]));
    match ty {
        b"flt " => b
            .get(..4)
            .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]) as f64),
        b"ui8 " | b"si8 " if !b.is_empty() => Some(if ty[0] == b's' {
            b[0] as i8 as f64
        } else {
            b[0] as f64
        }),
        b"ui16" => be16().map(|v| v as f64),
        b"si16" => be16().map(|v| v as i16 as f64),
        b"ui32" => b
            .get(..4)
            .map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]) as f64),
        b"flag" => b.first().map(|v| *v as f64),
        [b's', b'p', _, f] | [b'f', b'p', _, f] => {
            let frac = (*f as char).to_digit(16)?;
            let raw = be16()?;
            let v = if ty[0] == b's' {
                raw as i16 as f64
            } else {
                raw as f64
            };
            Some(v / f64::from(1u32 << frac))
        }
        _ => None,
    }
}

const SECRET_WORDS: &[&str] = &[
    "token",
    "password",
    "passwd",
    "secret",
    "apikey",
    "api-key",
    "api_key",
    "auth",
    "credential",
    "private-key",
    "access-key",
];

fn is_secret_name(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|w| l.contains(w))
}

pub fn redact_cmd(args: &[String], max: usize) -> String {
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut hide_next = false;
    for a in args {
        if hide_next {
            out.push("***".into());
            hide_next = false;
            continue;
        }
        if let Some((k, _)) = a.split_once('=') {
            if is_secret_name(k) {
                out.push(format!("{k}=***"));
                continue;
            }
        } else if a.starts_with('-') && is_secret_name(a) {
            hide_next = true;
        }
        out.push(a.clone());
    }
    let s = out.join(" ");
    if s.chars().count() > max {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($f:expr) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/washmybelly_fixtures/monitor/",
                $f
            ))
        };
    }

    #[test]
    fn apple_silicon_battery_uses_raw_max_capacity() {
        let v = plist(fixture!("ioreg_battery_m5.plist")).unwrap();
        let b = battery_from_ioreg(&v);
        assert!(b.present);
        assert_eq!(b.cycles, Some(17));
        assert_eq!(b.design_capacity, Some(5760.0));
        assert_eq!(b.full_charge_capacity, Some(5680.0));
        assert_eq!(b.health_percent, Some(98.6));
        assert_eq!(b.charge_percent, Some(100.0));
        assert_eq!(b.temperature_c, Some(31.4));
        assert_eq!(b.state, "full");
        assert_eq!(b.on_ac, Some(true));
        assert_eq!(b.time_remaining_min, None);
    }

    #[test]
    fn intel_battery_uses_max_capacity_in_mah() {
        let v = plist(fixture!("ioreg_battery_intel.plist")).unwrap();
        let b = battery_from_ioreg(&v);
        assert_eq!(b.cycles, Some(1043));
        assert_eq!(b.health_percent, Some(73.0));
        assert_eq!(b.charge_percent, Some(74.1));
        assert_eq!(b.state, "discharging");
        assert_eq!(b.power_w, Some(-11.9));
        assert_eq!(b.time_remaining_min, Some(184));
    }

    #[test]
    fn mac_without_battery() {
        let v = plist(fixture!("ioreg_battery_none.plist")).unwrap();
        let b = battery_from_ioreg(&v);
        assert!(!b.present);
        let j = serde_json::to_value(&b).unwrap();
        assert!(j.get("cycles").is_none());
    }

    #[test]
    fn windows_battery_report() {
        let b = battery_from_powercfg(fixture!("powercfg_batteryreport.xml"))
            .unwrap()
            .unwrap();
        assert_eq!(b.cycles, Some(143));
        assert_eq!(b.health_percent, Some(86.2));
        assert_eq!(b.capacity_unit, Some("mWh"));
        assert_eq!(battery_from_powercfg("<BatteryReport/>").unwrap(), None);
    }

    #[test]
    fn apple_gpu_from_ioreg() {
        let g = gpus_from_ioreg(fixture!("ioreg_gpu_m5.txt"));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "Apple M5");
        assert_eq!(g[0].vendor, "apple");
        assert_eq!(g[0].utilization_percent, Some(25.0));
        assert_eq!(g[0].renderer_percent, Some(25.0));
        assert_eq!(g[0].tiler_percent, Some(14.0));
        assert_eq!(g[0].cores, Some(10));
        assert_eq!(gpus_from_ioreg(""), vec![]);
    }

    #[test]
    fn nvidia_smi_csv() {
        let g = gpus_from_nvidia_smi(fixture!("nvidia_smi.csv"));
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].name, "NVIDIA GeForce RTX 3070");
        assert_eq!(g[0].utilization_percent, Some(37.0));
        assert_eq!(g[0].power_w, Some(118.52));
        assert_eq!(g[0].memory_total, Some(8192 * 1024 * 1024));
        assert_eq!(g[1].fan_percent, None);
        assert_eq!(g[1].power_w, None);
    }

    #[test]
    fn pdh_gpu_engines() {
        let items = vec![
            (
                "pid_1_luid_0x00000000_0x0000C2D4_phys_0_eng_0_engtype_3D".to_string(),
                30.0,
            ),
            (
                "pid_2_luid_0x00000000_0x0000C2D4_phys_0_eng_0_engtype_3D".to_string(),
                25.0,
            ),
            (
                "pid_2_luid_0x00000000_0x0000C2D4_phys_0_eng_3_engtype_VideoDecode".to_string(),
                70.0,
            ),
            (
                "pid_3_luid_0x00000000_0x0000AAAA_phys_0_eng_0_engtype_3D".to_string(),
                5.0,
            ),
        ];
        let u = gpu_util_from_pdh(&items);
        assert_eq!(
            u,
            vec![
                ("0x00000000_0x0000AAAA".to_string(), 5.0),
                ("0x00000000_0x0000C2D4".to_string(), 70.0)
            ]
        );
    }

    #[test]
    fn proc_stat_and_pressure() {
        let a = proc_stat_total(fixture!("proc_stat.txt")).unwrap();
        assert_eq!(a.iowait, 23060);
        assert_eq!(a.total, 4705 + 356 + 584 + 3699176 + 23060 + 277);
        let b = CpuTimes {
            busy: a.busy + 50,
            iowait: a.iowait + 25,
            total: a.total + 100,
        };
        assert_eq!(iowait_percent(a, b), Some(25.0));
        assert_eq!(psi_some_avg10(fixture!("pressure_memory.txt")), Some(12.5));
    }

    #[test]
    fn smc_values() {
        assert_eq!(smc_decode(b"sp78", &[0x2a, 0x80]), Some(42.5));
        assert_eq!(smc_decode(b"fpe2", &[0x1f, 0x40]), Some(2000.0));
        assert_eq!(smc_decode(b"ui8 ", &[2]), Some(2.0));
        assert_eq!(smc_decode(b"ui32", &[0, 0, 0x05, 0xdc]), Some(1500.0));
        assert_eq!(smc_decode(b"flt ", &42.25f32.to_le_bytes()), Some(42.25));
        assert_eq!(smc_decode(b"ch8*", &[1]), None);
    }

    #[test]
    fn command_lines_hide_secrets() {
        let a: Vec<String> = [
            "node",
            "server.js",
            "--token=abc123",
            "--password",
            "hunter2",
            "API_KEY=zzz",
            "--port",
            "80",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            redact_cmd(&a, 200),
            "node server.js --token=*** --password *** API_KEY=*** --port 80"
        );
        assert_eq!(redact_cmd(&a[..2], 6), "node s…");
    }
}
