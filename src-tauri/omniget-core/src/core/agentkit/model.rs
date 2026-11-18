use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

    pub fn format_key(self) -> &'static str {
        match self {
            ComponentKind::ProjectTemplate => "template",
            ComponentKind::SandboxRecipe => "sandbox",
            other => other.as_str(),
        }
    }
}

impl std::fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SourceRef {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Compat {
    Native,
    Converted,
    Degraded { lost: Vec<String> },
    Unsupported { reason: String },
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
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
    #[serde(default)]
    pub sha256: String,
    #[serde(default = "default_origin")]
    pub origin_tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub compat: BTreeMap<String, Compat>,
    pub body: ComponentBody,
    #[serde(default)]
    pub files: Vec<ComponentFile>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
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
    #[default]
    Root,
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub prompt: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    #[serde(default)]
    pub description: String,
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
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct McpSpec {
    pub servers: Vec<McpServer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SupportFile {
    pub source: String,
    pub destination: String,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookHandler {
    #[serde(rename = "type", default = "default_handler")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
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
    pub event: String,
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
    #[serde(default)]
    pub stdin_fields: Vec<String>,
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
    #[serde(default)]
    pub components: Vec<String>,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkflowStep {
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
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    pub dir_name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PluginSpec {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    pub manifest: Value,
    #[serde(default)]
    pub contents: BTreeMap<String, Vec<String>>,
    pub dir_name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TemplateSpec {
    #[serde(default)]
    pub detect: Value,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_md: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SandboxSpec {
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
