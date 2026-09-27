//! Playbooks: a `workflow` item of the catalog (steps with an agent and a tool
//! per step) turned into a chain of Jobs, where the output of each step becomes
//! the context of the next. cct only wrote the workflow YAML for Claude to read
//! (estudo 75 01 §2.10); here the host's Jobs engine runs the steps.

use serde::{Deserialize, Serialize};

use crate::core::agentkit::model::{Component, ComponentBody};

/// Who runs a step (or a loop round, or "run now").
/// - `agent`: an agent of the OmniGet roster (`id`), native or not;
/// - `account`: a Claude/Codex account of `cli_runtime::accounts` (`id` = cli,
///   `account` = account id; empty = the terminal's own login);
/// - `acp`: an ACP agent of the drivers table (`id` = `gemini`, `qwen` …);
/// - `tool`: a coding CLI run headless (`id` = `claude`, `codex` …).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunnerSpec {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `default | plan | accept_edits | bypass` (tool runners only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

impl RunnerSpec {
    pub fn label(&self) -> String {
        match self.kind.as_str() {
            "tool" => format!("{} (CLI)", self.id),
            "account" => format!(
                "{} · {}",
                self.id,
                self.account
                    .clone()
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| "default".into())
            ),
            "acp" => format!("{} (ACP)", self.id),
            _ => self.id.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaybookStep {
    pub name: String,
    /// Runner of this step; `None` = the playbook's runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<RunnerSpec>,
    /// Agent used as the role (catalog id, `path:agent:<file>`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Command whose body is the step's prompt (catalog id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playbook {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Catalog id of the workflow it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub runner: RunnerSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub steps: Vec<PlaybookStep>,
    /// MCP servers and other components the steps need, installed before.
    #[serde(default)]
    pub setup: Vec<String>,
}

/// `agent:dev/test-runner` inside `cct:workflows/x` → `cct:agents/dev/test-runner`.
pub fn reference_id(reference: &str, from_id: &str) -> String {
    if reference.starts_with("path:")
        || reference.contains(":agents/")
        || reference.contains(":commands/")
        || reference.contains(":mcps/")
    {
        return reference.to_string();
    }
    let source = from_id.split_once(':').map(|(s, _)| s).unwrap_or("cct");
    match reference.split_once(':') {
        Some((kind, path)) => {
            let plural = match kind {
                "mcp" | "mcps" => "mcps".to_string(),
                k if k.ends_with('s') => k.to_string(),
                k => format!("{k}s"),
            };
            format!("{source}:{plural}/{path}")
        }
        None => reference.to_string(),
    }
}

/// A workflow component → a playbook for `runner`.
pub fn from_workflow(
    c: &Component,
    runner: RunnerSpec,
    workspace: Option<String>,
) -> Result<Playbook, String> {
    let ComponentBody::Workflow(w) = &c.body else {
        return Err(format!(
            "{}: {} is a {}, not a workflow",
            super::runner::ERR_RUN,
            c.id,
            c.kind
        ));
    };
    let mut steps = Vec::new();
    let mut setup = Vec::new();
    for (i, s) in w.steps.iter().enumerate() {
        let n = i + 1;
        let prompt = s.prompt.clone().unwrap_or_default();
        match s.kind.as_str() {
            "mcp" => {
                if let Some(r) = &s.reference {
                    setup.push(reference_id(&format!("mcp:{r}"), &c.id));
                }
            }
            "agent" => steps.push(PlaybookStep {
                name: s.reference.clone().unwrap_or_else(|| format!("step {n}")),
                agent: s.reference.as_ref().map(|r| {
                    reference_id(
                        if r.contains(':') {
                            r.clone()
                        } else {
                            format!("agent:{r}")
                        }
                        .as_str(),
                        &c.id,
                    )
                }),
                prompt: if prompt.is_empty() {
                    format!("Do your part of the workflow \"{}\".", c.name)
                } else {
                    prompt
                },
                ..Default::default()
            }),
            "command" => steps.push(PlaybookStep {
                name: s.reference.clone().unwrap_or_else(|| format!("step {n}")),
                command: s.reference.as_ref().map(|r| {
                    reference_id(
                        if r.contains(':') {
                            r.clone()
                        } else {
                            format!("command:{r}")
                        }
                        .as_str(),
                        &c.id,
                    )
                }),
                prompt,
                ..Default::default()
            }),
            _ => {
                if !prompt.trim().is_empty() {
                    steps.push(PlaybookStep {
                        name: format!("step {n}"),
                        prompt,
                        ..Default::default()
                    })
                }
            }
        }
    }
    if steps.is_empty() {
        return Err(format!(
            "{}: workflow {} has no runnable step",
            super::runner::ERR_RUN,
            c.name
        ));
    }
    Ok(Playbook {
        name: c.name.clone(),
        description: c.description.clone(),
        source: Some(c.id.clone()),
        runner,
        workspace,
        steps,
        setup,
    })
}

/// Longest output of a previous step carried into the next prompt.
pub const CARRY_MAX: usize = 8_000;

fn tail(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    &s[cut..]
}

/// Prompt of step `index` given what the earlier steps answered.
pub fn step_prompt(
    playbook: &Playbook,
    index: usize,
    task: &str,
    previous: &[(String, String)],
) -> String {
    let step = &playbook.steps[index];
    let mut s = format!(
        "You are step {} of {} of the playbook \"{}\".\n",
        index + 1,
        playbook.steps.len(),
        playbook.name
    );
    if !playbook.description.trim().is_empty() {
        s.push_str(&format!("Playbook goal: {}\n", playbook.description.trim()));
    }
    if !previous.is_empty() {
        s.push_str("\nWhat the earlier steps reported:\n");
        for (name, out) in previous {
            s.push_str(&format!("\n### {name}\n{}\n", tail(out.trim(), CARRY_MAX)));
        }
    }
    s.push_str(&format!("\nYour step: {}\n", step.name));
    if !task.trim().is_empty() {
        s.push_str(&format!("\n{}\n", task.trim()));
    }
    s.push_str("\nEnd with a short report of what you did and what the next step needs to know.");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_outputs_become_context() {
        let pb = Playbook {
            name: "ship".into(),
            description: "Ship the feature".into(),
            runner: RunnerSpec {
                kind: "tool".into(),
                id: "claude".into(),
                ..Default::default()
            },
            steps: vec![
                PlaybookStep {
                    name: "plan".into(),
                    prompt: "Plan it".into(),
                    ..Default::default()
                },
                PlaybookStep {
                    name: "build".into(),
                    prompt: "Build it".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let p = step_prompt(
            &pb,
            1,
            "Build it",
            &[("plan".into(), "1. do x\n2. do y".into())],
        );
        assert!(
            p.contains("step 2 of 2") && p.contains("### plan\n1. do x") && p.contains("Build it")
        );
        assert_eq!(
            reference_id("agent:dev/test-runner", "cct:workflows/a"),
            "cct:agents/dev/test-runner"
        );
        assert_eq!(
            reference_id("mcp:devtools/github", "cct:workflows/a"),
            "cct:mcps/devtools/github"
        );
    }
}
