//! Report types of the guard. Everything here is serialised to the front as
//! camel-free snake_case JSON (the `guard.ts` types mirror it).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::skills::scan::ScanStatus;

/// How bad one finding is. Ordered: `Info < Low < Medium < High < Critical`.
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
    /// One or more steps down (never below `Info`).
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

    /// Points taken off the 0–100 score by the first finding of a code.
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

/// Which validator produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Validator {
    Structural,
    Integrity,
    Semantic,
    Reference,
    Provenance,
    /// Shell commands that will be executed (hooks, MCP launchers,
    /// statuslines, `!`cmd`` in commands, scripts, shell fences).
    Command,
    /// Hook / MCP / setting / statusline JSON semantics.
    Config,
    /// NVIDIA SkillSpector, when installed.
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

/// Overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Warn,
    Block,
}

/// One problem found in one file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Stable rule code, e.g. `SEM_E001`. See [`super::rules::RULES`].
    pub code: String,
    pub validator: Validator,
    pub severity: Severity,
    /// File path relative to the component root.
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// For JSON configs: where the offending value sits (`hooks.PreToolUse[0].hooks[1].command`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    /// The offending line or fragment, already redacted when it held a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// What exactly matched (dynamic part of the message, English).
    pub detail: String,
    /// True when the match was quoted, in a code block or negated, and the
    /// severity was lowered for it.
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

/// A command the component will execute, shown before anything is written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecCommand {
    pub file: String,
    /// `hook` | `mcp` | `statusline` | `setting` | `inline` (`!`cmd`` in a
    /// command) | `script` | `fence` (shell block in Markdown) | `agent_prompt`.
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Hook event or MCP server name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The exact command line (or prompt for `agent_prompt`).
    pub command: String,
    /// Programs the command runs, in order.
    pub programs: Vec<String>,
    /// Codes of the findings raised on this command.
    pub codes: Vec<String>,
    /// Worst severity of those findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worst: Option<Severity>,
    pub network: bool,
    /// Runs without the user asking (hooks, statuslines, `!`cmd``).
    pub auto_runs: bool,
}

/// Per-validator tally, in the original's style (`100 − 25·errors − 5·warnings`,
/// where errors = high/critical and warnings = medium/low).
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

/// Where a component came from, for the provenance validator.
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

/// Thresholds and switches. Defaults are what the installer uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardOptions {
    /// Score below this blocks (any critical finding blocks regardless).
    pub block_below: u8,
    /// Score below this warns (any high/medium finding warns regardless).
    pub warn_below: u8,
    /// Low findings also raise a warning; suspicious patterns count fully.
    pub strict: bool,
    /// Run SkillSpector on skills when it is installed.
    pub skillspector: bool,
    /// Expected sha256 per relative path (integrity validator).
    pub expected_sha256: Option<BTreeMap<String, String>>,
    pub provenance: Option<Provenance>,
    /// Project dir for resolving relative scripts in stats.
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

/// The result of scanning one component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardReport {
    /// Label: component name or path.
    pub label: String,
    /// Scanned root (file or dir) when scanned from disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub kind: String,
    /// The tool the caller says the format belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_tool: Option<String>,
    /// The tool the format looks native to (detected).
    pub detected_tool: String,
    /// Set when the component is written in another tool's format than the
    /// origin the caller claimed (e.g. Copilot tools in a Claude agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreign_format: Option<String>,
    pub files: Vec<FileDigest>,
    pub findings: Vec<Finding>,
    pub commands: Vec<ExecCommand>,
    pub validators: Vec<ValidatorSummary>,
    /// 0–100, **higher is safer** (the opposite of SkillSpector's number).
    pub score: u8,
    pub level: Level,
    pub counts: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skillspector: Option<ScanStatus>,
    pub block_below: u8,
    pub warn_below: u8,
    pub scanned_at: String,
}

/// Everything a validator run collects.
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
