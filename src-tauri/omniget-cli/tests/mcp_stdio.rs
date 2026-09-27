//! The packaged `omniget-mcp` binary, started the way a registry probes it:
//! no token, no desktop app, just `initialize` and `tools/list` on stdin.
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn answers_the_handshake_without_token_or_app() {
    // A port nothing listens on, so even a stray request could not succeed.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", closed.local_addr().unwrap());
    drop(closed);
    let mut child = Command::new(env!("CARGO_BIN_EXE_omniget-mcp"))
        .env_remove("OMNIGET_MCP_TOKEN")
        .env("OMNIGET_MCP_URL", url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"downloads_queue","arguments":{}}}),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for line in &lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 3, "{replies:?}");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "OmniGet");
    assert!(replies[1]["result"]["tools"].as_array().unwrap().len() >= 20);
    assert_eq!(
        replies[2]["result"]["structuredContent"]["error"]["code"],
        "TOKEN_MISSING"
    );
}
