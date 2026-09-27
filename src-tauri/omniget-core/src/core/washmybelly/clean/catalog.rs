//! Every cleanup rule of every source, in one list, built once per process.
//!
//! Order matters: the engine gives a path to the first rule that claims it,
//! so hand-written rules (ours, then per system) come before BleachBit, and
//! the broad "one item per app" sweeps come last.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{json, Value};

use super::super::{Destination, Os, Risk};
use super::cleanerml::{self, Skip, ALL_OS};
use super::rule::{Action, Requirement, Rule};

/// Rules that sweep a whole cache root, one item per child. They run last.
pub const SWEEPS: &[&str] = &[
    "macos.app_caches",
    "macos.container_caches",
    "linux.app_caches",
];

/// Hand-written rules that are offered but never ticked by default.
pub const OFF: &[&str] = &[
    "macos.saved_state",
    "macos.xcode_archives",
    "macos.xcode_device_support",
    "macos.simulator_runtimes",
    "macos.chrome_ai_models",
    "macos.ios_firmware",
    "macos.tm_snapshots",
    "ours.gradle",
    "ours.docker_prune",
    "ours.incomplete_downloads",
    "linux.flatpak_unused",
];

pub struct Catalog {
    pub rules: Vec<Rule>,
    pub skipped: Vec<Skip>,
}

pub fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(build)
}

fn build() -> Catalog {
    let bb = cleanerml::all();
    let mut rules = Vec::new();
    rules.extend(super::rules_ours::rules());
    rules.extend(super::rules_macos::rules());
    rules.extend(super::rules_linux::rules());
    rules.extend(super::rules_windows::rules());
    rules.extend(bb.rules);
    // Sweeps last, keeping their relative order.
    let (sweeps, mut rest): (Vec<Rule>, Vec<Rule>) = rules
        .into_iter()
        .partition(|r| SWEEPS.contains(&r.id.as_str()));
    rest.extend(sweeps);
    Catalog {
        rules: rest,
        skipped: bb.skipped,
    }
}

/// The rules for one system, in engine order.
pub fn rules_for(os: Os) -> Vec<&'static Rule> {
    catalog()
        .rules
        .iter()
        .filter(|r| r.os.contains(&os))
        .collect()
}

/// Whether an item of this rule starts ticked. Privacy, user files, the
/// Trash, database vacuums, admin work and rules with a warning start off.
pub fn default_selected(r: &Rule) -> bool {
    if r.risk >= Risk::Extreme || OFF.contains(&r.id.as_str()) || r.description.contains("Warning:")
    {
        return false;
    }
    if matches!(
        r.category.as_str(),
        "privacy" | "user_files" | "trash" | "database"
    ) {
        return false;
    }
    !r.requires.contains(&Requirement::Admin)
}

/// Counts per system and source, and what CleanerML left out and why.
pub fn stats() -> Value {
    let c = catalog();
    let mut per_os = serde_json::Map::new();
    for os in ALL_OS {
        let mut by_source: BTreeMap<&str, usize> = BTreeMap::new();
        for r in c.rules.iter().filter(|r| r.os.contains(&os)) {
            *by_source.entry(r.source.as_str()).or_default() += 1;
        }
        let total: usize = by_source.values().sum();
        per_os.insert(
            os.as_str().into(),
            json!({ "total": total, "by_source": by_source }),
        );
    }
    let mut skipped: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for s in &c.skipped {
        let e = skipped
            .entry(s.what.clone())
            .or_insert((0, s.reason.clone()));
        e.0 += 1;
    }
    json!({
        "rules": per_os,
        "cleanerml_files": cleanerml::EMBEDDED.len(),
        "cleanerml_skipped": skipped.into_iter().map(|(k, (n, why))| json!({ "what": k, "count": n, "reason": why })).collect::<Vec<_>>(),
        "protection_patterns": super::protection::pattern_count(),
    })
}

// ── Builder for hand-written rules ───────────────────────────────────

pub struct B(Rule);

/// A rule of ours: empties its targets, unlinks, destructive (a cache that
/// cannot come back but regenerates), GPL-3.0, label key from the id.
pub fn b(id: &str, os: &[Os], category: &str, label: &str) -> B {
    B(Rule {
        id: id.to_string(),
        os: os.to_vec(),
        category: category.to_string(),
        label_key: Some(format!("washmybelly.clean.rule.{}", id.replace('.', "_"))),
        label: label.to_string(),
        description: String::new(),
        targets: Vec::new(),
        include: Vec::new(),
        exclude: Vec::new(),
        running: Vec::new(),
        min_age_days: 0,
        action: Action::DeleteContents,
        destination: Destination::Unlink,
        risk: Risk::Destructive,
        requires: Vec::new(),
        source: "OmniGet".into(),
        license: "GPL-3.0".into(),
    })
}

impl B {
    pub fn t(mut self, targets: &[&str]) -> Self {
        self.0.targets.extend(targets.iter().map(|s| s.to_string()));
        self
    }
    /// Remove each match whole.
    pub fn delete(mut self) -> Self {
        self.0.action = Action::Delete;
        self
    }
    pub fn cmd(mut self, program: &str, args: &[&str], measure: &[&str]) -> Self {
        self.0.action = Action::Command {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            measure: measure.iter().map(|s| s.to_string()).collect(),
        };
        self.0.requires.push(Requirement::Program {
            name: program.into(),
        });
        self
    }
    pub fn vacuum(mut self) -> Self {
        self.0.action = Action::SqliteVacuum;
        self.0.risk = Risk::Reversible;
        self
    }
    pub fn running(mut self, names: &[&str]) -> Self {
        self.0.running.extend(names.iter().map(|s| s.to_string()));
        self
    }
    pub fn include(mut self, p: &[&str]) -> Self {
        self.0.include.extend(p.iter().map(|s| s.to_string()));
        self
    }
    pub fn exclude(mut self, p: &[&str]) -> Self {
        self.0.exclude.extend(p.iter().map(|s| s.to_string()));
        self
    }
    pub fn age(mut self, days: u32) -> Self {
        self.0.min_age_days = days;
        self
    }
    pub fn risk(mut self, r: Risk) -> Self {
        self.0.risk = r;
        self
    }
    pub fn dest(mut self, d: Destination) -> Self {
        self.0.destination = d;
        self
    }
    pub fn needs(mut self, r: Requirement) -> Self {
        self.0.requires.push(r);
        self
    }
    pub fn needs_path(self, path: &str) -> Self {
        self.needs(Requirement::PathExists { path: path.into() })
    }
    pub fn admin(self) -> Self {
        self.needs(Requirement::Admin)
    }
    pub fn fda(self) -> Self {
        self.needs(Requirement::FullDiskAccess)
    }
    pub fn from(mut self, source: &str, license: &str) -> Self {
        self.0.source = source.into();
        self.0.license = license.into();
        self
    }
    pub fn desc(mut self, d: &str) -> Self {
        self.0.description = d.into();
        self
    }
    /// Privacy data: separate item, quarantined, never ticked by default.
    pub fn privacy(mut self) -> Self {
        self.0.category = "privacy".into();
        self.0.destination = Destination::Quarantine;
        self.0.risk = Risk::Destructive;
        self
    }
    pub fn done(self) -> Rule {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_per_system() {
        for os in ALL_OS {
            let mut seen = std::collections::HashSet::new();
            for r in rules_for(os) {
                assert!(seen.insert(r.id.clone()), "duplicate id {} on {os:?}", r.id);
            }
        }
    }

    #[test]
    fn every_rule_is_well_formed() {
        for r in &catalog().rules {
            assert!(!r.os.is_empty(), "{}", r.id);
            assert!(!r.label.is_empty(), "{}", r.id);
            match &r.action {
                Action::Command { program, .. } => {
                    assert!(
                        super::super::super::plan::NATIVE_PROGRAMS.contains(&program.as_str()),
                        "{}: {program} not allowed",
                        r.id
                    )
                }
                _ => assert!(!r.targets.is_empty(), "{} has no targets", r.id),
            }
            // Browsing data and the Trash never start ticked.
            if r.category == "privacy" || r.category == "trash" {
                assert!(!default_selected(r), "{}", r.id);
            }
            // Never a whole cache root at once (the Stacer mistake).
            for t in &r.targets {
                let t = t.trim_end_matches('/');
                assert!(
                    !(matches!(r.action, Action::Delete)
                        && matches!(t, "~/.cache" | "$XDG_CACHE_HOME" | "~/Library/Caches")),
                    "{} deletes a whole cache root",
                    r.id
                );
            }
        }
    }

    #[test]
    fn sweeps_come_last() {
        for os in ALL_OS {
            let rules = rules_for(os);
            if let Some(first) = rules.iter().position(|r| SWEEPS.contains(&r.id.as_str())) {
                assert!(
                    rules[first..]
                        .iter()
                        .all(|r| SWEEPS.contains(&r.id.as_str())),
                    "{os:?}"
                );
            }
        }
    }

    /// Prints the catalog numbers and the English labels of our rules
    /// (for the handoff and `en.json`).
    #[test]
    #[ignore = "prints data for the handoff"]
    fn dump_stats_and_labels() {
        let mut labels = serde_json::Map::new();
        for r in &catalog().rules {
            if let Some(k) = &r.label_key {
                if let Some(leaf) = k.strip_prefix("washmybelly.clean.rule.") {
                    labels.insert(leaf.to_string(), json!(r.label));
                }
            }
        }
        println!("STATS {}", stats());
        println!("LABELS {}", Value::Object(labels));
    }

    #[test]
    fn stats_count_every_system() {
        let s = stats();
        for os in ["macos", "linux", "windows"] {
            assert!(s["rules"][os]["total"].as_u64().unwrap() > 0, "{os}");
        }
    }
}
