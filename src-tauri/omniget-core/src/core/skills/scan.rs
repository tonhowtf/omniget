use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const SCANNER_BIN: &str = "skillspector";

/// Ported from `skillspector/constants.py`: "Risk score threshold above which a
pub const RISK_THRESHOLD: u8 = 50;

pub const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

pub const MAX_STDOUT_BYTES: u64 = 4 * 1024 * 1024;

pub const MAX_STDERR_BYTES: u64 = 64 * 1024;

pub const MAX_ISSUES_KEPT: usize = 12;

const POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScanStatus {
    #[default]
    NotScanned,
    Scanned {
        score: u8,
        severity: String,
        recommendation: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_issue_severity: Option<String>,
        #[serde(default)]
        findings: usize,
        #[serde(default)]
        issues: Vec<ScanIssue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scanner_version: Option<String>,
        #[serde(default)]
        scanned_at: String,
        #[serde(default)]
        llm: bool,
    },
    Failed {
        reason: String,
    },
}

impl ScanStatus {
    pub fn score(&self) -> Option<u8> {
        match self {
            ScanStatus::Scanned { score, .. } => Some(*score),
            _ => None,
        }
    }

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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScanIssue {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
}

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

pub fn find_in_path(path_var: Option<&OsStr>, program: &str) -> Option<PathBuf> {
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

pub fn scanner_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("OMNIGET_SKILL_SCANNER") {
        let path = PathBuf::from(&explicit);
        return is_runnable(&path).then_some(path);
    }
    find_in_path(std::env::var_os("PATH").as_deref(), SCANNER_BIN)
}

pub fn is_available() -> bool {
    scanner_path().is_some()
}

pub fn scan_dir(dir: &Path) -> ScanStatus {
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

pub(super) fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
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
        .current_dir(dir)
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

fn read_capped(reader: &mut dyn Read, cap: u64) -> Result<Vec<u8>, u64> {
    let mut buf = Vec::new();
    let read = reader.take(cap + 1).read_to_end(&mut buf);
    if read.is_err() {
        return Ok(buf);
    }
    if buf.len() as u64 > cap {
        return Err(cap);
    }
    Ok(buf)
}

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
                assert!(!llm);
                assert_eq!(scanner_version.as_deref(), Some("0.7.0"));
            }
            other => panic!("expected a scan, got {other:?}"),
        }
        assert!(!status.is_high_risk());
        assert_eq!(status.score(), Some(0));
    }

    /// Ported from `cli.py`: a completed scan over the threshold exits **1**.
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
        assert!(at(10, "DO_NOT_INSTALL").is_high_risk());
        assert!(!ScanStatus::NotScanned.is_high_risk());
        assert!(!ScanStatus::Failed { reason: "x".into() }.is_high_risk());
    }

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
                assert_eq!(recommendation, "CAUTION");
            }
            other => panic!("expected a scan, got {other:?}"),
        }
    }

    #[test]
    fn an_out_of_range_score_is_clamped() {
        let over = status_from_output(Some(1), r#"{"risk_assessment":{"score":4000}}"#, "");
        assert_eq!(over.score(), Some(100));
        assert!(over.is_high_risk());
        let under = status_from_output(Some(0), r#"{"risk_assessment":{"score":-5}}"#, "");
        assert_eq!(under.score(), Some(0));
    }

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
        assert!(matches!(
            status_from_output(Some(0), r#"{"risk_assessment":{"sco"#, ""),
            ScanStatus::Failed { .. }
        ));
        assert!(matches!(
            status_from_output(Some(0), "[1,2,3]", ""),
            ScanStatus::Failed { .. }
        ));
    }

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

    #[test]
    fn a_missing_scanner_is_not_scanned_not_a_failure() {
        assert!(find_in_path(Some(OsStr::new("")), SCANNER_BIN).is_none());
        assert!(find_in_path(None, SCANNER_BIN).is_none());
        let dir = super::super::install::tests_support::temp_dir("scan-empty");
        let path = std::env::join_paths([&dir]).unwrap();
        assert!(find_in_path(Some(&path), SCANNER_BIN).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
            let plain = bin.dir.join("notascanner");
            std::fs::write(&plain, "x").unwrap();
            assert!(find_in_path(Some(&bin.path_var()), "notascanner").is_none());
        }

        #[test]
        fn runs_the_binary_and_reads_its_report() {
            let report = super::fixture("risky.json");
            let bin = Bin::new(
                "scan-run",
                &format!(
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
        fn a_scanner_that_crashes_without_a_report_is_a_failed_scan() {
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
