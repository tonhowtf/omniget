pub mod docker;
pub mod e2b;
pub mod tree;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::runner::{RunOutput, ToolRun};

pub const ERR_SANDBOX: &str = "ERR_AGENTKIT_SANDBOX";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SandboxOpts {
    pub provider: String,
    #[serde(default)]
    pub bind_original: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2b_key_id: Option<String>,
    #[serde(default = "yes")]
    pub network: bool,
}

fn yes() -> bool {
    true
}

pub fn root() -> Result<PathBuf, String> {
    crate::core::paths::app_data_dir()
        .map(|d| d.join("sandbox"))
        .ok_or_else(|| format!("{ERR_SANDBOX}: no app data dir"))
}

pub fn run_dir(id: &str) -> Result<PathBuf, String> {
    let safe: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe.is_empty() {
        return Err(format!("{ERR_SANDBOX}: bad run id"));
    }
    Ok(root()?.join("runs").join(safe))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub provider: String,
    pub tool: String,
    pub project: PathBuf,
    pub work: PathBuf,
    pub bind_original: bool,
    pub image: Option<String>,
    pub env_names: Vec<String>,
    pub argv_preview: Vec<String>,
    pub applied: Vec<String>,
}

pub fn save_record(dir: &Path, r: &RunRecord, base: Option<&tree::Manifest>) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{ERR_SANDBOX}: {e}"))?;
    std::fs::write(
        dir.join("run.json"),
        serde_json::to_vec_pretty(r).unwrap_or_default(),
    )
    .map_err(|e| format!("{ERR_SANDBOX}: {e}"))?;
    if let Some(b) = base {
        std::fs::write(
            dir.join("base.json"),
            serde_json::to_vec(b).unwrap_or_default(),
        )
        .map_err(|e| format!("{ERR_SANDBOX}: {e}"))?;
    }
    Ok(())
}

pub fn load_record(id: &str) -> Result<(RunRecord, tree::Manifest), String> {
    let dir = run_dir(id)?;
    let r: RunRecord = serde_json::from_slice(
        &std::fs::read(dir.join("run.json"))
            .map_err(|_| format!("{ERR_SANDBOX}: no sandbox run {id}"))?,
    )
    .map_err(|e| format!("{ERR_SANDBOX}: {e}"))?;
    let base: tree::Manifest = std::fs::read(dir.join("base.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    Ok((r, base))
}

pub fn diff(id: &str) -> Result<Vec<tree::Change>, String> {
    let (r, base) = load_record(id)?;
    if r.bind_original {
        return Ok(vec![]);
    }
    tree::changes(&r.project, &r.work, &base)
}

pub fn key_env(tool: &str, key_id: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let Some(id) = key_id.filter(|k| !k.is_empty()) else {
        return Ok(vec![]);
    };
    let entry = crate::core::tools::ai_keys::entry_with_secret(id)
        .map_err(|e| format!("{ERR_SANDBOX}: key {id}: {e}"))?;
    if entry.key.is_empty() {
        return Err(format!(
            "{ERR_SANDBOX}: the vault entry {} has no key",
            entry.name
        ));
    }
    let kind = crate::core::tools::ai_keys::kind_of(&entry.kind);
    let mut out = Vec::new();
    let var = match (tool, entry.kind.as_str()) {
        ("claude", "anthropic") => "ANTHROPIC_API_KEY".to_string(),
        ("codex", "openai") => "OPENAI_API_KEY".to_string(),
        ("gemini", "gemini") => "GEMINI_API_KEY".to_string(),
        (_, _) if !kind.env_var().is_empty() => kind.env_var().to_string(),
        _ => "OPENAI_API_KEY".to_string(),
    };
    out.push((var, entry.key.clone()));
    if !entry.base_url.is_empty()
        && entry.kind != "anthropic"
        && entry.kind != "openai"
        && entry.kind != "gemini"
    {
        out.push(("OPENAI_BASE_URL".into(), entry.base_url.clone()));
    }
    if tool == "claude"
        && entry.kind == "anthropic"
        && !entry.base_url.is_empty()
        && !entry.base_url.contains("api.anthropic.com")
    {
        out.push(("ANTHROPIC_BASE_URL".into(), entry.base_url.clone()));
    }
    Ok(out)
}

pub async fn run(
    id: &str,
    run: &ToolRun,
    opts: &SandboxOpts,
    cancel: CancellationToken,
    on_log: &mut (dyn FnMut(&str) + Send),
) -> Result<(RunOutput, RunRecord), String> {
    match opts.provider.as_str() {
        "docker" | "" => docker::run(id, run, opts, cancel, on_log).await,
        "e2b" => e2b::run(id, run, opts, cancel, on_log).await,
        other => Err(format!(
            "{ERR_SANDBOX}: unknown provider `{other}` (docker, e2b)"
        )),
    }
}
