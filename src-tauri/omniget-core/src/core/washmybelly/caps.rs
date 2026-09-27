//! `wmb_capabilities`: what this machine is and which modules make sense on
//! it. The screen and the agents decide what to show from this one answer,
//! so a module of another system never appears, and a module of this system
//! missing a requirement appears with the reason.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use super::{current_os, Os};

#[derive(Debug, Clone, Serialize, Default)]
pub struct Linux {
    pub distro_id: String,
    pub distro_like: Vec<String>,
    pub distro_name: String,
    pub package_managers: Vec<String>,
    pub flatpak_sandbox: bool,
    /// `power-profiles-daemon`, `tlp`, `auto-cpufreq`, `tuned`: the ones running.
    pub power_managers: Vec<String>,
    pub gamemode: bool,
    pub mangohud: bool,
    pub systemd: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Windows {
    pub edition: String,
    pub product_name: String,
    pub build: u32,
    pub performance_log_users: bool,
    pub winget: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Macos {
    pub apple_silicon: bool,
    pub full_disk_access: bool,
    pub homebrew: bool,
    pub xcode: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Module {
    pub id: &'static str,
    pub available: bool,
    /// i18n key under `washmybelly.reason.*` when something is missing.
    pub reason_key: Option<&'static str>,
    pub state: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub os: Os,
    pub os_version: String,
    pub arch: &'static str,
    pub is_admin: bool,
    pub has_battery: bool,
    pub linux: Option<Linux>,
    pub windows: Option<Windows>,
    pub macos: Option<Macos>,
    pub modules: Vec<Module>,
    pub source_url: &'static str,
    pub privacy_source_url: &'static str,
    pub elevation: &'static str,
    pub helper_installed: bool,
}

/// Every module in screen order, and the systems it exists on.
pub const MODULES: &[(&str, &[Os], &str)] = &[
    ("overview", &[], "stable"),
    ("clean", &[], "stable"),
    ("disk", &[], "stable"),
    ("uninstall", &[], "beta"),
    ("startup", &[], "stable"),
    ("privacy", &[], "beta"),
    ("tweaks", &[Os::Windows], "beta"),
    ("packages", &[], "beta"),
    ("power", &[], "beta"),
    ("games", &[], "beta"),
    ("perf", &[], "experimental"),
    ("security", &[Os::Macos], "beta"),
    ("maintenance", &[], "beta"),
    ("receipts", &[], "stable"),
];

fn on_path(program: &str) -> bool {
    let path = crate::core::process::enhanced_path().unwrap_or_default();
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    std::env::split_paths(&path).any(|d| exts.iter().any(|e| d.join(format!("{program}{e}")).is_file()))
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = crate::core::process::std_command(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn is_admin() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(windows)]
    {
        // `net session` only succeeds in an elevated token; no extra API.
        crate::core::process::std_command("net")
            .args(["session"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

fn has_battery() -> bool {
    match current_os() {
        Os::Linux => std::fs::read_dir("/sys/class/power_supply")
            .map(|d| {
                d.flatten().any(|e| {
                    std::fs::read_to_string(e.path().join("type"))
                        .map(|t| t.trim() == "Battery")
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        Os::Macos => output("pmset", &["-g", "batt"]).is_some_and(|t| t.contains("InternalBattery")),
        Os::Windows => output("powershell", &["-NoProfile", "-Command", "(Get-CimInstance Win32_Battery | Measure-Object).Count"])
            .and_then(|t| t.trim().parse::<u32>().ok())
            .is_some_and(|n| n > 0),
    }
}

fn detect_linux() -> Linux {
    let rel = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let m = super::parse::os_release(&rel);
    let managers = ["apt-get", "dnf", "pacman", "zypper", "snap", "flatpak"]
        .into_iter()
        .filter(|p| on_path(p))
        .map(|p| if p == "apt-get" { "apt".to_string() } else { p.to_string() })
        .collect();
    let systemd = Path::new("/run/systemd/system").exists();
    let active = |unit: &str| {
        systemd
            && crate::core::process::std_command("systemctl")
                .args(["is-active", "--quiet", unit])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    };
    let power_managers = [("power-profiles-daemon", "power-profiles-daemon.service"), ("tlp", "tlp.service"), ("auto-cpufreq", "auto-cpufreq.service"), ("tuned", "tuned.service")]
        .into_iter()
        .filter(|(_, unit)| active(unit))
        .map(|(n, _)| n.to_string())
        .collect();
    Linux {
        distro_id: m.get("ID").cloned().unwrap_or_default(),
        distro_like: m
            .get("ID_LIKE")
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_default(),
        distro_name: m.get("PRETTY_NAME").cloned().unwrap_or_default(),
        package_managers: managers,
        flatpak_sandbox: crate::core::dependencies::is_flatpak(),
        power_managers,
        gamemode: on_path("gamemoded") || Path::new("/usr/share/dbus-1/services/com.feralinteractive.GameMode.service").exists(),
        mangohud: on_path("mangohud"),
        systemd,
    }
}

fn detect_windows() -> Windows {
    let reg = output("reg", &["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion"]).unwrap_or_default();
    let vals = super::parse::reg_query(&reg);
    let get = |n: &str| vals.iter().find(|v| v.name == n).map(|v| v.data.clone()).unwrap_or_default();
    let groups = output("whoami", &["/groups", "/fo", "csv"]).unwrap_or_default();
    Windows {
        edition: get("EditionID"),
        product_name: get("ProductName"),
        build: get("CurrentBuild").parse().unwrap_or(0),
        performance_log_users: super::parse::whoami_has_sid(&groups, "S-1-5-32-559"),
        winget: on_path("winget"),
    }
}

/// Full Disk Access is only known by reading a protected file (the TCC
/// database of the user), as PureMac does.
fn full_disk_access() -> bool {
    let Some(home) = dirs::home_dir() else { return false };
    std::fs::File::open(home.join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok()
}

fn detect_macos() -> Macos {
    Macos {
        apple_silicon: cfg!(target_arch = "aarch64"),
        full_disk_access: full_disk_access(),
        homebrew: on_path("brew"),
        xcode: Path::new("/Applications/Xcode.app").exists() || output("xcode-select", &["-p"]).is_some(),
    }
}

fn os_version() -> String {
    match current_os() {
        Os::Macos => output("sw_vers", &["-productVersion"]).unwrap_or_default().trim().to_string(),
        Os::Linux => {
            let rel = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
            super::parse::os_release(&rel).get("VERSION_ID").cloned().unwrap_or_default()
        }
        Os::Windows => {
            let reg = output("reg", &["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "/v", "DisplayVersion"]).unwrap_or_default();
            super::parse::reg_query(&reg).first().map(|v| v.data.clone()).unwrap_or_default()
        }
    }
}

/// Which modules this machine gets. Pure over the detected facts, so the
/// rules are tested for all three systems on any CI.
pub fn modules_for(os: Os, linux: Option<&Linux>, macos: Option<&Macos>) -> Vec<Module> {
    MODULES
        .iter()
        .filter(|(_, platforms, _)| platforms.is_empty() || platforms.contains(&os))
        .map(|(id, _, state)| {
            let mut reason = None;
            match (*id, os) {
                ("uninstall" | "packages", Os::Linux) if linux.is_some_and(|l| l.flatpak_sandbox) => reason = Some("washmybelly.reason.flatpak_sandbox"),
                ("packages", Os::Linux) if linux.is_some_and(|l| l.package_managers.is_empty()) => reason = Some("washmybelly.reason.no_package_manager"),
                ("clean", Os::Macos) if macos.is_some_and(|m| !m.full_disk_access) => {
                    // Still available: most caches do not need Full Disk
                    // Access; the screen shows the hint for the rest.
                }
                _ => {}
            }
            Module {
                id,
                available: reason.is_none(),
                reason_key: reason,
                state,
            }
        })
        .collect()
}

pub fn detect() -> Capabilities {
    let os = current_os();
    let linux = (os == Os::Linux).then(detect_linux);
    let windows = (os == Os::Windows).then(detect_windows);
    let macos = (os == Os::Macos).then(detect_macos);
    let modules = modules_for(os, linux.as_ref(), macos.as_ref());
    let os_version = match &windows {
        Some(w) if w.build > 0 => format!("{} ({})", os_version(), w.build),
        _ => os_version(),
    };
    Capabilities {
        os,
        os_version,
        arch: std::env::consts::ARCH,
        is_admin: is_admin(),
        has_battery: has_battery(),
        linux,
        windows,
        macos,
        modules,
        source_url: super::SOURCE_URL,
        privacy_source_url: super::PRIVACY_SEXY_SOURCE_URL,
        elevation: match os {
            Os::Macos => "osascript",
            Os::Windows => "uac",
            Os::Linux => "pkexec",
        },
        helper_installed: super::elevate::helper_path().is_some(),
    }
}

static CACHE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

/// Cached for five minutes; `refresh` recomputes.
pub fn cached(refresh: bool) -> Value {
    if !refresh {
        if let Ok(c) = CACHE.lock() {
            if let Some((at, v)) = c.as_ref() {
                if at.elapsed() < Duration::from_secs(300) {
                    return v.clone();
                }
            }
        }
    }
    let v = serde_json::to_value(detect()).unwrap_or_else(|_| json!({}));
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((Instant::now(), v.clone()));
    }
    v
}

pub async fn tool_capabilities(a: Value) -> Result<Value, String> {
    let refresh = a.get("refresh").and_then(Value::as_bool).unwrap_or(false);
    super::tools::blocking(move || {
        let mut v = cached(refresh);
        v["tools"] = json!(crate::core::llm::tool_table::available()
            .filter(|e| e.name.starts_with("wmb_"))
            .map(|e| e.name)
            .collect::<Vec<_>>());
        Ok(v)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_never_gets_windows_tweaks_and_windows_never_gets_the_mac_inventory() {
        let mac: Vec<_> = modules_for(Os::Macos, None, None).into_iter().map(|m| m.id).collect();
        let win: Vec<_> = modules_for(Os::Windows, None, None).into_iter().map(|m| m.id).collect();
        let lin: Vec<_> = modules_for(Os::Linux, None, None).into_iter().map(|m| m.id).collect();
        assert!(!mac.contains(&"tweaks") && mac.contains(&"security"));
        assert!(win.contains(&"tweaks") && !win.contains(&"security"));
        assert!(!lin.contains(&"tweaks") && !lin.contains(&"security"));
    }

    #[test]
    fn inside_flatpak_uninstall_says_why() {
        let l = Linux {
            flatpak_sandbox: true,
            package_managers: vec!["flatpak".into()],
            ..Default::default()
        };
        let m = modules_for(Os::Linux, Some(&l), None);
        let u = m.iter().find(|m| m.id == "uninstall").unwrap();
        assert!(!u.available);
        assert_eq!(u.reason_key, Some("washmybelly.reason.flatpak_sandbox"));
    }
}
