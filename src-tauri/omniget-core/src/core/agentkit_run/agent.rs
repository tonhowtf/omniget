use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::agentkit::model::{Component, ComponentBody, ComponentKind};
use crate::core::agentkit::parse::{self, RawFiles};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentPrompt {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub model: Option<String>,
    pub tools: Vec<String>,
}

pub fn agent_prompt(c: &Component) -> Result<AgentPrompt, String> {
    match &c.body {
        ComponentBody::Agent(a) => Ok(AgentPrompt {
            id: c.id.clone(),
            name: if a.name.is_empty() {
                c.name.clone()
            } else {
                a.name.clone()
            },
            description: a.description.clone(),
            system_prompt: a.prompt.trim().to_string(),
            model: a.model.clone().filter(|m| m != "inherit"),
            tools: a.tools.clone(),
        }),
        ComponentBody::Rule(r) => Ok(AgentPrompt {
            id: c.id.clone(),
            name: c.name.clone(),
            description: r.description.clone(),
            system_prompt: r.markdown.trim().to_string(),
            model: None,
            tools: vec![],
        }),
        ComponentBody::Skill(_) => {
            let text = c.entry_text().unwrap_or_default();
            let (_, body) = parse::frontmatter(text);
            Ok(AgentPrompt {
                id: c.id.clone(),
                name: c.name.clone(),
                description: c.description.clone(),
                system_prompt: body.trim().to_string(),
                model: None,
                tools: vec![],
            })
        }
        _ => Err(format!(
            "{}: {} is a {}, not an agent",
            super::runner::ERR_RUN,
            c.id,
            c.kind
        )),
    }
}

pub fn command_prompt(c: &Component, args: &str) -> Option<String> {
    let ComponentBody::Command(cmd) = &c.body else {
        return None;
    };
    let mut text = cmd.body.replace("{{args}}", args);
    for i in 1..10 {
        text = text.replace(
            &format!("{{{{arg:{i}}}}}"),
            args.split_whitespace().nth(i - 1).unwrap_or(""),
        );
    }
    Some(text.trim().to_string())
}

pub fn agent_from_file(path: &Path) -> Result<AgentPrompt, String> {
    let c = parse::parse_path(ComponentKind::Agent, path).map_err(String::from)?;
    agent_prompt(&c)
}

pub fn agent_from_markdown(name: &str, markdown: &str) -> Result<AgentPrompt, String> {
    let file = format!("{}.md", parse::sanitize_name(name));
    let files: RawFiles = [(file.clone(), markdown.as_bytes().to_vec())]
        .into_iter()
        .collect();
    match parse::parse_raw(ComponentKind::Agent, &file, &files) {
        Ok(c) => agent_prompt(&c),
        Err(_) => Ok(AgentPrompt {
            id: format!("file:{name}"),
            name: name.to_string(),
            system_prompt: parse::frontmatter(markdown).1.trim().to_string(),
            ..Default::default()
        }),
    }
}
