pub mod parse;
pub mod score;

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
#[cfg(any(target_os = "macos", test))]
mod smc_keys;
#[cfg(windows)]
mod windows;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Map, Value};
use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind,
    ProcessesToUpdate, System, UpdateKind, Users,
};

use self::parse::{Battery, Gpu};
use self::score::{Readings, ScoreInputs};
use super::store::Ctx;
use super::tools::blocking;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Temp {
    pub name: String,
    pub group: &'static str,
    pub celsius: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical: Option<f64>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Fan {
    pub name: String,
    pub rpm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rpm: Option<f64>,
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn battery() -> Result<Battery, String> {
    #[cfg(target_os = "macos")]
    {
        macos::battery()
    }
    #[cfg(windows)]
    {
        windows::battery()
    }
    #[cfg(target_os = "linux")]
    {
        Ok(linux::read_battery(std::path::Path::new(
            "/sys/class/power_supply",
        )))
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        Err("battery: unsupported system".into())
    }
}

pub async fn tool_battery_health(_a: Value) -> Result<Value, String> {
    let b = blocking(battery).await?;
    serde_json::to_value(b).map_err(|e| e.to_string())
}

#[derive(Debug, Default)]
struct Sensors {
    temperatures: Vec<Temp>,
    fans: Vec<Fan>,
    power: Vec<(String, f64)>,
    gpus: Vec<Gpu>,
    cpu_c: Option<f64>,
    gpu_c: Option<f64>,
    reasons: Map<String, Value>,
}

fn reason(s: &mut Sensors, what: &str, why: impl Into<String>) {
    s.reasons
        .insert(what.to_string(), Value::String(why.into()));
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
fn component_temps() -> Vec<Temp> {
    let comps = sysinfo::Components::new_with_refreshed_list();
    comps
        .list()
        .iter()
        .filter_map(|c| {
            let t = c.temperature()? as f64;
            if !(t > 0.0 && t < 150.0) {
                return None;
            }
            let l = c.label().to_ascii_lowercase();
            let group = if l.contains("cpu")
                || l.contains("pacc")
                || l.contains("eacc")
                || l.contains("package")
                || l.contains("core")
            {
                "CPU"
            } else if l.contains("gpu") {
                "GPU"
            } else if l.contains("nand") || l.contains("ssd") || l.contains("nvme") {
                "disk"
            } else if l.contains("battery") || l.contains("gas gauge") {
                "battery"
            } else {
                "sensor"
            };
            Some(Temp {
                name: c.label().to_string(),
                group,
                celsius: r1(t),
                critical: c.critical().map(|x| x as f64),
                source: "sysinfo",
            })
        })
        .collect()
}

fn hottest(temps: &[Temp], group: &str) -> Option<f64> {
    temps
        .iter()
        .filter(|t| t.group == group)
        .map(|t| t.celsius)
        .reduce(f64::max)
}

#[cfg(target_os = "linux")]
fn linux_gpus(s: &mut Sensors) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static NO_NVIDIA_SMI: AtomicBool = AtomicBool::new(false);
    let mut gpus = linux::read_drm(std::path::Path::new("/sys/class/drm"));
    let wants_smi = gpus.is_empty() || gpus.iter().any(|g| g.vendor == "nvidia");
    if wants_smi && !NO_NVIDIA_SMI.load(Ordering::Relaxed) {
        match crate::core::process::std_command("nvidia-smi")
            .args([parse::NVIDIA_QUERY, "--format=csv,noheader,nounits"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let smi = parse::gpus_from_nvidia_smi(&String::from_utf8_lossy(&o.stdout));
                if !smi.is_empty() {
                    gpus.retain(|g| g.vendor != "nvidia");
                    gpus.extend(smi);
                }
            }
            Ok(_) => reason(s, "gpu_nvidia", "nvidia-smi failed"),
            Err(_) => {
                NO_NVIDIA_SMI.store(true, Ordering::Relaxed);
                if gpus.iter().any(|g| g.vendor == "nvidia") {
                    reason(s, "gpu_nvidia", "nvidia-smi not installed");
                }
            }
        }
    }
    s.gpus = gpus;
}

fn read_sensors(light: bool) -> Sensors {
    let mut s = Sensors::default();

    #[cfg(target_os = "macos")]
    {
        if light {
            let (c, g) = macos::main_temps();
            s.cpu_c = c;
            s.gpu_c = g;
        } else {
            match macos::smc_sensors() {
                Ok((t, f, p)) => {
                    s.temperatures = t;
                    s.fans = f;
                    s.power = p;
                }
                Err(e) => {
                    reason(&mut s, "smc", e);
                    s.temperatures = component_temps();
                }
            }
            let (c, g) = macos::main_temps();
            s.cpu_c = c.or_else(|| hottest(&s.temperatures, "CPU"));
            s.gpu_c = g.or_else(|| hottest(&s.temperatures, "GPU"));
            if s.fans.is_empty() {
                reason(
                    &mut s,
                    "fans",
                    "no fans reported (fanless Mac or SMC unreadable)",
                );
            }
        }
        match macos::gpus() {
            Ok(g) => s.gpus = g,
            Err(e) => reason(&mut s, "gpu", e),
        }
        if let (Some(t), Some(g)) = (s.gpu_c, s.gpus.first_mut()) {
            g.temperature_c.get_or_insert(t);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let (t, f, p) = linux::read_hwmon(std::path::Path::new("/sys/class/hwmon"));
        s.cpu_c = linux::cpu_temp(&t);
        s.gpu_c = hottest(&t, "GPU");
        if !light {
            if t.is_empty() {
                reason(&mut s, "temperatures", "no hwmon sensors readable");
            }
            if f.is_empty() {
                reason(&mut s, "fans", "no fan sensor in hwmon");
            }
            s.temperatures = t;
            s.fans = f;
            s.power = p;
        }
        linux_gpus(&mut s);
    }

    #[cfg(windows)]
    {
        let t = component_temps();
        s.cpu_c = hottest(&t, "CPU").or_else(|| t.iter().map(|x| x.celsius).reduce(f64::max));
        s.gpu_c = hottest(&t, "GPU");
        if !light {
            if t.is_empty() {
                reason(
                    &mut s,
                    "temperatures",
                    "Windows exposes no thermal zone to this user",
                );
            }
            reason(
                &mut s,
                "fans",
                "Windows does not expose fan speed without a vendor driver",
            );
            reason(&mut s, "power", "not exposed without a vendor driver");
            s.temperatures = t;
        }
        match windows::gpus() {
            Ok(g) => s.gpus = g,
            Err(e) => reason(&mut s, "gpu", e),
        }
    }

    if s.gpus.is_empty() && !s.reasons.contains_key("gpu") {
        reason(&mut s, "gpu", "no GPU load readable");
    }
    s
}

pub async fn tool_sensors(_a: Value) -> Result<Value, String> {
    blocking(|| {
        let s = read_sensors(false);
        let pick = |needle: &str| {
            s.power
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(needle))
                .map(|(_, w)| *w)
        };
        Ok(json!({
            "temperatures": s.temperatures,
            "fans": s.fans,
            "power": {
                "system_w": pick("System Total"),
                "battery_w": pick("Battery"),
                "dc_in_w": pick("DC In"),
                "readings": s.power.iter().map(|(n, w)| json!({ "name": n, "watts": w })).collect::<Vec<_>>(),
            },
            "gpu": s.gpus,
            "cpu_temperature_c": s.cpu_c,
            "gpu_temperature_c": s.gpu_c,
            "unavailable": s.reasons,
        }))
    })
    .await
}

fn memory_pressure(sys: &System) -> (Option<f64>, &'static str) {
    #[cfg(target_os = "macos")]
    if let Some(p) = macos::memory_pressure() {
        return (Some(p), "memorystatus_level");
    }
    let total = sys.total_memory();
    if total == 0 {
        return (None, "none");
    }
    let used = total.saturating_sub(sys.available_memory());
    (Some(r1(used as f64 / total as f64 * 100.0)), "used_percent")
}

#[cfg(target_os = "linux")]
fn psi_memory() -> Option<f64> {
    std::fs::read_to_string("/proc/pressure/memory")
        .ok()
        .and_then(|t| parse::psi_some_avg10(&t))
}

fn system_volume() -> Option<Value> {
    let disks = Disks::new_with_refreshed_list();
    #[cfg(windows)]
    let want = std::path::PathBuf::from(format!(
        "{}\\",
        std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
    ));
    #[cfg(not(windows))]
    let want = std::path::PathBuf::from("/");
    let d = disks
        .list()
        .iter()
        .find(|d| d.mount_point() == want)
        .or_else(|| disks.list().iter().max_by_key(|d| d.total_space()))?;
    let total = d.total_space();
    let free = d.available_space();
    Some(json!({
        "mount": d.mount_point().to_string_lossy(),
        "name": d.name().to_string_lossy(),
        "file_system": d.file_system().to_string_lossy(),
        "total": total,
        "free": free,
        "free_percent": if total > 0 { r1(free as f64 / total as f64 * 100.0) } else { 0.0 },
    }))
}

fn score_inputs() -> ScoreInputs {
    Ctx::app()
        .ok()
        .filter(|c| c.dir.join("wmb.db").exists())
        .and_then(|c| c.kv_get::<ScoreInputs>("score_inputs"))
        .unwrap_or_default()
}

fn cpu_window(ms: u64) -> Duration {
    if cfg!(test) {
        Duration::from_millis(20)
    } else {
        sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(ms))
    }
}

fn status(inputs: ScoreInputs) -> Result<Value, String> {
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    std::thread::sleep(cpu_window(200));
    sys.refresh_cpu_specifics(CpuRefreshKind::everything());
    sys.refresh_memory_specifics(MemoryRefreshKind::everything());

    let cpus = sys.cpus();
    let load = System::load_average();
    let (pressure, pressure_source) = memory_pressure(&sys);
    let volume = system_volume();
    let sensors = read_sensors(true);
    let bat = battery().ok();

    let readings = Readings {
        disk_total: volume.as_ref().and_then(|v| v["total"].as_u64()),
        disk_free: volume.as_ref().and_then(|v| v["free"].as_u64()),
        cpu_temp_c: sensors.cpu_c,
        battery_health_percent: bat.as_ref().and_then(|b| b.health_percent),
        battery_cycles: bat.as_ref().and_then(|b| b.cycles),
        memory_pressure: pressure,
        swap_used: Some(sys.used_swap()),
        inputs: inputs.clone(),
    };
    let (points, factors) = score::score(&readings);
    let gains = score::gains(&readings);

    #[allow(unused_mut)]
    let mut memory = json!({
        "total": sys.total_memory(),
        "used": sys.used_memory(),
        "available": sys.available_memory(),
        "swap_total": sys.total_swap(),
        "swap_used": sys.used_swap(),
        "pressure_percent": pressure,
        "pressure_source": pressure_source,
    });
    #[cfg(target_os = "linux")]
    if let Some(p) = psi_memory() {
        memory["psi_some_avg10"] = json!(p);
    }

    let mut temperature = json!({ "cpu_c": sensors.cpu_c, "gpu_c": sensors.gpu_c });
    if sensors.cpu_c.is_none() {
        temperature["reason"] = json!("no CPU temperature readable without root on this machine");
    }

    Ok(json!({
        "score": points,
        "grade": score::grade(points),
        "factors": factors,
        "gains": gains,
        "hooks": {
            "junk": inputs.junk_bytes.is_some(),
            "login_items": inputs.login_items.is_some(),
            "privacy": inputs.privacy_pending.is_some(),
        },
        "cpu": {
            "usage_percent": r1(sys.global_cpu_usage() as f64),
            "cores": cpus.len(),
            "physical_cores": System::physical_core_count(),
            "brand": cpus.first().map(|c| c.brand().trim().to_string()),
            "frequency_mhz": cpus.first().map(|c| c.frequency()).filter(|f| *f > 0),
            "load_average": if cfg!(windows) { Value::Null } else { json!([load.one, load.five, load.fifteen]) },
        },
        "memory": memory,
        "disk": volume,
        "temperature": temperature,
        "battery": bat.map(|b| serde_json::to_value(b).unwrap_or(Value::Null)),
        "uptime_secs": System::uptime(),
    }))
}

pub async fn tool_system_status(_a: Value) -> Result<Value, String> {
    blocking(|| status(score_inputs())).await
}

const PROTECTED_NAMES: &[&str] = &[
    "launchd",
    "loginwindow",
    "windowserver",
    "kernel_task",
    "systemd",
    "init",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "smss.exe",
    "lsass.exe",
    "services.exe",
    "dwm.exe",
    "system",
    "registry",
];

fn is_protected(pid: u32, name: &str) -> bool {
    pid == 0
        || pid == 1
        || pid == std::process::id()
        || PROTECTED_NAMES.contains(&name.to_ascii_lowercase().as_str())
}

fn list_processes(sort: &str, limit: usize, query: &str) -> Result<Value, String> {
    let mut sys = System::new();
    let first = ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_disk_usage()
        .with_user(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, first);
    let t0 = Instant::now();
    std::thread::sleep(cpu_window(300));
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage(),
    );
    let secs = t0.elapsed().as_secs_f64().max(0.001);
    let users = Users::new_with_refreshed_list();
    let own = Pid::from_u32(std::process::id());
    let me = sys.process(own).and_then(|p| p.user_id()).cloned();
    let q = query.trim().to_lowercase();

    let mut rows: Vec<(f64, Value)> = sys
        .processes()
        .values()
        .filter(|p| p.thread_kind().is_none())
        .filter_map(|p| {
            let name = p.name().to_string_lossy().to_string();
            let args: Vec<String> = p
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy().to_string())
                .collect();
            let pid = p.pid().as_u32();
            if !q.is_empty()
                && !name.to_lowercase().contains(&q)
                && !args.join(" ").to_lowercase().contains(&q)
                && pid.to_string() != q
            {
                return None;
            }
            let d = p.disk_usage();
            let read = d.read_bytes as f64 / secs;
            let write = d.written_bytes as f64 / secs;
            let cpu = p.cpu_usage() as f64;
            let mem = p.memory();
            let key = match sort {
                "memory" => mem as f64,
                "disk" => read + write,
                _ => cpu,
            };
            let user = p
                .user_id()
                .and_then(|u| users.get_user_by_id(u))
                .map(|u| u.name().to_string());
            let mine = matches!((p.user_id(), &me), (Some(a), Some(b)) if a == b);
            Some((
                key,
                json!({
                    "pid": pid,
                    "parent_pid": p.parent().map(|x| x.as_u32()),
                    "name": name,
                    "user": user,
                    "cpu_percent": r1(cpu),
                    "memory_bytes": mem,
                    "disk_read_per_s": read.round() as u64,
                    "disk_write_per_s": write.round() as u64,
                    "command": parse::redact_cmd(&args, 300),
                    "started": p.start_time(),
                    "status": p.status().to_string(),
                    "mine": mine,
                    "can_end": mine && !is_protected(pid, &name),
                }),
            ))
        })
        .collect();
    let total = rows.len();
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    rows.truncate(limit);
    Ok(json!({
        "processes": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "total": total,
        "sort": sort,
        "cpu_count": sys.cpus().len().max(num_cpus()),
    }))
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

pub async fn tool_processes(a: Value) -> Result<Value, String> {
    let sort = match a["sort"].as_str() {
        Some("memory") => "memory",
        Some("disk") => "disk",
        _ => "cpu",
    };
    let limit = a["limit"].as_u64().unwrap_or(25).clamp(1, 200) as usize;
    let query = a["query"].as_str().unwrap_or("").to_string();
    blocking(move || list_processes(sort, limit, &query)).await
}

fn check_target(pid: u32, started: Option<u64>) -> Result<(System, Pid), String> {
    if pid == 0 || pid == 1 || pid == std::process::id() {
        return Err("REFUSED: this process cannot be ended from here".into());
    }
    let target = Pid::from_u32(pid);
    let own = Pid::from_u32(std::process::id());
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target, own]),
        true,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
    );
    let p = sys
        .process(target)
        .ok_or("NOT_FOUND: the process is gone")?;
    if let Some(st) = started {
        if p.start_time() != st {
            return Err("CHANGED: that PID now belongs to another process".into());
        }
    }
    let name = p.name().to_string_lossy().to_string();
    if is_protected(pid, &name) {
        return Err(format!("REFUSED: {name} keeps the session running"));
    }
    let me = sys.process(own).and_then(|p| p.user_id()).cloned();
    match (p.user_id(), me) {
        (Some(a), Some(b)) if *a == b => {}
        _ => return Err("REFUSED: the process belongs to another user".into()),
    }
    Ok((sys, target))
}

fn pid_arg(a: &Value) -> Result<u32, String> {
    a["pid"]
        .as_u64()
        .filter(|p| *p <= u32::MAX as u64)
        .map(|p| p as u32)
        .ok_or_else(|| "pid (integer) is required".into())
}

fn kill(pid: u32, started: Option<u64>) -> Result<Value, String> {
    let (sys, target) = check_target(pid, started)?;
    let p = sys
        .process(target)
        .ok_or("NOT_FOUND: the process is gone")?;
    let (sent, how) = match p.kill_with(sysinfo::Signal::Term) {
        Some(ok) => (ok, "terminate"),
        None => (p.kill(), "kill"),
    };
    if !sent {
        return Err("the system refused to end the process".into());
    }
    Ok(json!({ "pid": pid, "signal": how, "sent": true }))
}

#[cfg(unix)]
fn set_priority(pid: u32, level: &str) -> Result<(), String> {
    let nice = match level {
        "low" => 10,
        "normal" => 0,
        "high" => -5,
        _ => return Err(format!("unknown priority {level}")),
    };
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS as _, pid as libc::id_t, nice) };
    if rc == 0 {
        Ok(())
    } else {
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EACCES) | Some(libc::EPERM) => {
                Err("NEEDS_ADMIN: raising a priority (or undoing a lowered one) needs administrator rights".into())
            }
            _ => Err(format!("setpriority: {e}")),
        }
    }
}

#[cfg(windows)]
fn set_priority(pid: u32, level: &str) -> Result<(), String> {
    windows::set_priority(pid, level)
}

fn priority(pid: u32, level: &str, started: Option<u64>) -> Result<Value, String> {
    if !matches!(level, "low" | "normal" | "high") {
        return Err("level must be low, normal or high".into());
    }
    check_target(pid, started)?;
    set_priority(pid, level)?;
    Ok(json!({ "pid": pid, "level": level }))
}

pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    let pid = pid_arg(&a);
    let started = a["started"].as_u64();
    match action {
        "kill" => {
            let pid = pid?;
            blocking(move || kill(pid, started)).await
        }
        "priority" => {
            let pid = pid?;
            let level = a["level"].as_str().unwrap_or("").to_string();
            blocking(move || priority(pid, &level, started)).await
        }
        _ => Err(format!("monitor: unknown screen action {action}")),
    }
}

struct Live {
    sys: System,
    nets: Networks,
    disks: Disks,
    last: Instant,
    #[cfg(target_os = "linux")]
    stat: Option<parse::CpuTimes>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

#[cfg(target_os = "linux")]
fn proc_stat() -> Option<parse::CpuTimes> {
    std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|t| parse::proc_stat_total(&t))
}

fn is_loopback(name: &str) -> bool {
    name == "lo" || name.starts_with("lo0") || name.to_ascii_lowercase().contains("loopback")
}

pub fn live_snapshot() -> Result<Value, String> {
    let mut guard = LIVE.lock().map_err(|_| "monitor lock")?;
    if guard.is_none() {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        let nets = Networks::new_with_refreshed_list();
        let disks =
            Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_io_usage());
        *guard = Some(Live {
            sys,
            nets,
            disks,
            last: Instant::now(),
            #[cfg(target_os = "linux")]
            stat: proc_stat(),
        });
        drop(guard);
        std::thread::sleep(cpu_window(250));
        guard = LIVE.lock().map_err(|_| "monitor lock")?;
    }
    let live = guard.as_mut().ok_or("monitor state")?;
    let secs = live.last.elapsed().as_secs_f64().max(0.05);
    live.last = Instant::now();
    live.sys.refresh_cpu_usage();
    live.sys
        .refresh_memory_specifics(MemoryRefreshKind::everything());
    live.nets.refresh(true);
    live.disks
        .refresh_specifics(false, DiskRefreshKind::nothing().with_io_usage());

    let (rx, tx) = live
        .nets
        .list()
        .iter()
        .filter(|(n, _)| !is_loopback(n))
        .fold((0u64, 0u64), |(r, t), (_, d)| {
            (r + d.received(), t + d.transmitted())
        });

    let mut seen = std::collections::HashSet::new();
    let (dr, dw) = live
        .disks
        .list()
        .iter()
        .map(|d| d.usage())
        .filter(|u| seen.insert((u.total_read_bytes, u.total_written_bytes)))
        .fold((0u64, 0u64), |(r, w), u| {
            (r + u.read_bytes, w + u.written_bytes)
        });

    let cores: Vec<f64> = live
        .sys
        .cpus()
        .iter()
        .map(|c| r1(c.cpu_usage() as f64))
        .collect();
    let (pressure, _) = memory_pressure(&live.sys);
    let mut out = json!({
        "t": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        "cpu": { "total": r1(live.sys.global_cpu_usage() as f64), "cores": cores },
        "memory": {
            "total": live.sys.total_memory(),
            "used": live.sys.used_memory(),
            "available": live.sys.available_memory(),
            "swap_total": live.sys.total_swap(),
            "swap_used": live.sys.used_swap(),
            "pressure_percent": pressure,
        },
        "net": { "rx_per_s": (rx as f64 / secs).round() as u64, "tx_per_s": (tx as f64 / secs).round() as u64 },
        "disk": { "read_per_s": (dr as f64 / secs).round() as u64, "write_per_s": (dw as f64 / secs).round() as u64 },
    });

    #[cfg(target_os = "linux")]
    {
        let now = proc_stat();
        if let (Some(a), Some(b)) = (live.stat, now) {
            out["iowait_percent"] = json!(parse::iowait_percent(a, b));
        }
        live.stat = now;
    }
    drop(guard);

    let s = read_sensors(true);
    let gpu = s
        .gpus
        .iter()
        .filter_map(|g| g.utilization_percent)
        .reduce(f64::max);
    out["gpu_percent"] = json!(gpu);
    out["temp_c"] = json!(s.cpu_c);
    out["gpu_temp_c"] = json!(s.gpu_c);
    Ok(out)
}
