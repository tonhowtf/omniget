//! `FakeProvider`: replays a scripted sequence of `TurnEvent`s with optional
//! delays. UI, coordinator and commands test against it. Owned by
//! f2-llm-providers; DRAFT by the orchestrator (minimal, keep the constructor).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};

use super::{Provider, WireCapture};
use crate::core::llm::error::LlmError;
use crate::core::llm::types::{FinishReason, TurnEvent, TurnRequest, Usage};

#[derive(Debug, Clone)]
pub struct FakeProvider {
    pub script: Vec<TurnEvent>,
    pub delay_per_event: Duration,
    capture: Arc<WireCapture>,
}

impl FakeProvider {
    pub fn new(script: Vec<TurnEvent>) -> Self {
        Self {
            script,
            delay_per_event: Duration::ZERO,
            capture: Arc::new(WireCapture::default()),
        }
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay_per_event = delay;
        self
    }

    /// A plain text answer split into `chunks` deltas, then usage and finish.
    pub fn text(answer: &str, chunks: usize) -> Self {
        let chunks = chunks.max(1);
        let chars: Vec<char> = answer.chars().collect();
        let per = chars.len().div_ceil(chunks).max(1);
        let mut script = vec![TurnEvent::Started {
            request_id: "fake".into(),
        }];
        for piece in chars.chunks(per) {
            script.push(TurnEvent::TextDelta {
                text: piece.iter().collect(),
            });
        }
        script.push(TurnEvent::Usage {
            usage: Usage {
                input_tokens: 10,
                output_tokens: chars.len() as u32 / 4,
                ..Usage::default()
            },
        });
        script.push(TurnEvent::Finished {
            reason: FinishReason::Stop,
        });
        Self::new(script)
    }
}

#[async_trait]
impl Provider for FakeProvider {
    async fn turn(&self, req: TurnRequest) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
        self.capture.record(
            serde_json::json!({ "model": req.model.model, "messages": req.messages.len() }),
            "",
        );
        let delay = self.delay_per_event;
        let cancel = req.cancel.clone();
        let events = self.script.clone();
        let s = stream::iter(events).then(move |ev| {
            let cancel = cancel.clone();
            async move {
                if delay > Duration::ZERO {
                    tokio::time::sleep(delay).await;
                }
                if cancel.is_cancelled() {
                    return TurnEvent::Error {
                        error: LlmError::new(
                            crate::core::llm::error::ERR_LLM_CANCELLED,
                            "cancelled",
                        ),
                    };
                }
                ev
            }
        });
        Ok(s.boxed())
    }

    fn wire_capture(&self) -> Option<Arc<WireCapture>> {
        Some(self.capture.clone())
    }
}
