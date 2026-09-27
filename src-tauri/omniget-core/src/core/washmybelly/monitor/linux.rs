//! Linux sensors from sysfs: hwmon (temperatures, fans, power), DRM (GPU
//! load and frequency) and power_supply (battery). Every reader takes the
//! class folder as an argument, so tests run them on fixture trees shaped
//! like `/sys/class/...` on any system.

#![allow(dead_code)]

use std::path::Path;

use super::parse::{health, Battery, Gpu};
use super::{Fan, Temp};

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
}

fn read_f(p: &Path) -> Option<f64> {
    read(p).and_then(|s| s.parse::<f64>().ok())
}

fn sorted_dir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Which hwmon chip drives which group.
fn hwmon_group(chip: &str) -> &'static str {
    match chip {
        "coretemp" | "k10temp" | "zenpower" | "cpu_thermal" | "cpu-thermal" | "soc_thermal" => {
            "CPU"
        }
        "amdgpu" | "radeon" | "nouveau" | "i915" | "xe" => "GPU",
        "nvme" | "drivetemp" => "disk",
        "BAT0" | "BAT1" | "battery" => "battery",
        _ => "system",
    }
}

/// Temperatures, fans and power readings of every hwmon chip.
pub fn read_hwmon(root: &Path) -> (Vec<Temp>, Vec<Fan>, Vec<(String, f64)>) {
    let mut temps = Vec::new();
    let mut fans = Vec::new();
    let mut power = Vec::new();
    for dir in sorted_dir(root) {
        let chip = read(&dir.join("name")).unwrap_or_else(|| "hwmon".into());
        let group = hwmon_group(&chip);
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        for f in &files {
            let Some(base) = f
                .strip_suffix("_input")
                .or_else(|| f.strip_suffix("_average"))
            else {
                continue;
            };
            let label = read(&dir.join(format!("{base}_label")));
            let name = match &label {
                Some(l) => format!("{chip} {l}"),
                None => format!("{chip} {base}"),
            };
            let Some(v) = read_f(&dir.join(f)) else {
                continue;
            };
            if base.starts_with("temp") && f.ends_with("_input") {
                let c = v / 1000.0;
                // Unplugged ACPI zones read -273 or 0; above 150 is garbage.
                if c > 0.0 && c < 150.0 {
                    temps.push(Temp {
                        name,
                        group,
                        celsius: (c * 10.0).round() / 10.0,
                        critical: read_f(&dir.join(format!("{base}_crit"))).map(|x| x / 1000.0),
                        source: "hwmon",
                    });
                }
            } else if base.starts_with("fan") && f.ends_with("_input") {
                fans.push(Fan {
                    name,
                    rpm: v,
                    min_rpm: read_f(&dir.join(format!("{base}_min"))),
                    max_rpm: read_f(&dir.join(format!("{base}_max"))),
                });
            } else if base.starts_with("power")
                && (f.ends_with("_input") || !files.contains(&format!("{base}_input")))
            {
                // µW; prefer _input, use _average when there is no _input.
                power.push((name, (v / 1e6 * 10.0).round() / 10.0));
            }
        }
    }
    (temps, fans, power)
}

/// The CPU temperature from hwmon: the package sensor when there is one,
/// else the hottest CPU sensor, else the first ACPI thermal zone.
pub fn cpu_temp(temps: &[Temp]) -> Option<f64> {
    let cpu: Vec<&Temp> = temps.iter().filter(|t| t.group == "CPU").collect();
    let pick = |needle: &str| {
        cpu.iter()
            .find(|t| t.name.to_ascii_lowercase().contains(needle))
            .map(|t| t.celsius)
    };
    pick("package")
        .or_else(|| pick("tctl"))
        .or_else(|| pick("tdie"))
        .or_else(|| cpu.iter().map(|t| t.celsius).reduce(f64::max))
        .or_else(|| {
            temps
                .iter()
                .find(|t| t.name.starts_with("acpitz"))
                .map(|t| t.celsius)
        })
}

/// GPUs from `/sys/class/drm/card*/device`: AMD `gpu_busy_percent` and VRAM,
/// Intel current frequency (i915 `gt_act_freq_mhz`, xe `tile0/gt0/freq0`).
/// NVIDIA cards are listed without load; `nvidia-smi` fills them.
pub fn read_drm(root: &Path) -> Vec<Gpu> {
    let mut out = Vec::new();
    for card in sorted_dir(root) {
        let Some(name) = card.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // card0, card1...; connectors (card0-DP-1) and render nodes skipped.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let dev = card.join("device");
        let vendor = match read(&dev.join("vendor")).as_deref() {
            Some("0x1002") => "amd",
            Some("0x8086") => "intel",
            Some("0x10de") => "nvidia",
            Some(_) => "unknown",
            None => continue,
        };
        let freq = read_f(&card.join("gt_act_freq_mhz"))
            .or_else(|| read_f(&card.join("gt/gt0/rps_act_freq_mhz")))
            .or_else(|| read_f(&dev.join("tile0/gt0/freq0/act_freq")));
        let max = read_f(&card.join("gt_max_freq_mhz"))
            .or_else(|| read_f(&card.join("gt/gt0/rps_max_freq_mhz")))
            .or_else(|| read_f(&dev.join("tile0/gt0/freq0/max_freq")));
        let temp = std::fs::read_dir(dev.join("hwmon"))
            .ok()
            .and_then(|mut rd| rd.next())
            .and_then(|e| e.ok())
            .and_then(|e| read_f(&e.path().join("temp1_input")))
            .map(|m| m / 1000.0);
        out.push(Gpu {
            name: read(&dev.join("product_name")).unwrap_or_else(|| format!("{name} ({vendor})")),
            vendor,
            utilization_percent: read_f(&dev.join("gpu_busy_percent")),
            temperature_c: temp,
            freq_mhz: freq,
            max_freq_mhz: max,
            memory_used: read_f(&dev.join("mem_info_vram_used")).map(|v| v as u64),
            memory_total: read_f(&dev.join("mem_info_vram_total")).map(|v| v as u64),
            source: "drm",
            ..Default::default()
        });
    }
    out
}

/// The first system battery under `/sys/class/power_supply`. Supplies with
/// `scope=Device` (mice, gamepads) and Logitech `hidpp_battery` are skipped.
pub fn read_battery(root: &Path) -> Battery {
    let supplies = sorted_dir(root);
    let on_ac = supplies
        .iter()
        .filter(|s| read(&s.join("type")).as_deref() == Some("Mains"))
        .filter_map(|s| read(&s.join("online")))
        .map(|o| o == "1")
        .reduce(|a, b| a || b);
    for s in &supplies {
        let name = s.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if read(&s.join("type")).as_deref() != Some("Battery")
            || read(&s.join("scope")).as_deref() == Some("Device")
            || name.starts_with("hidpp_battery")
            || read(&s.join("present")).as_deref() == Some("0")
        {
            continue;
        }
        let f = |k: &str| read_f(&s.join(k));
        let (full, design, unit) = match (f("energy_full"), f("energy_full_design")) {
            (Some(a), Some(b)) => (Some(a / 1000.0), Some(b / 1000.0), "mWh"),
            _ => (
                f("charge_full").map(|v| v / 1000.0),
                f("charge_full_design").map(|v| v / 1000.0),
                "mAh",
            ),
        };
        let state = match read(&s.join("status")).as_deref() {
            Some("Charging") => "charging",
            Some("Discharging") => "discharging",
            Some("Full") => "full",
            Some("Not charging") => "not_charging",
            _ => "unknown",
        };
        let voltage = f("voltage_now").map(|v| v / 1e6);
        let watts = f("power_now")
            .map(|p| p / 1e6)
            .or_else(|| match (f("current_now"), voltage) {
                (Some(i), Some(v)) => Some(i / 1e6 * v),
                _ => None,
            })
            .map(|w| (w * 10.0).round() / 10.0)
            .map(|w| if state == "discharging" { -w } else { w });
        return Battery {
            present: true,
            cycles: f("cycle_count").map(|c| c as i64).filter(|c| *c > 0),
            health_percent: match (full, design) {
                (Some(a), Some(b)) => health(a, b),
                _ => None,
            },
            design_capacity: design,
            full_charge_capacity: full,
            capacity_unit: Some(unit),
            charge_percent: f("capacity"),
            // Tenths of a degree, where the driver exposes it.
            temperature_c: f("temp").map(|t| t / 10.0),
            state,
            on_ac,
            voltage_v: voltage.map(|v| (v * 100.0).round() / 100.0),
            power_w: watts,
            name: Some(name.to_string()),
            source: "sysfs",
            ..Default::default()
        };
    }
    Battery {
        present: false,
        state: "unknown",
        on_ac,
        source: "sysfs",
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fx(p: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/washmybelly_fixtures/monitor")
            .join(p)
    }

    #[test]
    fn hwmon_tree() {
        let (t, f, p) = read_hwmon(&fx("hwmon"));
        assert!(t.iter().any(|x| x.name == "coretemp Package id 0"
            && x.celsius == 52.0
            && x.critical == Some(100.0)));
        assert!(t
            .iter()
            .any(|x| x.name == "nvme Composite" && x.group == "disk"));
        // acpitz at -273 °C is dropped.
        assert!(!t.iter().any(|x| x.name.starts_with("acpitz")));
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].rpm, 2890.0);
        assert_eq!(p, vec![("amdgpu power1".to_string(), 23.0)]);
        assert_eq!(cpu_temp(&t), Some(52.0));
    }

    #[test]
    fn drm_tree() {
        let g = read_drm(&fx("drm"));
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].vendor, "amd");
        assert_eq!(g[0].name, "AMD Radeon RX 6700 XT");
        assert_eq!(g[0].utilization_percent, Some(17.0));
        assert_eq!(g[0].memory_total, Some(8589934592));
        assert_eq!(g[1].vendor, "intel");
        assert_eq!(g[1].freq_mhz, Some(350.0));
        assert_eq!(g[1].max_freq_mhz, Some(1300.0));
        assert_eq!(g[1].utilization_percent, None);
    }

    #[test]
    fn battery_energy_and_charge() {
        let b = read_battery(&fx("power_supply"));
        assert!(b.present);
        assert_eq!(b.name.as_deref(), Some("BAT0"));
        assert_eq!(b.cycles, Some(312));
        assert_eq!(b.health_percent, Some(79.2));
        assert_eq!(b.capacity_unit, Some("mWh"));
        assert_eq!(b.state, "discharging");
        assert_eq!(b.on_ac, Some(false));
        assert_eq!(b.power_w, Some(-8.4));

        let c = read_battery(&fx("power_supply_charge"));
        assert_eq!(c.cycles, None);
        assert_eq!(c.health_percent, Some(84.0));
        assert_eq!(c.capacity_unit, Some("mAh"));
        assert_eq!(c.temperature_c, Some(29.1));
        assert_eq!(c.on_ac, Some(true));
        assert_eq!(c.state, "full");

        let none = read_battery(&fx("hwmon"));
        assert!(!none.present);
    }
}
