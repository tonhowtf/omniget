//! Linux: "one power manager only". power-profiles-daemon, TLP,
//! auto-cpufreq and TuneD all write the same sysfs knobs; two of them
//! running fight every few seconds and the last writer wins. This module
//! decides, from what is running, who manages the profile and which
//! conflict to show. Pure: the Linux reader feeds it.

use serde::Serialize;

/// A manager we know. `TlpPd` and `TunedPpd` are the D-Bus front ends of
/// TLP and TuneD (same API as power-profiles-daemon): same family.
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

    /// The family that owns the knobs.
    pub fn family(self) -> Family {
        match self {
            Manager::PowerProfilesDaemon => Family::Ppd,
            Manager::TlpPd | Manager::Tlp => Family::Tlp,
            Manager::TunedPpd | Manager::Tuned => Family::Tuned,
            Manager::AutoCpufreq => Family::AutoCpufreq,
        }
    }

    /// Speaks `org.freedesktop.UPower.PowerProfiles` (profile switch and
    /// `HoldProfile` without a password).
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
    /// Installed on purpose: it wins a conflict against the distro default.
    AutoCpufreq,
    Tlp,
    Tuned,
    /// The GNOME/KDE default.
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

/// Two families running at once, and the one action we offer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    pub a: &'static str,
    pub b: &'static str,
    /// The family we suggest keeping.
    pub keep: &'static str,
    /// Units to stop, disable and mask (reversible: unmask + enable).
    pub disable: Vec<&'static str>,
    /// `resolve_conflict:<keep>`: a plan item id suffix.
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    /// The manager in charge, when exactly one family runs.
    pub manager: Option<&'static str>,
    pub families: Vec<&'static str>,
    pub conflicts: Vec<Conflict>,
    /// Nothing runs: text only, we never install anything by ourselves.
    pub suggestion: Option<&'static str>,
}

/// Decide from the running managers (units active, or processes found).
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
            // Prefer naming the front end the person sees (tlp-pd over tlp).
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
            // `fams` is sorted by priority: the first is kept.
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

/// Units to turn off to keep only `keep` running.
pub fn units_to_disable(running: &[Manager], keep: &str) -> Vec<&'static str> {
    running
        .iter()
        .filter(|m| m.family().id() != keep)
        .map(|m| m.unit())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Manager::*;

    #[test]
    fn table_of_cases() {
        // (running, manager, conflict count, kept family, suggestion)
        type Case<'a> = (
            &'a [Manager],
            Option<&'a str>,
            usize,
            Option<&'a str>,
            Option<&'a str>,
        );
        let cases: &[Case<'_>] = &[
            (&[], None, 0, None, Some("power-profiles-daemon")),
            (
                &[PowerProfilesDaemon],
                Some("power-profiles-daemon"),
                0,
                None,
                None,
            ),
            (&[Tlp], Some("tlp"), 0, None, None),
            (&[Tlp, TlpPd], Some("tlp-pd"), 0, None, None),
            (&[Tuned, TunedPpd], Some("tuned-ppd"), 0, None, None),
            (&[AutoCpufreq], Some("auto-cpufreq"), 0, None, None),
            (&[Tlp, PowerProfilesDaemon], None, 1, Some("tlp"), None),
            (
                &[AutoCpufreq, PowerProfilesDaemon],
                None,
                1,
                Some("auto-cpufreq"),
                None,
            ),
            (&[AutoCpufreq, Tlp], None, 1, Some("auto-cpufreq"), None),
            (&[Tuned, PowerProfilesDaemon], None, 1, Some("tuned"), None),
            (
                &[AutoCpufreq, Tlp, PowerProfilesDaemon],
                None,
                3,
                Some("auto-cpufreq"),
                None,
            ),
        ];
        for (running, manager, n, keep, suggestion) in cases {
            let v = arbitrate(running);
            assert_eq!(v.manager, *manager, "{running:?}");
            assert_eq!(v.conflicts.len(), *n, "{running:?}");
            assert_eq!(v.suggestion, *suggestion, "{running:?}");
            if let Some(k) = keep {
                assert!(
                    v.conflicts
                        .iter()
                        .all(|c| c.action == format!("resolve_conflict:{k}")),
                    "{running:?}"
                );
                assert!(
                    v.conflicts.iter().all(|c| !c.disable.is_empty()),
                    "{running:?}"
                );
            }
        }
    }

    #[test]
    fn disabling_keeps_the_whole_family() {
        let running = [Tlp, TlpPd, PowerProfilesDaemon];
        assert_eq!(
            units_to_disable(&running, "tlp"),
            ["power-profiles-daemon.service"]
        );
        assert_eq!(
            units_to_disable(&running, "power-profiles-daemon"),
            ["tlp.service", "tlp-pd.service"]
        );
        let v = arbitrate(&running);
        assert_eq!(v.conflicts[0].disable, ["power-profiles-daemon.service"]);
        assert_eq!(v.conflicts[0].keep, "tlp");
    }

    #[test]
    fn units_round_trip() {
        for m in Manager::ALL {
            assert_eq!(Manager::from_unit(m.unit()), Some(m));
        }
        assert_eq!(Manager::from_unit("sshd.service"), None);
    }
}
