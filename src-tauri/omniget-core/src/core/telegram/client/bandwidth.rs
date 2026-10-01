use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const DEFAULT_DAILY_QUOTA_GB: u64 = 250;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandwidthState {
    pub used_today_bytes: u64,
    pub total_used_bytes: u64,
    pub date: String,
    pub quota_bytes: u64,
}

impl Default for BandwidthState {
    fn default() -> Self {
        Self {
            used_today_bytes: 0,
            total_used_bytes: 0,
            date: today_date(),
            quota_bytes: DEFAULT_DAILY_QUOTA_GB * 1024 * 1024 * 1024,
        }
    }
}

fn today_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let (y, m, d) = unix_days_to_ymd(days as i64);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

pub fn unix_days_to_ymd_pub(days: i64) -> (i32, u32, u32) {
    unix_days_to_ymd(days)
}

fn unix_days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = (if m <= 2 { y + 1 } else { y }) as i32;
    (y, m, d)
}

fn state_path(plugin_data_dir: &Path) -> PathBuf {
    plugin_data_dir.join("bandwidth.json")
}

static FILE_LOCK: Mutex<()> = Mutex::new(());

pub fn load(plugin_data_dir: &Path) -> BandwidthState {
    let _g = FILE_LOCK.lock().ok();
    let path = state_path(plugin_data_dir);
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut s: BandwidthState = serde_json::from_str(&raw).unwrap_or_default();
    let today = today_date();
    if s.date != today {
        s.used_today_bytes = 0;
        s.date = today;
        let _ = save_inner(plugin_data_dir, &s);
    }
    s
}

fn save_inner(plugin_data_dir: &Path, s: &BandwidthState) -> anyhow::Result<()> {
    let path = state_path(plugin_data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(s)?)?;
    Ok(())
}

pub fn record_bytes(plugin_data_dir: &Path, n: u64) {
    if n == 0 {
        return;
    }
    let _g = FILE_LOCK.lock().ok();
    let mut s: BandwidthState = match std::fs::read_to_string(state_path(plugin_data_dir).as_path())
    {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => BandwidthState::default(),
    };
    let today = today_date();
    if s.date != today {
        s.used_today_bytes = 0;
        s.date = today;
    }
    s.used_today_bytes = s.used_today_bytes.saturating_add(n);
    s.total_used_bytes = s.total_used_bytes.saturating_add(n);
    let _ = save_inner(plugin_data_dir, &s);
}

pub fn set_quota_gb(plugin_data_dir: &Path, gb: u64) -> anyhow::Result<BandwidthState> {
    let _g = FILE_LOCK.lock().ok();
    let mut s: BandwidthState = match std::fs::read_to_string(state_path(plugin_data_dir).as_path())
    {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => BandwidthState::default(),
    };
    s.quota_bytes = gb.saturating_mul(1024 * 1024 * 1024);
    save_inner(plugin_data_dir, &s)?;
    Ok(s)
}

pub fn reset_today(plugin_data_dir: &Path) -> anyhow::Result<BandwidthState> {
    let _g = FILE_LOCK.lock().ok();
    let mut s: BandwidthState = match std::fs::read_to_string(state_path(plugin_data_dir).as_path())
    {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => BandwidthState::default(),
    };
    s.used_today_bytes = 0;
    s.date = today_date();
    save_inner(plugin_data_dir, &s)?;
    Ok(s)
}

pub fn would_exceed(plugin_data_dir: &Path, additional: u64) -> bool {
    let s = load(plugin_data_dir);
    if s.quota_bytes == 0 {
        return false;
    }
    s.used_today_bytes.saturating_add(additional) > s.quota_bytes
}
