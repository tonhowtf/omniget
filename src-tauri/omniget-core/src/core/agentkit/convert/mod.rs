pub mod agent_common;
pub mod agent_kiro;
pub mod agent_md;
pub mod agent_roo;
pub mod agent_simulated;
pub mod agent_toml;
pub mod claude;
pub mod command_gemini;
pub mod command_goose;
pub mod command_md;
pub mod command_rovo;
pub mod hook_cline;
pub mod hook_code_plugin;
pub mod hook_common;
pub mod hook_convert;
pub mod hook_json;
pub mod hook_observe;
pub mod hook_shim;
pub mod hook_toml;
pub mod loop_runbook;
pub mod mcp;
pub mod plugin_convert;
pub mod rule_files;
pub mod setting_perm;
pub mod setting_tools;
pub mod skill_link;
pub mod statusline_stdin;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::edit::{DocFormat, Seg};
use super::model::{Compat, Component, ComponentKind};
use super::targets::TargetAdapter;
use super::{Env, Result, Scope};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum Dedupe {
    Equal,
    Fields { fields: Vec<String> },
    ClaudeHook,
    FlatHook,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOp {
    Set {
        path: Vec<Seg>,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rename_at: Option<usize>,
    },
    Append {
        path: Vec<Seg>,
        value: Value,
        dedupe: Dedupe,
    },
    TextBlock {
        id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    Write,
    Merge,
    Link,
}

pub type SupportPlan = (Vec<PlannedFile>, Vec<(String, String)>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedFile {
    pub target: String,
    pub component_id: String,
    pub path: PathBuf,
    pub action: FileAction,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "opt_bytes")]
    pub content: Option<Vec<u8>>,
    #[serde(default)]
    pub executable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DocFormat>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<PatchOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_to: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_root: Option<PathBuf>,
    #[serde(default)]
    pub primary: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    #[serde(default)]
    pub label: String,
}

mod opt_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        use base64::Engine;
        match v {
            None => s.serialize_none(),
            Some(b) => match std::str::from_utf8(b) {
                Ok(t) => serde_json::json!({ "text": t }).serialize(s),
                Err(_) => serde_json::json!({ "base64": base64::engine::general_purpose::STANDARD.encode(b) }).serialize(s),
            },
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        use base64::Engine;
        let v: Option<serde_json::Value> = Option::deserialize(d)?;
        Ok(match v {
            None => None,
            Some(v) => {
                if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                    Some(t.as_bytes().to_vec())
                } else if let Some(b) = v.get("base64").and_then(|t| t.as_str()) {
                    Some(
                        base64::engine::general_purpose::STANDARD
                            .decode(b)
                            .map_err(serde::de::Error::custom)?,
                    )
                } else {
                    None
                }
            }
        })
    }
}

impl PlannedFile {
    pub fn write(
        target: &str,
        c: &Component,
        path: PathBuf,
        content: impl Into<Vec<u8>>,
        label: &str,
    ) -> PlannedFile {
        PlannedFile {
            target: target.to_string(),
            component_id: c.id.clone(),
            path,
            action: FileAction::Write,
            content: Some(content.into()),
            executable: false,
            format: None,
            ops: vec![],
            link_to: None,
            unit_root: None,
            primary: false,
            losses: vec![],
            notes: vec![],
            commands: vec![],
            label: label.to_string(),
        }
    }

    pub fn merge(
        target: &str,
        c: &Component,
        path: PathBuf,
        format: DocFormat,
        ops: Vec<PatchOp>,
        label: &str,
    ) -> PlannedFile {
        PlannedFile {
            target: target.to_string(),
            component_id: c.id.clone(),
            path,
            action: FileAction::Merge,
            content: None,
            executable: false,
            format: Some(format),
            ops,
            link_to: None,
            unit_root: None,
            primary: false,
            losses: vec![],
            notes: vec![],
            commands: vec![],
            label: label.to_string(),
        }
    }
}

impl PlannedFile {
    pub fn link(
        target: &str,
        c: &Component,
        path: PathBuf,
        to: PathBuf,
        label: &str,
    ) -> PlannedFile {
        PlannedFile {
            target: target.to_string(),
            component_id: c.id.clone(),
            unit_root: Some(path.clone()),
            path,
            action: FileAction::Link,
            content: None,
            executable: false,
            format: None,
            ops: vec![],
            link_to: Some(to),
            primary: true,
            losses: vec![],
            notes: vec![],
            commands: vec![],
            label: label.to_string(),
        }
    }
}

pub struct ConvertCtx<'a> {
    pub env: &'a Env,
    pub project: Option<&'a Path>,
    pub scope: Scope,
    pub claude: &'a TargetAdapter,
    pub name_override: Option<String>,
    pub secret_values: &'a BTreeMap<String, String>,
}

impl ConvertCtx<'_> {
    pub fn name_for(&self, c: &Component) -> String {
        self.name_override
            .clone()
            .unwrap_or_else(|| super::parse::sanitize_name(&c.name))
    }
}

pub trait Converter: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool;
    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>>;
    fn import(
        &self,
        _kind: ComponentKind,
        _target: &TargetAdapter,
        _scope: Scope,
        _ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        Ok(vec![])
    }
    fn native_for(&self, _c: &Component, _target: &TargetAdapter) -> bool {
        false
    }
}

pub enum Resolution<'r> {
    Use {
        converter: &'r dyn Converter,
        via_claude: bool,
    },
    Unsupported(String),
}

pub struct Registry {
    converters: Vec<Box<dyn Converter>>,
}

impl Registry {
    pub fn builtin() -> Registry {
        Registry {
            converters: vec![
                Box::new(mcp::McpConverter),
                Box::new(skill_link::SkillLinkConverter),
                Box::new(claude::SkillDirConverter),
                Box::new(claude::ClaudeConverter::own_paths()),
                Box::new(setting_tools::SettingConverter),
                Box::new(statusline_stdin::StatuslineConverter),
                Box::new(loop_runbook::LoopConverter),
                Box::new(hook_convert::HookConverter),
                Box::new(plugin_convert::PluginConverter),
                Box::new(agent_toml::TomlAgentConverter {
                    format: "codex_toml",
                }),
                Box::new(agent_toml::TomlAgentConverter {
                    format: "vibe_toml",
                }),
                Box::new(agent_kiro::KiroAgentConverter),
                Box::new(agent_roo::RooModesConverter),
                Box::new(agent_simulated::SimulatedAgentConverter),
                Box::new(agent_md::MdAgentConverter {
                    format: "gemini_md",
                }),
                Box::new(agent_md::MdAgentConverter { format: "qwen_md" }),
                Box::new(agent_md::MdAgentConverter {
                    format: "opencode_md",
                }),
                Box::new(agent_md::MdAgentConverter {
                    format: "cursor_md",
                }),
                Box::new(agent_md::MdAgentConverter {
                    format: "copilot_agent_md",
                }),
                Box::new(agent_md::MdAgentConverter { format: "devin_md" }),
                Box::new(agent_md::MdAgentConverter { format: "droid_md" }),
                Box::new(agent_md::MdAgentConverter { format: "cline_md" }),
                Box::new(agent_md::MdAgentConverter {
                    format: "augment_md",
                }),
                Box::new(agent_md::MdAgentConverter {
                    format: "antigravity",
                }),
                Box::new(agent_md::MdAgentConverter {
                    format: "openhands_md",
                }),
                Box::new(agent_md::MdAgentConverter { format: "rovo" }),
                Box::new(command_gemini::GeminiCommandConverter),
                Box::new(command_goose::GooseRecipeConverter),
                Box::new(command_rovo::RovoPromptsConverter),
                Box::new(command_md::SkillCommandConverter),
                Box::new(command_md::CodexPromptConverter),
                Box::new(command_md::MdCommandConverter { format: "qwen_md" }),
                Box::new(command_md::MdCommandConverter {
                    format: "opencode_md",
                }),
                Box::new(command_md::MdCommandConverter {
                    format: "cursor_md",
                }),
                Box::new(command_md::MdCommandConverter {
                    format: "copilot_prompt",
                }),
                Box::new(command_md::MdCommandConverter {
                    format: "cline_workflow",
                }),
                Box::new(command_md::MdCommandConverter {
                    format: "devin_workflow",
                }),
                Box::new(command_md::MdCommandConverter {
                    format: "antigravity_workflow",
                }),
                Box::new(command_md::MdCommandConverter { format: "crush_md" }),
                Box::new(command_md::MdCommandConverter {
                    format: "kiro_prompt",
                }),
                Box::new(command_md::MdCommandConverter {
                    format: "pi_prompt",
                }),
                Box::new(command_md::MdCommandConverter {
                    format: "continue_prompt",
                }),
                Box::new(command_md::MdCommandConverter { format: "junie" }),
                Box::new(command_md::MdCommandConverter { format: "roo" }),
                Box::new(rule_files::RuleConverter),
            ],
        }
    }

    pub fn global() -> &'static Registry {
        static CELL: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
        CELL.get_or_init(Registry::builtin)
    }

    pub fn converters(&self) -> impl Iterator<Item = &dyn Converter> {
        self.converters.iter().map(|b| b.as_ref())
    }

    pub fn by_id(&self, id: &str) -> Option<&dyn Converter> {
        self.converters().find(|c| c.id() == id)
    }

    pub fn resolve(&self, kind: ComponentKind, target: &TargetAdapter) -> Resolution<'_> {
        if kind == ComponentKind::Mcp {
            return match self.by_id("mcp") {
                Some(c) if c.supports(kind, target) => Resolution::Use {
                    converter: c,
                    via_claude: false,
                },
                _ => Resolution::Unsupported(format!("{} has no MCP config file", target.name)),
            };
        }
        if matches!(kind, ComponentKind::Stack) {
            return Resolution::Unsupported("a stack is expanded into its components".into());
        }
        let fmt = target.format(kind);
        let claude = self.by_id("claude").expect("claude converter is built in");
        if fmt == Some("claude") && claude::claude_supports(kind) {
            return Resolution::Use {
                converter: claude,
                via_claude: false,
            };
        }
        if kind == ComponentKind::Skill && fmt == Some("skill_dir") {
            if let Some(c) = self.by_id("skill_dir") {
                return Resolution::Use {
                    converter: c,
                    via_claude: false,
                };
            }
        }
        let own_rule = kind == ComponentKind::Rule && self.own_converter(kind, target).is_some();
        if !own_rule && target.reads_claude(kind) && claude::claude_supports(kind) {
            return Resolution::Use {
                converter: claude,
                via_claude: true,
            };
        }
        if let Some(c) = self.own_converter(kind, target) {
            return Resolution::Use {
                converter: c,
                via_claude: false,
            };
        }
        match fmt {
            Some(f) => Resolution::Unsupported(format!(
                "{} → {} format `{f}`: converter not available yet",
                kind, target.name
            )),
            None => Resolution::Unsupported(format!("{} has no {}", target.name, kind)),
        }
    }

    pub fn own_converter(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
    ) -> Option<&dyn Converter> {
        self.converters()
            .find(|c| !matches!(c.id(), "claude" | "mcp" | "skill_dir") && c.supports(kind, target))
    }

    pub fn resolve_for(&self, c: &Component, target: &TargetAdapter) -> Resolution<'_> {
        let kind = c.kind;
        if !matches!(
            kind,
            ComponentKind::Mcp | ComponentKind::Stack | ComponentKind::Skill
        ) && !c.origin_tool.is_empty()
            && c.origin_tool == target.id
            && target.format(kind).is_some_and(|f| f != "claude")
        {
            if let Some(conv) = self.own_converter(kind, target) {
                return Resolution::Use {
                    converter: conv,
                    via_claude: false,
                };
            }
        }
        self.resolve(kind, target)
    }

    pub fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        ctx: &ConvertCtx,
    ) -> Result<Conversion> {
        match self.resolve_for(c, target) {
            Resolution::Unsupported(reason) => Ok(Conversion {
                files: vec![],
                compat: Compat::Unsupported { reason },
            }),
            Resolution::Use {
                converter,
                via_claude,
            } => {
                let files = if via_claude {
                    claude::ClaudeConverter::via(ctx.claude).convert(c, target, ctx.scope, ctx)?
                } else {
                    converter.convert(c, target, ctx.scope, ctx)?
                };
                let mut lost: Vec<String> = Vec::new();
                for f in &files {
                    for l in &f.losses {
                        if !lost.contains(l) {
                            lost.push(l.clone());
                        }
                    }
                }
                let compat = if files.is_empty() {
                    Compat::Unsupported {
                        reason: format!("nothing to write for {} on {}", c.kind, target.name),
                    }
                } else if !lost.is_empty() {
                    Compat::Degraded { lost }
                } else if via_claude || converter.native_for(c, target) {
                    Compat::Native
                } else {
                    Compat::Converted
                };
                Ok(Conversion { files, compat })
            }
        }
    }
}

pub fn import_installed(
    env: &Env,
    target_id: &str,
    scope: Scope,
    project: Option<&Path>,
) -> Result<Vec<Component>> {
    let all = super::targets::load_targets(env);
    let target = super::targets::find(&all, target_id)?;
    let claude = super::targets::find(&all, "claude")?;
    let empty = BTreeMap::new();
    let ctx = ConvertCtx {
        env,
        project,
        scope,
        claude,
        name_override: None,
        secret_values: &empty,
    };
    let reg = Registry::global();
    let mut out: Vec<Component> = Vec::new();
    for kind in [
        ComponentKind::Agent,
        ComponentKind::Command,
        ComponentKind::Skill,
        ComponentKind::Hook,
        ComponentKind::Mcp,
    ] {
        let found = match reg.resolve(kind, target) {
            Resolution::Use {
                converter,
                via_claude: true,
            } => {
                let _ = converter;
                claude::ClaudeConverter::via(claude).import(kind, target, scope, &ctx)?
            }
            Resolution::Use { converter, .. } => converter.import(kind, target, scope, &ctx)?,
            Resolution::Unsupported(_) => vec![],
        };
        for c in found {
            if !out.iter().any(|x| x.id == c.id) {
                out.push(c);
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Conversion {
    pub files: Vec<PlannedFile>,
    pub compat: Compat,
}

pub fn compute_compat(c: &Component, targets: &[TargetAdapter]) -> BTreeMap<String, Compat> {
    let env = Env::sandbox(
        Path::new("/agentkit-compat/home"),
        Env::system().map(|e| e.os).unwrap_or(super::Os::Linux),
    );
    let project = PathBuf::from("/agentkit-compat/project");
    let claude = targets
        .iter()
        .find(|t| t.id == "claude")
        .cloned()
        .or_else(|| super::targets::target("claude").cloned());
    let Some(claude) = claude else {
        return BTreeMap::new();
    };
    let empty = BTreeMap::new();
    let mut out = BTreeMap::new();
    for t in targets {
        let scope = if t.supports_scope(Scope::Project) {
            Scope::Project
        } else {
            Scope::Global
        };
        let ctx = ConvertCtx {
            env: &env,
            project: Some(&project),
            scope,
            claude: &claude,
            name_override: None,
            secret_values: &empty,
        };
        let compat = match Registry::global().convert(c, t, &ctx) {
            Ok(conv) => conv.compat,
            Err(e) => Compat::Unsupported {
                reason: e.to_string(),
            },
        };
        out.insert(t.id.clone(), compat);
    }
    out
}

pub fn kind_path(
    target: &TargetAdapter,
    key: &str,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Option<PathBuf> {
    let t = target.path_template(key, scope, ctx.env.os).or_else(|| {
        if scope == Scope::Local {
            target.path_template(key, Scope::Project, ctx.env.os)
        } else {
            None
        }
    })?;
    ctx.env.expand(t, ctx.project)
}

pub fn place_support(
    dest: &str,
    target: &TargetAdapter,
    dir_key: &str,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Option<(PathBuf, String)> {
    let dest = dest.trim();
    if let Some(rest) = dest.strip_prefix("~/") {
        let (abs, _) = map_claude_prefix(rest, target, dir_key, Scope::Global, ctx)
            .unwrap_or_else(|| (ctx.env.home.join(rest), String::new()));
        let shown = abs.display().to_string();
        return Some((abs, shown));
    }
    let rel = dest.trim_start_matches("./");
    if scope == Scope::Global || scope == Scope::Managed {
        let (abs, _) = map_claude_prefix(rel, target, dir_key, Scope::Global, ctx)?;
        let shown = abs.display().to_string();
        return Some((abs, shown));
    }
    let (abs, shown) = map_claude_prefix(rel, target, dir_key, Scope::Project, ctx)?;
    Some((abs, shown))
}

fn map_claude_prefix(
    rel: &str,
    target: &TargetAdapter,
    dir_key: &str,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Option<(PathBuf, String)> {
    let parts: Vec<&str> = rel.split('/').collect();
    let (dir_tpl, rest) = if parts.len() >= 3 && parts[0] == ".claude" {
        let tpl = target
            .path_template(dir_key, scope, ctx.env.os)
            .or_else(|| ctx.claude.path_template(dir_key, scope, ctx.env.os));
        (tpl.map(str::to_string), parts[2..].join("/"))
    } else {
        (None, rel.to_string())
    };
    match dir_tpl {
        Some(tpl) => {
            let base = ctx.env.expand(&tpl, ctx.project)?;
            let abs = rest.split('/').fold(base, |p, s| p.join(s));
            let shown = if scope == Scope::Project && !tpl.starts_with('~') && !tpl.starts_with('{')
            {
                format!("{}/{}", tpl.trim_end_matches('/'), rest)
            } else {
                abs.display().to_string()
            };
            Some((abs, shown))
        }
        None => {
            if scope == Scope::Project {
                let base = ctx.project?.to_path_buf();
                let abs = rest.split('/').fold(base, |p, s| p.join(s));
                Some((abs, rest))
            } else {
                let abs = rest.split('/').fold(ctx.env.home.clone(), |p, s| p.join(s));
                let shown = abs.display().to_string();
                Some((abs, shown))
            }
        }
    }
}

pub fn rewrite_command(cmd: &str, from: &str, to: &str) -> String {
    if from == to || from.is_empty() {
        return cmd.to_string();
    }
    let to_q = if to.contains(' ') && !to.starts_with('"') {
        format!("\"{to}\"")
    } else {
        to.to_string()
    };
    let mut out = cmd.to_string();
    for pre in [
        "\"$CLAUDE_PROJECT_DIR\"/",
        "$CLAUDE_PROJECT_DIR/",
        "${CLAUDE_PROJECT_DIR}/",
    ] {
        let pat = format!("{pre}{from}");
        if out.contains(&pat) {
            let repl = if Path::new(to).is_absolute() {
                to_q.clone()
            } else {
                format!("{pre}{to}")
            };
            out = out.replace(&pat, &repl);
        }
    }
    out.replace(from, &to_q)
}

pub fn format_for_path(p: &Path) -> DocFormat {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "toml" => DocFormat::Toml,
        "yaml" | "yml" => DocFormat::Yaml,
        "jsonc" => DocFormat::Jsonc,
        "json" => {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "settings.json" && p.to_string_lossy().contains("Code")
                || name == "opencode.json"
                || name == "kilo.json"
            {
                DocFormat::Jsonc
            } else {
                DocFormat::Json
            }
        }
        _ => DocFormat::Markdown,
    }
}
