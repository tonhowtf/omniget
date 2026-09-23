//! Canonical, tool-neutral shape of every catalog component (plan §4.1).
//!
//! A [`Component`] carries its raw files (so a converter can always fall back to
//! the original bytes) plus a typed [`ComponentBody`] per kind. Command bodies use
//! neutral markers (`{{args}}`, `{{arg:1}}`, `{{shell:cmd}}`, `{{file:path}}`),
//! see [`crate::core::agentkit::parse::placeholders`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Every kind the catalog knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Rule,
    Agent,
    Command,
    Skill,
    Mcp,
    Hook,
    Setting,
    Statusline,
    Loop,
    Workflow,
    Mod,
    Plugin,
    #[serde(alias = "template")]
    ProjectTemplate,
    #[serde(alias = "sandbox")]
    SandboxRecipe,
    Stack,
}

impl ComponentKind {
    pub const ALL: [ComponentKind; 15] = [
        ComponentKind::Rule,
        ComponentKind::Agent,
        ComponentKind::Command,
        ComponentKind::Skill,
        ComponentKind::Mcp,
        ComponentKind::Hook,
        ComponentKind::Setting,
        ComponentKind::Statusline,
        ComponentKind::Loop,
        ComponentKind::Workflow,
        ComponentKind::Mod,
        ComponentKind::Plugin,
        ComponentKind::ProjectTemplate,
        ComponentKind::SandboxRecipe,
        ComponentKind::Stack,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ComponentKind::Rule => "rule",
            ComponentKind::Agent => "agent",
            ComponentKind::Command => "command",
            ComponentKind::Skill => "skill",
            ComponentKind::Mcp => "mcp",
            ComponentKind::Hook => "hook",
            ComponentKind::Setting => "setting",
            ComponentKind::Statusline => "statusline",
            ComponentKind::Loop => "loop",
            ComponentKind::Workflow => "workflow",
            ComponentKind::Mod => "mod",
            ComponentKind::Plugin => "plugin",
            ComponentKind::ProjectTemplate => "project_template",
            ComponentKind::SandboxRecipe => "sandbox_recipe",
            ComponentKind::Stack => "stack",
        }
    }

    /// Accepts the catalog spelling (`template`, `sandbox`, plurals).
    pub fn parse(s: &str) -> Option<ComponentKind> {
        let s = s.trim().to_ascii_lowercase();
        let s = s.strip_suffix('s').unwrap_or(&s).to_string();
        Some(match s.as_str() {
            "rule" => ComponentKind::Rule,
            "agent" => ComponentKind::Agent,
            "command" => ComponentKind::Command,
            "skill" => ComponentKind::Skill,
            "mcp" => ComponentKind::Mcp,
            "hook" => ComponentKind::Hook,
            "setting" => ComponentKind::Setting,
            "statusline" => ComponentKind::Statusline,
            "loop" => ComponentKind::Loop,
            "workflow" => ComponentKind::Workflow,
            "mod" => ComponentKind::Mod,
            "plugin" => ComponentKind::Plugin,
            "template" | "project_template" | "projecttemplate" => ComponentKind::ProjectTemplate,
            "sandbox" | "sandbox_recipe" | "sandboxrecipe" => ComponentKind::SandboxRecipe,
            "stack" => ComponentKind::Stack,
            _ => return None,
        })
    }

    /// Key used in a target manifest's `[formats]`, `[paths]` and `[reads_claude]`.
    pub fn format_key(self) -> &'static str {
        match self {
            ComponentKind::ProjectTemplate => "template",
            ComponentKind::SandboxRecipe => "sandbox",
            other => other.as_str(),
        }
    }

    /// Path-table key where files of this kind go.
    pub fn paths_key(self) -> &'static str {
        match self {
            ComponentKind::Rule => "rules",
            ComponentKind::Agent => "agents",
            ComponentKind::Command => "commands",
            ComponentKind::Skill => "skills",
            ComponentKind::Mcp => "mcp",
            ComponentKind::Hook => "hooks",
            ComponentKind::Setting => "settings",
            ComponentKind::Statusline => "statusline",
            ComponentKind::Loop => "loops",
            ComponentKind::Workflow => "workflows",
            ComponentKind::Mod => "mods",
            ComponentKind::Plugin => "plugins",
            ComponentKind::ProjectTemplate => "templates",
            ComponentKind::SandboxRecipe => "sandbox",
            ComponentKind::Stack => "stacks",
        }
    }
}

impl std::fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a component came from.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SourceRef {
    /// `cct`, `local`, `git`, `installed:<target>` …
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Fidelity of a component on one target (plan §4.1 `compat`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Compat {
    /// The tool reads this exact format (Claude file written as-is).
    Native,
    /// Converted to the tool's own format without losing anything we know of.
    Converted,
    /// Converted, but these parts do not survive.
    Degraded { lost: Vec<String> },
    /// Cannot be installed on this tool (yet).
    Unsupported { reason: String },
}

/// One raw file of the component, relative to the component folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub executable: bool,
}

#[derive(Serialize, Deserialize)]
struct ComponentFileWire {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base64: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    executable: bool,
}

impl Serialize for ComponentFile {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use base64::Engine;
        let wire = match std::str::from_utf8(&self.bytes) {
            Ok(t) => ComponentFileWire {
                path: self.path.clone(),
                text: Some(t.to_string()),
                base64: None,
                executable: self.executable,
            },
            Err(_) => ComponentFileWire {
                path: self.path.clone(),
                text: None,
                base64: Some(base64::engine::general_purpose::STANDARD.encode(&self.bytes)),
                executable: self.executable,
            },
        };
        wire.serialize(s)
    }
}

impl<'de> Deserialize<'de> for ComponentFile {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use base64::Engine;
        let w = ComponentFileWire::deserialize(d)?;
        let bytes = match (w.text, w.base64) {
            (Some(t), _) => t.into_bytes(),
            (None, Some(b)) => base64::engine::general_purpose::STANDARD
                .decode(b)
                .map_err(serde::de::Error::custom)?,
            (None, None) => Vec::new(),
        };
        Ok(ComponentFile {
            path: w.path,
            bytes,
            executable: w.executable,
        })
    }
}

impl ComponentFile {
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }
}

/// A canonical component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    /// Catalog id (`cct:agents/development-team/frontend-developer`) or a derived
    /// `local:<kind>/<name>` id.
    pub id: String,
    pub kind: ComponentKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// SHA-256 over every file (path + bytes, sorted by path).
    #[serde(default)]
    pub sha256: String,
    /// Tool whose native format this is (`claude` for almost all of cct; `copilot`
    /// for the chatmode agents).
    #[serde(default = "default_origin")]
    pub origin_tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<Value>,
    /// Filled per target by `convert::compute_compat`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub compat: BTreeMap<String, Compat>,
    pub body: ComponentBody,
    /// Raw files (entry first). Paths are relative to the component folder.
    #[serde(default)]
    pub files: Vec<ComponentFile>,
    /// Entry file (`x.md`, `SKILL.md`, `x.json`).
    #[serde(default)]
    pub entry: String,
}

fn default_origin() -> String {
    "claude".into()
}

impl Component {
    pub fn file(&self, path: &str) -> Option<&ComponentFile> {
        self.files.iter().find(|f| f.path == path)
    }

    pub fn entry_text(&self) -> Option<&str> {
        self.file(&self.entry).and_then(|f| f.text())
    }

    /// Recomputes [`Component::sha256`] from the files.
    pub fn rehash(&mut self) {
        let mut files: Vec<&ComponentFile> = self.files.iter().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut buf = Vec::new();
        for f in files {
            buf.extend_from_slice(f.path.as_bytes());
            buf.push(0);
            buf.extend_from_slice(&(f.bytes.len() as u64).to_le_bytes());
            buf.extend_from_slice(&f.bytes);
        }
        self.sha256 = super::sha256_hex(&buf);
    }
}

/// Per-kind typed content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComponentBody {
    Rule(RuleSpec),
    Agent(AgentSpec),
    Command(CommandSpec),
    Skill(SkillSpec),
    Mcp(McpSpec),
    Hook(HookSpec),
    Setting(SettingSpec),
    Statusline(StatuslineSpec),
    Loop(LoopSpec),
    Workflow(WorkflowSpec),
    Mod(ModSpec),
    Plugin(PluginSpec),
    ProjectTemplate(TemplateSpec),
    SandboxRecipe(SandboxSpec),
    Stack(StackSpec),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    /// Always on, goes to the root rules file (AGENTS.md, CLAUDE.md …).
    #[default]
    Root,
    /// Applies to `globs` only.
    Scoped,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RuleSpec {
    pub markdown: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub globs: Vec<String>,
    #[serde(default)]
    pub always_apply: bool,
    #[serde(default)]
    pub scope: RuleScope,
}

/// Superset of the Claude subagent frontmatter.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// System prompt (Markdown body).
    pub prompt: String,
    /// Claude tool names. Empty = inherit every tool.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub mcp_servers: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub hooks: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_prompt: Option<String>,
    /// Frontmatter keys we do not model, kept as-is.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CommandSpec {
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Body with neutral markers.
    pub body: String,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SkillSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Folder name to install under (usually `name`).
    pub dir_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Stdio,
    Http,
    Sse,
}

/// A secret the server needs. Values never live in the component: the server
/// fields refer to it as `{{secret:NAME}}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SecretRef {
    /// Environment variable name the tool should read (`GITHUB_TOKEN`).
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Placeholder the source had (`<YOUR_HF_TOKEN>`, `${input:x}`), for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    #[serde(default)]
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<Value>,
    #[serde(default)]
    pub secrets: Vec<SecretRef>,
    #[serde(default)]
    pub description: String,
    /// Other keys of the source record (timeouts, `alwaysAllow` …).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct McpSpec {
    pub servers: Vec<McpServer>,
}

/// A support file shipped with a hook/setting/statusline (a script).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SupportFile {
    /// Path inside the component files.
    pub source: String,
    /// Destination as the source wrote it (`.claude/hooks/x.py`, `~/.claude/scripts/x/y.sh`).
    pub destination: String,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookHandler {
    /// `command | http | prompt | agent | mcp_tool`.
    #[serde(rename = "type", default = "default_handler")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, Value>,
}

fn default_handler() -> String {
    "command".into()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookEntry {
    /// Canonical (Claude) event name.
    pub event: String,
    /// Regex over canonical (Claude) tool names; `None` = all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub handler: HookHandler,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookSpec {
    pub entries: Vec<HookEntry>,
    #[serde(default)]
    pub supporting_files: Vec<SupportFile>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SettingSpec {
    /// Top-level settings keys in Claude's schema (`permissions`, `env`, `model` …).
    pub values: serde_json::Map<String, Value>,
    #[serde(default)]
    pub supporting_files: Vec<SupportFile>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StatuslineSpec {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<i64>,
    #[serde(default)]
    pub supporting_files: Vec<SupportFile>,
    /// stdin fields the command reads (`model.display_name` …), best effort.
    #[serde(default)]
    pub stdin_fields: Vec<String>,
    /// Other settings keys that came with it.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LoopSpec {
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<String>,
    /// References `kind:category/name`.
    #[serde(default)]
    pub components: Vec<String>,
    /// Runbook body.
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// `agent | command | mcp | prompt`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub steps: Vec<WorkflowStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModSpec {
    pub plugin_name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    /// Folder name to install under.
    pub dir_name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PluginSpec {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    /// `.claude-plugin/plugin.json` as found.
    pub manifest: Value,
    /// What the plugin ships, by folder (`agents`, `commands`, `skills`, `hooks`, `mcp`).
    #[serde(default)]
    pub contents: BTreeMap<String, Vec<String>>,
    pub dir_name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplateSpec {
    /// Stack detection hints (`package.json` deps, file globs …).
    #[serde(default)]
    pub detect: Value,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_md: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SandboxSpec {
    /// `docker | e2b | cloudflare`.
    pub provider: String,
    #[serde(default)]
    pub entry: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StackSpec {
    pub components: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}
