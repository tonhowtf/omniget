use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::channel::mpsc;
use futures::stream::{BoxStream, StreamExt};
use futures::SinkExt;
use serde_json::{json, Map, Value};

use super::{Provider, WireCapture};
use crate::core::llm::error::{
    LlmError, ERR_LLM_AUTH, ERR_LLM_CANCELLED, ERR_LLM_MODEL, ERR_LLM_NET, ERR_LLM_PARSE,
    ERR_LLM_RATE,
};
use crate::core::llm::sse::SseParser;
use crate::core::llm::types::{
    ContentPart, FinishReason, Message, ProviderId, Role, ToolSpec, TurnEvent, TurnRequest, Usage,
};

pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 8_000;

pub struct OpenAiCompat {
    provider: ProviderId,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
    capture: Option<Arc<WireCapture>>,
    max_attempts: u32,
}

pub fn default_base_url(provider: &ProviderId) -> Option<&'static str> {
    crate::core::tools::ai_keys::KINDS
        .iter()
        .find(|k| k.id == provider.as_str())
        .map(|k| k.base_url)
}

fn http_client() -> Result<reqwest::Client, LlmError> {
    crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| LlmError::new(ERR_LLM_NET, format!("HTTP client: {}", e)))
}

impl OpenAiCompat {
    pub fn new(
        provider: ProviderId,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, LlmError> {
        let mut base: String = base_url.into();
        base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            base = default_base_url(&provider)
                .ok_or_else(|| LlmError::new(ERR_LLM_MODEL, "no base URL for provider"))?
                .to_string();
        }
        let mut extra_headers = Vec::new();
        if provider.as_str() == "openrouter" {
            extra_headers.push((
                "HTTP-Referer".to_string(),
                "https://github.com/tonhowtf/omniget".to_string(),
            ));
            extra_headers.push(("X-Title".to_string(), "OmniGet".to_string()));
        }
        Ok(Self {
            provider,
            base_url: base,
            api_key: api_key.into(),
            extra_headers,
            client: http_client()?,
            capture: None,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        })
    }

    pub fn with_capture(mut self, capture: Arc<WireCapture>) -> Self {
        self.capture = Some(capture);
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }

    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = n.max(1);
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

pub fn f32_json(v: f32) -> Value {
    let cleaned: f64 = v.to_string().parse().unwrap_or(v as f64);
    serde_json::Number::from_f64(cleaned)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn data_uri(mime: &str, data_b64: &str) -> String {
    format!("data:{};base64,{}", mime, data_b64)
}

fn push_message(m: &Message, out: &mut Vec<Value>) {
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    for p in &m.parts {
        if let ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } = p
        {
            let body = if *is_error {
                format!("ERROR: {}", content)
            } else {
                content.clone()
            };
            out.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": body }));
        }
    }

    let has_image = m
        .parts
        .iter()
        .any(|p| matches!(p, ContentPart::Image { .. }));
    let mut text = String::new();
    let mut blocks: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for p in &m.parts {
        match p {
            ContentPart::Text { text: t } => {
                if has_image {
                    blocks.push(json!({ "type": "text", "text": t }));
                } else {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(t);
                }
            }
            ContentPart::Image { mime, data_b64 } => blocks.push(json!({
                "type": "image_url",
                "image_url": { "url": data_uri(mime, data_b64) }
            })),
            ContentPart::ToolUse { id, name, input } => tool_calls.push(json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()) }
            })),
            ContentPart::ToolResult { .. } => {}
        }
    }
    if text.is_empty() && blocks.is_empty() && tool_calls.is_empty() {
        return;
    }
    let mut msg = Map::new();
    msg.insert("role".into(), json!(role));
    if has_image {
        msg.insert("content".into(), Value::Array(blocks));
    } else {
        msg.insert("content".into(), json!(text));
    }
    if !tool_calls.is_empty() {
        msg.insert("tool_calls".into(), Value::Array(tool_calls));
    }
    out.push(Value::Object(msg));
}

pub fn tools_json(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect()
}

pub fn build_body(req: &TurnRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model.model));
    let mut msgs: Vec<Value> = Vec::with_capacity(req.messages.len() + 2);
    for m in &req.messages {
        push_message(m, &mut msgs);
    }
    body.insert("messages".into(), Value::Array(msgs));
    body.insert("stream".into(), json!(true));
    body.insert("stream_options".into(), json!({ "include_usage": true }));
    if !req.tools.is_empty() {
        body.insert("tools".into(), Value::Array(tools_json(&req.tools)));
    }
    let p = &req.params;
    if let Some(v) = p.temperature {
        body.insert("temperature".into(), f32_json(v));
    }
    if let Some(v) = p.top_p {
        body.insert("top_p".into(), f32_json(v));
    }
    if let Some(v) = p.max_tokens {
        body.insert("max_tokens".into(), json!(v));
    }
    if !p.stop.is_empty() {
        body.insert("stop".into(), json!(p.stop));
    }
    if let Some(e) = &p.reasoning_effort {
        if req.model.provider.as_str() == "openrouter" {
            body.insert("reasoning".into(), json!({ "effort": e }));
        } else {
            body.insert("reasoning_effort".into(), json!(e));
        }
    }
    for (k, v) in p.extra.iter() {
        if v.is_null() {
            body.remove(k);
        } else {
            body.insert(k.clone(), v.clone());
        }
    }
    Value::Object(body)
}

#[derive(Debug, Default)]
pub struct OpenAiStreamState {
    tool_ids: BTreeMap<i64, String>,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
    request_id: Option<String>,
    error: Option<LlmError>,
}

impl OpenAiStreamState {
    pub fn finish_reason(&self) -> Option<FinishReason> {
        self.finish
    }
    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
    pub fn error(&self) -> Option<&LlmError> {
        self.error.as_ref()
    }
    pub fn open_tool_ids(&mut self) -> Vec<String> {
        std::mem::take(&mut self.tool_ids).into_values().collect()
    }
}

fn finish_reason_of(s: &str) -> FinishReason {
    match s {
        "stop" | "end_turn" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "function_call" | "tool_use" => FinishReason::ToolUse,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

fn u32_at(v: Option<&Value>, key: &str) -> u32 {
    v.and_then(|u| u.get(key))
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32
}

pub fn usage_of(u: &Value) -> Usage {
    let details = u.get("prompt_tokens_details");
    let cache_read = if let Some(d) = details {
        u32_at(Some(d), "cached_tokens")
    } else {
        u32_at(Some(u), "prompt_cache_hit_tokens")
    };
    Usage {
        input_tokens: u32_at(Some(u), "prompt_tokens"),
        output_tokens: u32_at(Some(u), "completion_tokens"),
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
        first_token_ms: None,
        total_ms: 0,
        cost_usd: u.get("cost").and_then(|c| c.as_f64()),
    }
}

pub fn parse_chunk(state: &mut OpenAiStreamState, v: &Value, out: &mut Vec<TurnEvent>) {
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("provider error")
            .to_string();
        let code = err.get("code").and_then(|c| c.as_str()).unwrap_or("");
        let mapped = match code {
            "rate_limit_exceeded" | "429" => ERR_LLM_RATE,
            "invalid_api_key" | "401" => ERR_LLM_AUTH,
            "model_not_found" | "404" => ERR_LLM_MODEL,
            _ => ERR_LLM_NET,
        };
        let error = LlmError::new(mapped, msg);
        state.error = Some(error.clone());
        out.push(TurnEvent::Error { error });
        return;
    }
    if state.request_id.is_none() {
        if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
            state.request_id = Some(id.to_string());
        }
    }
    if let Some(u) = v.get("usage") {
        if !u.is_null() {
            state.usage = Some(usage_of(u));
        }
    }
    let Some(choices) = v.get("choices").and_then(|c| c.as_array()) else {
        return;
    };
    for ch in choices {
        let delta = ch.get("delta").or_else(|| ch.get("message"));
        if let Some(d) = delta {
            for key in ["reasoning_content", "reasoning"] {
                if let Some(t) = d.get(key).and_then(|x| x.as_str()) {
                    if !t.is_empty() {
                        out.push(TurnEvent::ThinkingDelta {
                            text: t.to_string(),
                        });
                    }
                }
            }
            if let Some(t) = d.get("content").and_then(|x| x.as_str()) {
                if !t.is_empty() {
                    out.push(TurnEvent::TextDelta {
                        text: t.to_string(),
                    });
                }
            }
            if let Some(calls) = d.get("tool_calls").and_then(|x| x.as_array()) {
                for call in calls {
                    let index = call.get("index").and_then(|i| i.as_i64()).unwrap_or(0);
                    let known = state.tool_ids.contains_key(&index);
                    if !known {
                        let id = call
                            .get("id")
                            .and_then(|i| i.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("call_{}", index));
                        let name = call
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        state.tool_ids.insert(index, id.clone());
                        out.push(TurnEvent::ToolCallStart { id, name });
                    }
                    if let Some(args) = call
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                    {
                        if !args.is_empty() {
                            if let Some(id) = state.tool_ids.get(&index) {
                                out.push(TurnEvent::ToolCallDelta {
                                    id: id.clone(),
                                    input_json_delta: args.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
        if let Some(fr) = ch.get("finish_reason").and_then(|f| f.as_str()) {
            state.finish = Some(finish_reason_of(fr));
            if state.finish == Some(FinishReason::ToolUse) {
                for id in state.open_tool_ids() {
                    out.push(TurnEvent::ToolCallEnd { id });
                }
            }
        }
    }
}

pub(crate) fn status_to_error(status: u16, body: &str) -> LlmError {
    let snippet: String = body.chars().take(300).collect();
    let (code, retryable) = match status {
        401 | 403 => (ERR_LLM_AUTH, false),
        404 => (ERR_LLM_MODEL, false),
        429 => (ERR_LLM_RATE, true),
        400 | 422 => (ERR_LLM_MODEL, false),
        500..=599 => (ERR_LLM_NET, true),
        _ => (ERR_LLM_NET, false),
    };
    LlmError {
        code: std::borrow::Cow::Borrowed(code),
        message: format!("HTTP {}: {}", status, snippet),
        retryable,
        retry_after_ms: None,
    }
}

pub(crate) fn retry_after_ms(headers: &reqwest::header::HeaderMap, attempt: u32) -> u64 {
    if let Some(v) = headers.get(reqwest::header::RETRY_AFTER) {
        if let Some(secs) = v.to_str().ok().and_then(|s| s.trim().parse::<f64>().ok()) {
            if secs >= 0.0 {
                return ((secs * 1000.0) as u64).min(60_000);
            }
        }
    }
    backoff_ms(attempt)
}

pub(crate) fn backoff_ms(attempt: u32) -> u64 {
    (BASE_BACKOFF_MS << attempt.min(6)).min(MAX_BACKOFF_MS)
}

#[async_trait]
impl Provider for OpenAiCompat {
    async fn turn(&self, req: TurnRequest) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
        let external = crate::core::llm::code_tools::current_turn()
            .is_some_and(|ctx| crate::core::assist::authority::external(&ctx.conversation));
        let max_attempts = if external { 1 } else { self.max_attempts };
        let body = build_body(&req);
        if let Some(c) = &self.capture {
            c.record_request(body.clone());
        }
        let url = self.endpoint();
        let started = Instant::now();

        let mut attempt = 0u32;
        let resp = loop {
            if req.cancel.is_cancelled() {
                return Err(LlmError::new(ERR_LLM_CANCELLED, "cancelled"));
            }
            let mut rb = self
                .client
                .post(&url)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&body);
            if !self.api_key.is_empty() {
                rb = rb.bearer_auth(&self.api_key);
            }
            for (k, v) in &self.extra_headers {
                rb = rb.header(k.as_str(), v.as_str());
            }
            let sent = tokio::select! {
                biased;
                _ = req.cancel.cancelled() => {
                    return Err(LlmError::new(ERR_LLM_CANCELLED, "cancelled"));
                }
                r = rb.send() => r,
            };
            let (err, wait) = match sent {
                Ok(r) if r.status().is_success() => break r,
                Ok(r) => {
                    let status = r.status().as_u16();
                    let wait = retry_after_ms(r.headers(), attempt);
                    let text = tokio::select! {
                        biased;
                        _ = req.cancel.cancelled() => return Err(LlmError::new(ERR_LLM_CANCELLED, "cancelled")),
                        text = r.text() => text.unwrap_or_default(),
                    };
                    (status_to_error(status, &text), wait)
                }
                Err(e) => (
                    LlmError {
                        code: std::borrow::Cow::Borrowed(ERR_LLM_NET),
                        message: format!("request failed: {}", e),
                        retryable: !e.is_builder(),
                        retry_after_ms: None,
                    },
                    backoff_ms(attempt),
                ),
            };
            attempt += 1;
            if !err.retryable || attempt >= max_attempts {
                return Err(LlmError {
                    retry_after_ms: Some(wait),
                    ..err
                });
            }
            tokio::select! {
                biased;
                _ = req.cancel.cancelled() => {
                    return Err(LlmError::new(ERR_LLM_CANCELLED, "cancelled"));
                }
                _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
            }
        };

        let request_id = resp
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let capture = self.capture.clone();
        let cancel = req.cancel.clone();
        let (mut tx, rx) = mpsc::channel::<TurnEvent>(64);

        tokio::spawn(async move {
            let mut parser = SseParser::new();
            let mut state = OpenAiStreamState::default();
            let mut events: Vec<super::super::sse::SseEvent> = Vec::new();
            let mut out: Vec<TurnEvent> = Vec::new();
            let mut first_token: Option<Instant> = None;
            let mut announced = false;
            let mut fatal: Option<LlmError> = None;
            let mut done = false;
            let mut body_stream = resp.bytes_stream();

            loop {
                let next = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        fatal = Some(LlmError::new(ERR_LLM_CANCELLED, "cancelled"));
                        None
                    }
                    n = body_stream.next() => n,
                };
                let Some(chunk) = next else { break };
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        fatal = Some(LlmError::new(ERR_LLM_NET, format!("stream broke: {}", e)));
                        break;
                    }
                };
                if let Some(c) = &capture {
                    c.append_response(&chunk);
                }
                events.clear();
                parser.push(&chunk, &mut events);
                if let Some(e) = parser.take_overflow() {
                    fatal = Some(e);
                    break;
                }
                for ev in events.iter() {
                    if ev.is_done() {
                        done = true;
                        continue;
                    }
                    let Some(v) = ev.json() else {
                        if !ev.data.trim().is_empty() {
                            fatal = Some(LlmError::new(
                                ERR_LLM_PARSE,
                                format!(
                                    "bad chunk: {}",
                                    ev.data.chars().take(120).collect::<String>()
                                ),
                            ));
                        }
                        continue;
                    };
                    out.clear();
                    parse_chunk(&mut state, &v, &mut out);
                    if !announced {
                        announced = true;
                        let id = state
                            .request_id()
                            .map(|s| s.to_string())
                            .or_else(|| request_id.clone())
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                        if tx
                            .send(TurnEvent::Started { request_id: id })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    for e in out.drain(..) {
                        if matches!(
                            e,
                            TurnEvent::TextDelta { .. } | TurnEvent::ThinkingDelta { .. }
                        ) && first_token.is_none()
                        {
                            first_token = Some(Instant::now());
                        }
                        if tx.send(e).await.is_err() {
                            return;
                        }
                    }
                }
                if done {
                    break;
                }
            }

            events.clear();
            parser.finish(&mut events);
            for ev in events.iter() {
                if ev.is_done() {
                    continue;
                }
                if let Some(v) = ev.json() {
                    out.clear();
                    parse_chunk(&mut state, &v, &mut out);
                    for e in out.drain(..) {
                        if tx.send(e).await.is_err() {
                            return;
                        }
                    }
                }
            }

            if !announced {
                let id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if tx
                    .send(TurnEvent::Started { request_id: id })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            for id in state.open_tool_ids() {
                if tx.send(TurnEvent::ToolCallEnd { id }).await.is_err() {
                    return;
                }
            }
            let mut usage = state.usage().cloned().unwrap_or_default();
            usage.total_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
            usage.first_token_ms = first_token
                .map(|t| t.duration_since(started).as_millis().min(u32::MAX as u128) as u32);
            if tx.send(TurnEvent::Usage { usage }).await.is_err() {
                return;
            }
            match fatal {
                Some(err) => {
                    let cancelled = err.code == ERR_LLM_CANCELLED;
                    let _ = tx.send(TurnEvent::Error { error: err }).await;
                    if cancelled {
                        let _ = tx
                            .send(TurnEvent::Finished {
                                reason: FinishReason::Cancelled,
                            })
                            .await;
                    }
                }
                None => {
                    let reason = state.finish_reason().unwrap_or(if state.error().is_some() {
                        FinishReason::Other
                    } else {
                        FinishReason::Stop
                    });
                    let _ = tx.send(TurnEvent::Finished { reason }).await;
                }
            }
        });

        Ok(rx.boxed())
    }

    fn wire_capture(&self) -> Option<Arc<WireCapture>> {
        self.capture.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::types::{GenParams, ModelRef};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn external_failed_post_is_never_retried_inside_adapter() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for disconnect in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let count = Arc::new(AtomicUsize::new(0));
            let seen = count.clone();
            let server = tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let mut buf = [0u8; 8192];
                    let _ = socket.read(&mut buf).await;
                    seen.fetch_add(1, Ordering::SeqCst);
                    if !disconnect {
                        let _=socket.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    }
                }
            });
            let provider = OpenAiCompat::new(ProviderId::new("custom"), base, "")
                .unwrap()
                .with_max_attempts(3);
            let request = req(
                vec![Message::text(Role::User, "fixture")],
                GenParams::default(),
                vec![],
            );
            let result = tokio::time::timeout(
                Duration::from_secs(3),
                crate::core::llm::code_tools::scope(
                    "external-mcp-fixture",
                    "bot",
                    "request",
                    provider.turn(request),
                ),
            )
            .await;
            server.abort();
            assert!(matches!(result, Ok(Err(_))));
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn cancel_interrupts_stalled_error_response_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (sent, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 100000\r\n\r\nx")
                .await
                .unwrap();
            let _ = sent.send(());
            std::future::pending::<()>().await;
        });
        let provider = OpenAiCompat::new(ProviderId::new("custom"), base, "").unwrap();
        let request = req(
            vec![Message::text(Role::User, "fixture")],
            GenParams::default(),
            vec![],
        );
        let cancel = request.cancel.clone();
        let call = tokio::spawn(async move {
            crate::core::llm::code_tools::scope(
                "external-mcp-fixture",
                "bot",
                "request",
                provider.turn(request),
            )
            .await
        });
        ready.await.unwrap();
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), call).await;
        server.abort();
        match result {
            Ok(Ok(Err(e))) => assert_eq!(e.code, ERR_LLM_CANCELLED),
            _ => panic!("cancel must interrupt response-body wait"),
        }
    }

    fn req(messages: Vec<Message>, params: GenParams, tools: Vec<ToolSpec>) -> TurnRequest {
        TurnRequest {
            model: ModelRef {
                provider: ProviderId::new("openai"),
                model: "gpt-4o-mini".into(),
            },
            messages,
            tools,
            params,
            cancel: CancellationToken::new(),
            agent_id: None,
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("llm_sse_fixtures")
            .join(name);
        std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {}: {}", p.display(), e))
    }

    fn run_fixture(name: &str, chunk: usize) -> (Vec<TurnEvent>, OpenAiStreamState) {
        let bytes = fixture(name);
        let mut parser = SseParser::new();
        let mut state = OpenAiStreamState::default();
        let mut out = Vec::new();
        let mut evs = Vec::new();
        for piece in bytes.chunks(chunk) {
            evs.clear();
            parser.push(piece, &mut evs);
            for ev in evs.iter() {
                if ev.is_done() {
                    continue;
                }
                if let Some(v) = ev.json() {
                    parse_chunk(&mut state, &v, &mut out);
                }
            }
        }
        (out, state)
    }

    fn text_of(events: &[TurnEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn openai_fixture_streams_text_and_usage() {
        let (events, state) = run_fixture("openai_chat_text.txt", 64);
        assert_eq!(text_of(&events), "Hello! How can I help you today?");
        assert_eq!(state.finish_reason(), Some(FinishReason::Stop));
        let u = state.usage().expect("usage chunk");
        assert_eq!(u.input_tokens, 19);
        assert_eq!(u.output_tokens, 9);
        assert_eq!(u.cache_read_tokens, 0);
        assert_eq!(state.request_id(), Some("chatcmpl-B9kZ1"));
    }

    #[test]
    fn openai_fixture_is_chunk_size_independent() {
        let a = run_fixture("openai_chat_text.txt", 1).0;
        let b = run_fixture("openai_chat_text.txt", 4096).0;
        assert_eq!(a, b);
    }

    #[test]
    fn openai_tool_call_fixture_emits_start_delta_end() {
        let (events, state) = run_fixture("openai_tool_call.txt", 37);
        let starts: Vec<&TurnEvent> = events
            .iter()
            .filter(|e| matches!(e, TurnEvent::ToolCallStart { .. }))
            .collect();
        assert_eq!(starts.len(), 1);
        assert!(matches!(
            starts[0],
            TurnEvent::ToolCallStart { name, .. } if name == "get_weather"
        ));
        let args: String = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::ToolCallDelta {
                    input_json_delta, ..
                } => Some(input_json_delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(args, "{\"location\":\"Rio de Janeiro\"}");
        assert!(events
            .iter()
            .any(|e| matches!(e, TurnEvent::ToolCallEnd { .. })));
        assert_eq!(state.finish_reason(), Some(FinishReason::ToolUse));
    }

    #[test]
    fn openrouter_fixture_reports_reasoning_and_cost() {
        let (events, state) = run_fixture("openrouter_reasoning.txt", 64);
        let think: String = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::ThinkingDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(think, "The user greets me.");
        assert_eq!(text_of(&events), "Hi there!");
        let u = state.usage().expect("usage");
        assert_eq!(u.cost_usd, Some(0.000042));
    }

    #[test]
    fn deepseek_fixture_reports_cache_hits() {
        let (_events, state) = run_fixture("deepseek_cache.txt", 64);
        let u = state.usage().expect("usage");
        assert_eq!(u.input_tokens, 1024);
        assert_eq!(u.cache_read_tokens, 896);
    }

    #[test]
    fn groq_fixture_streams_text() {
        let (events, state) = run_fixture("groq_chat_text.txt", 33);
        assert_eq!(text_of(&events), "Fast answer.");
        assert_eq!(state.finish_reason(), Some(FinishReason::Stop));
        assert_eq!(state.usage().map(|u| u.output_tokens), Some(3));
    }

    #[test]
    fn ollama_openai_shim_fixture_streams_text() {
        let (events, state) = run_fixture("ollama_v1_chat.txt", 50);
        assert!(text_of(&events).contains("Ollama"));
        assert_eq!(state.finish_reason(), Some(FinishReason::Stop));
    }

    #[test]
    fn usage_is_read_with_and_without_choices() {
        let (_e, openai) = run_fixture("openai_chat_text.txt", 64);
        assert_eq!(openai.usage().map(|u| u.input_tokens), Some(19));

        let (_e, router) = run_fixture("openrouter_reasoning.txt", 64);
        let u = router.usage().expect("usage on a chunk that has choices");
        assert_eq!(u.input_tokens, 12);
        assert_eq!(u.output_tokens, 21);
        assert_eq!(u.cost_usd, Some(0.000042));
        assert_eq!(router.finish_reason(), Some(FinishReason::Stop));

        let (_e, deepseek) = run_fixture("deepseek_cache.txt", 64);
        let u = deepseek.usage().expect("usage on a chunk that has choices");
        assert_eq!(u.input_tokens, 1024);
        assert_eq!(u.cache_read_tokens, 896);
        assert_eq!(deepseek.finish_reason(), Some(FinishReason::Stop));
    }
}
