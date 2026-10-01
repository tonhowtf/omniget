//! Provider trait and clients. Owned by f2-llm-providers. DRAFT trait by the
//! orchestrator from docs/agents/f2-llm-providers.md §5; `WireCapture` is the
//! seam f2-wire-probe reads.

pub mod anthropic;
pub mod fake;
pub mod openai_compat;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

use super::error::LlmError;
use super::types::{TurnEvent, TurnRequest};

/// Last request body sent (secrets redacted) and raw response, capped at 256 KB.
#[derive(Debug, Default)]
pub struct WireCapture {
    inner: Mutex<WireCaptureInner>,
}

#[derive(Debug, Default, Clone)]
pub struct WireCaptureInner {
    pub sent_body: Option<Value>,
    pub raw_response: String,
}

pub const WIRE_CAPTURE_CAP: usize = 256 * 1024;

impl WireCapture {
    pub fn record(&self, sent_body: Value, raw_response: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.sent_body = Some(sent_body);
        let mut raw = raw_response.to_string();
        raw.truncate(WIRE_CAPTURE_CAP);
        g.raw_response = raw;
    }

    /// Stores the body about to be sent and clears the previous response.
    /// Callers must hand over an already-redacted value: nothing here ever
    /// sees a header, so no key can reach it, but a body that embedded one
    /// would. Additive to the draft; `record` still works.
    pub fn record_request(&self, sent_body: Value) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.sent_body = Some(sent_body);
        g.raw_response.clear();
    }

    /// Appends raw response bytes as they arrive, stopping at the 256 KB cap
    /// so a long stream cannot grow unbounded.
    pub fn append_response(&self, bytes: &[u8]) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let room = WIRE_CAPTURE_CAP.saturating_sub(g.raw_response.len());
        if room == 0 {
            return;
        }
        let take = bytes.len().min(room);
        g.raw_response
            .push_str(&String::from_utf8_lossy(&bytes[..take]));
    }

    pub fn snapshot(&self) -> WireCaptureInner {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn turn(&self, req: TurnRequest) -> Result<BoxStream<'static, TurnEvent>, LlmError>;
    fn wire_capture(&self) -> Option<Arc<WireCapture>> {
        None
    }
}
