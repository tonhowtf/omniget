//! One MCP server, seen as a client. Owned by f3-mcp-core.
//!
//! Primary source for every protocol decision here: the MCP specification
//! 2025-06-18 — "Base Protocol → Lifecycle" and "Base Protocol → Transports"
//! (<https://modelcontextprotocol.io/specification/2025-06-18/basic/transports>,
//! read 2026-09-18). The lifecycle it pins is:
//!
//! 1. the client sends `initialize` with its protocol version and capabilities;
//! 2. the server answers with the version it will speak;
//! 3. the client sends the `notifications/initialized` notification;
//! 4. only then are normal requests allowed.
//!
//! Everything else this file does — `tools/list` with cursor paging, a
//! `tools/call` whose `isError: true` becomes `ERR_MCP_TOOL`, `resources/list`
//! and `prompts/list` asked for only when the server declared the capability —
//! follows from that document plus the budget in the prompt: the process is
//! born on the first `tools()`/`call()`, `tools/list` is cached for the life of
//! the session, and nothing runs in the background.

use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::error::{McpError, ERR_MCP_PROTO};
use super::stdio::StdioTransport;
use super::streamable_http::HttpTransport;
use super::types::{
    default_lookup, initialize_params, is_tool_error, parse_tools, tool_result_text,
    InitializeResult, McpServerConfig, SecretLookup, ServerInfo, ToolDef, Transport,
    SUPPORTED_PROTOCOL_VERSIONS,
};

/// A `tools/list` that keeps handing out cursors is either huge or broken.
const MAX_TOOL_PAGES: usize = 20;

enum Wire {
    // Boxed: on Linux the stdio transport is almost three times the HTTP one.
    Stdio(Box<StdioTransport>),
    Http(HttpTransport),
}

impl Wire {
    async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, McpError> {
        match self {
            Wire::Stdio(t) => t.request(method, params, timeout, cancel).await,
            Wire::Http(t) => t.request(method, params, timeout, cancel).await,
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        match self {
            Wire::Stdio(t) => t.notify(method, params).await,
            Wire::Http(t) => t.notify(method, params).await,
        }
    }

    async fn close(&self) {
        match self {
            Wire::Stdio(t) => t.close().await,
            Wire::Http(t) => t.close().await,
        }
    }
}

pub struct McpClient {
    id: String,
    name: String,
    wire: Wire,
    timeout: Duration,
    init: InitializeResult,
    tools: RwLock<Option<Arc<Vec<ToolDef>>>>,
    last_used: StdMutex<Instant>,
}

impl McpClient {
    /// Connects with the app's secret store behind `secret:<id>` references.
    pub async fn connect(cfg: &McpServerConfig) -> Result<Self, McpError> {
        Self::connect_with(cfg, &default_lookup()).await
    }

    /// Connects and completes the handshake. Returns only after the server has
    /// answered `initialize` and been told `notifications/initialized`.
    /// `lookup` resolves the `secret:<id>` references in headers **and** in the
    /// environment of a stdio server.
    pub async fn connect_with(
        cfg: &McpServerConfig,
        lookup: &SecretLookup,
    ) -> Result<Self, McpError> {
        if !cfg.enabled {
            return Err(McpError::spawn(format!(
                "the MCP server {:?} is turned off",
                cfg.id
            )));
        }
        let timeout = cfg.timeout();
        let wire = match &cfg.transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => Wire::Stdio(Box::new(
                StdioTransport::connect(command, args, env, cwd.as_deref(), lookup).await?,
            )),
            Transport::Http { url, headers } => {
                Wire::Http(HttpTransport::connect(url, headers, lookup)?)
            }
        };

        let result = wire
            .request(
                "initialize",
                Some(initialize_params(env!("CARGO_PKG_VERSION"))),
                timeout,
                None,
            )
            .await;
        let result = match result {
            Ok(v) => v,
            Err(e) => {
                wire.close().await;
                return Err(e);
            }
        };
        let init: InitializeResult = serde_json::from_value(result.clone()).unwrap_or_default();
        if init.protocol_version.is_empty() {
            wire.close().await;
            return Err(McpError::proto(
                "the server answered initialize without a protocolVersion",
            ));
        }
        if let Wire::Http(t) = &wire {
            t.set_protocol(&init.protocol_version);
        }
        // A version we have not seen is not a reason to refuse: the four
        // methods we use have not changed across the three published versions,
        // and a newer server is more likely to work than to break.
        if let Err(e) = wire.notify("notifications/initialized", None).await {
            wire.close().await;
            return Err(e);
        }

        Ok(Self {
            id: cfg.id.clone(),
            name: if cfg.name.trim().is_empty() {
                cfg.id.clone()
            } else {
                cfg.name.clone()
            },
            wire,
            timeout,
            init,
            tools: RwLock::new(None),
            last_used: StdMutex::new(Instant::now()),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.init.server_info
    }

    pub fn protocol_version(&self) -> &str {
        &self.init.protocol_version
    }

    /// `false` when the server negotiated a version this client has not been
    /// tested against. The UI shows a banner; nothing is blocked.
    pub fn protocol_is_known(&self) -> bool {
        SUPPORTED_PROTOCOL_VERSIONS.contains(&self.init.protocol_version.as_str())
    }

    pub fn instructions(&self) -> Option<&str> {
        self.init.instructions.as_deref()
    }

    /// How long since the last request. The registry's reaper reads this.
    pub fn idle(&self) -> Duration {
        self.last_used
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    fn touch(&self) {
        if let Ok(mut t) = self.last_used.lock() {
            *t = Instant::now();
        }
    }

    /// Tool list, cached for the life of the session (budget §6). The cache is
    /// only dropped by [`Self::refresh_tools`].
    pub async fn tools(&self) -> Result<Arc<Vec<ToolDef>>, McpError> {
        if let Some(cached) = self.tools.read().ok().and_then(|g| g.clone()) {
            return Ok(cached);
        }
        self.refresh_tools().await
    }

    /// Re-reads `tools/list` and replaces the cache.
    pub async fn refresh_tools(&self) -> Result<Arc<Vec<ToolDef>>, McpError> {
        self.touch();
        let mut all: Vec<ToolDef> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor
                .as_ref()
                .map(|c| json!({ "cursor": c }))
                .unwrap_or_else(|| json!({}));
            let result = self
                .wire
                .request("tools/list", Some(params), self.timeout, None)
                .await?;
            all.extend(parse_tools(&result));
            cursor = result
                .get("nextCursor")
                .and_then(|c| c.as_str())
                .filter(|c| !c.is_empty())
                .map(|c| c.to_string());
            if cursor.is_none() {
                break;
            }
        }
        let tools = Arc::new(all);
        if let Ok(mut g) = self.tools.write() {
            *g = Some(tools.clone());
        }
        Ok(tools)
    }

    /// Calls a tool. `isError: true` and a JSON-RPC error both come back as
    /// `ERR_MCP_TOOL`: from the caller's seat, the tool failed either way.
    pub async fn call(
        &self,
        name: &str,
        args: Value,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, McpError> {
        self.touch();
        let params = json!({ "name": name, "arguments": args });
        let result = self
            .wire
            .request("tools/call", Some(params), self.timeout, cancel)
            .await
            .map_err(|e| tool_level(e, name))?;
        self.touch();
        if is_tool_error(&result) {
            let text = tool_result_text(&result);
            return Err(McpError::tool(if text.is_empty() {
                format!("{} failed", name)
            } else {
                text
            }));
        }
        Ok(result)
    }

    /// For display only. Asked for just when the server declared it, so a
    /// server without resources is never bothered.
    pub async fn resources(&self) -> Result<Vec<Value>, McpError> {
        self.list_display("resources/list", "resources").await
    }

    pub async fn prompts(&self) -> Result<Vec<Value>, McpError> {
        self.list_display("prompts/list", "prompts").await
    }

    async fn list_display(&self, method: &str, key: &str) -> Result<Vec<Value>, McpError> {
        if !self.init.supports(key) {
            return Ok(Vec::new());
        }
        self.touch();
        let result = self
            .wire
            .request(method, Some(json!({})), self.timeout, None)
            .await?;
        Ok(result
            .get(key)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Ends the session and stops the process. Idempotent.
    pub async fn close(&self) {
        self.wire.close().await;
    }

    /// The child's pid, when this is a stdio server. Tests use it to prove the
    /// process is gone after `close()`.
    pub fn is_stdio(&self) -> bool {
        matches!(self.wire, Wire::Stdio(_))
    }
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("id", &self.id)
            .field("transport", &if self.is_stdio() { "stdio" } else { "http" })
            .field("protocol", &self.init.protocol_version)
            .field("server", &self.init.server_info.name)
            .finish()
    }
}

/// A protocol error raised by `tools/call` is a tool error: the peer answered,
/// it just refused this call (unknown tool, bad arguments).
fn tool_level(err: McpError, name: &str) -> McpError {
    if err.code == ERR_MCP_PROTO && err.message.starts_with("JSON-RPC error") {
        return McpError::tool(format!("{}: {}", name, err.message));
    }
    err
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Helpers around `scripts/mcp-fixture.mjs`. Everything spawned here is
    //! killed on drop, which is what makes `pgrep -f mcp-fixture` empty after
    //! the suite.

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::process::Stdio;

    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Child;

    use crate::core::mcp::types::{McpServerConfig, Transport};

    pub fn script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("scripts")
            .join("mcp-fixture.mjs")
    }

    pub fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("mcp_fixtures")
    }

    /// `false` when `node` is not installed: those tests then skip loudly
    /// instead of failing a machine that cannot run them.
    pub fn have_node() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn stdio_config(id: &str, extra: &[&str], timeout_ms: Option<u64>) -> McpServerConfig {
        let mut args = vec![script().to_string_lossy().into_owned(), "--stdio".into()];
        args.extend(extra.iter().map(|s| s.to_string()));
        McpServerConfig {
            id: id.into(),
            name: "fixture".into(),
            transport: Transport::Stdio {
                command: "node".into(),
                args,
                env: BTreeMap::new(),
                cwd: None,
            },
            enabled: true,
            timeout_ms,
        }
    }

    /// A running HTTP fixture. Killed when dropped.
    pub struct HttpFixture {
        child: Child,
        pub port: u16,
    }

    impl HttpFixture {
        pub fn url(&self) -> String {
            format!("http://127.0.0.1:{}/mcp", self.port)
        }

        pub fn config(&self, id: &str, timeout_ms: Option<u64>) -> McpServerConfig {
            McpServerConfig {
                id: id.into(),
                name: "fixture-http".into(),
                transport: Transport::Http {
                    url: self.url(),
                    headers: BTreeMap::new(),
                },
                enabled: true,
                timeout_ms,
            }
        }

        pub async fn stop(mut self) {
            let _ = self.child.kill().await;
        }
    }

    pub async fn http(extra: &[&str]) -> HttpFixture {
        let mut cmd = tokio::process::Command::new("node");
        cmd.arg(script()).arg("--http").arg("--port").arg("0");
        cmd.args(extra);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().expect("node should start the HTTP fixture");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut lines = BufReader::new(stdout).lines();
        let port = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(rest) = line.strip_prefix("MCP_FIXTURE_PORT ") {
                    return rest.trim().parse::<u16>().ok();
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
        .expect("the fixture should print its port");
        HttpFixture { child, port }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixture::{have_node, http, stdio_config};
    use serde_json::json;

    macro_rules! need_node {
        () => {
            if !have_node() {
                eprintln!("skipping: node is not on PATH");
                return;
            }
        };
    }

    #[tokio::test]
    async fn stdio_handshake_lists_tools_and_calls_one() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &[], None))
            .await
            .expect("connect");
        assert_eq!(client.server_info().name, "mcp-fixture");
        assert!(client.protocol_is_known(), "{}", client.protocol_version());
        assert!(client.instructions().is_some());

        let tools = client.tools().await.expect("tools/list");
        assert_eq!(tools.len(), 7, "{:?}", tools);
        assert!(tools.iter().all(|t| t.schema_is_object()));

        let out = client
            .call("add", json!({ "a": 2, "b": 3 }), None)
            .await
            .expect("tools/call");
        assert_eq!(out["structuredContent"]["sum"], 5);
        assert_eq!(tool_result_text(&out), "5");
        client.close().await;
    }

    #[tokio::test]
    async fn stdio_tools_are_cached_for_the_session() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &[], None))
            .await
            .unwrap();
        let a = client.tools().await.unwrap();
        let b = client.tools().await.unwrap();
        assert!(Arc::ptr_eq(&a, &b), "the second call must not hit the wire");
        let c = client.refresh_tools().await.unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        assert_eq!(a.len(), c.len());
        client.close().await;
    }

    #[tokio::test]
    async fn stdio_a_failing_tool_is_err_mcp_tool() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &[], None))
            .await
            .unwrap();
        let err = client.call("fail", json!({}), None).await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_TOOL);
        assert!(err.message.contains("on purpose"), "{}", err.message);
        client.close().await;
    }

    #[tokio::test]
    async fn stdio_an_unknown_tool_is_also_a_tool_error() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &[], None))
            .await
            .unwrap();
        let err = client.call("nope", json!({}), None).await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_TOOL);
        client.close().await;
    }

    /// The gate's "servidor que morre no meio devolve ERR_MCP_PROTO, não pânico".
    #[tokio::test]
    async fn stdio_a_server_that_dies_mid_session_fails_with_proto() {
        need_node!();
        // Exits right after answering `initialize`. The roomy window only
        // covers `node`'s start-up on a loaded machine: the crash answers at
        // once, so it never waits it out.
        let client = McpClient::connect(&stdio_config("fx", &["--crash-after", "1"], Some(30_000)))
            .await
            .expect("the handshake still completes");
        let err = client.tools().await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_PROTO, "{}", err);
        // And a second attempt is a clean error too, not a hang or a panic.
        let again = client.call("add", json!({}), None).await.unwrap_err();
        assert_eq!(again.code, super::super::error::ERR_MCP_PROTO);
        client.close().await;
    }

    #[tokio::test]
    async fn stdio_noise_on_stdout_does_not_break_the_session() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &["--garbage"], None))
            .await
            .expect("a banner line before the answer is survivable");
        assert_eq!(client.tools().await.unwrap().len(), 7);
        client.close().await;
    }

    #[tokio::test]
    async fn stdio_a_slow_tool_hits_the_timeout() {
        need_node!();
        // The handshake gets the roomy window (`node`'s start-up on a loaded
        // machine); the call gets a short one, far below the tool's minute.
        let mut client = McpClient::connect(&stdio_config("fx", &[], Some(30_000)))
            .await
            .unwrap();
        client.timeout = Duration::from_millis(300);
        let err = client
            .call("slow", json!({ "ms": 60_000 }), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_TIMEOUT);
        assert!(err.retryable);
        client.close().await;
    }

    #[tokio::test]
    async fn stdio_a_call_can_be_cancelled() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &[], Some(30_000)))
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
        });
        let started = Instant::now();
        let err = client
            .call("slow", json!({ "ms": 10_000 }), Some(&cancel))
            .await
            .unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(5));
        client.close().await;
    }

    #[tokio::test]
    async fn stdio_carries_a_payload_bigger_than_the_sse_ceiling() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &[], Some(30_000)))
            .await
            .unwrap();
        // 2 MiB in one line: over the 1 MiB SSE ceiling, under the stdio one.
        let out = client
            .call("big", json!({ "kb": 2048 }), None)
            .await
            .unwrap();
        assert_eq!(tool_result_text(&out).len(), 2048 * 1024);
        client.close().await;
    }

    #[tokio::test]
    async fn a_disabled_server_is_never_started() {
        let mut cfg = stdio_config("fx", &[], None);
        cfg.enabled = false;
        let err = McpClient::connect(&cfg).await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_SPAWN);
    }

    #[tokio::test]
    async fn a_missing_binary_is_err_mcp_spawn() {
        let mut cfg = stdio_config("fx", &[], None);
        cfg.transport = Transport::Stdio {
            command: "omniget-no-such-binary".into(),
            args: vec![],
            env: Default::default(),
            cwd: None,
        };
        let err = McpClient::connect(&cfg).await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_SPAWN);
    }

    #[tokio::test]
    async fn http_json_mode_does_the_whole_round_trip() {
        need_node!();
        let server = http(&[]).await;
        let client = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap();
        assert_eq!(client.server_info().name, "mcp-fixture");
        assert_eq!(client.tools().await.unwrap().len(), 7);
        let out = client
            .call("echo", json!({ "text": "oi" }), None)
            .await
            .unwrap();
        assert_eq!(tool_result_text(&out), "oi");
        client.close().await;
        server.stop().await;
    }

    #[tokio::test]
    async fn http_sse_answers_are_read_past_the_server_notifications() {
        need_node!();
        let server = http(&["--sse"]).await;
        let client = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap();
        let out = client
            .call("echo", json!({ "text": "sse" }), None)
            .await
            .unwrap();
        assert_eq!(tool_result_text(&out), "sse");
        client.close().await;
        server.stop().await;
    }

    #[tokio::test]
    async fn http_a_stream_cut_before_the_response_is_proto() {
        need_node!();
        let server = http(&["--partial-sse"]).await;
        let err = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_PROTO, "{}", err);
        server.stop().await;
    }

    #[tokio::test]
    async fn http_an_expired_session_says_so_and_is_retryable() {
        need_node!();
        // Two POSTs after `initialize` (the `initialized` notification and one
        // `tools/list`), then every later one is 404.
        let server = http(&["--expire-after", "2"]).await;
        let client = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap();
        assert!(client.tools().await.is_ok());
        let err = client.refresh_tools().await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_HTTP);
        assert!(err.retryable);
        server.stop().await;
    }

    #[tokio::test]
    async fn http_works_against_a_server_that_issues_no_session() {
        need_node!();
        let server = http(&["--no-session"]).await;
        let client = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap();
        assert_eq!(client.tools().await.unwrap().len(), 7);
        client.close().await;
        server.stop().await;
    }

    #[tokio::test]
    async fn http_a_server_error_status_is_err_mcp_http() {
        need_node!();
        let server = http(&["--status", "503"]).await;
        let client = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap();
        let err = client.tools().await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_HTTP);
        assert!(err.retryable, "5xx is worth another try");
        server.stop().await;
    }

    #[tokio::test]
    async fn http_resources_and_prompts_are_listed_for_display() {
        need_node!();
        let server = http(&[]).await;
        let client = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap();
        assert_eq!(client.resources().await.unwrap().len(), 1);
        assert_eq!(client.prompts().await.unwrap().len(), 1);
        client.close().await;
        server.stop().await;
    }

    #[tokio::test]
    async fn http_get_opens_a_server_stream_and_405_is_not_an_error() {
        need_node!();
        let server = http(&[]).await;
        let t =
            HttpTransport::connect(&server.url(), &Default::default(), &default_lookup()).unwrap();
        let mut stream = t
            .open_server_stream()
            .await
            .unwrap()
            .expect("this fixture offers a stream");
        let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("a notification arrives")
            .expect("some value");
        assert_eq!(first["method"], "notifications/message");
        drop(stream);
        server.stop().await;

        let quiet = http(&["--no-get"]).await;
        let t2 =
            HttpTransport::connect(&quiet.url(), &Default::default(), &default_lookup()).unwrap();
        assert!(t2.open_server_stream().await.unwrap().is_none());
        quiet.stop().await;
    }

    #[tokio::test]
    async fn http_keeps_the_session_id_and_drops_it_on_close() {
        need_node!();
        let server = http(&[]).await;
        let t =
            HttpTransport::connect(&server.url(), &Default::default(), &default_lookup()).unwrap();
        t.request(
            "initialize",
            Some(initialize_params("test")),
            Duration::from_secs(10),
            None,
        )
        .await
        .unwrap();
        assert!(t.session_id().is_some(), "the fixture hands out a session");
        t.close().await;
        assert_eq!(t.session_id(), None);
        server.stop().await;
    }

    #[tokio::test]
    async fn the_idle_clock_restarts_on_every_call() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &[], None))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(client.idle() >= Duration::from_millis(100));
        client.call("echo", json!({ "text": "x" }), None).await.ok();
        assert!(client.idle() < Duration::from_millis(100));
        client.close().await;
    }

    /// A message past the 8 MiB ceiling is dropped and reported as a protocol
    /// error, instead of hanging until the timeout — and the framer never
    /// buffers it, so 9 MiB on one line does not become 9 MiB of ours.
    #[tokio::test]
    async fn stdio_a_line_over_the_ceiling_is_proto_not_a_hang() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &["--huge-line", "9"], Some(30_000)))
            .await
            .expect("the handshake is a normal-sized message");
        let started = Instant::now();
        let err = client.tools().await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_PROTO, "{}", err);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "it must fail on the drop, not on the 30 s timeout"
        );
        client.close().await;
    }

    /// Ids come back as `"3"` instead of `3` on real servers.
    #[tokio::test]
    async fn stdio_string_ids_are_matched_against_our_numeric_ones() {
        need_node!();
        let client = McpClient::connect(&stdio_config("fx", &["--string-ids"], Some(10_000)))
            .await
            .expect("initialize already needs the id to match");
        assert_eq!(client.tools().await.unwrap().len(), 7);
        let out = client
            .call("add", json!({ "a": 20, "b": 22 }), None)
            .await
            .unwrap();
        assert_eq!(out["structuredContent"]["sum"], 42);
        client.close().await;
    }

    #[tokio::test]
    async fn http_string_ids_are_matched_too() {
        need_node!();
        let server = http(&["--string-ids"]).await;
        let client = McpClient::connect(&server.config("h", Some(10_000)))
            .await
            .unwrap();
        assert_eq!(client.tools().await.unwrap().len(), 7);
        client.close().await;
        server.stop().await;
    }

    // ── Recorded fixtures of real servers (plan §3, "Plano B") ────────

    fn recorded(name: &str) -> Value {
        let path = fixture::fixtures_dir().join(format!("{}.json", name));
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
    }

    const RECORDED: &[&str] = &["filesystem", "github", "fetch", "playwright", "context7"];

    #[test]
    fn every_recorded_server_parses_into_tools() {
        for name in RECORDED {
            let fx = recorded(name);
            let tools = parse_tools(&fx["tools_list"]["result"]);
            assert!(!tools.is_empty(), "{} has no tools", name);
            for t in &tools {
                assert!(!t.name.is_empty(), "{}: nameless tool", name);
                assert!(
                    t.schema_is_object(),
                    "{}: {} has a non-object schema",
                    name,
                    t.name
                );
            }
        }
    }

    #[test]
    fn every_recorded_server_carries_a_call_we_can_read() {
        for name in RECORDED {
            let fx = recorded(name);
            let call = &fx["tools_call"];
            let result = &call["response"]["result"];
            assert!(
                result.is_object(),
                "{}: the recorded call has no result",
                name
            );
            let text = tool_result_text(result);
            let errored = is_tool_error(result);
            assert!(
                !text.is_empty() || result.get("structuredContent").is_some() || errored,
                "{}: the recorded call says nothing",
                name
            );
            // The tool that was called is in the recorded listing.
            let called = call["request"]["params"]["name"].as_str().unwrap_or("");
            let tools = parse_tools(&fx["tools_list"]["result"]);
            assert!(
                tools.iter().any(|t| t.name == called),
                "{}: called {:?}, which is not in tools/list",
                name,
                called
            );
        }
    }

    #[test]
    fn no_recorded_fixture_contains_a_token() {
        for name in RECORDED {
            let path = fixture::fixtures_dir().join(format!("{}.json", name));
            let text = std::fs::read_to_string(&path).unwrap();
            let lower = text.to_lowercase();
            for needle in [
                "ghp_",
                "github_pat_",
                "authorization",
                "sk-",
                "bearer ",
                "api_key\":\"",
            ] {
                assert!(!lower.contains(needle), "{} contains {:?}", name, needle);
            }
        }
    }

    #[test]
    fn the_recorded_initialize_results_are_versions_we_speak() {
        for name in RECORDED {
            let fx = recorded(name);
            let init: InitializeResult =
                serde_json::from_value(fx["initialize"]["result"].clone()).unwrap();
            assert!(
                SUPPORTED_PROTOCOL_VERSIONS.contains(&init.protocol_version.as_str()),
                "{}: {}",
                name,
                init.protocol_version
            );
            assert!(!init.server_info.name.is_empty(), "{}", name);
        }
    }
}
