pub mod acp_registry;
pub mod detect;
pub mod exec;
pub mod latest;
pub mod owner;
pub mod plan;
pub mod run;
pub mod table;
pub mod terminal;

use serde::Serialize;

pub const ERR_NOT_FOUND: &str = "CLITOOLS_NOT_FOUND";
pub const ERR_FAILED: &str = "CLITOOLS_FAILED";
pub const ERR_ACP: &str = "CLITOOLS_ACP";
pub const ERR_UNVERIFIED: &str = "CLITOOLS_UNVERIFIED";

#[derive(Debug, Clone, Serialize)]
pub struct ToolRow {
    pub tool: table::CliTool,
    pub detection: Option<detect::Detection>,
    pub last_run: Option<run::RunResult>,
}

pub fn list() -> Vec<ToolRow> {
    let cached = detect::cached();
    table::all()
        .iter()
        .map(|t| ToolRow {
            tool: t.clone(),
            detection: cached
                .as_ref()
                .and_then(|v| v.iter().find(|d| d.id == t.id).cloned()),
            last_run: run::last_run(&t.id),
        })
        .collect()
}
