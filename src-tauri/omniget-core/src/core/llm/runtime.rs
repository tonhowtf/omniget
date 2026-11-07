use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::BoxStream;

use super::agent::{AgentDef, RuntimeKind};
use super::error::{LlmError, ERR_LLM_MODEL};
use super::providers::Provider;
use super::types::{TurnEvent, TurnRequest};

#[async_trait]
pub trait AgentRuntime: Send + Sync {
    async fn turn(
        &self,
        agent: &AgentDef,
        req: TurnRequest,
    ) -> Result<BoxStream<'static, TurnEvent>, LlmError>;
}

#[derive(Default)]
pub struct NativeRuntime {
    providers: Mutex<HashMap<String, Arc<dyn Provider>>>,
}

impl std::fmt::Debug for NativeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.providers.lock().map(|p| p.len()).unwrap_or_default();
        f.debug_struct("NativeRuntime")
            .field("providers", &n)
            .finish()
    }
}

impl NativeRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, provider_id: impl Into<String>, provider: Arc<dyn Provider>) {
        let mut map = self.providers.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(provider_id.into(), provider);
    }

    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn Provider>> {
        let map = self.providers.lock().unwrap_or_else(|e| e.into_inner());
        map.get(provider_id).cloned()
    }

    pub fn len(&self) -> usize {
        self.providers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl AgentRuntime for NativeRuntime {
    async fn turn(
        &self,
        _agent: &AgentDef,
        req: TurnRequest,
    ) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
        let id = req.model.provider.as_str().to_string();
        let provider = self.get(&id).ok_or_else(|| {
            LlmError::new(ERR_LLM_MODEL, format!("no provider registered for `{id}`"))
        })?;
        provider.turn(req).await
    }
}

pub struct CompositeRuntime {
    native: Arc<dyn AgentRuntime>,
    cli: Option<Arc<dyn AgentRuntime>>,
    acp: Option<Arc<dyn AgentRuntime>>,
}

impl CompositeRuntime {
    pub fn new(native: Arc<dyn AgentRuntime>) -> Self {
        Self {
            native,
            cli: None,
            acp: None,
        }
    }

    pub fn with_cli(mut self, cli: Arc<dyn AgentRuntime>) -> Self {
        self.cli = Some(cli);
        self
    }

    pub fn with_acp(mut self, acp: Arc<dyn AgentRuntime>) -> Self {
        self.acp = Some(acp);
        self
    }
}

#[async_trait]
impl AgentRuntime for CompositeRuntime {
    async fn turn(
        &self,
        agent: &AgentDef,
        req: TurnRequest,
    ) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
        match &agent.runtime {
            RuntimeKind::Cli { cli, account } => match &self.cli {
                Some(rt) => rt.turn(agent, req).await,
                None => Err(LlmError::new(
                    ERR_LLM_MODEL,
                    format!("no CLI runtime registered for `{cli}` account `{account}`"),
                )),
            },
            RuntimeKind::Acp { command, .. } => match &self.acp {
                Some(rt) => rt.turn(agent, req).await,
                None => Err(LlmError::new(
                    ERR_LLM_MODEL,
                    format!("no ACP runtime registered for `{command}`"),
                )),
            },
            RuntimeKind::Native => self.native.turn(agent, req).await,
        }
    }
}
