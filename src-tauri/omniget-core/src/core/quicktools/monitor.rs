use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{DiskRefreshKind, Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System};

pub const TOP_PROCESSES: usize = 5;
const DISK_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    pub cpu_percent: Option<f32>,
    pub memory_used: u64,
    pub memory_total: u64,
    pub disk_free: Option<u64>,
    pub disk_total: Option<u64>,
    pub net_down_per_s: u64,
    pub net_up_per_s: u64,
    pub top: Vec<Proc>,
}

struct Monitor {
    sys: System,
    nets: Networks,
    disk: Option<(u64, u64)>,
    disk_at: Option<Instant>,
    last: Instant,
    warm: bool,
}

static MONITOR: Mutex<Option<Monitor>> = Mutex::new(None);

fn round1(value: f32) -> f32 {
    (value * 10.0).round() / 10.0
}

fn per_second(bytes: u64, secs: f64) -> u64 {
    (bytes as f64 / secs.max(0.2)).round() as u64
}

fn top(mut procs: Vec<Proc>, cores: usize, keep: usize) -> Vec<Proc> {
    procs.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    procs.truncate(keep);
    for p in &mut procs {
        p.cpu_percent = round1(p.cpu_percent / cores.max(1) as f32);
    }
    procs
}

fn disk_for<'a, T>(home: &Path, mounts: &'a [(std::path::PathBuf, T)]) -> Option<&'a T> {
    mounts
        .iter()
        .filter(|(mount, _)| home.starts_with(mount))
        .max_by_key(|(mount, _)| mount.as_os_str().len())
        .map(|(_, value)| value)
}

fn read_disk() -> Option<(u64, u64)> {
    let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage());
    let mounts: Vec<_> = disks
        .list()
        .iter()
        .filter(|d| d.total_space() > 0)
        .map(|d| {
            (
                d.mount_point().to_path_buf(),
                (d.available_space(), d.total_space()),
            )
        })
        .collect();
    let home = dirs::home_dir()?;
    disk_for(&home, &mounts).copied()
}

fn is_loopback(name: &str) -> bool {
    name == "lo" || name.starts_with("lo0") || name.to_ascii_lowercase().contains("loopback")
}

pub fn sample() -> Stats {
    let mut guard = MONITOR.lock().unwrap_or_else(|e| e.into_inner());
    let m = guard.get_or_insert_with(|| Monitor {
        sys: System::new(),
        nets: Networks::new_with_refreshed_list(),
        disk: None,
        disk_at: None,
        last: Instant::now(),
        warm: false,
    });
    let secs = m.last.elapsed().as_secs_f64();
    m.last = Instant::now();
    m.sys.refresh_cpu_usage();
    m.sys.refresh_memory();
    m.sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );
    m.nets.refresh(true);
    if m.disk_at.is_none_or(|at| at.elapsed() >= DISK_EVERY) {
        m.disk = read_disk();
        m.disk_at = Some(Instant::now());
    }

    let (down, up) = m
        .nets
        .list()
        .iter()
        .filter(|(name, _)| !is_loopback(name))
        .fold((0u64, 0u64), |(d, u), (_, data)| {
            (d + data.received(), u + data.transmitted())
        });
    let warm = std::mem::replace(&mut m.warm, true);
    let procs = if warm {
        m.sys
            .processes()
            .iter()
            .map(|(pid, p)| Proc {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: p.cpu_usage(),
            })
            .collect()
    } else {
        Vec::new()
    };
    Stats {
        cpu_percent: warm.then(|| round1(m.sys.global_cpu_usage())),
        memory_used: m.sys.used_memory(),
        memory_total: m.sys.total_memory(),
        disk_free: m.disk.map(|d| d.0),
        disk_total: m.disk.map(|d| d.1),
        net_down_per_s: if warm { per_second(down, secs) } else { 0 },
        net_up_per_s: if warm { per_second(up, secs) } else { 0 },
        top: top(procs, m.sys.cpus().len(), TOP_PROCESSES),
    }
}

pub fn sample_settled() -> Stats {
    let first = sample();
    if first.cpu_percent.is_some() {
        return first;
    }
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)));
    sample()
}

pub fn release() {
    *MONITOR.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn proc(pid: u32, cpu: f32) -> Proc {
        Proc {
            pid,
            name: format!("p{pid}"),
            cpu_percent: cpu,
        }
    }

    #[test]
    fn the_busiest_processes_come_first_scaled_to_the_whole_machine() {
        let list = vec![proc(1, 20.0), proc(2, 160.0), proc(3, 0.0), proc(4, 80.04)];
        let out = top(list, 8, 3);
        assert_eq!(out.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2, 4, 1]);
        assert_eq!(out[0].cpu_percent, 20.0);
        assert_eq!(out[1].cpu_percent, 10.0);
        assert_eq!(out[2].cpu_percent, 2.5);
        assert!(top(Vec::new(), 0, 5).is_empty());
    }

    #[test]
    fn the_disk_is_the_one_that_holds_the_home_folder() {
        let mounts = vec![
            (PathBuf::from("/"), "root"),
            (PathBuf::from("/home"), "home"),
            (PathBuf::from("/mnt/usb"), "usb"),
        ];
        assert_eq!(disk_for(Path::new("/home/ana"), &mounts), Some(&"home"));
        assert_eq!(disk_for(Path::new("/root"), &mounts), Some(&"root"));
        assert_eq!(disk_for(Path::new("relative"), &mounts), None);
    }

    #[test]
    fn rates_are_per_second_and_a_tiny_interval_does_not_explode() {
        assert_eq!(per_second(3000, 2.0), 1500);
        assert_eq!(per_second(1000, 0.0), 5000);
    }

    #[test]
    fn a_settled_sample_has_real_numbers_and_release_frees_the_sampler() {
        let stats = sample_settled();
        assert!(stats.memory_total > 0);
        assert!(stats.cpu_percent.is_some());
        assert!(stats.top.len() <= TOP_PROCESSES);
        release();
        assert!(MONITOR.lock().unwrap().is_none());
    }
}
