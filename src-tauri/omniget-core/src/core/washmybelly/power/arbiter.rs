use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Manager {
    PowerProfilesDaemon,
    TlpPd,
    TunedPpd,
    Tlp,
    AutoCpufreq,
    Tuned,
}

impl Manager {
    pub fn id(self) -> &'static str {
        match self {
            Manager::PowerProfilesDaemon => "power-profiles-daemon",
            Manager::TlpPd => "tlp-pd",
            Manager::TunedPpd => "tuned-ppd",
            Manager::Tlp => "tlp",
            Manager::AutoCpufreq => "auto-cpufreq",
            Manager::Tuned => "tuned",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Manager::PowerProfilesDaemon => "power-profiles-daemon.service",
            Manager::TlpPd => "tlp-pd.service",
            Manager::TunedPpd => "tuned-ppd.service",
            Manager::Tlp => "tlp.service",
            Manager::AutoCpufreq => "auto-cpufreq.service",
            Manager::Tuned => "tuned.service",
        }
    }

    pub const ALL: [Manager; 6] = [
        Manager::PowerProfilesDaemon,
        Manager::TlpPd,
        Manager::TunedPpd,
        Manager::Tlp,
        Manager::AutoCpufreq,
        Manager::Tuned,
    ];

    pub fn from_unit(unit: &str) -> Option<Manager> {
        Manager::ALL.into_iter().find(|m| m.unit() == unit)
    }

    pub fn family(self) -> Family {
        match self {
            Manager::PowerProfilesDaemon => Family::Ppd,
            Manager::TlpPd | Manager::Tlp => Family::Tlp,
            Manager::TunedPpd | Manager::Tuned => Family::Tuned,
            Manager::AutoCpufreq => Family::AutoCpufreq,
        }
    }

    pub fn has_profiles_api(self) -> bool {
        matches!(
            self,
            Manager::PowerProfilesDaemon | Manager::TlpPd | Manager::TunedPpd
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    AutoCpufreq,
    Tlp,
    Tuned,
    Ppd,
}

impl Family {
    pub fn id(self) -> &'static str {
        match self {
            Family::AutoCpufreq => "auto-cpufreq",
            Family::Tlp => "tlp",
            Family::Tuned => "tuned",
            Family::Ppd => "power-profiles-daemon",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    pub a: &'static str,
    pub b: &'static str,
    pub keep: &'static str,
    pub disable: Vec<&'static str>,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    pub manager: Option<&'static str>,
    pub families: Vec<&'static str>,
    pub conflicts: Vec<Conflict>,
    pub suggestion: Option<&'static str>,
}

pub fn arbitrate(running: &[Manager]) -> Verdict {
    let mut fams: Vec<Family> = running.iter().map(|m| m.family()).collect();
    fams.sort();
    fams.dedup();
    let families = fams.iter().map(|f| f.id()).collect();
    match fams.len() {
        0 => Verdict {
            manager: None,
            families,
            conflicts: vec![],
            suggestion: Some("power-profiles-daemon"),
        },
        1 => {
            let fam = fams[0];
            let m = running
                .iter()
                .filter(|m| m.family() == fam)
                .min_by_key(|m| !m.has_profiles_api())
                .copied()
                .unwrap();
            Verdict {
                manager: Some(m.id()),
                families,
                conflicts: vec![],
                suggestion: None,
            }
        }
        _ => {
            let keep = fams[0];
            let mut conflicts = Vec::new();
            for (i, a) in fams.iter().enumerate() {
                for b in &fams[i + 1..] {
                    let disable = running
                        .iter()
                        .filter(|m| m.family() != keep && (m.family() == *a || m.family() == *b))
                        .map(|m| m.unit())
                        .collect();
                    conflicts.push(Conflict {
                        a: a.id(),
                        b: b.id(),
                        keep: keep.id(),
                        disable,
                        action: format!("resolve_conflict:{}", keep.id()),
                    });
                }
            }
            Verdict {
                manager: None,
                families,
                conflicts,
                suggestion: None,
            }
        }
    }
}

pub fn units_to_disable(running: &[Manager], keep: &str) -> Vec<&'static str> {
    running
        .iter()
        .filter(|m| m.family().id() != keep)
        .map(|m| m.unit())
        .collect()
}
