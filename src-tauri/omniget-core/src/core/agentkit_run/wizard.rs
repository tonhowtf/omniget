use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::stack::StackReport;
use crate::core::agentkit::model::Component;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: String,
    pub template: String,
    pub selected: bool,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip)]
    pub component: Option<Component>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project: PathBuf,
    pub stack: StackReport,
    pub templates: Vec<String>,
    pub pieces: Vec<Piece>,
    pub agents_md: String,
    pub warnings: Vec<String>,
}
