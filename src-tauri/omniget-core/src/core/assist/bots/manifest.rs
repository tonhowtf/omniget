use serde::{Deserialize, Serialize};

use super::profile::{BotProfile, Capability};
use super::skills::{BindingStatus, Fix};
use crate::core::llm::caps::RuntimeCaps;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapState {
    Available,
    Partial,
    Missing,
    NotRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityStatus {
    pub id: Capability,
    pub requested: bool,
    pub state: CapState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub tools: Vec<String>,
    pub missing_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnavailableGrant {
    pub tool: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub bot_id: String,
    pub runtime_kind: String,
    pub runtime: RuntimeCaps,
    pub projectless: bool,
    pub profile: BotProfile,
    pub capabilities: Vec<CapabilityStatus>,
    pub skills: Vec<BindingStatus>,
    pub offered_tools: Vec<String>,
    pub unavailable_grants: Vec<UnavailableGrant>,
}
