use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    #[default]
    System,
    Embedded,
}

#[derive(Debug, Clone, Serialize)]
pub struct Launch {
    pub surface: Surface,
    pub command: String,
    pub env: BTreeMap<String, String>,
    pub unset: Vec<String>,
    pub display: String,
    pub script_path: Option<PathBuf>,
    pub script: Option<String>,
    pub program: Option<String>,
    pub args: Vec<String>,
    pub hint: Option<String>,
}
