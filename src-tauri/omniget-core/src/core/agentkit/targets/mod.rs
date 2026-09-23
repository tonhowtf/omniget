//! Per-tool adapters as data (plan §4.2). One TOML file per tool lives in
//! `targets/data/<id>.toml` and is embedded at build time; a file with the same
//! name in `<app_data>/agentkit/targets/` overrides keys of the embedded one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::ComponentKind;
use super::{AgentkitError, Env, Os, Result, Scope};

/// The embedded manifests, in tier order of the plan.
pub const EMBEDDED: &[(&str, &str)] = &[
    ("claude", include_str!("data/claude.toml")),
    ("codex", include_str!("data/codex.toml")),
    ("gemini", include_str!("data/gemini.toml")),
    ("qwen", include_str!("data/qwen.toml")),
    ("opencode", include_str!("data/opencode.toml")),
    ("cursor", include_str!("data/cursor.toml")),
    ("copilot", include_str!("data/copilot.toml")),
    ("kilo", include_str!("data/kilo.toml")),
    ("crush", include_str!("data/crush.toml")),
    ("cline", include_str!("data/cline.toml")),
    ("goose", include_str!("data/goose.toml")),
    ("zed", include_str!("data/zed.toml")),
    ("droid", include_str!("data/droid.toml")),
    ("kimi", include_str!("data/kimi.toml")),
    ("pi", include_str!("data/pi.toml")),
    ("amp", include_str!("data/amp.toml")),
    ("junie", include_str!("data/junie.toml")),
    ("auggie", include_str!("data/auggie.toml")),
    ("grok", include_str!("data/grok.toml")),
    ("kiro", include_str!("data/kiro.toml")),
    ("devin", include_str!("data/devin.toml")),
    ("windsurf", include_str!("data/windsurf.toml")),
    ("warp", include_str!("data/warp.toml")),
    ("trae", include_str!("data/trae.toml")),
    ("qoder", include_str!("data/qoder.toml")),
    ("vibe", include_str!("data/vibe.toml")),
    ("letta", include_str!("data/letta.toml")),
    ("rovo", include_str!("data/rovo.toml")),
    ("antigravity", include_str!("data/antigravity.toml")),
    ("codebuff", include_str!("data/codebuff.toml")),
    ("aider", include_str!("data/aider.toml")),
    ("openhands", include_str!("data/openhands.toml")),
    ("roo", include_str!("data/roo.toml")),
    ("continue", include_str!("data/continue.toml")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetStatus {
    #[default]
    Active,
    Maintenance,
    Deprecated,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Detect {
    #[serde(default)]
    pub binaries: Vec<String>,
    #[serde(default)]
    pub home_dirs: Vec<String>,
    #[serde(default)]
    pub vscode_extensions: Vec<String>,
    #[serde(default)]
    pub app_bundles: Vec<String>,
    #[serde(default)]
    pub app_paths_windows: Vec<String>,
    #[serde(default)]
    pub app_paths_linux: Vec<String>,
    #[serde(default)]
    pub version_args: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scopes {
    #[serde(default)]
    pub project: bool,
    #[serde(default)]
    pub global: bool,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub project_markers: Vec<String>,
    #[serde(default)]
    pub global_dir: Option<String>,
    #[serde(default)]
    pub config_dir_env: Option<String>,
}

/// Where one kind lives, per scope and OS.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathSpec {
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub project_macos: Option<String>,
    #[serde(default)]
    pub project_linux: Option<String>,
    #[serde(default)]
    pub project_windows: Option<String>,
    #[serde(default)]
    pub global: Option<String>,
    #[serde(default)]
    pub global_macos: Option<String>,
    #[serde(default)]
    pub global_linux: Option<String>,
    #[serde(default)]
    pub global_windows: Option<String>,
    #[serde(default)]
    pub local: Option<String>,
    #[serde(default)]
    pub managed_macos: Option<String>,
    #[serde(default)]
    pub managed_linux: Option<String>,
    #[serde(default)]
    pub managed_windows: Option<String>,
    #[serde(default)]
    pub project_alt: Vec<String>,
    #[serde(default)]
    pub global_alt: Vec<String>,
}

impl PathSpec {
    /// Template for a scope on an OS.
    pub fn template(&self, scope: Scope, os: Os) -> Option<&str> {
        fn pick<'a>(
            os: Os,
            base: &'a Option<String>,
            m: &'a Option<String>,
            l: &'a Option<String>,
            w: &'a Option<String>,
        ) -> Option<&'a str> {
            let over = match os {
                Os::Macos => m,
                Os::Linux => l,
                Os::Windows => w,
            };
            over.as_deref().or(base.as_deref())
        }
        match scope {
            Scope::Project => pick(
                os,
                &self.project,
                &self.project_macos,
                &self.project_linux,
                &self.project_windows,
            ),
            Scope::Global => pick(
                os,
                &self.global,
                &self.global_macos,
                &self.global_linux,
                &self.global_windows,
            ),
            Scope::Local => self.local.as_deref(),
            Scope::Managed => match os {
                Os::Macos => self.managed_macos.as_deref(),
                Os::Linux => self.managed_linux.as_deref(),
                Os::Windows => self.managed_windows.as_deref(),
            },
        }
    }

    /// Alternative locations the tool also reads (for import/inventory).
    pub fn alternatives(&self, scope: Scope) -> &[String] {
        match scope {
            Scope::Project | Scope::Local => &self.project_alt,
            Scope::Global | Scope::Managed => &self.global_alt,
        }
    }
}

/// One MCP config surface of a tool (Copilot has VS Code and CLI surfaces).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpTarget {
    #[serde(default)]
    pub surface: String,
    /// `json | jsonc | toml | yaml | yaml_file_per_server`.
    pub format: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub global: Option<String>,
    #[serde(default)]
    pub global_macos: Option<String>,
    #[serde(default)]
    pub global_linux: Option<String>,
    #[serde(default)]
    pub global_windows: Option<String>,
    /// Path of the servers container inside the file.
    #[serde(default)]
    pub key: Vec<String>,
    /// `map` (keyed by name) or `array` (objects with `name_field`).
    #[serde(default = "default_container")]
    pub container: String,
    #[serde(default)]
    pub name_field: Option<String>,
    #[serde(default)]
    pub stdio_shape: String,
    #[serde(default)]
    pub remote_shape: String,
    #[serde(default)]
    pub transports: Vec<String>,
    /// `env_ref | vscode_inputs | bearer_env_var | plain`.
    #[serde(default)]
    pub secret_mode: Option<String>,
    /// How the tool expands an env var inside strings; `VAR` is the name.
    #[serde(default)]
    pub env_ref: Option<String>,
    #[serde(default)]
    pub prefer_cli: Option<String>,
    #[serde(default = "yes")]
    pub default: bool,
}

fn default_container() -> String {
    "map".into()
}

fn yes() -> bool {
    true
}

impl McpTarget {
    pub fn template(&self, scope: Scope, os: Os) -> Option<&str> {
        match scope {
            Scope::Project | Scope::Local => self.project.as_deref(),
            Scope::Global => {
                let over = match os {
                    Os::Macos => &self.global_macos,
                    Os::Linux => &self.global_linux,
                    Os::Windows => &self.global_windows,
                };
                over.as_deref().or(self.global.as_deref())
            }
            Scope::Managed => None,
        }
    }

    pub fn supports(&self, transport: &str) -> bool {
        self.transports.is_empty() && transport == "stdio"
            || self.transports.iter().any(|t| t == transport)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookConfig {
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub timeout_unit: Option<String>,
    #[serde(default)]
    pub version: Option<Value>,
    #[serde(default)]
    pub env_aliases: Vec<String>,
    #[serde(default)]
    pub events_count: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatuslineConfig {
    #[serde(default)]
    pub key: Vec<String>,
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub stdin_fields: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub syntax: String,
    #[serde(default)]
    pub key: Vec<String>,
    #[serde(default)]
    pub lists: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranscriptsConfig {
    #[serde(default)]
    pub globs: Vec<String>,
    #[serde(default)]
    pub parser: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub cmd: String,
    #[serde(default)]
    pub headless: Vec<String>,
    #[serde(default)]
    pub json_flag: Vec<String>,
    #[serde(default)]
    pub resume_flag: Option<String>,
    #[serde(default)]
    pub system_prompt_flag: Option<String>,
    #[serde(default)]
    pub model_flag: Option<String>,
    #[serde(default)]
    pub permission_flags: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AcpConfig {
    #[serde(default)]
    pub native: bool,
    #[serde(default)]
    pub cmd: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub registry_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustConfig {
    #[serde(default)]
    pub requires_review: bool,
    #[serde(default)]
    pub how: Option<String>,
}

/// Everything the installer knows about one tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAdapter {
    pub id: String,
    pub name: String,
    #[serde(default = "tier_two")]
    pub tier: u8,
    #[serde(default)]
    pub status: TargetStatus,
    /// Off by default in the UI (not verified on the owner's machine).
    #[serde(default)]
    pub beta: bool,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub reads_agents_dir: bool,
    #[serde(default)]
    pub reads_agents_md: Option<String>,
    #[serde(default)]
    pub detect: Detect,
    #[serde(default)]
    pub scopes: Scopes,
    #[serde(default)]
    pub formats: BTreeMap<String, String>,
    #[serde(default)]
    pub reads_claude: BTreeMap<String, bool>,
    #[serde(default)]
    pub paths: BTreeMap<String, PathSpec>,
    #[serde(default)]
    pub mcp: Vec<McpTarget>,
    #[serde(default)]
    pub hook_event_map: BTreeMap<String, String>,
    #[serde(default)]
    pub hooks: HookConfig,
    #[serde(default)]
    pub tool_name_map: BTreeMap<String, String>,
    #[serde(default)]
    pub placeholder_map: BTreeMap<String, String>,
    #[serde(default)]
    pub model_map: BTreeMap<String, String>,
    #[serde(default)]
    pub statusline: Option<StatuslineConfig>,
    #[serde(default)]
    pub permissions: Option<PermissionsConfig>,
    #[serde(default)]
    pub transcripts: Option<TranscriptsConfig>,
    #[serde(default)]
    pub runner: Option<RunnerConfig>,
    #[serde(default)]
    pub acp: Option<AcpConfig>,
    #[serde(default)]
    pub trust: Option<TrustConfig>,
}

fn tier_two() -> u8 {
    2
}

impl TargetAdapter {
    /// Parses one manifest.
    pub fn from_toml(text: &str) -> Result<TargetAdapter> {
        let v = super::edit::toml::parse(text)?;
        Self::from_value(v)
    }

    pub fn from_value(v: Value) -> Result<TargetAdapter> {
        serde_json::from_value(v).map_err(|e| AgentkitError::new("AGENTKIT_TARGET", e.to_string()))
    }

    /// Format id for a kind (`[formats]`), if the tool supports that kind.
    pub fn format(&self, kind: ComponentKind) -> Option<&str> {
        self.formats
            .get(kind.format_key())
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// Does the tool read the Claude file for this kind natively?
    pub fn reads_claude(&self, kind: ComponentKind) -> bool {
        let key = match kind {
            ComponentKind::Rule => "rules",
            ComponentKind::Agent => "agents",
            ComponentKind::Command => "commands",
            ComponentKind::Hook => "hooks",
            ComponentKind::Skill => "skills",
            ComponentKind::Plugin | ComponentKind::Mod => "plugins",
            ComponentKind::Setting => "settings",
            ComponentKind::Statusline => "statusline",
            _ => return false,
        };
        self.id == "claude" || self.reads_claude.get(key).copied().unwrap_or(false)
    }

    pub fn supports_scope(&self, scope: Scope) -> bool {
        match scope {
            Scope::Project => self.scopes.project,
            Scope::Global => self.scopes.global,
            Scope::Local => self.scopes.local,
            Scope::Managed => self.scopes.managed,
        }
    }

    /// Template for a path kind (`agents`, `hooks` …) and scope.
    pub fn path_template(&self, key: &str, scope: Scope, os: Os) -> Option<&str> {
        self.paths.get(key)?.template(scope, os)
    }

    /// Absolute path for a path kind and scope; `None` when the tool has no such
    /// location or a project path is asked for without a project.
    pub fn path(
        &self,
        key: &str,
        scope: Scope,
        env: &Env,
        project: Option<&Path>,
    ) -> Option<PathBuf> {
        let t = self.path_template(key, scope, env.os)?;
        env.expand(t, project)
    }

    /// MCP surfaces written by default.
    pub fn default_mcp(&self) -> impl Iterator<Item = &McpTarget> {
        self.mcp.iter().filter(|m| m.default)
    }

    /// Target event names for a canonical Claude event (fan-out allowed).
    pub fn map_event(&self, event: &str) -> Vec<String> {
        match self.hook_event_map.get(event) {
            Some(v) if !v.trim().is_empty() => v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Some(_) => vec![],
            None if self.id == "claude" => vec![event.to_string()],
            None => vec![],
        }
    }

    /// Translates a Claude matcher (`Edit|Write`, `Bash`) with `tool_name_map`.
    /// Unknown names pass through; names mapped to "" are dropped.
    pub fn map_matcher(&self, matcher: &str) -> String {
        if self.tool_name_map.is_empty() || matcher.is_empty() || matcher == "*" {
            return matcher.to_string();
        }
        let parts: Vec<String> = matcher
            .split('|')
            .filter_map(|p| {
                let bare = p.trim().trim_start_matches('^').trim_end_matches('$');
                match self.tool_name_map.get(bare) {
                    Some(m) if m.is_empty() => None,
                    Some(m) => Some(p.replace(bare, m.split(',').next().unwrap_or(m).trim())),
                    None => Some(p.to_string()),
                }
            })
            .collect();
        let mut seen = Vec::new();
        for p in parts {
            if !seen.contains(&p) {
                seen.push(p);
            }
        }
        seen.join("|")
    }

    /// Model value for a Claude alias; `None` = omit the field.
    pub fn map_model(&self, model: &str) -> Option<String> {
        if self.id == "claude" {
            return Some(model.to_string());
        }
        match self.model_map.get(model) {
            Some(m) if !m.is_empty() => Some(m.clone()),
            Some(_) => None,
            // not an alias: a concrete id passes through only if it looks qualified
            None => (!matches!(model, "sonnet" | "opus" | "haiku" | "fable" | "inherit"))
                .then(|| model.to_string()),
        }
    }
}

/// Loads every embedded manifest (cached). A broken manifest is a build bug, so
/// it is skipped with a log line instead of taking the whole installer down.
pub fn all_targets() -> &'static [TargetAdapter] {
    static CELL: OnceLock<Vec<TargetAdapter>> = OnceLock::new();
    CELL.get_or_init(|| {
        EMBEDDED
            .iter()
            .filter_map(|(id, text)| match TargetAdapter::from_toml(text) {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!("agentkit target {id}: {e}");
                    None
                }
            })
            .collect()
    })
}

/// One embedded target by id.
pub fn target(id: &str) -> Option<&'static TargetAdapter> {
    all_targets().iter().find(|t| t.id == id)
}

/// Embedded targets with the user's overrides from `<app_data>/agentkit/targets/*.toml`
/// deep-merged on top. A user file with a new id adds a target.
pub fn load_targets(env: &Env) -> Vec<TargetAdapter> {
    let mut out: Vec<TargetAdapter> = all_targets().to_vec();
    let dir = env.agentkit_dir().join("targets");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "toml").unwrap_or(false))
        .collect();
    files.sort();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(over) = super::edit::toml::parse(&text) else {
            tracing::warn!("agentkit override {} does not parse", f.display());
            continue;
        };
        let id = over
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| f.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();
        let base = EMBEDDED
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, t)| super::edit::toml::parse(t).ok());
        let mut merged = base.unwrap_or_else(|| Value::Object(Default::default()));
        deep_merge(&mut merged, over);
        if let Value::Object(m) = &mut merged {
            m.entry("id").or_insert(Value::String(id.clone()));
        }
        match TargetAdapter::from_value(merged) {
            Ok(t) => {
                out.retain(|x| x.id != t.id);
                out.push(t);
            }
            Err(e) => tracing::warn!("agentkit override {}: {e}", f.display()),
        }
    }
    out
}

fn deep_merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) if slot.is_object() && v.is_object() => deep_merge(slot, v),
                    _ => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

/// Looks a target up in a list (embedded or overridden).
pub fn find<'a>(targets: &'a [TargetAdapter], id: &str) -> Result<&'a TargetAdapter> {
    targets
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| AgentkitError::new("AGENTKIT_TARGET", format!("unknown target `{id}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_manifest_loads() {
        for (id, text) in EMBEDDED {
            let t = TargetAdapter::from_toml(text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(&t.id, id);
            assert!(t.tier >= 1 && t.tier <= 3, "{id} tier");
        }
        assert_eq!(all_targets().len(), EMBEDDED.len());
        let claude = target("claude").unwrap();
        assert!(claude.reads_claude(ComponentKind::Agent));
        assert_eq!(claude.map_event("PreToolUse"), vec!["PreToolUse"]);
        let gemini = target("gemini").unwrap();
        assert_eq!(gemini.map_event("PreToolUse"), vec!["BeforeTool"]);
    }

    #[test]
    fn paths_expand_per_scope() {
        let env = Env::sandbox(Path::new("/h"), Os::Linux);
        let claude = target("claude").unwrap();
        assert_eq!(
            claude.path("agents", Scope::Global, &env, None).unwrap(),
            PathBuf::from("/h/.claude/agents")
        );
        assert_eq!(
            claude
                .path("agents", Scope::Project, &env, Some(Path::new("/p")))
                .unwrap(),
            PathBuf::from("/p/.claude/agents")
        );
        assert!(claude.path("agents", Scope::Project, &env, None).is_none());
    }
}
