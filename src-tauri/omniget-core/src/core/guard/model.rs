use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::skills::scan::ScanStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn down(self, steps: u8) -> Severity {
        let mut s = self;
        for _ in 0..steps {
            s = match s {
                Severity::Critical => Severity::High,
                Severity::High => Severity::Medium,
                Severity::Medium => Severity::Low,
                _ => Severity::Info,
            };
        }
        s
    }

    pub fn weight(self) -> f64 {
        match self {
            Severity::Critical => 60.0,
            Severity::High => 20.0,
            Severity::Medium => 8.0,
            Severity::Low => 2.0,
            Severity::Info => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Validator {
    Structural,
    Integrity,
    Semantic,
    Reference,
    Provenance,
    Command,
    Config,
    Skillspector,
}

impl Validator {
    pub const ALL: [Validator; 8] = [
        Validator::Structural,
        Validator::Integrity,
        Validator::Semantic,
        Validator::Reference,
        Validator::Provenance,
        Validator::Command,
        Validator::Config,
        Validator::Skillspector,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Warn,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub validator: Validator,
    pub severity: Severity,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub detail: String,
    #[serde(default)]
    pub downgraded: bool,
}

impl Finding {
    pub fn new(
        code: &str,
        validator: Validator,
        severity: Severity,
        file: &str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code: code.to_string(),
            validator,
            severity,
            file: file.to_string(),
            line: None,
            column: None,
            json_path: None,
            snippet: None,
            detail: detail.into(),
            downgraded: false,
        }
    }

    pub fn at(mut self, line: u32, column: u32) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    pub fn line(mut self, line: Option<u32>) -> Self {
        self.line = line;
        self
    }

    pub fn snippet(mut self, s: impl Into<String>) -> Self {
        let s = s.into();
        self.snippet = (!s.trim().is_empty()).then(|| super::text::clip(&s, 240));
        self
    }

    pub fn path(mut self, p: impl Into<String>) -> Self {
        self.json_path = Some(p.into());
        self
    }

    pub fn lowered(mut self, steps: u8) -> Self {
        if steps > 0 {
            self.severity = self.severity.down(steps);
            self.downgraded = true;
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecCommand {
    pub file: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub command: String,
    pub programs: Vec<String>,
    pub codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worst: Option<Severity>,
    pub network: bool,
    pub auto_runs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSummary {
    pub validator: Validator,
    pub score: u8,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDigest {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Provenance {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardOptions {
    pub block_below: u8,
    pub warn_below: u8,
    pub strict: bool,
    pub skillspector: bool,
    pub expected_sha256: Option<BTreeMap<String, String>>,
    pub provenance: Option<Provenance>,
    pub project_dir: Option<String>,
}

impl Default for GuardOptions {
    fn default() -> Self {
        Self {
            block_below: 20,
            warn_below: 80,
            strict: false,
            skillspector: true,
            expected_sha256: None,
            provenance: None,
            project_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardReport {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_tool: Option<String>,
    pub detected_tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreign_format: Option<String>,
    pub files: Vec<FileDigest>,
    pub findings: Vec<Finding>,
    pub commands: Vec<ExecCommand>,
    pub validators: Vec<ValidatorSummary>,
    pub score: u8,
    pub level: Level,
    pub counts: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skillspector: Option<ScanStatus>,
    pub block_below: u8,
    pub warn_below: u8,
    pub scanned_at: String,
}

#[derive(Default)]
pub struct Sink {
    pub findings: Vec<Finding>,
    pub commands: Vec<ExecCommand>,
}

impl Sink {
    pub fn push(&mut self, f: Finding) {
        self.findings.push(f);
    }
}
