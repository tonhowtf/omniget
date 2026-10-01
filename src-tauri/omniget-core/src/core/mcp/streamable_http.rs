use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::error::{from_status, McpError};
use super::types::{
    classify, id_key, notification, request, resolve_headers, Incoming, SecretLookup,
};
use crate::core::llm::sse::SseParser;

pub const ACCEPT: &str = "application/json, text/event-stream";
const SESSION_HEADER: &str = "Mcp-Session-Id";
const PROTOCOL_HEADER: &str = "MCP-Protocol-Version";

pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    session: StdMutex<Option<String>>,
    protocol: StdMutex<Option<String>>,
    next_id: AtomicI64,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("url", &self.url)
            .field("session", &self.session_id().is_some())
            .finish()
    }
}

pub struct ServerStream {
    pub rx: mpsc::Receiver<Value>,
    task: JoinHandle<()>,
}

impl ServerStream {
    pub async fn next(&mut self) -> Option<Value> {
        self.rx.recv().await
    }
}

impl Drop for ServerStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn is_loopback(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .map(|h| h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]")
        .unwrap_or(false)
}

fn is_event_stream(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .map(|t| t.trim().eq_ignore_ascii_case("text/event-stream"))
        .unwrap_or(false)
}

fn match_response(body: &str, key: &str) -> Option<Result<Value, McpError>> {
    let value: Value = serde_json::from_str(body).ok()?;
    let items: Vec<&Value> = match &value {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    for item in items {
        match classify(item) {
            Incoming::Result { id, result } if id == key => return Some(Ok(result)),
            Incoming::Failure { id, error } if id == key => {
                return Some(Err(McpError::proto(format!(
                    "JSON-RPC error {}: {}",
                    error.code, error.message
                ))))
            }
            _ => {}
        }
    }
    None
}

impl HttpTransport {
    pub fn connect(
        url: &str,
        headers: &BTreeMap<String, String>,
        lookup: &SecretLookup,
    ) -> Result<Self, McpError> {
        let url = url.trim().to_string();
        if url.is_empty() {
            return Err(McpError::http("no URL configured"));
        }
        let headers = resolve_headers(headers, lookup)?;
        let builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(20));
        let builder = if is_loopback(&url) {
            builder.no_proxy()
        } else {
            crate::core::http_client::apply_global_proxy(builder)
        };
        let client = builder
            .build()
            .map_err(|e| McpError::http(format!("HTTP client: {}", e)))?;
        Ok(Self {
            client,
            url,
            headers,
            session: StdMutex::new(None),
            protocol: StdMutex::new(None),
            next_id: AtomicI64::new(1),
        })
    }

    pub fn session_id(&self) -> Option<String> {
        self.session.lock().ok().and_then(|g| g.clone())
    }

    pub fn set_protocol(&self, version: &str) {
        if let Ok(mut g) = self.protocol.lock() {
            *g = Some(version.to_string());
        }
    }

    fn build(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
        let mut req = self.client.request(method, &self.url);
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(session) = self.session_id() {
            req = req.header(SESSION_HEADER, session);
        }
        if let Some(version) = self.protocol.lock().ok().and_then(|g| g.clone()) {
            req = req.header(PROTOCOL_HEADER, version);
        }
        req
    }

    pub async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let key = id_key(&Value::from(id)).unwrap_or_else(|| id.to_string());
        let body = request(id, method, params);
        let call = self.post_and_read(&body, &key);
        let cancelled = async {
            match cancel {
                Some(c) => c.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            out = call => out,
            _ = tokio::time::sleep(timeout) => Err(McpError::timeout(format!(
                "{} took longer than {} ms", method, timeout.as_millis()
            ))),
            _ = cancelled => Err(McpError::cancelled()),
        }
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let resp = self
            .build(reqwest::Method::POST)
            .header("Accept", ACCEPT)
            .json(&notification(method, params))
            .send()
            .await
            .map_err(|e| McpError::http(format!("{}: {}", self.url, e)).retryable())?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(from_status(status.as_u16(), &body))
    }

    async fn post_and_read(&self, body: &Value, key: &str) -> Result<Value, McpError> {
        let resp = self
            .build(reqwest::Method::POST)
            .header("Accept", ACCEPT)
            .json(body)
            .send()
            .await
            .map_err(|e| McpError::http(format!("{}: {}", self.url, e)).retryable())?;

        if let Some(sid) = resp
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
        {
            if let Ok(mut g) = self.session.lock() {
                *g = Some(sid.to_string());
            }
        }

        let status = resp.status();
        if status.as_u16() == 404 && self.session_id().is_some() {
            if let Ok(mut g) = self.session.lock() {
                *g = None;
            }
            return Err(McpError::http("the MCP session expired (HTTP 404)").retryable());
        }
        if status.as_u16() == 202 {
            return Err(McpError::proto(
                "the server accepted the request (202) without answering it",
            ));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(from_status(status.as_u16(), &body));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if !is_event_stream(&content_type) {
            let text = resp
                .text()
                .await
                .map_err(|e| McpError::http(format!("could not read the answer: {}", e)))?;
            return match_response(&text, key).unwrap_or_else(|| {
                Err(McpError::proto(format!(
                    "the answer did not contain a reply to request {}: {}",
                    key,
                    super::error::clip(&text, 200)
                )))
            });
        }

        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|e| McpError::http(format!("the stream broke: {}", e)).retryable())?;
            parser.push(&chunk, &mut events);
            if let Some(e) = parser.take_overflow() {
                return Err(McpError::proto(format!("oversized SSE event: {}", e)));
            }
            for ev in events.drain(..) {
                if let Some(found) = ev.json().and_then(|v| pick(&v, key)) {
                    return found;
                }
            }
        }
        parser.finish(&mut events);
        for ev in events.drain(..) {
            if let Some(found) = ev.json().and_then(|v| pick(&v, key)) {
                return found;
            }
        }
        Err(McpError::proto("the SSE stream ended before the response").retryable())
    }

    pub async fn open_server_stream(&self) -> Result<Option<ServerStream>, McpError> {
        let resp = self
            .build(reqwest::Method::GET)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|e| McpError::http(format!("{}: {}", self.url, e)).retryable())?;
        if resp.status().as_u16() == 405 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(from_status(status, &body));
        }
        let (tx, rx) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            let mut stream = resp.bytes_stream();
            let mut parser = SseParser::new();
            let mut events = Vec::new();
            while let Some(Ok(chunk)) = stream.next().await {
                parser.push(&chunk, &mut events);
                let _ = parser.take_overflow();
                for ev in events.drain(..) {
                    if let Some(v) = ev.json() {
                        if tx.send(v).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Some(ServerStream { rx, task }))
    }

    pub async fn close(&self) {
        let had_session = self.session_id().is_some();
        if had_session {
            let _ = self.build(reqwest::Method::DELETE).send().await;
        }
        if let Ok(mut g) = self.session.lock() {
            *g = None;
        }
    }
}

fn pick(value: &Value, key: &str) -> Option<Result<Value, McpError>> {
    match classify(value) {
        Incoming::Result { id, result } if id == key => Some(Ok(result)),
        Incoming::Failure { id, error } if id == key => Some(Err(McpError::proto(format!(
            "JSON-RPC error {}: {}",
            error.code, error.message
        )))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mcp::types::PROTOCOL_VERSION;
    use serde_json::json;

    #[test]
    fn accept_lists_both_content_types() {
        assert!(ACCEPT.contains("application/json"));
        assert!(ACCEPT.contains("text/event-stream"));
    }

    #[test]
    fn event_stream_is_detected_with_and_without_parameters() {
        assert!(is_event_stream("text/event-stream"));
        assert!(is_event_stream("text/event-stream; charset=utf-8"));
        assert!(is_event_stream("TEXT/EVENT-STREAM"));
        assert!(!is_event_stream("application/json"));
        assert!(!is_event_stream(""));
    }

    #[test]
    fn loopback_urls_skip_the_proxy() {
        assert!(is_loopback("http://127.0.0.1:47720/mcp"));
        assert!(is_loopback("http://localhost:3000/mcp"));
        assert!(!is_loopback("https://api.githubcopilot.com/mcp/"));
        assert!(!is_loopback("not a url"));
    }

    #[test]
    fn a_single_object_answer_is_matched_by_id() {
        let body = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#;
        assert_eq!(
            match_response(body, "2").unwrap().unwrap(),
            json!({ "tools": [] })
        );
        assert!(match_response(body, "3").is_none());
    }

    #[test]
    fn a_batch_answer_is_searched_for_our_id() {
        let body = r#"[{"jsonrpc":"2.0","id":1,"result":{"a":1}},{"jsonrpc":"2.0","id":2,"result":{"b":2}}]"#;
        assert_eq!(
            match_response(body, "2").unwrap().unwrap(),
            json!({ "b": 2 })
        );
    }

    #[test]
    fn a_json_rpc_error_in_the_body_becomes_err_mcp_proto() {
        let body = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"bad params"}}"#;
        let err = match_response(body, "1").unwrap().unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_PROTO);
        assert!(err.message.contains("bad params"));
    }

    #[test]
    fn garbage_in_the_body_matches_nothing() {
        assert!(match_response("<html>502</html>", "1").is_none());
    }

    #[test]
    fn pick_ignores_server_notifications_on_the_stream() {
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/message" });
        assert!(pick(&note, "1").is_none());
        let other = json!({ "jsonrpc": "2.0", "id": 9, "result": {} });
        assert!(pick(&other, "1").is_none());
        let ours = json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } });
        assert_eq!(pick(&ours, "1").unwrap().unwrap(), json!({ "ok": true }));
    }

    #[test]
    fn a_transport_without_a_url_is_refused() {
        let err = HttpTransport::connect(
            "   ",
            &BTreeMap::new(),
            &crate::core::mcp::types::default_lookup(),
        )
        .unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_HTTP);
    }

    #[test]
    fn the_protocol_version_is_only_sent_after_it_is_negotiated() {
        let t = HttpTransport::connect(
            "http://127.0.0.1:1/mcp",
            &BTreeMap::new(),
            &crate::core::mcp::types::default_lookup(),
        )
        .unwrap();
        assert_eq!(t.protocol.lock().unwrap().clone(), None);
        t.set_protocol(PROTOCOL_VERSION);
        assert_eq!(
            t.protocol.lock().unwrap().clone(),
            Some(PROTOCOL_VERSION.to_string())
        );
        assert_eq!(t.session_id(), None);
    }
}
