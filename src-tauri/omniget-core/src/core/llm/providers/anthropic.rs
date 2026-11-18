use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::channel::mpsc;
use futures::stream::{BoxStream, StreamExt};
use futures::SinkExt;
use serde_json::{json, Map, Value};

use super::openai_compat::{backoff_ms, retry_after_ms, status_to_error, DEFAULT_MAX_ATTEMPTS};
use super::{Provider, WireCapture};
use crate::core::llm::error::{
    LlmError, ERR_LLM_CANCELLED, ERR_LLM_NET, ERR_LLM_PARSE, ERR_LLM_RATE,
};
use crate::core::llm::sse::SseParser;
use crate::core::llm::types::{
    ContentPart, FinishReason, ProviderId, Role, ToolSpec, TurnEvent, TurnRequest, Usage,
};

pub const API_VERSION: &str = "2023-06-01";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicProvider {
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
    capture: Option<Arc<WireCapture>>,
    max_attempts: u32,
}

impl AnthropicProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Result<Self, LlmError> {
        let mut base: String = base_url.into();
        base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            base = DEFAULT_BASE_URL.to_string();
        }
        let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .connect_timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| LlmError::new(ERR_LLM_NET, format!("HTTP client: {}", e)))?;
        Ok(Self {
            base_url: base,
            api_key: api_key.into(),
            extra_headers: Vec::new(),
            client,
            capture: None,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        })
    }

    pub fn with_capture(mut self, capture: Arc<WireCapture>) -> Self {
        self.capture = Some(capture);
        self
    }

    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = n.max(1);
        self
    }

    pub fn provider_id() -> ProviderId {
        ProviderId::new("anthropic")
    }

    fn endpoint(&self) -> String {
        format!("{}/messages", self.base_url)
    }
}

fn part_block(p: &ContentPart) -> Option<Value> {
    match p {
        ContentPart::Text { text } => Some(json!({ "type": "text", "text": text })),
        ContentPart::Image { mime, data_b64 } => Some(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": mime, "data": data_b64 }
        })),
        ContentPart::ToolUse { id, name, input } => {
            Some(json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
        }
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        })),
    }
}

fn wants_cache(extra: &Map<String, Value>) -> bool {
    match extra.get("cache") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty() && s != "false",
        Some(Value::Null) | None => true,
        Some(_) => true,
    }
}

pub fn build_body(req: &TurnRequest) -> Value {
    let cache = wants_cache(&req.params.extra);
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model.model));
    body.insert(
        "max_tokens".into(),
        json!(req.params.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    body.insert("stream".into(), json!(true));

    let mut system_blocks: Vec<Value> = Vec::new();
    let mut msgs: Vec<Value> = Vec::new();
    for m in &req.messages {
        if m.role == Role::System {
            for p in &m.parts {
                if let ContentPart::Text { text } = p {
                    system_blocks.push(json!({ "type": "text", "text": text }));
                }
            }
            continue;
        }
        let role = match m.role {
            Role::Assistant => "assistant",
            _ => "user",
        };
        let blocks: Vec<Value> = m.parts.iter().filter_map(part_block).collect();
        if blocks.is_empty() {
            continue;
        }
        match msgs.last_mut() {
            Some(prev) if prev["role"] == role => {
                if let Some(arr) = prev["content"].as_array_mut() {
                    arr.extend(blocks);
                }
            }
            _ => msgs.push(json!({ "role": role, "content": blocks })),
        }
    }
    if !system_blocks.is_empty() {
        if cache {
            if let Some(last) = system_blocks.last_mut() {
                last["cache_control"] = json!({ "type": "ephemeral" });
            }
        }
        body.insert("system".into(), Value::Array(system_blocks));
    }
    if cache {
        if let Some(last) = msgs
            .iter_mut()
            .rev()
            .find(|m| m["role"] == "user")
            .and_then(|m| m["content"].as_array_mut())
            .and_then(|a| a.last_mut())
        {
            last["cache_control"] = json!({ "type": "ephemeral" });
        }
    }
    body.insert("messages".into(), Value::Array(msgs));

    if !req.tools.is_empty() {
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t: &ToolSpec| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
        if cache {
            if let Some(last) = tools.last_mut() {
                last["cache_control"] = json!({ "type": "ephemeral" });
            }
        }
        body.insert("tools".into(), Value::Array(tools));
    }

    let p = &req.params;
    if let Some(v) = p.temperature {
        body.insert("temperature".into(), super::openai_compat::f32_json(v));
    }
    if let Some(v) = p.top_p {
        body.insert("top_p".into(), super::openai_compat::f32_json(v));
    }
    if !p.stop.is_empty() {
        body.insert("stop_sequences".into(), json!(p.stop));
    }
    if let Some(effort) = &p.reasoning_effort {
        let max = p.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let budget = match effort.as_str() {
            "low" => max / 4,
            "high" => max * 3 / 4,
            _ => max / 2,
        };
        if budget >= 1024 {
            body.insert(
                "thinking".into(),
                json!({ "type": "enabled", "budget_tokens": budget }),
            );
            body.remove("temperature");
            body.remove("top_p");
        }
    }
    for (k, v) in p.extra.iter() {
        if k == "cache" {
            continue;
        }
        if v.is_null() {
            body.remove(k);
        } else {
            body.insert(k.clone(), v.clone());
        }
    }
    Value::Object(body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    Tool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RawUsage {
    input: u32,
    output: u32,
    cache_read: u32,
    cache_write: u32,
}

#[derive(Debug, Default)]
pub struct AnthropicStreamState {
    blocks: BTreeMap<i64, (BlockKind, String)>,
    raw: RawUsage,
    finish: Option<FinishReason>,
    message_id: Option<String>,
    error: Option<LlmError>,
}

impl AnthropicStreamState {
    pub fn finish_reason(&self) -> Option<FinishReason> {
        self.finish
    }

    pub fn usage(&self) -> Usage {
        Usage {
            input_tokens: self
                .raw
                .input
                .saturating_add(self.raw.cache_read)
                .saturating_add(self.raw.cache_write),
            output_tokens: self.raw.output,
            cache_read_tokens: self.raw.cache_read,
            cache_write_tokens: self.raw.cache_write,
            ..Usage::default()
        }
    }

    pub fn message_id(&self) -> Option<&str> {
        self.message_id.as_deref()
    }

    pub fn error(&self) -> Option<&LlmError> {
        self.error.as_ref()
    }
}

fn merge_usage(raw: &mut RawUsage, u: &Value) {
    let present = |key: &str| u.get(key).and_then(|v| v.as_u64()).map(|n| n as u32);
    if let Some(v) = present("input_tokens") {
        raw.input = v;
    }
    if let Some(v) = present("output_tokens") {
        raw.output = v;
    }
    if let Some(v) = present("cache_read_input_tokens") {
        raw.cache_read = v;
    }
    if let Some(v) = present("cache_creation_input_tokens") {
        raw.cache_write = v;
    }
}

fn stop_reason_of(s: &str) -> FinishReason {
    match s {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" | "pause_turn" => FinishReason::ToolUse,
        "refusal" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

pub fn parse_event(state: &mut AnthropicStreamState, v: &Value, out: &mut Vec<TurnEvent>) {
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "message_start" => {
            let msg = v.get("message").unwrap_or(&Value::Null);
            if let Some(id) = msg.get("id").and_then(|i| i.as_str()) {
                state.message_id = Some(id.to_string());
            }
            if let Some(u) = msg.get("usage") {
                merge_usage(&mut state.raw, u);
            }
        }
        "content_block_start" => {
            let index = v.get("index").and_then(|i| i.as_i64()).unwrap_or(0);
            let block = v.get("content_block").unwrap_or(&Value::Null);
            match block.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "tool_use" | "server_tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    state.blocks.insert(index, (BlockKind::Tool, id.clone()));
                    out.push(TurnEvent::ToolCallStart { id, name });
                }
                "thinking" | "redacted_thinking" => {
                    state
                        .blocks
                        .insert(index, (BlockKind::Thinking, String::new()));
                }
                _ => {
                    state.blocks.insert(index, (BlockKind::Text, String::new()));
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        if !t.is_empty() {
                            out.push(TurnEvent::TextDelta {
                                text: t.to_string(),
                            });
                        }
                    }
                }
            }
        }
        "content_block_delta" => {
            let index = v.get("index").and_then(|i| i.as_i64()).unwrap_or(0);
            let delta = v.get("delta").unwrap_or(&Value::Null);
            match delta.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "text_delta" => {
                    if let Some(t) = delta.get("text").and_then(|t| t.as_str()) {
                        out.push(TurnEvent::TextDelta {
                            text: t.to_string(),
                        });
                    }
                }
                "thinking_delta" => {
                    if let Some(t) = delta.get("thinking").and_then(|t| t.as_str()) {
                        out.push(TurnEvent::ThinkingDelta {
                            text: t.to_string(),
                        });
                    }
                }
                "signature_delta" => {}
                "input_json_delta" => {
                    if let Some(t) = delta.get("partial_json").and_then(|t| t.as_str()) {
                        if let Some((BlockKind::Tool, id)) = state.blocks.get(&index) {
                            out.push(TurnEvent::ToolCallDelta {
                                id: id.clone(),
                                input_json_delta: t.to_string(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let index = v.get("index").and_then(|i| i.as_i64()).unwrap_or(0);
            if let Some((BlockKind::Tool, id)) = state.blocks.remove(&index) {
                out.push(TurnEvent::ToolCallEnd { id });
            }
        }
        "message_delta" => {
            if let Some(u) = v.get("usage") {
                merge_usage(&mut state.raw, u);
            }
            if let Some(sr) = v
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|s| s.as_str())
            {
                state.finish = Some(stop_reason_of(sr));
            }
        }
        "message_stop" => {
            if state.finish.is_none() {
                state.finish = Some(FinishReason::Stop);
            }
        }
        "error" => {
            let err = v.get("error").unwrap_or(&Value::Null);
            let kind = err.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("provider error")
                .to_string();
            let code = match kind {
                "authentication_error" | "permission_error" => {
                    crate::core::llm::error::ERR_LLM_AUTH
                }
                "rate_limit_error" | "overloaded_error" => ERR_LLM_RATE,
                "not_found_error" | "invalid_request_error" => {
                    crate::core::llm::error::ERR_LLM_MODEL
                }
                _ => ERR_LLM_NET,
            };
            let error = LlmError {
                code: std::borrow::Cow::Borrowed(code),
                message: format!("{}: {}", kind, msg),
                retryable: code == ERR_LLM_RATE,
                retry_after_ms: None,
            };
            state.error = Some(error.clone());
            out.push(TurnEvent::Error { error });
        }
        _ => {}
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
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
                .header("anthropic-version", API_VERSION)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&body);
            if !self.api_key.is_empty() {
                rb = rb.header("x-api-key", self.api_key.as_str());
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
            .get("request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let capture = self.capture.clone();
        let cancel = req.cancel.clone();
        let (mut tx, rx) = mpsc::channel::<TurnEvent>(64);

        tokio::spawn(async move {
            let mut parser = SseParser::new();
            let mut state = AnthropicStreamState::default();
            let mut sse_events = Vec::new();
            let mut out: Vec<TurnEvent> = Vec::new();
            let mut first_token: Option<Instant> = None;
            let mut announced = false;
            let mut fatal: Option<LlmError> = None;
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
                sse_events.clear();
                parser.push(&chunk, &mut sse_events);
                if let Some(e) = parser.take_overflow() {
                    fatal = Some(e);
                    break;
                }
                for ev in sse_events.iter() {
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
                    parse_event(&mut state, &v, &mut out);
                    if !announced {
                        announced = true;
                        let id = state
                            .message_id()
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
            }

            sse_events.clear();
            parser.finish(&mut sse_events);
            for ev in sse_events.iter() {
                if let Some(v) = ev.json() {
                    out.clear();
                    parse_event(&mut state, &v, &mut out);
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
            let mut usage = state.usage();
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
    use crate::core::llm::types::{GenParams, Message, ModelRef};
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
            let provider = AnthropicProvider::new(base, "")
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
        let provider = AnthropicProvider::new(base, "").unwrap();
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
                provider: ProviderId::new("anthropic"),
                model: "claude-sonnet-4-5".into(),
            },
            messages,
            tools,
            params,
            cancel: CancellationToken::new(),
            agent_id: None,
        }
    }

    fn run_fixture(name: &str, chunk: usize) -> (Vec<TurnEvent>, AnthropicStreamState) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("llm_sse_fixtures")
            .join(name);
        let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {}: {}", p.display(), e));
        let mut parser = SseParser::new();
        let mut state = AnthropicStreamState::default();
        let mut out = Vec::new();
        let mut evs = Vec::new();
        for piece in bytes.chunks(chunk) {
            evs.clear();
            parser.push(piece, &mut evs);
            for ev in evs.iter() {
                if let Some(v) = ev.json() {
                    parse_event(&mut state, &v, &mut out);
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
    fn text_fixture_streams_text_usage_and_cache_read() {
        let (events, state) = run_fixture("anthropic_text.txt", 64);
        assert_eq!(text_of(&events), "Hello! I am Claude.");
        assert_eq!(state.finish_reason(), Some(FinishReason::Stop));
        assert_eq!(state.usage().input_tokens, 37);
        assert_eq!(state.usage().output_tokens, 15);
        assert_eq!(state.usage().cache_read_tokens, 12);
        assert_eq!(state.message_id(), Some("msg_01XFDUDYJgAACzvnptvVoYEL"));
    }

    #[test]
    fn text_fixture_is_chunk_size_independent() {
        assert_eq!(
            run_fixture("anthropic_text.txt", 1).0,
            run_fixture("anthropic_text.txt", 8192).0
        );
    }

    #[test]
    fn tool_use_fixture_emits_thinking_then_the_tool_call() {
        let (events, state) = run_fixture("anthropic_tool_use.txt", 41);
        let think: String = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::ThinkingDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(think, "I need the weather tool.");
        assert!(matches!(
            events.iter().find(|e| matches!(e, TurnEvent::ToolCallStart { .. })),
            Some(TurnEvent::ToolCallStart { id, name })
                if id == "toolu_01T1x1fJ34qAmk2" && name == "get_weather"
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
            .any(|e| matches!(e, TurnEvent::ToolCallEnd { id } if id == "toolu_01T1x1fJ34qAmk2")));
        assert_eq!(state.finish_reason(), Some(FinishReason::ToolUse));
        assert_eq!(state.usage().cache_write_tokens, 300);
        assert_eq!(state.usage().input_tokens, 772);
    }

    #[test]
    fn mid_stream_error_becomes_a_retryable_error_event() {
        let (events, state) = run_fixture("anthropic_error.txt", 64);
        let err = events
            .iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error),
                _ => None,
            })
            .expect("error event");
        assert_eq!(err.code, ERR_LLM_RATE);
        assert!(err.retryable);
        assert!(err.message.contains("overloaded_error"));
        assert_eq!(state.finish_reason(), None);
    }

    #[test]
    fn message_delta_usage_overrides_message_start() {
        let (_events, state) = run_fixture("anthropic_cumulative_usage.txt", 64);
        let u = state.usage();
        assert_eq!(u.cache_read_tokens, 50);
        assert_eq!(
            u.cache_write_tokens, 200,
            "cache_creation must come from the delta"
        );
        assert_eq!(u.output_tokens, 120);
        assert_eq!(u.input_tokens, 650);
        assert_eq!(state.finish_reason(), Some(FinishReason::Stop));
    }

    #[test]
    fn the_unterminated_last_event_is_dispatched_by_finish() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("llm_sse_fixtures")
            .join("anthropic_cumulative_usage.txt");
        let bytes = std::fs::read(&p).unwrap();
        assert!(
            !bytes.ends_with(b"\n\n"),
            "fixture must end without the blank line for this test to mean anything"
        );
        let mut parser = SseParser::new();
        let mut evs = Vec::new();
        parser.push(&bytes, &mut evs);
        let before = evs.len();
        parser.finish(&mut evs);
        assert_eq!(evs.len(), before + 1);
        assert_eq!(evs.last().unwrap().json().unwrap()["type"], "message_stop");
    }

    #[test]
    fn a_stream_error_is_recorded_on_the_state() {
        let (_events, state) = run_fixture("anthropic_error.txt", 64);
        let err = state.error().expect("the state must remember the error");
        assert_eq!(err.code, ERR_LLM_RATE);
        assert_eq!(state.finish_reason(), None);
    }
}
