use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionState {
    Unknown,
    Current,
    BehindLatest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Latest {
    pub id: String,
    pub source: String,
    pub key: String,
    pub latest: Option<String>,
    pub installed: Option<String>,
    pub state: VersionState,
    pub checked_at: u64,
    pub error: Option<String>,
}
