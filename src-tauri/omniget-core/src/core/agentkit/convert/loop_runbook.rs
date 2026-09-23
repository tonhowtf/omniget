//! Loops (estudo 03 §6): a loop is a runbook (goal, interval, stop condition,
//! budget) plus references to other components. The plan already installs the
//! referenced components (`plan::expand` through the catalog resolver); this
//! module writes the runbook for tools that have no loops folder of their own
//! (`.agents/loops/<name>.md`, shared by every tool) and turns a loop into an
//! OmniGet [`LoopPlan`] that the Loops engine (`jobs.rs` `LoopDef`) runs with any
//! runner, with a real interval, stop check and budget.

use serde::{Deserialize, Serialize};

use super::{ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Result, Scope};

pub struct LoopConverter;

impl Converter for LoopConverter {
    fn id(&self) -> &'static str {
        "loop_runbook"
    }

    /// Every tool without a loop format of its own (Claude has `.claude/loops`).
    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Loop && target.format(kind).is_none()
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let ComponentBody::Loop(_) = &c.body else {
            return Ok(vec![]);
        };
        let base = match scope {
            Scope::Project | Scope::Local => ctx
                .project
                .ok_or_else(|| {
                    AgentkitError::new("AGENTKIT_NO_PATH", "a project loop needs a project")
                })?
                .join(".agents"),
            Scope::Global | Scope::Managed => ctx.env.home.join(".agents"),
        };
        let name = ctx.name_for(c);
        let content = c.entry_text().unwrap_or_default().to_string();
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            base.join("loops").join(format!("{name}.md")),
            content,
            "loop runbook",
        );
        pf.primary = true;
        pf.notes.push(format!(
            "{} has no loop engine: OmniGet runs the loop (LLM → Loops) with {} as the runner; the runbook is shared in .agents/loops",
            target.name, target.name
        ));
        Ok(vec![pf])
    }
}

/// A loop ready for OmniGet's Loops engine (mirrors `jobs::LoopDef` inputs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopPlan {
    /// Catalog id of the loop.
    pub loop_id: String,
    pub name: String,
    /// Tool that runs each round (`claude`, `codex` …) or an OmniGet agent id.
    pub runner: String,
    /// Prompt of each round (the loop's `/loop` line, else its goal).
    pub prompt: String,
    /// Seconds between rounds; `None` = back to back (`on-demand`).
    pub interval_secs: Option<u64>,
    /// The interval as written (`10m`, `daily`, `on-demand`).
    pub interval: Option<String>,
    /// Shell command whose exit 0 ends the loop, when the loop names one.
    pub check_command: Option<String>,
    /// Stop condition in words (also appended to the prompt).
    pub stop_condition: Option<String>,
    pub max_rounds: u32,
    pub max_minutes: u32,
    /// Budget text of the runbook.
    pub budget: Option<String>,
    /// Components the loop brings (`agent:dev/x` …), installed with it.
    pub components: Vec<String>,
    pub workspace: Option<String>,
}

/// `10m` → 600, `1h` → 3600, `24h`, `7d`, `daily`/`nightly` → 86400,
/// `hourly`, `weekly`; `on-demand` / unknown → `None`.
pub fn interval_secs(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "hourly" => return Some(3600),
        "daily" | "nightly" => return Some(86_400),
        "weekly" => return Some(7 * 86_400),
        _ => {}
    }
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: u64 = digits.parse().ok()?;
    let unit = s[digits.len()..].trim();
    let mult = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "" | "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        "w" | "week" | "weeks" => 7 * 86_400,
        _ => return None,
    };
    Some(n * mult)
}

/// Text of a `## <title>` section (emoji prefixes ignored).
fn section<'a>(body: &'a str, title: &str) -> Option<&'a str> {
    let lower_title = title.to_ascii_lowercase();
    let mut start = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("## ") {
            if let Some(s) = start {
                return Some(body[s..offset].trim());
            }
            if h.to_ascii_lowercase().contains(&lower_title) {
                start = Some(offset + line.len());
            }
        }
        offset += line.len();
    }
    start.map(|s| body[s..].trim())
}

/// The quoted prompt of a `/loop <interval> "<prompt>"` line.
pub fn loop_line_prompt(body: &str) -> Option<String> {
    for line in body.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("/loop") else {
            continue;
        };
        let q = rest.find('"')?;
        let inner = &rest[q + 1..];
        let end = inner.rfind('"')?;
        let p = inner[..end].replace("\\\"", "\"");
        if !p.trim().is_empty() {
            return Some(p.trim().to_string());
        }
    }
    None
}

/// First inline-code span that looks like a check command (`npm test` …).
fn check_command(texts: &[&str]) -> Option<String> {
    const STARTS: &[&str] = &[
        "npm ",
        "pnpm ",
        "yarn ",
        "bun ",
        "npx ",
        "cargo ",
        "go ",
        "pytest",
        "python ",
        "python3 ",
        "make",
        "just ",
        "mvn ",
        "gradle",
        "./gradlew",
        "dotnet ",
        "deno ",
        "tox",
        "ruff",
        "eslint",
        "tsc",
        "bash ",
        "sh ",
        "./",
    ];
    for t in texts {
        let mut rest = *t;
        while let Some(i) = rest.find('`') {
            let after = &rest[i + 1..];
            let Some(j) = after.find('`') else { break };
            let code = after[..j].trim();
            if STARTS.iter().any(|s| code.starts_with(s)) && !code.contains('\n') {
                return Some(code.to_string());
            }
            rest = &after[j + 1..];
        }
    }
    None
}

/// Numbers the budget text states (`max 20 iterations`, `30 minutes`, `2h`).
fn budget_numbers(text: &str) -> (Option<u32>, Option<u32>) {
    let lower = text.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '(' || c == ')')
        .filter(|w| !w.is_empty())
        .collect();
    let mut rounds = None;
    let mut minutes = None;
    for (i, w) in words.iter().enumerate() {
        let n: Option<u32> = w.trim_end_matches(['m', 'h']).parse().ok();
        let Some(n) = n else { continue };
        let next = words.get(i + 1).copied().unwrap_or("");
        if w.ends_with('h') && w.len() > 1 {
            minutes.get_or_insert(n * 60);
        } else if w.ends_with('m') && w.len() > 1 {
            minutes.get_or_insert(n);
        } else if next.starts_with("iteration")
            || next.starts_with("round")
            || next.starts_with("turn")
            || next.starts_with("pass")
            || next.starts_with("attempt")
        {
            rounds.get_or_insert(n);
        } else if next.starts_with("minute") || next.starts_with("min") {
            minutes.get_or_insert(n);
        } else if next.starts_with("hour") {
            minutes.get_or_insert(n * 60);
        }
    }
    (rounds, minutes)
}

/// Default caps when the runbook states none (loops are meant to be bounded).
pub const DEFAULT_MAX_ROUNDS: u32 = 10;
pub const DEFAULT_MAX_MINUTES: u32 = 60;

/// Turns a loop component into an OmniGet loop for `runner`.
pub fn loop_plan(c: &Component, runner: &str, workspace: Option<&str>) -> Result<LoopPlan> {
    let ComponentBody::Loop(l) = &c.body else {
        return Err(AgentkitError::new(
            "AGENTKIT_KIND",
            format!("{} is a {}, not a loop", c.id, c.kind),
        ));
    };
    let body = l.body.as_str();
    let stop_section = section(body, "stopping condition");
    let budget_section = section(body, "budget");
    let stop = l
        .stop_condition
        .clone()
        .or_else(|| stop_section.map(str::to_string));
    let budget = l
        .budget
        .clone()
        .or_else(|| budget_section.map(str::to_string));
    let mut prompt = loop_line_prompt(body).unwrap_or_else(|| {
        section(body, "goal")
            .map(str::to_string)
            .filter(|g| !g.is_empty())
            .unwrap_or_else(|| l.goal.clone())
    });
    if let Some(s) = &stop {
        if !prompt.to_ascii_lowercase().contains("stop") {
            prompt.push_str(&format!("\n\nStop when: {s}"));
        }
    }
    let check = check_command(&[
        l.stop_condition.as_deref().unwrap_or(""),
        stop_section.unwrap_or(""),
        section(body, "iteration").unwrap_or(""),
    ]);
    let (rounds, minutes) = budget_numbers(budget.as_deref().unwrap_or(""));
    Ok(LoopPlan {
        loop_id: c.id.clone(),
        name: c.name.clone(),
        runner: runner.to_string(),
        prompt,
        interval_secs: l.interval.as_deref().and_then(interval_secs),
        interval: l.interval.clone(),
        check_command: check,
        stop_condition: stop,
        max_rounds: rounds.unwrap_or(DEFAULT_MAX_ROUNDS),
        max_minutes: minutes.unwrap_or(DEFAULT_MAX_MINUTES),
        budget,
        components: l.components.clone(),
        workspace: workspace.map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::parse;

    #[test]
    fn loop_plan_from_runbook() {
        let md = "---\nname: build-test-fix-loop\ndescription: Builds then tests.\ncategory: engineering\ninterval: 10m\nstop-condition: The build is green and `npm test` passes.\ncomponents: [agent:development-team/test-runner, command:testing/generate-tests]\n---\n\n# X\n\n## 🎯 Goal\nImplement.\n\n## ▶️ Run it\n```\n/loop 10m \"Build the next item, run tests. Stop when green.\"\n```\n\n## 🛑 Stopping condition\nAll green.\n\n## 💰 Budget & guardrails\nCap at 15 iterations and 45 minutes.\n";
        let files: parse::RawFiles = [("x.md".to_string(), md.as_bytes().to_vec())]
            .into_iter()
            .collect();
        let mut c = parse::parse_raw(ComponentKind::Loop, "x.md", &files).unwrap();
        c.id = "cct:loops/engineering/build-test-fix-loop".into();
        let p = loop_plan(&c, "codex", Some("/w")).unwrap();
        assert_eq!(p.prompt, "Build the next item, run tests. Stop when green.");
        assert_eq!(p.interval_secs, Some(600));
        assert_eq!(p.check_command.as_deref(), Some("npm test"));
        assert_eq!((p.max_rounds, p.max_minutes), (15, 45));
        assert_eq!(p.components.len(), 2);
        assert_eq!(interval_secs("on-demand"), None);
        assert_eq!(interval_secs("7d"), Some(604_800));
        assert_eq!(interval_secs("daily"), Some(86_400));
    }
}
