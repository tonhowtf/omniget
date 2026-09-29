//! Scanning a staged skill with NVIDIA **SkillSpector** before it is installed.
//!
//! Source of the contract (read 18/09/2026, `git clone --depth 1
//! https://github.com/NVIDIA/SkillSpector`):
//!
//! - `src/skillspector/cli.py::scan` — the subcommand is
//!   `skillspector scan <path>`, with `--no-llm` (static analysis only) and
//!   `--format json`. With no `--output` the report goes to **stdout**; every
//!   warning and every error line goes to stderr for the machine-readable
//!   formats, so stdout is the report and nothing else.
//! - `src/skillspector/nodes/report.py::_format_json` — the exact JSON shape
//!   ported into [`ScanReport`] below.
//! - `src/skillspector/constants.py` — `RISK_THRESHOLD = 50`. **A high score
//!   means high risk**: `cli.py` exits `1` when `risk_score > RISK_THRESHOLD`,
//!   and `report.py::_compute_risk_score` bands `0..=20 LOW`, `21..=50 MEDIUM`,
//!   `51..=80 HIGH`, `81..=100 CRITICAL`, mapping `LOW → SAFE`,
//!   `MEDIUM → CAUTION`, `HIGH`/`CRITICAL → DO_NOT_INSTALL`. We port the
//!   threshold as [`RISK_THRESHOLD`] instead of inventing one.
//! - Exit codes (README "Exit codes"): `0` clean, `1` **scan completed** and the
//!   score is over the threshold (or a `--fail-on-*` gate fired), `2` a real
//!   error. So a non-zero exit with a valid report is a *verdict*, not a
//!   failure, and this module keys off the parsed report, not off the code.
//!
//! Nothing of SkillSpector's Python is embedded: we only look for its binary on
//! the `PATH` and run it. That keeps the dependency optional, which is the
//! whole point of [`ScanStatus::NotScanned`].
//!
//! **Fail-open, by design.** The scanner is advice, not a gate we can trust to
//! be there: no binary is [`ScanStatus::NotScanned`], and a crash, a timeout or
//! unparseable output is [`ScanStatus::Failed`]. In both cases the install goes
//! through and the UI says so. Only a *successful* scan with a score over the
//! threshold holds the skill back for a second click — see
//! [`super::install::InstallOutcome`].

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The program we look for on the `PATH`.
pub const SCANNER_BIN: &str = "skillspector";

/// Ported from `skillspector/constants.py`: "Risk score threshold above which a
/// scan is treated as unsafe." The comparison there is `score > threshold`.
pub const RISK_THRESHOLD: u8 = 50;

/// How long one static scan may take before we kill it. A `--no-llm` scan of a
/// skill that fits our install budget is seconds; this is the runaway guard.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

/// Ceiling on the report we are willing to read. The scanner is a third-party
/// binary reading attacker-controlled files; its stdout is not allowed to eat
/// the heap. A report past this is treated as a failed scan.
pub const MAX_STDOUT_BYTES: u64 = 4 * 1024 * 1024;

/// Ceiling on the stderr tail we keep for the failure message.
pub const MAX_STDERR_BYTES: u64 = 64 * 1024;

/// How many issues travel to the UI. The report can carry hundreds; the card
/// shows the worst few and the count.
pub const MAX_ISSUES_KEPT: usize = 12;

/// How often we poll the child while waiting for the deadline.
const POLL: Duration = Duration::from_millis(25);

// ---------------------------------------------------------------- verdict

/// What we know about a skill's security scan. Persisted in the install
/// sidecar and carried on [`super::manifest::SkillManifest`], so the UI can
/// badge a skill it did not just install.
///
/// Serialised internally tagged (`{"status":"scanned", …}`) to match the other
/// skill enums.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScanStatus {
    /// No scanner on this machine, so nothing was checked. Never claim a skill
    /// is safe from this.
    #[default]
    NotScanned,
    /// The scanner ran and produced a report.
    Scanned {
        /// `risk_assessment.score`, 0–100. **Higher is worse.**
        score: u8,
        /// `LOW | MEDIUM | HIGH | CRITICAL`.
        severity: String,
        /// `SAFE | CAUTION | DO_NOT_INSTALL`.
        recommendation: String,
        /// Worst severity present in `issues[]`, which can be higher than the
        /// confidence-weighted `severity` band above.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_issue_severity: Option<String>,
        /// How many active issues the report carried (before our cut).
        #[serde(default)]
        findings: usize,
        /// The worst [`MAX_ISSUES_KEPT`] of them.
        #[serde(default)]
        issues: Vec<ScanIssue>,
        /// `metadata.skillspector_version`, when the report carried one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scanner_version: Option<String>,
        /// When we ran it, RFC 3339.
        #[serde(default)]
        scanned_at: String,
        /// True when the report says LLM analysis actually ran. We always pass
        /// `--no-llm`, so this is false: the scan is static analysis only and
        /// the UI must not oversell it.
        #[serde(default)]
        llm: bool,
    },
    /// The scanner exists but we got no usable verdict out of it: it crashed,
    /// timed out, wrote something that is not a report, or reported its own
    /// execution as unsuccessful. The install still goes through.
    Failed { reason: String },
}

impl ScanStatus {
    /// `risk_assessment.score`, when there is one.
    pub fn score(&self) -> Option<u8> {
        match self {
            ScanStatus::Scanned { score, .. } => Some(*score),
            _ => None,
        }
    }

    /// True when a *successful* scan came back over SkillSpector's own
    /// threshold, or with its `DO_NOT_INSTALL` recommendation. This is the only
    /// state that holds an install back: `NotScanned` and `Failed` never do.
    pub fn is_high_risk(&self) -> bool {
        match self {
            ScanStatus::Scanned {
                score,
                recommendation,
                ..
            } => *score > RISK_THRESHOLD || recommendation.eq_ignore_ascii_case("DO_NOT_INSTALL"),
            _ => false,
        }
    }
}

/// One issue, cut down to what a card can show. Field names follow
/// `Finding.to_dict()` in `skillspector/models.py`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScanIssue {
    /// `id` there: the rule id, e.g. `SC8`.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// `explanation` there, which falls back to the finding's message.
    #[serde(default)]
    pub message: String,
    /// `location.file`, plus `location.start_line` when there is one.
    #[serde(default)]
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
}

// ------------------------------------------------------- the report on the wire

/// `_format_json` in `skillspector/nodes/report.py`. Every field is
/// `#[serde(default)]`: the README says to "treat any additional fields as
/// best-effort", and a newer scanner dropping or adding one must not turn a
/// good scan into a failure.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScanReport {
    #[serde(default)]
    pub skill: ReportSkill,
    #[serde(default)]
    pub risk_assessment: RiskAssessment,
    #[serde(default)]
    pub issues: Vec<ReportIssue>,
    #[serde(default)]
    pub suppressed_count: u64,
    #[serde(default)]
    pub metadata: ReportMetadata,
    /// The scanner's own "did this run to completion". `cli.py` exits `2` when
    /// it is false. Absent means true, like every other consumer assumes.
    #[serde(default = "yes")]
    pub execution_successful: bool,
    #[serde(default)]
    pub analysis_completeness: Completeness,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReportSkill {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub scanned_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RiskAssessment {
    /// Signed and wide on purpose: we clamp rather than refuse a report whose
    /// score does not fit `u8`.
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub recommendation: String,
    #[serde(default)]
    pub max_issue_severity: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReportIssue {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(default)]
    pub remediation: Option<String>,
    #[serde(default)]
    pub location: IssueLocation,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IssueLocation {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub start_line: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReportMetadata {
    #[serde(default)]
    pub has_executable_scripts: bool,
    #[serde(default)]
    pub skillspector_version: Option<String>,
    #[serde(default)]
    pub llm_requested: bool,
    #[serde(default)]
    pub llm_available: bool,
    #[serde(default)]
    pub llm_error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Completeness {
    #[serde(default = "yes")]
    pub is_complete: bool,
}

/// Rank used to pick the issues the card shows. Same order as `_SEVERITY_RANK`
/// in `report.py`, which that file keeps deliberately stable for consumers.
fn severity_rank(severity: &str) -> u8 {
    match severity.to_ascii_uppercase().as_str() {
        "CRITICAL" => 4,
        "HIGH" => 3,
        "MEDIUM" => 2,
        "LOW" => 1,
        _ => 0,
    }
}

impl ScanReport {
    /// Turn a parsed report into the verdict we persist.
    pub fn into_status(self) -> ScanStatus {
        if !self.execution_successful {
            return ScanStatus::Failed {
                reason: "the scanner reported its own run as unsuccessful".into(),
            };
        }
        let mut issues = self.issues;
        issues.sort_by_key(|i| std::cmp::Reverse(severity_rank(&i.severity)));
        let findings = issues.len();
        let kept = issues
            .into_iter()
            .take(MAX_ISSUES_KEPT)
            .map(|i| ScanIssue {
                id: i.id,
                severity: i.severity.to_ascii_uppercase(),
                category: i.category,
                message: i.explanation.unwrap_or_default(),
                file: i.location.file,
                line: i.location.start_line,
            })
            .collect();
        ScanStatus::Scanned {
            score: self.risk_assessment.score.clamp(0, 100) as u8,
            severity: upper_or(&self.risk_assessment.severity, "UNKNOWN"),
            recommendation: upper_or(&self.risk_assessment.recommendation, "CAUTION"),
            max_issue_severity: self
                .risk_assessment
                .max_issue_severity
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_uppercase()),
            findings,
            issues: kept,
            scanner_version: self.metadata.skillspector_version.filter(|s| !s.is_empty()),
            // The report's own timestamp when it has one, ours otherwise.
            scanned_at: if self.skill.scanned_at.is_empty() {
                chrono::Utc::now().to_rfc3339()
            } else {
                self.skill.scanned_at
            },
            llm: self.metadata.llm_requested && self.metadata.llm_available,
        }
    }
}

fn upper_or(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_ascii_uppercase()
    }
}

// ---------------------------------------------------------------- finding it

/// The scanner's path, looked up in `path_var` exactly as a shell would. Pure
/// apart from the `is_file` probe, so a test can hand it a `PATH` it built.
///
/// `None` is the normal case on a machine without SkillSpector and must never
/// read as an error.
pub fn find_in_path(path_var: Option<&OsStr>, program: &str) -> Option<PathBuf> {
    // A name with a separator in it is a path, not something to look up.
    if program.contains('/') || program.contains('\\') {
        let direct = PathBuf::from(program);
        return is_runnable(&direct).then_some(direct);
    }
    let path_var = path_var?;
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for candidate in candidates(program) {
            let full = dir.join(&candidate);
            if is_runnable(&full) {
                return Some(full);
            }
        }
    }
    None
}

/// The file names one program name can have. Windows needs the extensions; a
/// pip/uv install puts `skillspector.exe` there.
fn candidates(program: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let mut out = vec![program.to_string()];
        for ext in ["exe", "cmd", "bat"] {
            out.push(format!("{program}.{ext}"));
        }
        out
    }
    #[cfg(not(windows))]
    {
        vec![program.to_string()]
    }
}

#[cfg(unix)]
fn is_runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_runnable(path: &Path) -> bool {
    path.is_file()
}

/// The scanner on this machine's `PATH`, or `None`.
pub fn scanner_path() -> Option<PathBuf> {
    // An explicit override wins, so a user with the tool outside the PATH (or a
    // packaged build) can point at it without us guessing.
    if let Some(explicit) = std::env::var_os("OMNIGET_SKILL_SCANNER") {
        let path = PathBuf::from(&explicit);
        return is_runnable(&path).then_some(path);
    }
    find_in_path(std::env::var_os("PATH").as_deref(), SCANNER_BIN)
}

/// True when a scan will actually run. The UI asks so it can explain why a
/// skill shows "not scanned".
pub fn is_available() -> bool {
    scanner_path().is_some()
}

// ---------------------------------------------------------------- running it

/// Scan one staged skill directory with whatever scanner this machine has.
/// Never fails: no scanner is [`ScanStatus::NotScanned`].
pub fn scan_dir(dir: &Path) -> ScanStatus {
    // An opt-out, because a scan is a third-party binary running over the
    // user's files and someone will want it off.
    if matches!(
        std::env::var("OMNIGET_SKILL_SCAN").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    ) {
        return ScanStatus::NotScanned;
    }
    match scanner_path() {
        Some(program) => scan_dir_with(&program, dir, SCAN_TIMEOUT),
        None => ScanStatus::NotScanned,
    }
}

/// [`scan_dir`] with the program and the deadline spelled out. Tests drive this
/// with a fake scanner on an injected `PATH`.
///
/// The argument list is fixed here and never goes through a shell: the only
/// caller-controlled part is the directory, which is one of our own staging
/// folders.
/// Kills the scanner and everything it started.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // The child leads its own group (see `process_group(0)`), so its pid is
        // the group id. The syscall, never the `kill` command: procps on Linux
        // and the BSD one on macOS do not read a negative argument the same
        // way, and a misread `-1` takes every process of the user with it (on
        // CI that is the runner itself).
        let pgid = child.id() as i32;
        if pgid > 1 {
            // SAFETY: plain syscall; a group that is already gone is ESRCH.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn scan_dir_with(program: &Path, dir: &Path, timeout: Duration) -> ScanStatus {
    let mut cmd = Command::new(program);
    cmd.arg("scan")
        .arg(dir)
        .arg("--no-llm")
        .arg("--format")
        .arg("json")
        // The scan is about this folder; nothing it does should resolve
        // relative to wherever the app happens to be running.
        .current_dir(dir)
        // Keep stdout a clean report: no colour codes, no progress chatter.
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("SKILLSPECTOR_LOG_LEVEL", "ERROR")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    // Its own process group: the scanner is a script that starts other
    // processes, and those inherit our pipes. Killing only the script would
    // leave them holding stdout open long after the timeout.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = match crate::core::process::spawn_retrying_busy(|| cmd.spawn()) {
        Ok(child) => child,
        Err(e) => {
            return ScanStatus::Failed {
                reason: format!("could not run the scanner: {e}"),
            }
        }
    };

    // Drain both pipes on their own threads: a child that fills a pipe we are
    // not reading blocks forever, and the timeout below would never be reached.
    let out_thread = child
        .stdout
        .take()
        .map(|mut p| std::thread::spawn(move || read_capped(&mut p, MAX_STDOUT_BYTES)));
    let err_thread = child
        .stderr
        .take()
        .map(|mut p| std::thread::spawn(move || read_capped(&mut p, MAX_STDERR_BYTES)));

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    kill_group(&mut child);
                    break None;
                }
                std::thread::sleep(POLL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return ScanStatus::Failed {
                    reason: format!("waiting for the scanner failed: {e}"),
                };
            }
        }
    };

    // Reported before the readers are joined: if anything still holds the
    // pipe, waiting for end-of-file here would undo the timeout.
    if timed_out {
        return ScanStatus::Failed {
            reason: format!("the scanner did not finish within {}s", timeout.as_secs()),
        };
    }

    let stdout = out_thread
        .and_then(|t| t.join().ok())
        .unwrap_or(Ok(Vec::new()));
    let stderr = err_thread
        .and_then(|t| t.join().ok())
        .unwrap_or(Ok(Vec::new()))
        .unwrap_or_default();

    let stdout = match stdout {
        Ok(bytes) => bytes,
        Err(over) => {
            return ScanStatus::Failed {
                reason: format!("the scanner wrote more than {over} bytes of output"),
            }
        }
    };
    status_from_output(
        status.and_then(|s| s.code()),
        &String::from_utf8_lossy(&stdout),
        &String::from_utf8_lossy(&stderr),
    )
}

/// Read at most `cap` bytes. `Err(cap)` when the reader had more: a report we
/// had to cut is not a report we may parse.
fn read_capped(reader: &mut dyn Read, cap: u64) -> Result<Vec<u8>, u64> {
    let mut buf = Vec::new();
    // One byte past the cap, so going over is visible instead of silently
    // truncating the JSON into something that would fail to parse anyway.
    let read = reader.take(cap + 1).read_to_end(&mut buf);
    if read.is_err() {
        return Ok(buf);
    }
    if buf.len() as u64 > cap {
        return Err(cap);
    }
    Ok(buf)
}

/// Decide the verdict from one finished run.
///
/// The exit code is deliberately *not* the decision: `cli.py` exits `1` on a
/// completed scan whose score is over the threshold, which is exactly the case
/// we most need to record. A parsed report wins; the code only shapes the
/// message when there is no report.
pub fn status_from_output(code: Option<i32>, stdout: &str, stderr: &str) -> ScanStatus {
    match parse_report(stdout) {
        Some(report) => report.into_status(),
        None => ScanStatus::Failed {
            reason: no_report_reason(code, stderr),
        },
    }
}

fn no_report_reason(code: Option<i32>, stderr: &str) -> String {
    let tail: String = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .join(" / ");
    let where_ = match code {
        Some(c) => format!("the scanner exited with {c}"),
        None => "the scanner was killed".to_string(),
    };
    if tail.is_empty() {
        format!("{where_} and wrote no report")
    } else {
        format!("{where_} and wrote no report: {tail}")
    }
}

/// Parse the report out of stdout.
///
/// Tolerant on purpose: `--format json` prints the report on its own, but a
/// future version (or a wrapper script) may put a line in front of it, so we
/// take the first JSON object and ignore whatever trails it. Anything that is
/// not an object is not a report.
pub fn parse_report(stdout: &str) -> Option<ScanReport> {
    let start = stdout.find('{')?;
    let mut stream = serde_json::Deserializer::from_str(&stdout[start..]).into_iter::<ScanReport>();
    stream.next()?.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/skills_fixtures/scan")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The real shape, straight out of `_format_json`. A clean skill: score 0,
    /// LOW, SAFE — and the CLI would have exited 0.
    #[test]
    fn parses_a_clean_report() {
        let status = status_from_output(Some(0), &fixture("clean.json"), "");
        match status {
            ScanStatus::Scanned {
                score,
                ref severity,
                ref recommendation,
                findings,
                llm,
                ref scanner_version,
                ..
            } => {
                assert_eq!(score, 0);
                assert_eq!(severity, "LOW");
                assert_eq!(recommendation, "SAFE");
                assert_eq!(findings, 0);
                // We always pass --no-llm: never claim an LLM looked at it.
                assert!(!llm);
                assert_eq!(scanner_version.as_deref(), Some("0.7.0"));
            }
            other => panic!("expected a scan, got {other:?}"),
        }
        assert!(!status.is_high_risk());
        assert_eq!(status.score(), Some(0));
    }

    /// Ported from `cli.py`: a completed scan over the threshold exits **1**.
    /// That is a verdict we must keep, not a failure.
    #[test]
    fn exit_one_with_a_report_is_a_verdict_not_a_failure() {
        let status = status_from_output(Some(1), &fixture("risky.json"), "");
        match status {
            ScanStatus::Scanned {
                score,
                ref severity,
                ref recommendation,
                ref max_issue_severity,
                findings,
                ref issues,
                ..
            } => {
                assert_eq!(score, 85);
                assert_eq!(severity, "CRITICAL");
                assert_eq!(recommendation, "DO_NOT_INSTALL");
                assert_eq!(max_issue_severity.as_deref(), Some("CRITICAL"));
                assert_eq!(findings, 3);
                // Worst first, so a truncated list still shows the worst.
                assert_eq!(issues[0].severity, "CRITICAL");
                assert_eq!(issues[0].id, "SC8");
                assert_eq!(issues[0].file, "scripts/setup.sh");
                assert_eq!(issues[0].line, Some(12));
                assert!(issues[2].severity == "LOW" || issues[2].severity == "MEDIUM");
            }
            other => panic!("expected a scan, got {other:?}"),
        }
        assert!(status.is_high_risk());
    }

    /// The threshold itself: `cli.py` compares `score > RISK_THRESHOLD`, so 50
    /// is still fine and 51 is not.
    #[test]
    fn the_threshold_is_ported_exclusive() {
        let at = |score: u8, rec: &str| ScanStatus::Scanned {
            score,
            severity: "HIGH".into(),
            recommendation: rec.into(),
            max_issue_severity: None,
            findings: 0,
            issues: vec![],
            scanner_version: None,
            scanned_at: String::new(),
            llm: false,
        };
        assert_eq!(RISK_THRESHOLD, 50);
        assert!(!at(50, "CAUTION").is_high_risk());
        assert!(at(51, "CAUTION").is_high_risk());
        // A DO_NOT_INSTALL under the threshold still holds the install back.
        assert!(at(10, "DO_NOT_INSTALL").is_high_risk());
        assert!(!ScanStatus::NotScanned.is_high_risk());
        assert!(!ScanStatus::Failed { reason: "x".into() }.is_high_risk());
    }

    /// A report with fields we do not know, and without half of the ones we do,
    /// still parses: the README calls extra fields best-effort.
    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let json = r#"{"risk_assessment":{"score":30},"future_section":{"a":[1,2]}}"#;
        match status_from_output(Some(0), json, "") {
            ScanStatus::Scanned {
                score,
                severity,
                recommendation,
                ..
            } => {
                assert_eq!(score, 30);
                assert_eq!(severity, "UNKNOWN");
                // No recommendation is not a licence to say SAFE.
                assert_eq!(recommendation, "CAUTION");
            }
            other => panic!("expected a scan, got {other:?}"),
        }
    }

    /// A score outside 0–100 is clamped, never wrapped into something small.
    #[test]
    fn an_out_of_range_score_is_clamped() {
        let over = status_from_output(Some(1), r#"{"risk_assessment":{"score":4000}}"#, "");
        assert_eq!(over.score(), Some(100));
        assert!(over.is_high_risk());
        let under = status_from_output(Some(0), r#"{"risk_assessment":{"score":-5}}"#, "");
        assert_eq!(under.score(), Some(0));
    }

    /// `execution_successful: false` is the scanner saying its own run broke;
    /// `cli.py` exits 2 on it. It is a failed scan, not a score of zero.
    #[test]
    fn an_unsuccessful_execution_is_a_failure_not_a_clean_bill() {
        let json = r#"{"risk_assessment":{"score":0,"severity":"LOW","recommendation":"SAFE"},"execution_successful":false}"#;
        match status_from_output(Some(2), json, "") {
            ScanStatus::Failed { reason } => assert!(reason.contains("unsuccessful"), "{reason}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_is_a_failure_carrying_the_stderr_tail() {
        match status_from_output(
            Some(2),
            "Traceback (most recent call last)",
            "Error: boom\n",
        ) {
            ScanStatus::Failed { reason } => {
                assert!(reason.contains("exited with 2"), "{reason}");
                assert!(reason.contains("boom"), "{reason}");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        // Truncated JSON is not a report either.
        assert!(matches!(
            status_from_output(Some(0), r#"{"risk_assessment":{"sco"#, ""),
            ScanStatus::Failed { .. }
        ));
        // Neither is a JSON array.
        assert!(matches!(
            status_from_output(Some(0), "[1,2,3]", ""),
            ScanStatus::Failed { .. }
        ));
    }

    /// A line in front of the report, and trailing noise behind it, do not
    /// stop us finding the object.
    #[test]
    fn a_report_wrapped_in_chatter_still_parses() {
        let body = fixture("clean.json");
        let noisy = format!("warning: shipped baseline detected\n{body}\nDone.\n");
        assert_eq!(status_from_output(Some(0), &noisy, "").score(), Some(0));
    }

    #[test]
    fn the_status_round_trips_through_json() {
        let status = status_from_output(Some(1), &fixture("risky.json"), "");
        let text = serde_json::to_string(&status).unwrap();
        assert!(text.contains("\"status\":\"scanned\""), "{text}");
        let back: ScanStatus = serde_json::from_str(&text).unwrap();
        assert_eq!(back, status);

        for value in [
            ScanStatus::NotScanned,
            ScanStatus::Failed {
                reason: "timed out".into(),
            },
        ] {
            let text = serde_json::to_string(&value).unwrap();
            assert_eq!(serde_json::from_str::<ScanStatus>(&text).unwrap(), value);
        }
        assert_eq!(ScanStatus::default(), ScanStatus::NotScanned);
    }

    #[test]
    fn read_capped_refuses_one_byte_past_the_cap() {
        let mut small: &[u8] = b"hello";
        assert_eq!(read_capped(&mut small, 16).unwrap(), b"hello");
        let mut exact: &[u8] = b"hello";
        assert_eq!(read_capped(&mut exact, 5).unwrap(), b"hello");
        let mut endless = std::io::repeat(b'a');
        assert_eq!(read_capped(&mut endless, 32), Err(32));
    }

    // ---------------------------------------------------- the binary itself

    #[test]
    fn a_missing_scanner_is_not_scanned_not_a_failure() {
        // An empty PATH cannot hold anything.
        assert!(find_in_path(Some(OsStr::new("")), SCANNER_BIN).is_none());
        assert!(find_in_path(None, SCANNER_BIN).is_none());
        let dir = super::super::install::tests_support::temp_dir("scan-empty");
        let path = std::env::join_paths([&dir]).unwrap();
        assert!(find_in_path(Some(&path), SCANNER_BIN).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Fake scanners are shell scripts, so this half of the suite is unix-only.
    /// The parser tests above carry the contract on every platform.
    #[cfg(unix)]
    mod fake {
        use super::*;

        struct Bin {
            dir: PathBuf,
        }

        impl Drop for Bin {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }

        impl Bin {
            /// Writes an executable `skillspector` whose body is `script`.
            fn new(tag: &str, script: &str) -> Self {
                let dir = super::super::super::install::tests_support::temp_dir(tag);
                let path = dir.join(SCANNER_BIN);
                crate::core::process::test_exec::script(&path, &format!("{script}\n"));
                Self { dir }
            }

            fn path_var(&self) -> std::ffi::OsString {
                std::env::join_paths([&self.dir]).unwrap()
            }

            fn bin(&self) -> PathBuf {
                self.dir.join(SCANNER_BIN)
            }
        }

        fn scratch(tag: &str) -> PathBuf {
            super::super::super::install::tests_support::temp_dir(tag)
        }

        #[test]
        fn finds_the_scanner_on_an_injected_path() {
            let bin = Bin::new("scan-find", "exit 0");
            let found = find_in_path(Some(&bin.path_var()), SCANNER_BIN).expect("should find it");
            assert_eq!(found, bin.bin());
            // A file that is there but not executable is not the scanner.
            let plain = bin.dir.join("notascanner");
            std::fs::write(&plain, "x").unwrap();
            assert!(find_in_path(Some(&bin.path_var()), "notascanner").is_none());
        }

        /// The whole happy path through a real process: arguments, stdout,
        /// exit 1 for a risky skill.
        #[test]
        fn runs_the_binary_and_reads_its_report() {
            let report = super::fixture("risky.json");
            let bin = Bin::new(
                "scan-run",
                &format!(
                    // Prove the flags reach the child: without them, no report.
                    "case \"$*\" in *--no-llm*--format*json*) :;; *) echo 'bad args: '\"$*\" >&2; exit 2;; esac\ncat <<'OMNIGET_EOF'\n{report}\nOMNIGET_EOF\nexit 1"
                ),
            );
            let dir = scratch("scan-run-dir");
            let status = scan_dir_with(&bin.bin(), &dir, Duration::from_secs(30));
            assert_eq!(status.score(), Some(85), "{status:?}");
            assert!(status.is_high_risk());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_scanner_that_crashes_without_a_report_fails_open() {
            let bin = Bin::new("scan-crash", "echo 'Error: unreadable source' >&2\nexit 2");
            let dir = scratch("scan-crash-dir");
            match scan_dir_with(&bin.bin(), &dir, Duration::from_secs(30)) {
                ScanStatus::Failed { reason } => {
                    assert!(reason.contains("exited with 2"), "{reason}");
                    assert!(reason.contains("unreadable source"), "{reason}");
                }
                other => panic!("expected a failure, got {other:?}"),
            }
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_scanner_that_hangs_is_killed_and_reported() {
            let bin = Bin::new("scan-hang", "sleep 30");
            let dir = scratch("scan-hang-dir");
            let started = Instant::now();
            match scan_dir_with(&bin.bin(), &dir, Duration::from_millis(300)) {
                ScanStatus::Failed { reason } => {
                    assert!(reason.contains("did not finish"), "{reason}")
                }
                other => panic!("expected a failure, got {other:?}"),
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the timeout did not cut it short"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// A scanner that floods stdout must not be parsed and must not be
        /// read into memory without bound.
        #[test]
        fn output_past_the_ceiling_is_refused() {
            let bin = Bin::new("scan-flood", "exec yes 0123456789abcdef");
            let dir = scratch("scan-flood-dir");
            match scan_dir_with(&bin.bin(), &dir, Duration::from_secs(30)) {
                ScanStatus::Failed { reason } => assert!(
                    reason.contains("more than") || reason.contains("did not finish"),
                    "{reason}"
                ),
                other => panic!("expected a failure, got {other:?}"),
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
