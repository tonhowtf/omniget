use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub const MIN_N: u32 = 1;
pub const MAX_N: u32 = 16;

pub const GLOBAL_CONNECTION_BUDGET: u32 = 24;

static ACTIVE_DOWNLOADS: AtomicU32 = AtomicU32::new(0);

pub fn active_downloads() -> u32 {
    ACTIVE_DOWNLOADS.load(Ordering::Relaxed)
}

pub struct ActiveDownloadGuard(());

impl ActiveDownloadGuard {
    pub fn new() -> Self {
        ACTIVE_DOWNLOADS.fetch_add(1, Ordering::Relaxed);
        Self(())
    }
}

impl Default for ActiveDownloadGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ActiveDownloadGuard {
    fn drop(&mut self) {
        ACTIVE_DOWNLOADS.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn per_download_ceiling(budget: u32, active: u32) -> u32 {
    (budget / active.max(1)).clamp(MIN_N, MAX_N)
}

pub const MEANINGFUL_GAIN: f64 = 1.10;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Sample {
    pub concurrency: u32,
    pub throughput_bps: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HostStats {
    samples: Vec<Sample>,
    rate_limit_hits: u32,
}

impl HostStats {
    pub fn record(&mut self, concurrency: u32, throughput_bps: f64) {
        if throughput_bps.is_finite() && throughput_bps > 0.0 {
            self.samples.push(Sample {
                concurrency,
                throughput_bps,
            });
        }
    }

    pub fn record_rate_limit(&mut self) {
        self.rate_limit_hits = self.rate_limit_hits.saturating_add(1);
    }

    fn best_at(&self, concurrency: u32) -> Option<f64> {
        self.samples
            .iter()
            .filter(|s| s.concurrency == concurrency)
            .map(|s| s.throughput_bps)
            .fold(None, |acc: Option<f64>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            })
    }
}

pub fn next_concurrency(stats: &HostStats, current: u32) -> u32 {
    let current = current.clamp(MIN_N, MAX_N);

    if stats.rate_limit_hits > 0 {
        let penalizado = current / 2u32.pow(stats.rate_limit_hits.min(3));
        return penalizado.clamp(MIN_N, MAX_N);
    }

    let Some(atual) = stats.best_at(current) else {
        return current;
    };

    match stats.best_at(current.saturating_sub(1)) {
        Some(anterior) if anterior > atual * MEANINGFUL_GAIN => (current - 1).clamp(MIN_N, MAX_N),
        _ => {
            let vale_subir = match stats.best_at(current.saturating_sub(1)) {
                Some(anterior) => atual > anterior * MEANINGFUL_GAIN,
                None => true,
            };
            if vale_subir {
                (current + 1).clamp(MIN_N, MAX_N)
            } else {
                current
            }
        }
    }
}

pub const RATE_LIMIT_MEMORY_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RememberedLimit {
    pub rate_limit_hits: u32,
    pub last_at: u64,
}

pub fn load_remembered(path: &Path, now: u64) -> HashMap<String, RememberedLimit> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let map: HashMap<String, RememberedLimit> = serde_json::from_slice(&bytes).unwrap_or_default();
    map.into_iter()
        .filter(|(_, r)| now.saturating_sub(r.last_at) < RATE_LIMIT_MEMORY_SECS)
        .collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct ConcurrencyTuner {
    hosts: HashMap<String, HostStats>,
    store: Option<PathBuf>,
}

impl ConcurrencyTuner {
    pub fn with_store(path: PathBuf) -> Self {
        let mut hosts: HashMap<String, HostStats> = HashMap::new();
        for (host, r) in load_remembered(&path, unix_now()) {
            hosts.entry(host).or_default().rate_limit_hits = r.rate_limit_hits;
        }
        Self {
            hosts,
            store: Some(path),
        }
    }

    fn persist(&self, host: &str) {
        let Some(path) = &self.store else { return };
        let now = unix_now();
        let mut map = load_remembered(path, now);
        let hits = self.hosts.get(host).map_or(0, |s| s.rate_limit_hits);
        let entry = map.entry(host.to_string()).or_default();
        entry.rate_limit_hits = entry.rate_limit_hits.max(hits);
        entry.last_at = now;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension(format!("json.{}", std::process::id()));
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    pub fn record(&mut self, host: &str, concurrency: u32, throughput_bps: f64) {
        self.hosts
            .entry(host.to_string())
            .or_default()
            .record(concurrency, throughput_bps);
    }

    pub fn record_rate_limit(&mut self, host: &str) {
        self.hosts
            .entry(host.to_string())
            .or_default()
            .record_rate_limit();
        self.persist(host);
    }

    pub fn suggest(&self, host: &str, current: u32, ceiling: u32) -> u32 {
        let shared = per_download_ceiling(GLOBAL_CONNECTION_BUDGET, active_downloads());
        self.suggest_with_ceiling(host, current, ceiling.min(shared))
    }

    pub fn suggest_with_ceiling(&self, host: &str, current: u32, ceiling: u32) -> u32 {
        let teto = ceiling.clamp(MIN_N, MAX_N);
        match self.hosts.get(host) {
            Some(stats) => next_concurrency(stats, current).min(teto),
            None => current.min(teto),
        }
    }
}

#[cfg(test)]
impl ConcurrencyTuner {
    pub fn forget(&mut self, host: &str) {
        self.hosts.remove(host);
    }
}

pub fn global() -> &'static tokio::sync::Mutex<ConcurrencyTuner> {
    static TUNER: std::sync::OnceLock<tokio::sync::Mutex<ConcurrencyTuner>> =
        std::sync::OnceLock::new();
    TUNER.get_or_init(|| {
        let store = if cfg!(test) {
            None
        } else {
            crate::core::paths::app_data_dir().map(|d| d.join("tools").join("host-limits.json"))
        };
        tokio::sync::Mutex::new(
            store.map_or_else(ConcurrencyTuner::default, ConcurrencyTuner::with_store),
        )
    })
}
