use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunnerSpec {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<RunnerSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub runner: RunnerSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub steps: Vec<PlaybookStep>,
    #[serde(default)]
    pub setup: Vec<String>,
}

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
