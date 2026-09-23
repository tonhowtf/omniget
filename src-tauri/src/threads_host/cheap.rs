//! The "cheap model" of the threads glue: branch names and commit messages.
//! One stateless call on the first configured model that can answer without
//! a CLI: local servers first (Ollama, LM Studio, llama-server), then an API
//! key. No model → `None`, and the callers fall back to their heuristics.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use omniget_core::core::llm::agent::RuntimeKind;
use omniget_core::core::llm::types::{GenParams, Message, ModelRef, Role, TurnEvent, TurnRequest};
use omniget_core::core::threads::git::{TextFuture, TextGen};
use tokio_util::sync::CancellationToken;

use crate::llm_manager::{provider_available, LlmManager};

const LOCAL: &[&str] = &["ollama", "lmstudio", "llama-server"];
const MAX_ANSWER: usize = 4000;

/// Candidate models, local ones first, each once.
fn candidates(llm: &LlmManager) -> Vec<ModelRef> {
    let mut out: Vec<ModelRef> = Vec::new();
    for agent in llm.roster() {
        if !matches!(agent.runtime, RuntimeKind::Native) {
            continue;
        }
        let m = llm.model_of(&agent);
        let provider = m.provider.as_str();
        if provider == "fake" || !provider_available(provider) {
            continue;
        }
        if !out.iter().any(|o| o == &m) {
            out.push(m);
        }
    }
    out.sort_by_key(|m| !LOCAL.contains(&m.provider.as_str()));
    out
}

async fn ask(llm: &LlmManager, model: ModelRef, system: &str, prompt: &str) -> Option<String> {
    let provider = llm.provider_for(&model.provider)?;
    let req = TurnRequest {
        model,
        messages: vec![
            Message::text(Role::System, system),
            Message::text(Role::User, prompt),
        ],
        tools: Vec::new(),
        params: GenParams {
            temperature: Some(0.2),
            max_tokens: Some(300),
            ..Default::default()
        },
        cancel: CancellationToken::new(),
        agent_id: None,
    };
    let mut stream = provider.turn(req).await.ok()?;
    let mut out = String::new();
    while let Some(ev) = stream.next().await {
        match ev {
            TurnEvent::TextDelta { text } => {
                out.push_str(&text);
                if out.len() > MAX_ANSWER {
                    break;
                }
            }
            TurnEvent::Error { .. } => return None,
            TurnEvent::Finished { .. } => break,
            _ => {}
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// The hook `ThreadGit` calls. Tries up to two models, 15 s each.
pub fn text_gen(llm: Arc<LlmManager>) -> TextGen {
    Arc::new(move |system: String, prompt: String| -> TextFuture {
        let llm = llm.clone();
        Box::pin(async move {
            for model in candidates(&llm).into_iter().take(2) {
                let answer = tokio::time::timeout(
                    Duration::from_secs(15),
                    ask(&llm, model, &system, &prompt),
                )
                .await
                .ok()
                .flatten();
                if answer.is_some() {
                    return answer;
                }
            }
            None
        })
    })
}
