//! One stateless model call for the city: the founding loop and the council
//! round both speak through [`ask`], so usage, cancellation and the "answer
//! is JSON" parsing live in one place. The app picks the provider (it owns the
//! vault); this module only drives the stream.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::core::llm::providers::Provider;
use crate::core::llm::types::{
    ContentPart, GenParams, Message, ModelRef, Role, TurnEvent, TurnRequest, Usage,
};

/// A model the city may talk to.
#[derive(Clone)]
pub struct Llm {
    pub provider: Arc<dyn Provider>,
    pub model: ModelRef,
    /// Whether to attach preview images (the model has vision).
    pub vision: bool,
}

impl Llm {
    pub fn new(provider: Arc<dyn Provider>, model: ModelRef) -> Self {
        let vision = guess_vision(model.provider.as_str(), &model.model);
        Llm {
            provider,
            model,
            vision,
        }
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.model.provider.as_str(), self.model.model)
    }
}

/// Hosted APIs take images; local servers only for known vision families.
/// A wrong guess is harmless: [`ask`] retries once without the images.
pub fn guess_vision(provider: &str, model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    let local = matches!(provider, "ollama" | "lmstudio" | "llama-server");
    if !local {
        return !(m.contains("deepseek") || m.contains("o1-mini") || m.contains("codestral"));
    }
    [
        "vision",
        "llava",
        "-vl",
        "vl:",
        "gemma3",
        "minicpm-v",
        "moondream",
        "qwen2.5vl",
    ]
    .iter()
    .any(|k| m.contains(k))
}

/// What one call returned.
#[derive(Debug, Clone, Default)]
pub struct Answer {
    pub text: String,
    pub usage: Usage,
}

impl Answer {
    pub fn tokens(&self) -> u64 {
        self.usage.input_tokens as u64 + self.usage.output_tokens as u64
    }
}

/// One turn, no tools. Images in `messages` are dropped and the call retried
/// once when the provider refuses them.
pub async fn ask(
    llm: &Llm,
    messages: Vec<Message>,
    max_tokens: u32,
    cancel: CancellationToken,
) -> Result<Answer, String> {
    let has_image = messages.iter().any(|m| {
        m.parts
            .iter()
            .any(|p| matches!(p, ContentPart::Image { .. }))
    });
    match ask_once(llm, messages.clone(), max_tokens, cancel.clone()).await {
        Err(e) if has_image && !cancel.is_cancelled() => {
            let stripped = messages
                .into_iter()
                .map(|mut m| {
                    m.parts.retain(|p| !matches!(p, ContentPart::Image { .. }));
                    m
                })
                .collect();
            ask_once(llm, stripped, max_tokens, cancel)
                .await
                .map_err(|e2| format!("{e2} (with image: {e})"))
        }
        other => other,
    }
}

async fn ask_once(
    llm: &Llm,
    messages: Vec<Message>,
    max_tokens: u32,
    cancel: CancellationToken,
) -> Result<Answer, String> {
    let req = TurnRequest {
        model: llm.model.clone(),
        messages,
        tools: Vec::new(),
        params: GenParams {
            temperature: Some(0.3),
            max_tokens: Some(max_tokens),
            ..Default::default()
        },
        cancel,
        agent_id: None,
    };
    let mut stream = llm.provider.turn(req).await.map_err(|e| e.to_string())?;
    let mut out = Answer::default();
    let mut failure = None;
    while let Some(ev) = stream.next().await {
        match ev {
            TurnEvent::TextDelta { text } => out.text.push_str(&text),
            TurnEvent::Usage { usage } => out.usage = usage,
            TurnEvent::Error { error } => failure = Some(error.to_string()),
            _ => {}
        }
    }
    record_usage(llm, &out);
    if let Some(e) = failure {
        if out.text.trim().is_empty() {
            return Err(e);
        }
    }
    if out.text.trim().is_empty() {
        return Err("empty answer from the model".into());
    }
    Ok(out)
}

/// The same cost ledger `ai::chat` feeds (Tools → AI costs).
fn record_usage(llm: &Llm, a: &Answer) {
    let mut entry = crate::core::tools::usage::UsageEntry::now(
        "city3d",
        llm.model.provider.as_str(),
        &llm.model.model,
    );
    entry.input_tokens = a.usage.input_tokens as u64;
    entry.output_tokens = a.usage.output_tokens as u64;
    entry.cache_read_tokens = a.usage.cache_read_tokens as u64;
    entry.cache_write_tokens = a.usage.cache_write_tokens as u64;
    entry.first_token_ms = a.usage.first_token_ms;
    entry.cost_usd = a.usage.cost_usd;
    if entry.input_tokens > 0 || entry.output_tokens > 0 {
        crate::core::tools::usage::record_entry(entry);
    }
}

pub fn user(text: impl Into<String>) -> Message {
    Message::text(Role::User, text)
}

pub fn system(text: impl Into<String>) -> Message {
    Message::text(Role::System, text)
}

pub fn assistant(text: impl Into<String>) -> Message {
    Message::text(Role::Assistant, text)
}

/// A user message with a PNG attached.
pub fn user_with_png(text: impl Into<String>, png: &[u8]) -> Message {
    use base64::Engine;
    let mut parts = vec![ContentPart::Text { text: text.into() }];
    if !png.is_empty() {
        parts.push(ContentPart::Image {
            mime: "image/png".into(),
            data_b64: base64::engine::general_purpose::STANDARD.encode(png),
        });
    }
    Message {
        role: Role::User,
        parts,
    }
}

/// The first JSON object of an answer: fenced (```json) or bare, with prose
/// around it tolerated.
pub fn extract_json(text: &str) -> Result<Value, String> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    if let Some(start) = t.find("```") {
        let rest = &t[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        if let Some(end) = rest.find("```") {
            if let Ok(v) = serde_json::from_str::<Value>(rest[..end].trim()) {
                return Ok(v);
            }
        }
    }
    // Balanced braces from the first `{`, respecting strings.
    let bytes = t.as_bytes();
    let Some(open) = t.find('{') else {
        return Err("no JSON object in the answer".into());
    };
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if in_str {
            match b {
                _ if esc => esc = false,
                b'\\' => esc = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&t[open..=i]).map_err(|e| e.to_string());
                }
            }
            _ => {}
        }
    }
    Err("unterminated JSON object in the answer".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_found_in_fences_and_prose() {
        assert_eq!(extract_json("{\"a\":1}").unwrap()["a"], 1);
        assert_eq!(
            extract_json("Here:\n```json\n{\"a\":2}\n```\nDone").unwrap()["a"],
            2
        );
        assert_eq!(
            extract_json("ok {\"a\":{\"b\":\"}\"}} tail").unwrap()["a"]["b"],
            "}"
        );
        assert!(extract_json("nothing").is_err());
    }

    #[test]
    fn local_models_need_a_vision_family() {
        assert!(guess_vision("anthropic", "claude-sonnet-4-5"));
        assert!(!guess_vision("ollama", "llama3.1:8b"));
        assert!(guess_vision("ollama", "qwen2.5vl:7b"));
    }
}
