//! "Simulated" subagents for tools without native ones (Crush, Pi, Warp, Amp
//! without a plugin …), as rulesync does: the agent file goes to
//! `.agents/agents/<name>.md` (Claude format) and a marked block in the tool's
//! rules file (AGENTS.md) tells the model when to read it and act as that role.

use std::path::PathBuf;

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, PatchOp, PlannedFile};
use crate::core::agentkit::edit::DocFormat;
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

pub struct SimulatedAgentConverter;

/// Where simulated agents live when the manifest has no `paths.agents`.
/// `paths.simulated_agents` wins over `paths.agents` (Codebuff's `.agents/`
/// holds TypeScript agent definitions, not Markdown).
fn agents_dir(target: &TargetAdapter, scope: Scope, ctx: &ConvertCtx) -> Option<PathBuf> {
    kind_path(target, "simulated_agents", scope, ctx)
        .or_else(|| kind_path(target, "agents", scope, ctx))
        .or_else(|| match scope {
            Scope::Global | Scope::Managed => Some(ctx.env.home.join(".agents").join("agents")),
            _ => ctx.project.map(|p| p.join(".agents").join("agents")),
        })
}

/// The Claude-format agent file (raw when possible, so it matches what Goose,
/// OpenHands and friends read from the same folder).
pub(crate) fn claude_text(c: &Component, a: &AgentSpec, name: &str) -> String {
    if c.origin_tool == "claude" && name == parse::sanitize_name(&a.name) {
        if let Some(t) = c.entry_text() {
            return t.to_string();
        }
    }
    let mut spec = a.clone();
    let (tools, _) = claude_tools(&a.tools, &c.origin_tool);
    spec.tools = tools;
    if c.origin_tool == "copilot" {
        spec.model = None;
        spec.extra = Default::default();
    }
    parse::render_frontmatter(&super::claude::agent_frontmatter(&spec, name), &spec.prompt)
}

/// Text of the AGENTS.md block that points at the agent file.
pub fn pointer_block(name: &str, description: &str, shown_path: &str) -> String {
    let desc = parse::short_description(description);
    let mut s = format!("## Subagent `{name}`\n\n");
    if !desc.is_empty() {
        s.push_str(&format!("Use for: {desc}\n\n"));
    }
    s.push_str(&format!(
        "When a task matches, call the `{name}` subagent: read `{shown_path}` and follow it as that role until the task is done, then continue as before.\n"
    ));
    s
}

impl Converter for SimulatedAgentConverter {
    fn id(&self) -> &'static str {
        "agent_simulated"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Agent && target.format(kind) == Some("simulated")
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let Some(a) = agent_of(c) else {
            return Ok(vec![]);
        };
        let name = ctx.name_for(c);
        let dir = agents_dir(target, scope, ctx).ok_or_else(|| no_path(target, "agents", scope))?;
        // the AGENTS.md the tool reads (never a first-match file such as Zed's
        // `.rules`, which would shadow the user's AGENTS.md)
        let rules = super::rule_files::agents_md(target, scope, ctx)
            .map(Ok)
            .unwrap_or_else(|| dir_for(target, "rules", scope, ctx))?;
        let file = dir.join(format!("{name}.md"));
        let shown = match (scope, ctx.project) {
            (Scope::Project | Scope::Local, Some(p)) => {
                rel_slash(p, &file).unwrap_or_else(|| file.display().to_string())
            }
            _ => file.display().to_string(),
        };
        let mut agent = PlannedFile::write(&target.id, c, file, claude_text(c, a, &name), "agent");
        agent.primary = true;
        agent.losses.push(format!(
            "{} has no subagents: the agent runs inside the main conversation (no own context, tools or model)",
            target.name
        ));
        let block = PlannedFile::merge(
            &target.id,
            c,
            rules,
            DocFormat::Markdown,
            vec![PatchOp::TextBlock {
                id: format!("{}:{}:subagent", c.id, name),
                content: pointer_block(&name, &a.description, &shown),
            }],
            "agent pointer",
        );
        Ok(vec![agent, block])
    }

    fn import(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        if kind != ComponentKind::Agent {
            return Ok(vec![]);
        }
        let Some(dir) = agents_dir(target, scope, ctx) else {
            return Ok(vec![]);
        };
        let mut out = Vec::new();
        for p in files_with(&dir, &[".md"]) {
            if let Ok(mut comp) = parse::parse_path(ComponentKind::Agent, &p) {
                super::claude::tag_installed(&mut comp, target, &p);
                out.push(comp);
            }
        }
        Ok(out)
    }
}
