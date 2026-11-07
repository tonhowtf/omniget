pub mod authority;
pub mod bots;
pub mod ctx;
pub mod db;
pub mod external_config;
pub mod external_files;
pub mod groups;
pub mod media_provider;
pub(crate) mod media_retrieval;
pub mod media_tools;
pub mod memory;
pub mod missions;
pub mod projection;
pub mod runs;
pub mod tools;
pub mod web;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub struct NoAugment;

impl crate::core::llm::coordinator::TurnAugment for NoAugment {
    fn augment(
        &self,
        _agent: &mut crate::core::llm::agent::AgentDef,
        _conversation_id: &str,
        _user_input: &str,
    ) -> Option<String> {
        None
    }
}

pub fn augments() -> Vec<std::sync::Arc<dyn crate::core::llm::coordinator::TurnAugment>> {
    vec![bots::augment(), groups::augment(), memory::augment()]
}
