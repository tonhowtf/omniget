use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::owner::{CommandSpec, OwnerMethod};
use super::table::CliTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Install,
    Update,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub id: String,
    pub tool_id: String,
    pub action: Action,
    pub method: String,
    pub command: CommandSpec,
    pub runnable: bool,
    pub reason: Option<String>,
    pub recommended: bool,
    pub warnings: Vec<String>,
    pub owner: Option<OwnerMethod>,
    pub prefix: Option<PathBuf>,
}

fn installer_line(tool: &CliTool) -> Option<String> {
    if cfg!(windows) {
        tool.installer
            .windows
            .as_ref()
            .map(|u| format!("irm '{u}' | iex"))
    } else {
        tool.installer
            .unix
            .as_ref()
            .map(|u| format!("curl -fsSL '{}' | {}", u, tool.installer.unix_shell))
    }
}

pub fn installer_spec(tool: &CliTool) -> Option<CommandSpec> {
    installer_line(tool).map(|l| CommandSpec::shell(l, format!("installer:{}", tool.id)))
}

pub fn manual_install_line(tool: &CliTool) -> Option<String> {
    installer_line(tool)
        .or_else(|| {
            tool.npm
                .as_ref()
                .map(|p| format!("npm install -g {p}@latest"))
        })
        .or_else(|| {
            tool.brew.as_ref().map(|b| {
                if b.cask {
                    format!("brew install --cask {}", b.name)
                } else {
                    format!("brew install {}", b.name)
                }
            })
        })
        .or_else(|| tool.pip.as_ref().map(|p| format!("uv tool install {p}")))
}

pub fn manual_update_line(tool: &CliTool) -> Option<String> {
    tool.npm
        .as_ref()
        .map(|p| format!("npm install -g {p}@latest"))
        .or_else(|| installer_line(tool))
        .or_else(|| tool.pip.as_ref().map(|p| format!("uv tool upgrade {p}")))
}
