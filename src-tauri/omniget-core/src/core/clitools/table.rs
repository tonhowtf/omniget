use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Active,
    Maintenance,
    Discontinued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Cli,
    App,
    #[serde(rename = "cli+app")]
    CliApp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Installer {
    #[serde(default)]
    pub unix: Option<String>,
    #[serde(default = "default_shell")]
    pub unix_shell: String,
    #[serde(default)]
    pub windows: Option<String>,
}

fn default_shell() -> String {
    "bash".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brew {
    pub name: String,
    #[serde(default)]
    pub cask: bool,
}

impl Brew {
    pub fn short(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AcpInfo {
    #[serde(default)]
    pub registry_id: Option<String>,
    #[serde(default)]
    pub native_args: Option<Vec<String>>,
    #[serde(default)]
    pub adapter: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelfUpdate {
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub rerun_installer: bool,
    #[serde(default)]
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Apps {
    #[serde(default)]
    pub macos: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deprecated {
    pub path: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliTool {
    pub id: String,
    pub name: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    #[serde(default)]
    pub status_note: Option<String>,
    pub docs: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub binaries: Vec<String>,
    #[serde(default)]
    pub subcommand: Vec<String>,
    #[serde(default = "default_version_args")]
    pub version_args: Vec<String>,
    #[serde(default)]
    pub apps: Apps,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub npm_scripts: bool,
    #[serde(default)]
    pub pip: Option<String>,
    #[serde(default)]
    pub cargo: Option<String>,
    #[serde(default)]
    pub installer: Installer,
    #[serde(default)]
    pub brew: Option<Brew>,
    #[serde(default)]
    pub winget: Option<String>,
    #[serde(default)]
    pub scoop: Option<String>,
    #[serde(default)]
    pub acp: AcpInfo,
    #[serde(default)]
    pub login: Vec<String>,
    #[serde(default)]
    pub login_hint: Option<String>,
    #[serde(default)]
    pub auth_status: Option<Vec<String>>,
    #[serde(default)]
    pub auth_status_exit_means_auth: bool,
    #[serde(default)]
    pub auth_files: Vec<String>,
    #[serde(default)]
    pub config_env: Option<String>,
    #[serde(default)]
    pub self_update: Option<SelfUpdate>,
    #[serde(default)]
    pub auto_updates: bool,
    #[serde(default)]
    pub config_paths: Vec<String>,
    #[serde(default)]
    pub log_paths: Vec<String>,
    #[serde(default)]
    pub deprecated: Vec<Deprecated>,
}

fn default_version_args() -> Vec<String> {
    vec!["--version".into()]
}

impl CliTool {
    pub fn latest_source(&self) -> LatestSource {
        if let Some(p) = &self.npm {
            LatestSource::Npm(p.clone())
        } else if let Some(p) = &self.pip {
            LatestSource::Pypi(p.clone())
        } else if let Some(r) = &self.repo {
            LatestSource::Github(r.clone())
        } else {
            LatestSource::None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "key", rename_all = "snake_case")]
pub enum LatestSource {
    Npm(String),
    Pypi(String),
    Github(String),
    None,
}

#[derive(Deserialize)]
struct TableFile {
    tools: Vec<CliTool>,
}

const DATA: &str = include_str!("data/tools.json");

pub fn all() -> &'static [CliTool] {
    static TABLE: OnceLock<Vec<CliTool>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str::<TableFile>(DATA)
            .map(|t| t.tools)
            .unwrap_or_else(|e| {
                tracing::error!("[clitools] invalid table: {e}");
                Vec::new()
            })
    })
}

pub fn get(id: &str) -> Option<&'static CliTool> {
    all().iter().find(|t| t.id == id)
}

pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return rest.split('/').fold(home, |acc, seg| acc.join(seg));
        }
    }
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}
