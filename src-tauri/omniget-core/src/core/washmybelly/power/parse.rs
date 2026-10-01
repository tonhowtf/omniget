use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PmsetSettings {
    pub low_power_mode: Option<bool>,
    pub power_mode: Option<u8>,
    pub sleep_minutes: Option<u32>,
    pub display_sleep_minutes: Option<u32>,
    pub sleep_prevented_by: Vec<String>,
}

pub fn pmset_settings(text: &str) -> PmsetSettings {
    let mut s = PmsetSettings::default();
    for line in text.lines() {
        let t = line.trim();
        let mut parts = t.split_whitespace();
        let (Some(key), Some(val)) = (parts.next(), parts.next()) else {
            continue;
        };
        match key {
            "lowpowermode" => s.low_power_mode = val.parse::<u8>().ok().map(|v| v != 0),
            "powermode" => s.power_mode = val.parse().ok(),
            "sleep" => {
                s.sleep_minutes = val.parse().ok();
                if let Some(open) = t.find("(sleep prevented by ") {
                    let inner = &t[open + "(sleep prevented by ".len()..];
                    let inner = inner.trim_end_matches(')');
                    for n in inner.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                        if !s.sleep_prevented_by.iter().any(|x| x == n) {
                            s.sleep_prevented_by.push(n.to_string());
                        }
                    }
                }
            }
            "displaysleep" => s.display_sleep_minutes = val.parse().ok(),
            _ => {}
        }
    }
    s
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SleepBlocker {
    pub process: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub kind: String,
    pub reason: String,
    pub display: bool,
}

static ASSERTION: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^\s*pid (\d+)\(([^)]*)\): \[[^\]]*\] \S+ (\w+) named: "(.*)"\s*$"#).unwrap()
});

const BLOCKING: &[&str] = &[
    "PreventUserIdleSystemSleep",
    "PreventUserIdleDisplaySleep",
    "PreventSystemSleep",
    "NoIdleSleepAssertion",
    "NoDisplaySleepAssertion",
    "NetworkClientActive",
];

pub fn pmset_assertions(text: &str) -> Vec<SleepBlocker> {
    let mut out = Vec::new();
    let mut in_list = false;
    for line in text.lines() {
        if line.starts_with("Listed by owning process") {
            in_list = true;
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_list = false;
            continue;
        }
        if !in_list {
            continue;
        }
        let Some(c) = ASSERTION.captures(line) else {
            continue;
        };
        let kind = c[3].to_string();
        if !BLOCKING.contains(&kind.as_str()) {
            continue;
        }
        out.push(SleepBlocker {
            process: c[2].to_string(),
            pid: c[1].parse().ok(),
            display: kind.contains("Display"),
            kind,
            reason: c[4].trim().to_string(),
        });
    }
    out
}

static SCHEME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\s+\(([^)]*)\)(\s*\*)?",
    )
    .unwrap()
});

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scheme {
    pub guid: String,
    pub name: String,
    pub active: bool,
}

pub fn powercfg_active(text: &str) -> Option<Scheme> {
    let c = SCHEME.captures(text)?;
    Some(Scheme {
        guid: c[1].to_ascii_lowercase(),
        name: c[2].trim().to_string(),
        active: true,
    })
}

pub fn powercfg_list(text: &str) -> Vec<Scheme> {
    text.lines()
        .filter_map(|l| SCHEME.captures(l))
        .map(|c| Scheme {
            guid: c[1].to_ascii_lowercase(),
            name: c[2].trim().to_string(),
            active: c.get(3).is_some(),
        })
        .collect()
}

pub fn powercfg_requests(text: &str) -> Vec<SleepBlocker> {
    let mut out: Vec<SleepBlocker> = Vec::new();
    let mut cat = String::new();
    let mut last_open = false;
    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() {
            last_open = false;
            continue;
        }
        if l.ends_with(':') && l.chars().all(|c| c.is_ascii_uppercase() || c == ':') {
            cat = l.trim_end_matches(':').to_string();
            last_open = false;
            continue;
        }
        if cat.is_empty() || l == "None." {
            continue;
        }
        if let Some(rest) = l.strip_prefix('[') {
            let Some((ty, name)) = rest.split_once(']') else {
                continue;
            };
            let name = name.trim();
            let process = if ty == "PROCESS" {
                name.rsplit('\\').next().unwrap_or(name).to_string()
            } else {
                name.to_string()
            };
            out.push(SleepBlocker {
                process,
                pid: None,
                kind: cat.clone(),
                reason: String::new(),
                display: cat == "DISPLAY",
            });
            last_open = true;
        } else if last_open {
            if let Some(b) = out.last_mut() {
                if b.reason.is_empty() {
                    b.reason = l.to_string();
                }
            }
        }
    }
    out
}

pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
pub const HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
pub const POWER_SAVER: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";
pub const ULTIMATE_TEMPLATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

pub fn overlay_name(guid: &str) -> &'static str {
    match guid.to_ascii_lowercase().as_str() {
        "961cc777-2547-4f9d-8174-7d86181b8a7a" => "best_efficiency",
        "00000000-0000-0000-0000-000000000000" => "balanced",
        "ded574b5-45a0-4f42-8737-46345c09c238" => "best_performance",
        _ => "unknown",
    }
}

pub fn is_guid(s: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap()
    });
    RE.is_match(s)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TlpStat {
    pub version: Option<String>,
    pub enabled: Option<bool>,
    pub mode: Option<String>,
    pub profile: Option<String>,
}

pub fn tlp_stat(text: &str) -> TlpStat {
    let mut s = TlpStat::default();
    for l in text.lines() {
        let l = l.trim();
        if let Some(rest) = l.strip_prefix("--- TLP ") {
            s.version = rest.split_whitespace().next().map(String::from);
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "State" => s.enabled = Some(v == "enabled"),
            "Mode" => s.mode = Some(v),
            "Power profile" => s.profile = Some(v),
            _ => {}
        }
    }
    s
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CpuKnobs {
    pub driver: Option<String>,
    pub governor: Option<String>,
    pub governors: Vec<String>,
    pub epp: Option<String>,
    pub epps: Vec<String>,
    pub turbo: Option<bool>,
    pub platform_profile: Option<String>,
    pub platform_profiles: Vec<String>,
    pub cpus: usize,
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn words(p: &Path) -> Vec<String> {
    read_trim(p)
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

pub fn cpufreq_dirs(sys: &Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(sys.join("devices/system/cpu"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.len() > 3 && n.starts_with("cpu") && n[3..].chars().all(|c| c.is_ascii_digit())
        })
        .map(|e| e.path().join("cpufreq"))
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

pub fn cpu_knobs(sys: &Path) -> CpuKnobs {
    let dirs = cpufreq_dirs(sys);
    let mut k = CpuKnobs {
        cpus: dirs.len(),
        ..Default::default()
    };
    if let Some(c0) = dirs.first() {
        k.driver = read_trim(&c0.join("scaling_driver"));
        k.governor = read_trim(&c0.join("scaling_governor"));
        k.governors = words(&c0.join("scaling_available_governors"));
        k.epp = read_trim(&c0.join("energy_performance_preference"));
        k.epps = words(&c0.join("energy_performance_available_preferences"));
    }
    let cpu = sys.join("devices/system/cpu");
    k.turbo = read_trim(&cpu.join("intel_pstate/no_turbo"))
        .map(|v| v == "0")
        .or_else(|| read_trim(&cpu.join("cpufreq/boost")).map(|v| v == "1"));
    let pp = sys.join("firmware/acpi");
    k.platform_profile = read_trim(&pp.join("platform_profile"));
    k.platform_profiles = words(&pp.join("platform_profile_choices"));
    k
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChargeLimit {
    pub battery: String,
    pub end: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u8>,
}

pub fn valid_battery_name(n: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_-]{0,31}$").unwrap());
    RE.is_match(n)
}

pub fn charge_limits(sys: &Path) -> Vec<ChargeLimit> {
    let root = sys.join("class/power_supply");
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let p = e.path();
        if read_trim(&p.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        if read_trim(&p.join("scope")).as_deref() == Some("Device") || !valid_battery_name(&name) {
            continue;
        }
        let end = read_trim(&p.join("charge_control_end_threshold"))
            .or_else(|| read_trim(&p.join("charge_stop_threshold")))
            .and_then(|v| v.parse().ok());
        let start = read_trim(&p.join("charge_control_start_threshold"))
            .or_else(|| read_trim(&p.join("charge_start_threshold")))
            .and_then(|v| v.parse().ok());
        if let Some(end) = end {
            out.push(ChargeLimit {
                battery: name,
                end,
                start,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/washmybelly_fixtures/power")
            .join(name);
        std::fs::read_to_string(p).unwrap()
    }

    fn fx_dir(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/washmybelly_fixtures/power")
            .join(name)
    }

    #[test]
    fn pmset_g_reads_low_power_mode_and_who_blocks_sleep() {
        let s = pmset_settings(&fx("pmset_g.txt"));
        assert_eq!(s.low_power_mode, Some(false));
        assert_eq!(s.power_mode, None);
        assert_eq!(s.sleep_minutes, Some(1));
        assert_eq!(s.display_sleep_minutes, Some(10));
        assert_eq!(
            s.sleep_prevented_by,
            [
                "native-core",
                "caffeinate",
                "Claude",
                "coreaudiod",
                "powerd"
            ]
        );
        let s = pmset_settings(&fx("pmset_g_battery_lpm.txt"));
        assert_eq!(s.low_power_mode, Some(true));
        assert_eq!(s.power_mode, Some(1));
        assert!(s.sleep_prevented_by.is_empty());
    }

    #[test]
    fn pmset_assertions_lists_blockers_but_not_user_activity() {
        let b = pmset_assertions(&fx("pmset_assertions.txt"));
        let names: Vec<_> = b.iter().map(|x| x.process.as_str()).collect();
        assert!(names.contains(&"caffeinate"));
        assert!(names.contains(&"coreaudiod"));
        assert!(names.contains(&"Claude"));
        assert!(
            !names.contains(&"WindowServer"),
            "UserIsActive is not a blocker"
        );
        let ca = b.iter().find(|x| x.process == "caffeinate").unwrap();
        assert_eq!(ca.pid, Some(1253));
        assert_eq!(ca.reason, "caffeinate command-line tool");
        assert!(!ca.display);
        assert!(b.iter().all(|x| x.pid.is_some()));
        assert_eq!(b.len(), 7);
    }

    #[test]
    fn powercfg_active_scheme_in_any_language() {
        let s = powercfg_active(&fx("powercfg_getactivescheme.txt")).unwrap();
        assert_eq!(s.guid, BALANCED);
        assert_eq!(s.name, "Balanced");
        let s = powercfg_active(&fx("powercfg_getactivescheme_pt.txt")).unwrap();
        assert_eq!(s.guid, HIGH_PERFORMANCE);
        assert_eq!(s.name, "Alto desempenho");
        assert!(powercfg_active("Access denied").is_none());
    }

    #[test]
    fn powercfg_list_marks_the_active_plan() {
        let l = powercfg_list(&fx("powercfg_list.txt"));
        assert_eq!(l.len(), 3);
        assert!(l[0].active && !l[1].active && !l[2].active);
        assert_eq!(l[2].guid, POWER_SAVER);
        let l = powercfg_list(&fx("powercfg_list_ultimate.txt"));
        assert_eq!(l.len(), 2);
        assert!(l[1].active);
        assert_eq!(l[1].name, "Ultimate Performance");
        let d = powercfg_active(&fx("powercfg_duplicatescheme.txt")).unwrap();
        assert_eq!(d.guid, "5a6c1a2e-7b19-4f55-9d2d-0c3a1e8b7f10");
    }

    #[test]
    fn powercfg_requests_keeps_category_and_reason() {
        let r = powercfg_requests(&fx("powercfg_requests.txt"));
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].kind, "DISPLAY");
        assert_eq!(r[0].process, "firefox.exe");
        assert_eq!(r[0].reason, "Playing video");
        assert!(r[0].display);
        assert_eq!(r[1].kind, "SYSTEM");
        assert!(r[1].process.starts_with("Realtek"));
        assert_eq!(r[1].reason, "An audio stream is currently in use.");
        assert_eq!(r[2].process, "omniget.exe");
    }

    #[test]
    fn overlays_and_guids() {
        assert_eq!(
            overlay_name("DED574B5-45A0-4F42-8737-46345C09C238"),
            "best_performance"
        );
        assert_eq!(
            overlay_name("00000000-0000-0000-0000-000000000000"),
            "balanced"
        );
        assert!(is_guid(BALANCED));
        assert!(!is_guid("381b4222-f694-41f0-9685-ff5bb260df2e; del"));
    }

    #[test]
    fn tlp_stat_state_and_mode() {
        let s = tlp_stat(&fx("tlp_stat_s.txt"));
        assert_eq!(s.version.as_deref(), Some("1.6.1"));
        assert_eq!(s.enabled, Some(true));
        assert_eq!(s.mode.as_deref(), Some("battery"));
        assert_eq!(
            tlp_stat("Error: missing root privilege."),
            TlpStat::default()
        );
    }

    #[test]
    fn sysfs_knobs_intel_pstate() {
        let k = cpu_knobs(&fx_dir("sysfs_intel"));
        assert_eq!(k.cpus, 2);
        assert_eq!(k.driver.as_deref(), Some("intel_pstate"));
        assert_eq!(k.governor.as_deref(), Some("powersave"));
        assert_eq!(k.governors, ["performance", "powersave"]);
        assert_eq!(k.epp.as_deref(), Some("balance_performance"));
        assert_eq!(k.epps.len(), 5);
        assert_eq!(k.turbo, Some(true));
        assert_eq!(k.platform_profile.as_deref(), Some("balanced"));
        assert_eq!(
            k.platform_profiles,
            ["low-power", "balanced", "performance"]
        );
        let c = charge_limits(&fx_dir("sysfs_intel"));
        assert_eq!(
            c,
            vec![ChargeLimit {
                battery: "BAT0".into(),
                end: 80,
                start: Some(75)
            }]
        );
    }

    #[test]
    fn sysfs_knobs_acpi_cpufreq_without_epp_and_mouse_battery_ignored() {
        let root = fx_dir("sysfs_amd");
        let k = cpu_knobs(&root);
        assert_eq!(k.cpus, 1);
        assert_eq!(k.governor.as_deref(), Some("schedutil"));
        assert!(k.epp.is_none() && k.epps.is_empty());
        assert_eq!(k.turbo, Some(true));
        assert!(k.platform_profile.is_none());
        assert!(
            charge_limits(&root).is_empty(),
            "a Logitech mouse has no charge limit to offer"
        );
    }
}
