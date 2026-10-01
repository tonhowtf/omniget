use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ItemCheck {
    pub url: String,
    pub title: Option<String>,
    pub estimated_bytes: Option<u64>,
    pub problem: Option<Problem>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    Unsupported,
    NeedsAuth,
    NotFound,
    AlreadyHave,
}

impl Problem {}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PreflightReport {
    pub total: usize,
    pub ready: usize,
    pub estimated_bytes: u64,
    pub available_bytes: Option<u64>,
    pub problems: Vec<ItemCheck>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Go,
    GoWithSkips,
    Stop,
}

pub const HEADROOM: f64 = 1.10;

pub fn build_report(checks: Vec<ItemCheck>, available_bytes: Option<u64>) -> PreflightReport {
    let total = checks.len();
    let ready = checks.iter().filter(|c| c.problem.is_none()).count();

    let estimated_bytes: u64 = checks
        .iter()
        .filter(|c| c.problem.is_none())
        .filter_map(|c| c.estimated_bytes)
        .sum();

    let fits = match available_bytes {
        Some(free) => (estimated_bytes as f64 * HEADROOM) <= free as f64,
        None => true,
    };

    let verdict = if ready == 0 || !fits {
        Verdict::Stop
    } else if ready < total {
        Verdict::GoWithSkips
    } else {
        Verdict::Go
    };

    PreflightReport {
        total,
        ready,
        estimated_bytes,
        available_bytes,
        problems: checks.into_iter().filter(|c| c.problem.is_some()).collect(),
        verdict,
    }
}

pub fn available_space(path: &std::path::Path) -> Option<u64> {
    fs4::available_space(path).ok()
}
