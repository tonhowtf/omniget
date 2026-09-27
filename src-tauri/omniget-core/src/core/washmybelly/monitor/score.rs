//! The health score (0-100) and what lowers it. Pure: the caller gathers
//! the readings, this only weighs them, so every threshold is tested.
//!
//! A reading that is missing costs nothing: the score never guesses. Other
//! modules add their findings through `score_inputs` in the washmybelly
//! store (see [`ScoreInputs`]).

use serde::{Deserialize, Serialize};

/// Findings other modules leave for the score, in the store under
/// `score_inputs` (read, change your field, write back). Missing means the
/// module has not looked yet, and costs nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ScoreInputs {
    /// Bytes the last junk scan found (module `clean`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub junk_bytes: Option<u64>,
    /// Login and startup items that are on (module `startup`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_items: Option<u32>,
    /// Privacy settings the person has not decided on (module `privacy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_pending: Option<u32>,
    /// When each field was written (unix seconds), by field name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<std::collections::BTreeMap<String, i64>>,
}

/// Everything the score weighs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Readings {
    pub disk_total: Option<u64>,
    pub disk_free: Option<u64>,
    pub cpu_temp_c: Option<f64>,
    pub battery_health_percent: Option<f64>,
    pub battery_cycles: Option<i64>,
    /// 0-100, higher is worse (macOS memory pressure, Linux PSI, else used %).
    pub memory_pressure: Option<f64>,
    pub swap_used: Option<u64>,
    pub inputs: ScoreInputs,
}

/// One thing the score looked at.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Factor {
    /// `disk_space`, `temperature`, `battery`, `memory`, `junk`,
    /// `login_items`, `privacy`.
    pub id: &'static str,
    /// Points taken from 100.
    pub penalty: u32,
    /// `ok`, `warn` or `bad`.
    pub severity: &'static str,
    /// The reading behind it, in the unit of the factor.
    pub value: f64,
    /// One sentence for the person (English; the screen translates by id).
    pub detail: String,
}

/// A gain the person can take now ("12.4 GB of junk", "battery at 87%").
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Gain {
    pub id: &'static str,
    /// The module that acts on it.
    pub module: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    pub detail: String,
}

const GB: f64 = 1_000_000_000.0;

fn sev(p: u32, bad_at: u32) -> &'static str {
    if p == 0 {
        "ok"
    } else if p >= bad_at {
        "bad"
    } else {
        "warn"
    }
}

fn gb(b: f64) -> String {
    format!("{:.1} GB", b / GB)
}

/// Score and the factors behind it (every factor that could be read, the
/// ones that cost nothing included, worst first).
pub fn score(r: &Readings) -> (u32, Vec<Factor>) {
    let mut f: Vec<Factor> = Vec::new();

    if let (Some(total), Some(free)) = (r.disk_total, r.disk_free) {
        if total > 0 {
            let pct = free as f64 / total as f64 * 100.0;
            let fb = free as f64;
            let mut p: u32 = if pct < 5.0 || fb < 5.0 * GB {
                30
            } else if pct < 10.0 || fb < 10.0 * GB {
                20
            } else if pct < 15.0 {
                10
            } else if pct < 20.0 {
                5
            } else {
                0
            };
            if fb < 20.0 * GB {
                p = p.max(10);
            }
            f.push(Factor {
                id: "disk_space",
                penalty: p,
                severity: sev(p, 20),
                value: (pct * 10.0).round() / 10.0,
                detail: format!("{} free on the system disk ({:.0}%)", gb(fb), pct),
            });
        }
    }

    if let Some(t) = r.cpu_temp_c {
        let p = if t >= 95.0 {
            15
        } else if t >= 85.0 {
            8
        } else {
            0
        };
        f.push(Factor {
            id: "temperature",
            penalty: p,
            severity: sev(p, 15),
            value: t,
            detail: format!("Processor at {t:.0} °C"),
        });
    }

    if r.battery_health_percent.is_some() || r.battery_cycles.is_some() {
        let h = r.battery_health_percent.unwrap_or(100.0);
        let mut p = if h < 60.0 {
            15
        } else if h < 80.0 {
            8
        } else {
            0
        };
        if r.battery_cycles.unwrap_or(0) >= 1000 {
            p += 3;
        }
        let detail = match (r.battery_health_percent, r.battery_cycles) {
            (Some(h), Some(c)) => {
                format!("Battery holds {h:.0}% of its original charge after {c} cycles")
            }
            (Some(h), None) => format!("Battery holds {h:.0}% of its original charge"),
            (None, Some(c)) => format!("Battery has {c} charge cycles"),
            _ => String::new(),
        };
        f.push(Factor {
            id: "battery",
            penalty: p,
            severity: sev(p, 15),
            value: h,
            detail,
        });
    }

    if let Some(m) = r.memory_pressure {
        let mut p = if m >= 90.0 {
            12
        } else if m >= 75.0 {
            6
        } else {
            0
        };
        // Swap alone is normal (macOS keeps GBs there); it only adds when
        // memory is already under pressure.
        if p > 0 && r.swap_used.unwrap_or(0) as f64 > 4.0 * GB {
            p += 3;
        }
        f.push(Factor {
            id: "memory",
            penalty: p,
            severity: sev(p, 12),
            value: m,
            detail: match r.swap_used {
                Some(s) if s > 0 => format!("Memory pressure {m:.0}%, {} in swap", gb(s as f64)),
                _ => format!("Memory pressure {m:.0}%"),
            },
        });
    }

    if let Some(j) = r.inputs.junk_bytes {
        let jb = j as f64;
        let p = if jb >= 20.0 * GB {
            10
        } else if jb >= 5.0 * GB {
            5
        } else if jb >= GB {
            2
        } else {
            0
        };
        f.push(Factor {
            id: "junk",
            penalty: p,
            severity: sev(p, 10),
            value: jb,
            detail: format!("{} of junk found", gb(jb)),
        });
    }

    if let Some(n) = r.inputs.login_items {
        let p = if n > 10 {
            8
        } else if n > 5 {
            4
        } else {
            0
        };
        f.push(Factor {
            id: "login_items",
            penalty: p,
            severity: sev(p, 8),
            value: n as f64,
            detail: format!("{n} items open at login"),
        });
    }

    if let Some(n) = r.inputs.privacy_pending {
        let p = n.min(5);
        f.push(Factor {
            id: "privacy",
            penalty: p,
            severity: sev(p, 5),
            value: n as f64,
            detail: format!("{n} privacy settings to review"),
        });
    }

    let lost: u32 = f.iter().map(|x| x.penalty).sum();
    f.sort_by(|a, b| b.penalty.cmp(&a.penalty));
    (100u32.saturating_sub(lost), f)
}

/// `excellent` ≥ 85, `good` ≥ 65, `fair` ≥ 45, else `poor`.
pub fn grade(score: u32) -> &'static str {
    match score {
        85.. => "excellent",
        65..=84 => "good",
        45..=64 => "fair",
        _ => "poor",
    }
}

/// What the person can win now, for the cards of the overview.
pub fn gains(r: &Readings) -> Vec<Gain> {
    let mut g = Vec::new();
    if let Some(j) = r.inputs.junk_bytes.filter(|j| *j >= 100_000_000) {
        g.push(Gain {
            id: "junk",
            module: "clean",
            bytes: Some(j),
            count: None,
            percent: None,
            detail: format!("{} of junk can go", gb(j as f64)),
        });
    }
    if let (Some(total), Some(free)) = (r.disk_total, r.disk_free) {
        if total > 0 && (free as f64) / (total as f64) < 0.2 {
            g.push(Gain {
                id: "free_space",
                module: "disk",
                bytes: Some(free),
                count: None,
                percent: Some(((free as f64 / total as f64) * 1000.0).round() / 10.0),
                detail: format!("Only {} free: see what takes the space", gb(free as f64)),
            });
        }
    }
    if let Some(n) = r.inputs.login_items.filter(|n| *n > 5) {
        g.push(Gain {
            id: "login_items",
            module: "startup",
            bytes: None,
            count: Some(n),
            percent: None,
            detail: format!("{n} apps open at login"),
        });
    }
    if let Some(h) = r.battery_health_percent.filter(|h| *h < 90.0) {
        g.push(Gain {
            id: "battery",
            module: "power",
            bytes: None,
            count: None,
            percent: Some(h),
            detail: format!("Battery at {h:.0}% of its original capacity"),
        });
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Readings {
        Readings {
            disk_total: Some(500 * GB as u64),
            disk_free: Some(200 * GB as u64),
            cpu_temp_c: Some(55.0),
            battery_health_percent: Some(98.0),
            battery_cycles: Some(17),
            memory_pressure: Some(30.0),
            swap_used: Some(0),
            inputs: ScoreInputs::default(),
        }
    }

    #[test]
    fn healthy_machine_scores_100() {
        let (s, f) = score(&base());
        assert_eq!(s, 100);
        assert_eq!(f.len(), 4);
        assert!(f.iter().all(|x| x.severity == "ok"));
        assert_eq!(grade(s), "excellent");
    }

    #[test]
    fn nothing_known_costs_nothing() {
        let (s, f) = score(&Readings::default());
        assert_eq!(s, 100);
        assert!(f.is_empty());
    }

    #[test]
    fn full_disk_hot_cpu_worn_battery() {
        let r = Readings {
            disk_free: Some(8 * GB as u64),
            cpu_temp_c: Some(96.0),
            battery_health_percent: Some(58.0),
            battery_cycles: Some(1100),
            memory_pressure: Some(92.0),
            swap_used: Some(6 * GB as u64),
            ..base()
        };
        let (s, f) = score(&r);
        // disk 30 (1.6% free), temp 15, battery 18, memory 15.
        assert_eq!(s, 100 - 30 - 15 - 18 - 15);
        assert_eq!(f[0].id, "disk_space");
        assert_eq!(f[0].severity, "bad");
        assert_eq!(grade(s), "poor");
    }

    #[test]
    fn small_disk_still_needs_20_gb() {
        // 30% free of a 50 GB disk is only 15 GB.
        let r = Readings {
            disk_total: Some(50 * GB as u64),
            disk_free: Some(15 * GB as u64),
            ..base()
        };
        let (s, f) = score(&r);
        assert_eq!(s, 90);
        assert_eq!(f[0].id, "disk_space");
        assert_eq!(f[0].severity, "warn");
    }

    #[test]
    fn inputs_from_other_modules() {
        let r = Readings {
            inputs: ScoreInputs {
                junk_bytes: Some(12_400_000_000),
                login_items: Some(7),
                privacy_pending: Some(9),
                updated: None,
            },
            ..base()
        };
        let (s, _) = score(&r);
        assert_eq!(s, 100 - 5 - 4 - 5);
        let g = gains(&r);
        assert_eq!(
            g.iter().map(|x| x.id).collect::<Vec<_>>(),
            vec!["junk", "login_items"]
        );
        assert_eq!(g[0].detail, "12.4 GB of junk can go");
    }

    #[test]
    fn gains_for_space_and_battery() {
        let r = Readings {
            disk_free: Some(40 * GB as u64),
            battery_health_percent: Some(87.0),
            ..base()
        };
        let g = gains(&r);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].id, "free_space");
        assert_eq!(g[0].percent, Some(8.0));
        assert_eq!(g[1].id, "battery");
    }

    #[test]
    fn score_inputs_round_trip() {
        let v: ScoreInputs = serde_json::from_str(r#"{"junk_bytes":5}"#).unwrap();
        assert_eq!(v.junk_bytes, Some(5));
        assert_eq!(v.login_items, None);
    }
}
