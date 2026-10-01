//! Linux sysfs knobs written by the administrator helper, and the owner
//! of the profiles API. No D-Bus here, so the tests run on every OS.

use std::path::Path;

use super::arbiter::Manager;
use super::parse;

/// Who owns the profiles API, from the process name or the `Version`.
pub fn owner_manager(comm: Option<&str>, version: Option<&str>) -> Option<Manager> {
    if version.is_some_and(|v| v.starts_with("tlp-pd")) {
        return Some(Manager::TlpPd);
    }
    match comm? {
        c if c.starts_with("power-profiles") => Some(Manager::PowerProfilesDaemon),
        c if c.starts_with("tlp-pd") => Some(Manager::TlpPd),
        c if c.starts_with("tuned-ppd") => Some(Manager::TunedPpd),
        _ => None,
    }
}

/// Write `value` to `name` in every cpufreq folder, after checking it is
/// one of the values the kernel lists as available.
pub fn write_cpu_knob(
    sys: &Path,
    name: &str,
    available: &str,
    value: &str,
) -> Result<String, String> {
    let dirs = parse::cpufreq_dirs(sys);
    let first = dirs.first().ok_or("no cpufreq on this machine")?;
    let allowed =
        std::fs::read_to_string(first.join(available)).map_err(|e| format!("{available}: {e}"))?;
    if !allowed.split_whitespace().any(|v| v == value) {
        return Err(format!("{value} is not one of: {}", allowed.trim()));
    }
    let mut n = 0;
    for d in &dirs {
        let p = d.join(name);
        if p.exists() {
            std::fs::write(&p, value).map_err(|e| format!("{}: {e}", p.display()))?;
            n += 1;
        }
    }
    Ok(format!("{name} = {value} on {n} CPUs (until restart)"))
}

/// `BAT0:80`.
pub fn write_charge_limit(sys: &Path, value: &str) -> Result<String, String> {
    let (bat, pct) = value
        .split_once(':')
        .ok_or("charge_limit: expected BATTERY:PERCENT")?;
    if !parse::valid_battery_name(bat) {
        return Err(format!("charge_limit: invalid battery {bat:?}"));
    }
    let pct: u8 = pct
        .parse()
        .map_err(|_| "charge_limit: percent must be a number")?;
    if !(20..=100).contains(&pct) {
        return Err("charge_limit: percent must be 20-100".into());
    }
    if !parse::charge_limits(sys).iter().any(|c| c.battery == bat) {
        return Err(format!("charge_limit: {bat} has no charge limit"));
    }
    let dir = sys.join("class/power_supply").join(bat);
    let file = ["charge_control_end_threshold", "charge_stop_threshold"]
        .into_iter()
        .map(|f| dir.join(f))
        .find(|p| p.exists())
        .ok_or("charge_limit: no threshold file")?;
    // A start threshold above the new end is refused by most drivers.
    for start in ["charge_control_start_threshold", "charge_start_threshold"] {
        let sp = dir.join(start);
        if let Some(cur) = std::fs::read_to_string(&sp)
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
        {
            if cur >= pct {
                let _ = std::fs::write(&sp, pct.saturating_sub(5).to_string());
            }
        }
    }
    std::fs::write(&file, pct.to_string()).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(format!("{bat} stops charging at {pct}%"))
}
