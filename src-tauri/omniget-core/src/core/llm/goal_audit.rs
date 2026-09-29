//! Loops that know when to stop (roadmap 3.7, S22).
//!
//! - **Auditor** (84#P1): a Loop without a `check_command` no longer ends
//!   because the agent wrote "DONE". After each round a side model of the
//!   agent's own provider (see [`super::router::TaskKind::Side`]) gets the
//!   objective, the round's [`Evidence`] (its real tool calls and outputs)
//!   and the last reply, and answers
//!   `{"verdict":"continue|complete|blocked","note":"…"}`. `complete` ends the
//!   Loop; `blocked` ends it only the [`blocked_stop`]-th time in a row; after
//!   [`audit_fail_stop`] audits in a row that gave no verdict the Loop stops
//!   as `blocked` ("audit unavailable"), never going on blind. The agent's
//!   own claim is recorded ([`claims_done`]) and nothing else.
//! - **Continuation guide** (83#P7): from round 2 on, when the runtime
//!   resumes the provider's session (Claude `--resume`, Codex
//!   `thread/resume`), the task is already in the conversation, so the round
//!   sends a short guide plus the last verdict or check output instead of the
//!   whole prompt again. Without resume the full prompt goes, as before.
//!
//! The texts and the numbers live in `goal_audit.json`, which
//! `scripts/llm/loops_eval.py` (BN9) reads too, so the benchmark sends what
//! the app sends. Prompts and stop rules adapted from OpenChamber's
//! `session-goal/runtime.js` (MIT, Copyright (c) 2025 Bohdan Triapitsyn).

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `goal_audit.json`, parsed once.
#[derive(Debug, Deserialize)]
pub struct Texts {
    pub max_rounds: u32,
    pub blocked_stop: u32,
    pub audit_fail_stop: u32,
    pub disagree_stop: u32,
    pub note_max_chars: usize,
    pub reply_max_chars: usize,
    pub evidence_max_chars: usize,
    pub evidence_input_chars: usize,
    pub evidence_output_chars: usize,
    pub audit_system: String,
    pub audit_user: String,
    pub audit_evidence: String,
    pub audit_previous: String,
    pub round_footer: String,
    pub footer_short: String,
    pub guide: String,
    pub guide_verdict: String,
    pub guide_check: String,
    pub full_audited: String,
}

pub static TEXTS: LazyLock<Texts> = LazyLock::new(|| {
    serde_json::from_str(include_str!("goal_audit.json")).expect("goal_audit.json is valid")
});

/// Hard ceiling of rounds of an audited Loop (the audit and the budget are
/// the intended stops; this only prevents a runaway).
pub fn max_rounds() -> u32 {
    TEXTS.max_rounds
}

/// `blocked` verdicts in a row that end the Loop: one snag must not.
pub fn blocked_stop() -> u32 {
    TEXTS.blocked_stop
}

/// Audits in a row without a verdict that end the Loop.
pub fn audit_fail_stop() -> u32 {
    TEXTS.audit_fail_stop
}

/// Rounds in a row in which the agent says it is done and the auditor says
/// `continue`: a person settles it instead of the two arguing until the
/// round ceiling (BN9 real: an auditor that only reads the last reply kept 10
/// of 33 finished loops going to the ceiling).
pub fn disagree_stop() -> u32 {
    TEXTS.disagree_stop
}

/// The system prompt of the auditor.
pub fn audit_system() -> &'static str {
    &TEXTS.audit_system
}

/// Fills `{{name}}` placeholders in one pass: a value that itself contains
/// `{{…}}` (a goal that quotes a template) is never expanded again.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = &after[..end];
                match vars.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push_str("{{");
                        out.push_str(name);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn clip_chars(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// The tail of a long reply is where the `Done/Verified/Missing` lines are.
fn tail_chars(s: &str, max: usize) -> String {
    let s = s.trim();
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n - max).collect();
    format!("…{tail}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Continue,
    Complete,
    Blocked,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Continue => "continue",
            Verdict::Complete => "complete",
            Verdict::Blocked => "blocked",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "continue" => Some(Verdict::Continue),
            "complete" | "completed" => Some(Verdict::Complete),
            "blocked" => Some(Verdict::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Audit {
    pub verdict: Verdict,
    pub note: String,
}

/// Keys that say what a tool call did, in this order; the others follow
/// sorted, `description` (the model's own caption) left out.
const EVIDENCE_KEYS: [&str; 7] = [
    "command",
    "file_path",
    "path",
    "pattern",
    "content",
    "old_string",
    "new_string",
];

/// Input kept per call while it streams: a large `Write` is clipped to
/// [`Texts::evidence_input_chars`] anyway.
const EVIDENCE_INPUT_KEEP: usize = 64 * 1024;

/// Calls kept per round (the digest keeps only its tail anyway).
const EVIDENCE_CALLS_KEEP: usize = 200;

#[derive(Debug, Clone, Default)]
struct EvidenceCall {
    id: String,
    name: String,
    input: String,
    output: Option<(String, bool)>,
}

/// What the agent's tools really did in one round, gathered from the turn's
/// own events (`ToolCallStart` / `ToolCallDelta` / `ToolResult`), for the
/// auditor. BN9 real (3.7) with the reply alone: the auditor asked for more
/// proof in 29 rounds whose work already passed, because a reply that says
/// "`cat en.txt` shows the translation" is a claim, while the call and its
/// output are a fact. Native tools emit no `ToolResult` (the Coordinator
/// runs them): their calls show without an output.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    calls: Vec<EvidenceCall>,
}

impl Evidence {
    pub fn feed(&mut self, event: &super::types::TurnEvent) {
        use super::types::TurnEvent;
        match event {
            TurnEvent::ToolCallStart { id, name } => {
                if self.calls.len() >= EVIDENCE_CALLS_KEEP {
                    self.calls.remove(0);
                }
                self.calls.push(EvidenceCall {
                    id: id.clone(),
                    name: name.clone(),
                    ..EvidenceCall::default()
                });
            }
            TurnEvent::ToolCallDelta {
                id,
                input_json_delta,
            } => {
                if let Some(c) = self.calls.iter_mut().rev().find(|c| &c.id == id) {
                    if c.input.len() < EVIDENCE_INPUT_KEEP {
                        c.input.push_str(input_json_delta);
                    }
                }
            }
            TurnEvent::ToolResult {
                id,
                content,
                is_error,
            } => {
                if let Some(c) = self.calls.iter_mut().rev().find(|c| &c.id == id) {
                    let keep = TEXTS.evidence_output_chars * 2;
                    c.output = Some((content.chars().take(keep).collect(), *is_error));
                }
            }
            _ => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// `name: key=value …` of one call's input (the streamed JSON; a partial
    /// or broken one goes as text).
    fn input_line(input: &str) -> String {
        let text = match serde_json::from_str::<Value>(input) {
            Ok(Value::Object(map)) => {
                let mut keys: Vec<&String> = EVIDENCE_KEYS
                    .iter()
                    .filter_map(|k| map.get_key_value(*k).map(|(k, _)| k))
                    .collect();
                let mut rest: Vec<&String> = map
                    .keys()
                    .filter(|k| !EVIDENCE_KEYS.contains(&k.as_str()) && k.as_str() != "description")
                    .collect();
                rest.sort();
                keys.extend(rest);
                keys.iter()
                    .map(|k| format!("{k}={}", evidence_value(&map[k.as_str()])))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            Ok(Value::Null) => String::new(),
            Ok(v) => evidence_value(&v),
            Err(_) => input.to_string(),
        };
        clip_chars(&text, TEXTS.evidence_input_chars)
    }

    /// One entry per call, oldest first; only the tail that fits in
    /// [`Texts::evidence_max_chars`] goes (the latest checks matter most).
    /// Empty when the round called no tool.
    pub fn render(&self) -> String {
        if self.calls.is_empty() {
            return String::new();
        }
        let lines: Vec<String> = self
            .calls
            .iter()
            .map(|c| {
                let head = format!("- {}: {}", c.name, Self::input_line(&c.input));
                let tail = match &c.output {
                    None => "(no result recorded)".to_string(),
                    Some((out, err)) => {
                        let body = clip_chars(out, TEXTS.evidence_output_chars);
                        format!(
                            "{}{}",
                            if *err { "(error) " } else { "" },
                            if body.is_empty() { "(empty)" } else { &body }
                        )
                    }
                };
                format!("{}\n  → {tail}", head.trim_end())
            })
            .collect();
        tail_chars(&lines.join("\n"), TEXTS.evidence_max_chars)
    }
}

fn evidence_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => canonical_json(other),
    }
}

/// Compact JSON with sorted keys (what `json.dumps(sort_keys=True)` gives
/// the benchmark), whatever map order serde_json was built with.
fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(&map[k.as_str()])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => other.to_string(),
    }
}

/// The auditor's user message: the objective (as data), the round's
/// evidence ([`Evidence::render`], empty = left out), the last reply and the
/// previous verdict, so a round that finished what was missing is read
/// against what was missing.
pub fn audit_input(goal: &str, reply: &str, evidence: &str, previous: Option<&Audit>) -> String {
    let sample: String = goal
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect();
    let previous = previous
        .map(|a| {
            fill(
                &TEXTS.audit_previous,
                &[
                    ("verdict", a.verdict.as_str()),
                    ("note", if a.note.is_empty() { "-" } else { &a.note }),
                ],
            )
        })
        .unwrap_or_default();
    let evidence = if evidence.trim().is_empty() {
        String::new()
    } else {
        fill(&TEXTS.audit_evidence, &[("evidence", evidence)])
    };
    fill(
        &TEXTS.audit_user,
        &[
            ("goal", goal.trim()),
            ("evidence", &evidence),
            ("reply", &tail_chars(reply, TEXTS.reply_max_chars)),
            ("previous", &previous),
            ("sample", &sample.replace('"', "'")),
        ],
    )
}

/// Reads the auditor's answer, tolerating fences and prose around the
/// object. `None` = no usable verdict (counts as a failed audit).
pub fn parse(text: &str) -> Option<Audit> {
    let t = text.trim();
    if let (Some(a), Some(b)) = (t.find('{'), t.rfind('}')) {
        if a < b {
            if let Ok(v) = serde_json::from_str::<Value>(&t[a..=b]) {
                if let Some(verdict) = v["verdict"].as_str().and_then(Verdict::parse) {
                    let verdict = consistent(verdict, &v);
                    let note = v["note"].as_str().unwrap_or("");
                    return Some(Audit {
                        verdict,
                        note: clip_chars(note, TEXTS.note_max_chars),
                    });
                }
            }
        }
    }
    // A broken object: take the verdict word that follows "verdict".
    let lower = t.to_ascii_lowercase();
    let at = lower.find("\"verdict\"")?;
    let rest = &lower[at + 9..];
    let word: String = rest
        .trim_start_matches(|c: char| c == ':' || c == '"' || c.is_whitespace())
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    Verdict::parse(&word).map(|verdict| Audit {
        verdict,
        note: String::new(),
    })
}

/// Holds the verdict to the fields the auditor filled before it: a small
/// model without thinking said `blocked` for "no proof shown" (4 of 11
/// missions, Haiku, 2.1.284) although its own `needs_user` was false, and a
/// `complete` with requirements still listed as unproven is not complete.
/// Both become `continue`. Absent fields leave the verdict as it is.
fn consistent(verdict: Verdict, v: &Value) -> Verdict {
    match verdict {
        Verdict::Blocked if v["needs_user"].as_bool() == Some(false) => Verdict::Continue,
        Verdict::Complete if v["unproven"].as_array().is_some_and(|u| !u.is_empty()) => {
            Verdict::Continue
        }
        other => other,
    }
}

/// Whether the agent itself says it finished: a line `DONE`, or a
/// `Missing:` line that says nothing is left. Recorded next to the verdict
/// (the "DONE-own" of BN9); it never ends an audited Loop.
pub fn claims_done(reply: &str) -> bool {
    reply.lines().any(|line| {
        let l = line
            .trim()
            .trim_matches(|c: char| c == '*' || c == '`' || c == '_' || c == '-');
        let l = l.trim();
        if l == "DONE" {
            return true;
        }
        let lower = l.to_ascii_lowercase();
        match lower.strip_prefix("missing:") {
            Some(v) => {
                let v = v
                    .trim()
                    .trim_matches(|c: char| c == '*' || c == '`' || c == '.' || c == '_');
                matches!(v.trim(), "nothing" | "none" | "n/a" | "-" | "—" | "")
            }
            None => false,
        }
    })
}

/// Counters an audited Loop keeps between rounds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Streaks {
    pub blocked: u32,
    pub failed: u32,
    /// The agent said done and the auditor said `continue`.
    pub disagree: u32,
}

/// What the Loop does after an audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    /// Ends as `done` (`audit_complete`).
    Complete,
    /// Ends as `blocked` with this stop reason.
    Blocked(&'static str),
}

pub const STOP_COMPLETE: &str = "audit_complete";
pub const STOP_BLOCKED: &str = "audit_blocked";
pub const STOP_UNAVAILABLE: &str = "audit_unavailable";
pub const STOP_DISAGREE: &str = "audit_disagrees";
pub const STOP_TOKEN_BUDGET: &str = "token_budget";
pub const STOP_ROUND_FAILED: &str = "round_failed";

/// The stop rules of 84#P1 for one audit (`None` = the audit gave no
/// verdict), plus [`disagree_stop`]. `claimed` = the agent said it was done.
pub fn step(streaks: &mut Streaks, audit: Option<&Audit>, claimed: bool) -> Step {
    let Some(audit) = audit else {
        streaks.failed += 1;
        return if streaks.failed >= audit_fail_stop() {
            Step::Blocked(STOP_UNAVAILABLE)
        } else {
            Step::Continue
        };
    };
    streaks.failed = 0;
    match audit.verdict {
        Verdict::Complete => Step::Complete,
        Verdict::Blocked => {
            streaks.blocked += 1;
            if streaks.blocked >= blocked_stop() {
                Step::Blocked(STOP_BLOCKED)
            } else {
                Step::Continue
            }
        }
        Verdict::Continue => {
            streaks.blocked = 0;
            if claimed {
                streaks.disagree += 1;
                if streaks.disagree >= disagree_stop() {
                    return Step::Blocked(STOP_DISAGREE);
                }
            } else {
                streaks.disagree = 0;
            }
            Step::Continue
        }
    }
}

/// The round ceiling a Loop runs under: an audited one never passes
/// [`max_rounds`], whatever it asked for.
pub fn effective_max(max_rounds: Option<u32>, audited: bool) -> Option<u32> {
    if audited {
        Some(
            max_rounds
                .unwrap_or(self::max_rounds())
                .min(self::max_rounds()),
        )
    } else {
        max_rounds
    }
}

/// Everything the text of one round depends on.
#[derive(Debug, Clone, Copy)]
pub struct Round<'a> {
    pub round: u32,
    pub max: Option<u32>,
    pub prompt: &'a str,
    pub check_command: Option<&'a str>,
    pub last_check: Option<&'a str>,
    pub last_audit: Option<&'a Audit>,
    /// Judged by the auditor (no check command, an agent that can be asked).
    pub audited: bool,
    /// The provider holds the conversation: round ≥ 2 sends only the guide.
    pub resumed: bool,
}

/// The text of a round and whether it is the short guide.
pub fn round_prompt(r: &Round<'_>) -> (String, bool) {
    let check = r.check_command.filter(|c| !c.trim().is_empty());
    if r.round <= 1 {
        let text = match check {
            Some(c) => format!(
                "{}\n\nYou are done when this command exits 0: `{c}`. Run it yourself before you stop.",
                r.prompt
            ),
            None if r.audited => format!("{}\n\n{}", r.prompt, TEXTS.round_footer),
            None => r.prompt.to_string(),
        };
        return (text, false);
    }
    let round = r.round.to_string();
    let max = r
        .max
        .map(|m| m.to_string())
        .unwrap_or_else(|| "∞".to_string());
    let verdict = r.last_audit.map(|a| {
        fill(
            &TEXTS.guide_verdict,
            &[
                ("verdict", a.verdict.as_str()),
                ("note", if a.note.is_empty() { "-" } else { &a.note }),
            ],
        )
    });
    let failing = match (check, r.last_check) {
        (Some(_), Some(out)) => Some(out),
        _ => None,
    };
    let full = match failing {
        // The text every check Loop sent before 3.7.
        Some(out) => format!(
            "Round {round}. The check still fails:\n```\n{out}\n```\nKeep working on the task: {}",
            r.prompt
        ),
        None if r.audited => {
            let mut parts = vec![fill(
                &TEXTS.full_audited,
                &[("round", &round), ("max", &max), ("prompt", r.prompt)],
            )];
            if let Some(v) = verdict.clone() {
                parts.push(v);
            }
            parts.push(TEXTS.round_footer.clone());
            parts.join("\n\n")
        }
        None => format!(
            "Round {round}. Continue the task; say DONE on a line of its own when nothing is left. Task: {}",
            r.prompt
        ),
    };
    if r.resumed {
        let mut parts = vec![fill(&TEXTS.guide, &[("round", &round), ("max", &max)])];
        if let Some(out) = failing {
            parts.push(fill(&TEXTS.guide_check, &[("check", out)]));
        }
        if let Some(v) = verdict.filter(|_| r.audited) {
            parts.push(v);
        }
        if r.audited {
            parts.push(TEXTS.footer_short.clone());
        }
        let guide = parts.join("\n\n");
        // A task shorter than the guide costs less sent again (B1: 312
        // against 342 characters): the guide only goes when it is shorter.
        if guide.len() < full.len() {
            return (guide, true);
        }
    }
    (full, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit(v: Verdict) -> Audit {
        Audit {
            verdict: v,
            note: "tests pass, docs missing".into(),
        }
    }

    #[test]
    fn the_shared_file_carries_the_rules_of_84p1() {
        assert_eq!(max_rounds(), 20);
        assert_eq!(blocked_stop(), 3);
        assert_eq!(audit_fail_stop(), 2);
        assert_eq!(disagree_stop(), 3);
        assert!(audit_system().contains("\"verdict\""));
    }

    #[test]
    fn fill_is_one_pass_and_keeps_unknown_names() {
        assert_eq!(
            fill("a {{x}} b {{y}} {{z}}", &[("x", "{{y}}"), ("y", "2")]),
            "a {{y}} b 2 {{z}}"
        );
        assert_eq!(fill("open {{x", &[("x", "1")]), "open {{x");
    }

    #[test]
    fn a_verdict_is_read_through_fences_and_prose() {
        let a = parse("```json\n{\"verdict\": \"Complete\", \"note\": \"all tests pass\"}\n```")
            .unwrap();
        assert_eq!(a.verdict, Verdict::Complete);
        assert_eq!(a.note, "all tests pass");
        let b = parse("Sure. {\"verdict\":\"blocked\",\"note\":\"needs the API key\"} ok").unwrap();
        assert_eq!(b.verdict, Verdict::Blocked);
        // Trailing comma: the loose reading still finds the verdict.
        let c = parse("{\"verdict\": \"continue\", \"note\": \"x\",}").unwrap();
        assert_eq!(c.verdict, Verdict::Continue);
        assert!(parse("I think it is done").is_none());
        assert!(parse("{\"verdict\":\"maybe\"}").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn a_verdict_that_contradicts_its_own_fields_is_continue() {
        let blocked_without_user =
            r#"{"unproven":["no check shown"],"needs_user":false,"verdict":"blocked","note":"x"}"#;
        assert_eq!(
            parse(blocked_without_user).unwrap().verdict,
            Verdict::Continue
        );
        let real_block =
            r#"{"unproven":["deploy"],"needs_user":true,"verdict":"blocked","note":"key"}"#;
        assert_eq!(parse(real_block).unwrap().verdict, Verdict::Blocked);
        let early = r#"{"unproven":["tests"],"needs_user":false,"verdict":"complete","note":"x"}"#;
        assert_eq!(parse(early).unwrap().verdict, Verdict::Continue);
        let done = r#"{"unproven":[],"needs_user":false,"verdict":"complete","note":"ok"}"#;
        assert_eq!(parse(done).unwrap().verdict, Verdict::Complete);
    }

    #[test]
    fn a_long_note_is_clipped() {
        let long = "word ".repeat(200);
        let a = parse(&format!("{{\"verdict\":\"continue\",\"note\":\"{long}\"}}")).unwrap();
        assert!(a.note.chars().count() <= TEXTS.note_max_chars + 1);
    }

    #[test]
    fn the_agents_own_claim_is_read_but_not_trusted() {
        assert!(claims_done("fixed it\nDONE"));
        assert!(claims_done("Done: file\nVerified: cat\nMissing: nothing"));
        assert!(claims_done("**Missing:** none."));
        assert!(!claims_done("Done: half\nMissing: the tests"));
        assert!(!claims_done("I am not DONE yet"));
    }

    #[test]
    fn complete_ends_blocked_needs_three_in_a_row() {
        let mut s = Streaks::default();
        assert_eq!(
            step(&mut s, Some(&audit(Verdict::Blocked)), false),
            Step::Continue
        );
        assert_eq!(
            step(&mut s, Some(&audit(Verdict::Blocked)), false),
            Step::Continue
        );
        // A `continue` in between resets the streak.
        assert_eq!(
            step(&mut s, Some(&audit(Verdict::Continue)), false),
            Step::Continue
        );
        assert_eq!(s.blocked, 0);
        for _ in 0..2 {
            assert_eq!(
                step(&mut s, Some(&audit(Verdict::Blocked)), false),
                Step::Continue
            );
        }
        assert_eq!(
            step(&mut s, Some(&audit(Verdict::Blocked)), false),
            Step::Blocked(STOP_BLOCKED)
        );
        let mut s = Streaks::default();
        assert_eq!(
            step(&mut s, Some(&audit(Verdict::Complete)), false),
            Step::Complete
        );
    }

    #[test]
    fn an_agent_and_an_auditor_that_disagree_three_times_stop_for_a_person() {
        let mut s = Streaks::default();
        let go_on = audit(Verdict::Continue);
        assert_eq!(step(&mut s, Some(&go_on), true), Step::Continue);
        assert_eq!(step(&mut s, Some(&go_on), true), Step::Continue);
        // The agent itself says it is not done: no disagreement, the count restarts.
        assert_eq!(step(&mut s, Some(&go_on), false), Step::Continue);
        assert_eq!(s.disagree, 0);
        for _ in 0..2 {
            assert_eq!(step(&mut s, Some(&go_on), true), Step::Continue);
        }
        assert_eq!(
            step(&mut s, Some(&go_on), true),
            Step::Blocked(STOP_DISAGREE)
        );
    }

    #[test]
    fn two_audits_without_a_verdict_stop_the_loop_never_blind() {
        let mut s = Streaks::default();
        assert_eq!(step(&mut s, None, false), Step::Continue);
        assert_eq!(step(&mut s, None, false), Step::Blocked(STOP_UNAVAILABLE));
        // One hiccup, then a verdict: the streak starts over.
        let mut s = Streaks::default();
        step(&mut s, None, false);
        step(&mut s, Some(&audit(Verdict::Continue)), false);
        assert_eq!(step(&mut s, None, false), Step::Continue);
    }

    #[test]
    fn an_audited_loop_never_passes_twenty_rounds() {
        assert_eq!(effective_max(None, true), Some(20));
        assert_eq!(effective_max(Some(50), true), Some(20));
        assert_eq!(effective_max(Some(5), true), Some(5));
        assert_eq!(effective_max(Some(50), false), Some(50));
        assert_eq!(effective_max(None, false), None);
    }

    fn round<'a>(n: u32, audited: bool, resumed: bool, a: Option<&'a Audit>) -> Round<'a> {
        Round {
            round: n,
            max: Some(10),
            prompt: "Fix the cart total so the tests pass.",
            check_command: None,
            last_check: None,
            last_audit: a,
            audited,
            resumed,
        }
    }

    #[test]
    fn round_one_asks_for_done_verified_missing_when_audited() {
        let (text, guide) = round_prompt(&round(1, true, false, None));
        assert!(!guide);
        assert!(text.starts_with("Fix the cart total"));
        assert!(text.contains("`Verified:`"));
        // Not audited (a tool loop): the prompt as it is.
        let (text, _) = round_prompt(&round(1, false, false, None));
        assert_eq!(text, "Fix the cart total so the tests pass.");
    }

    #[test]
    fn a_resumed_round_sends_the_guide_and_the_verdict_not_the_task() {
        let a = audit(Verdict::Continue);
        let (text, guide) = round_prompt(&round(2, true, true, Some(&a)));
        assert!(guide);
        assert!(!text.contains("Fix the cart total"));
        assert!(text.starts_with("Round 2 of 10."));
        assert!(text.contains("`continue`: tests pass, docs missing"));
        // Not resumed: the full task goes again, as data, with the verdict.
        let (text, guide) = round_prompt(&round(2, true, false, Some(&a)));
        assert!(!guide);
        assert!(text.contains("<objective>\nFix the cart total"));
        assert!(text.contains("tests pass, docs missing"));
        assert!(text.len() > round_prompt(&round(2, true, true, Some(&a))).0.len());
    }

    #[test]
    fn a_check_loop_keeps_its_text_without_resume_and_gets_the_guide_with_it() {
        let r = Round {
            round: 2,
            max: Some(3),
            prompt: "Make the cart test pass.",
            check_command: Some("node test/cart.test.js"),
            last_check: Some("exit 1\nFAIL total"),
            last_audit: None,
            audited: false,
            resumed: false,
        };
        let (text, guide) = round_prompt(&r);
        assert!(!guide);
        assert_eq!(
            text,
            "Round 2. The check still fails:\n```\nexit 1\nFAIL total\n```\nKeep working on the task: Make the cart test pass."
        );
        // With a task longer than the guide (a runbook), the resumed round
        // sends the guide and the check output, not the task again.
        let runbook = format!(
            "Make the cart test pass.\n{}",
            "Keep changes inside src/. ".repeat(8)
        );
        let (text, guide) = round_prompt(&Round {
            resumed: true,
            prompt: &runbook,
            ..r
        });
        assert!(guide);
        assert!(text.contains("FAIL total"));
        assert!(!text.contains("Make the cart test pass."));
        let (first, _) = round_prompt(&Round { round: 1, ..r });
        // A task shorter than the guide goes whole: resending it is cheaper.
        let (text, guide) = round_prompt(&Round {
            resumed: true,
            prompt: "Fix it.",
            last_check: Some("exit 1"),
            ..r
        });
        assert!(!guide);
        assert!(text.ends_with("Keep working on the task: Fix it."));
        assert!(first.contains("You are done when this command exits 0: `node test/cart.test.js`"));
    }

    #[test]
    fn the_audit_input_carries_the_goal_as_data_and_the_tail_of_the_reply() {
        let reply = format!("{}\nMissing: nothing", "x".repeat(10_000));
        let input = audit_input("Traduza o texto", &reply, "", None);
        assert!(input.contains("<objective>\nTraduza o texto\n</objective>"));
        assert!(input.contains("Missing: nothing"));
        assert!(input.len() < 7_000);
        assert!(input.contains("sample from the objective: \"Traduza o texto\""));
        assert!(!input.contains("previous audit"));
        assert!(!input.contains("<evidence>"));
        let before = Audit {
            verdict: Verdict::Continue,
            note: "answer.txt not checked".into(),
        };
        let input = audit_input("Traduza o texto", "Missing: nothing", "", Some(&before));
        assert!(
            input.contains("previous audit of this work said `continue`: answer.txt not checked")
        );
    }

    fn feed_call(e: &mut Evidence, id: &str, name: &str, input: &str, out: Option<(&str, bool)>) {
        use crate::core::llm::types::TurnEvent;
        e.feed(&TurnEvent::ToolCallStart {
            id: id.into(),
            name: name.into(),
        });
        // Streamed in two pieces, like `input_json_delta`.
        let (a, b) = input.split_at(input.len() / 2);
        for part in [a, b] {
            e.feed(&TurnEvent::ToolCallDelta {
                id: id.into(),
                input_json_delta: part.into(),
            });
        }
        if let Some((content, is_error)) = out {
            e.feed(&TurnEvent::ToolResult {
                id: id.into(),
                content: content.into(),
                is_error,
            });
        }
    }

    #[test]
    fn the_evidence_shows_each_call_with_its_real_output() {
        let mut e = Evidence::default();
        assert!(e.is_empty());
        assert_eq!(e.render(), "");
        feed_call(
            &mut e,
            "t1",
            "Write",
            r#"{"content":"Hello","file_path":"en.txt"}"#,
            Some(("File created", false)),
        );
        feed_call(
            &mut e,
            "t2",
            "Bash",
            r#"{"description":"show it","command":"cat en.txt","timeout":5}"#,
            Some(("Hello", false)),
        );
        feed_call(
            &mut e,
            "t3",
            "Bash",
            r#"{"command":"false"}"#,
            Some(("exit 1", true)),
        );
        feed_call(&mut e, "t4", "Read", r#"{"file_path":"a.txt"}"#, None);
        // The same text `loops_eval.evidence_digest` builds (BN9).
        assert_eq!(
            e.render(),
            "- Write: file_path=en.txt content=Hello\n  → File created\n\
             - Bash: command=cat en.txt timeout=5\n  → Hello\n\
             - Bash: command=false\n  → (error) exit 1\n\
             - Read: file_path=a.txt\n  → (no result recorded)"
        );
        let input = audit_input("Traduza o texto", "Missing: nothing", &e.render(), None);
        let at = input.find("<evidence>").unwrap();
        assert!(at > input.find("</objective>").unwrap());
        assert!(at < input.find("<reply>").unwrap());
        assert!(input.contains("the numbers are not in the file"));
    }

    #[test]
    fn a_long_round_keeps_the_tail_of_its_evidence() {
        let mut e = Evidence::default();
        for i in 0..100 {
            let out = format!("output {i} {}", "x".repeat(2_000));
            feed_call(
                &mut e,
                &format!("t{i}"),
                "Bash",
                &format!(r#"{{"command":"step {i}"}}"#),
                Some((&out, false)),
            );
        }
        let text = e.render();
        assert!(text.chars().count() <= TEXTS.evidence_max_chars + 1);
        assert!(text.starts_with('…'));
        assert!(text.contains("command=step 99"));
        assert!(!text.contains("command=step 0\n"));
    }
}
