//! OmniGet profiles (plan §4.3 "Perfis", F6): one permission/provider intent
//! written once in Claude's settings schema and applied to every detected tool
//! through the normal plan (each tool's converter translates it: Codex sandbox,
//! OpenCode `permission`, Gemini policies, Cursor `Shell()/Read()/Write()` …).
//!
//! Built-in profiles: `read-only`, `dev` (edits freely, asks before shell),
//! `no-network`, `autonomous-guarded` (runs on its own, dangerous commands and
//! secrets blocked) and `provider` (another endpoint: base URL + key read from
//! an environment variable, never the key itself).

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::convert::compute_compat;
use super::model::*;
use super::plan::{self, ConflictPolicy, InstallPlan, PlanRequest};
use super::targets;
use super::{AgentkitError, Env, Result, Scope};

/// One parameter of a profile (only `provider` has any).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileParam {
    pub key: String,
    pub label: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub params: Vec<ProfileParam>,
    /// The Claude-schema settings the profile writes (without params filled).
    pub settings: Value,
}

pub const PROFILE_IDS: &[&str] = &[
    "read-only",
    "dev",
    "no-network",
    "autonomous-guarded",
    "provider",
];

fn read_only() -> Value {
    json!({
        "permissions": {
            "allow": ["Read", "Glob", "Grep"],
            "deny": ["Edit", "Write", "NotebookEdit", "Bash"]
        }
    })
}

fn dev() -> Value {
    json!({
        "permissions": {
            "allow": ["Read", "Glob", "Grep", "Edit", "Write", "NotebookEdit"],
            "ask": ["Bash", "WebFetch"],
            "deny": ["Read(./.env)", "Read(./.env.*)", "Edit(./.env)", "Edit(./.env.*)"],
            "defaultMode": "acceptEdits"
        }
    })
}

fn no_network() -> Value {
    json!({
        "permissions": {
            "deny": [
                "WebFetch", "WebSearch",
                "Bash(curl *)", "Bash(wget *)", "Bash(ssh *)", "Bash(scp *)", "Bash(rsync *)", "Bash(nc *)"
            ]
        }
    })
}

fn autonomous_guarded() -> Value {
    json!({
        "permissions": {
            "allow": ["Read", "Glob", "Grep", "Edit", "Write", "NotebookEdit", "Bash", "WebFetch", "WebSearch"],
            "ask": ["Bash(git push *)"],
            "deny": [
                "Bash(rm -rf *)", "Bash(sudo *)", "Bash(git push --force *)", "Bash(git push -f *)",
                "Bash(git reset --hard *)", "Bash(git clean -fd *)", "Bash(mkfs *)", "Bash(dd *)",
                "Read(./.env)", "Read(./.env.*)", "Edit(./.env)", "Edit(./.env.*)",
                "Read(~/.ssh/**)", "Read(~/.aws/**)"
            ],
            "defaultMode": "acceptEdits"
        }
    })
}

/// Every built-in profile.
pub fn list() -> Vec<ProfileInfo> {
    PROFILE_IDS.iter().filter_map(|id| info(id)).collect()
}

pub fn info(id: &str) -> Option<ProfileInfo> {
    let p = |id: &str, name: &str, description: &str, settings: Value| ProfileInfo {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        params: vec![],
        settings,
    };
    Some(match id {
        "read-only" => p(
            id,
            "Read-only",
            "Reads and searches the code; no edits and no shell. Codex runs with a read-only sandbox.",
            read_only(),
        ),
        "dev" => p(
            id,
            "Developer",
            "Edits files freely, asks before shell commands and web fetches, never reads .env.",
            dev(),
        ),
        "no-network" => p(
            id,
            "No network",
            "Blocks web fetch, web search and network commands (curl, wget, ssh…). Codex sandbox without network.",
            no_network(),
        ),
        "autonomous-guarded" => p(
            id,
            "Autonomous with guard",
            "Runs on its own: edits and shell without asking, but destructive commands, force pushes and secrets are blocked and pushes ask.",
            autonomous_guarded(),
        ),
        "provider" => ProfileInfo {
            id: id.into(),
            name: "Alternative provider".into(),
            description: "Points each tool at another endpoint (base URL) with the key read from an environment variable.".into(),
            params: vec![
                ProfileParam {
                    key: "protocol".into(),
                    label: "API protocol".into(),
                    required: true,
                    choices: vec!["anthropic".into(), "openai".into()],
                    example: Some("anthropic".into()),
                },
                ProfileParam {
                    key: "base_url".into(),
                    label: "Base URL".into(),
                    required: true,
                    choices: vec![],
                    example: Some("https://api.z.ai/api/anthropic".into()),
                },
                ProfileParam {
                    key: "key_env".into(),
                    label: "Environment variable with the key".into(),
                    required: true,
                    choices: vec![],
                    example: Some("ZAI_API_KEY".into()),
                },
                ProfileParam {
                    key: "model".into(),
                    label: "Model".into(),
                    required: false,
                    choices: vec![],
                    example: Some("glm-4.7".into()),
                },
                ProfileParam {
                    key: "small_model".into(),
                    label: "Small/fast model".into(),
                    required: false,
                    choices: vec![],
                    example: Some("glm-4.5-air".into()),
                },
            ],
            settings: json!({}),
        },
        _ => return None,
    })
}

fn param<'a>(params: &'a Value, k: &str) -> Option<&'a str> {
    params
        .get(k)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn valid_env_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Settings of the `provider` profile for its parameters.
pub fn provider_settings(params: &Value) -> Result<(Value, bool)> {
    let bad = |m: &str| AgentkitError::new("AGENTKIT_PROFILE", m.to_string());
    let protocol = param(params, "protocol").unwrap_or("anthropic");
    let base = param(params, "base_url").ok_or_else(|| bad("base_url is required"))?;
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err(bad("base_url must start with http:// or https://"));
    }
    let key = param(params, "key_env").ok_or_else(|| bad("key_env is required"))?;
    if !valid_env_name(key) {
        return Err(bad(
            "key_env must be an environment variable name like MY_API_KEY",
        ));
    }
    let model = param(params, "model");
    let small = param(params, "small_model");
    let mut env = Map::new();
    let mut values = Map::new();
    let claude_ok = match protocol {
        "anthropic" => {
            env.insert("ANTHROPIC_BASE_URL".into(), json!(base));
            if let Some(m) = model {
                env.insert("ANTHROPIC_MODEL".into(), json!(m));
            }
            if let Some(s) = small {
                env.insert("ANTHROPIC_SMALL_FAST_MODEL".into(), json!(s));
            }
            // Claude sends the helper's output as the key: the value stays in the variable
            values.insert(
                "apiKeyHelper".into(),
                json!(format!("printf %s \"${key}\"")),
            );
            true
        }
        "openai" => {
            env.insert("OPENAI_BASE_URL".into(), json!(base));
            env.insert(
                "OPENAI_API_KEY".into(),
                json!(format!("{{{{secret:{key}}}}}")),
            );
            if let Some(m) = model {
                env.insert("OPENAI_MODEL".into(), json!(m));
            }
            false
        }
        other => {
            return Err(bad(&format!(
                "unknown protocol `{other}` (anthropic | openai)"
            )))
        }
    };
    values.insert("env".into(), Value::Object(env));
    Ok((Value::Object(values), claude_ok))
}

/// The profile as a Setting component (`omniget:profiles/<id>`).
pub fn component(id: &str, params: &Value) -> Result<Component> {
    let inf = info(id).ok_or_else(|| {
        AgentkitError::new(
            "AGENTKIT_PROFILE",
            format!("unknown profile `{id}` ({})", PROFILE_IDS.join(", ")),
        )
    })?;
    let settings = if id == "provider" {
        provider_settings(params)?.0
    } else {
        inf.settings.clone()
    };
    let values = settings.as_object().cloned().unwrap_or_default();
    let mut doc = Map::new();
    doc.insert("description".into(), json!(inf.description));
    for (k, v) in &values {
        doc.insert(k.clone(), v.clone());
    }
    let entry = format!("{id}.json");
    let text = serde_json::to_string_pretty(&Value::Object(doc)).unwrap_or_default() + "\n";
    let mut c = Component {
        id: format!("omniget:profiles/{id}"),
        kind: ComponentKind::Setting,
        name: id.to_string(),
        category: Some("profiles".into()),
        description: inf.description.clone(),
        source: Some(SourceRef {
            id: "omniget".into(),
            repo: None,
            commit: None,
            path: Some(format!("profiles/{entry}")),
            url: None,
        }),
        license: Some("MIT".into()),
        author: Some("OmniGet".into()),
        sha256: String::new(),
        origin_tool: "claude".into(),
        security: None,
        compat: Default::default(),
        body: ComponentBody::Setting(SettingSpec {
            values,
            supporting_files: vec![],
        }),
        files: vec![ComponentFile {
            path: entry.clone(),
            bytes: text.into_bytes(),
            executable: false,
        }],
        entry,
    };
    c.rehash();
    Ok(c)
}

/// Where a profile can go: every target with its compatibility (tools that
/// cannot hold it come back `unsupported`). The `provider` profile over the
/// OpenAI protocol is unsupported on Claude Code (it only speaks Anthropic's).
pub fn compat(c: &Component, all: &[targets::TargetAdapter]) -> Vec<(String, Compat)> {
    let mut map = compute_compat(c, all);
    if c.id == "omniget:profiles/provider" {
        if let ComponentBody::Setting(s) = &c.body {
            let openai = s
                .values
                .get("env")
                .and_then(|e| e.get("OPENAI_BASE_URL"))
                .is_some();
            if openai {
                map.insert(
                    "claude".into(),
                    Compat::Unsupported {
                        reason: "Claude Code talks only to Anthropic-compatible endpoints".into(),
                    },
                );
            }
        }
    }
    all.iter()
        .filter_map(|t| map.remove(&t.id).map(|c| (t.id.clone(), c)))
        .collect()
}

/// Request of [`plan_profile`].
#[derive(Debug, Clone, Default)]
pub struct ProfileRequest {
    pub id: String,
    pub params: Value,
    /// `None`: every installed tool that can hold the profile.
    pub targets: Option<Vec<String>>,
    pub scope: Option<Scope>,
    pub project_dir: Option<PathBuf>,
    pub policy: ConflictPolicy,
}

/// Plans a profile on many tools at once (the plan is stored like any other:
/// apply it with `writer::apply`).
pub fn plan_profile(env: &Env, req: ProfileRequest) -> Result<InstallPlan> {
    let c = component(&req.id, &req.params)?;
    let all = targets::load_targets(env);
    let compat = compat(&c, &all);
    let ok = |id: &str| {
        compat
            .iter()
            .any(|(t, cp)| t == id && !matches!(cp, Compat::Unsupported { .. }))
    };
    let chosen: Vec<String> = match req.targets {
        Some(t) => t,
        None => super::detect::detect_all(env, req.project_dir.as_deref())
            .into_iter()
            .filter(|d| d.installed && ok(&d.id))
            .map(|d| d.id)
            .collect(),
    };
    let mut warnings = Vec::new();
    let mut targets_ok = Vec::new();
    for t in chosen {
        if ok(&t) {
            targets_ok.push(t);
        } else {
            let reason = compat
                .iter()
                .find(|(id, _)| *id == t)
                .map(|(_, c)| match c {
                    Compat::Unsupported { reason } => reason.clone(),
                    _ => String::new(),
                })
                .unwrap_or_else(|| "unknown tool".into());
            warnings.push(format!("{t}: profile `{}` not applied ({reason})", req.id));
        }
    }
    if targets_ok.is_empty() {
        return Err(AgentkitError::new(
            "AGENTKIT_PROFILE",
            format!(
                "no tool can take profile `{}`{}",
                req.id,
                if warnings.is_empty() {
                    String::new()
                } else {
                    format!(": {}", warnings.join("; "))
                }
            ),
        ));
    }
    let scope = req.scope.or(Some(if req.project_dir.is_some() {
        Scope::Project
    } else {
        Scope::Global
    }));
    let mut p = plan::plan(
        env,
        PlanRequest {
            components: vec![c],
            targets: targets_ok,
            scope,
            project_dir: req.project_dir,
            policy: req.policy,
            secret_values: Default::default(),
        },
    )?;
    // tools left out are reported with the plan (the stored copy only needs the units)
    p.warnings.extend(warnings);
    Ok(p)
}
