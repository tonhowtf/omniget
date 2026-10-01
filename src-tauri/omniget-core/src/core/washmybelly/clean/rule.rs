use serde::{Deserialize, Serialize};

use super::super::{Destination, Os, Risk};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Delete,
    DeleteContents,
    Truncate,
    SqliteVacuum,
    Command {
        program: String,
        args: Vec<String>,
        #[serde(default)]
        measure: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Requirement {
    PathExists { path: String },
    Program { name: String },
    FullDiskAccess,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub os: Vec<Os>,
    pub category: String,
    pub label_key: Option<String>,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub running: Vec<String>,
    #[serde(default)]
    pub min_age_days: u32,
    pub action: Action,
    pub destination: Destination,
    pub risk: Risk,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    pub source: String,
    pub license: String,
}
