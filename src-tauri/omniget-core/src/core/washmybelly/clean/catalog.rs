use std::sync::OnceLock;

use super::super::{Destination, Os, Risk};
use super::cleanerml::{self, Skip};
use super::rule::{Action, Requirement, Rule};

pub const SWEEPS: &[&str] = &[
    "macos.app_caches",
    "macos.container_caches",
    "linux.app_caches",
];

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
    let (sweeps, mut rest): (Vec<Rule>, Vec<Rule>) = rules
        .into_iter()
        .partition(|r| SWEEPS.contains(&r.id.as_str()));
    rest.extend(sweeps);
    Catalog {
        rules: rest,
        skipped: bb.skipped,
    }
}

pub fn rules_for(os: Os) -> Vec<&'static Rule> {
    catalog()
        .rules
        .iter()
        .filter(|r| r.os.contains(&os))
        .collect()
}

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

pub struct B(Rule);

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
