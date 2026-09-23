#!/usr/bin/env node
// Cliente MCP de fora contra o servidor embutido do OmniGet (`POST /mcp` no
// bridge local, mesmo bearer da extensão). É o contrário do `mcp-fixture.mjs`,
// onde nós somos o cliente de um servidor de mentira.
//
// Uso (app aberto, Tools → AI → MCP server ligado, uma pasta anexada no /llm):
//   node scripts/mcp-client-smoke.mjs http://127.0.0.1:47720/mcp <token> [arquivo] [agente]
//
// Faz `initialize`, `tools/list`, `tools/call fs_read` na pasta ativa e, se um
// agente for passado, `tools/call agent_delegate`. Sai com 1 no primeiro erro.
//
// Rodado em 2026-09-19 contra o build de debug da 0.10.0 (perfil isolado, pasta
// ~/omniget-demo, agente claude-code):
//   node scripts/mcp-client-smoke.mjs http://127.0.0.1:47781/mcp $TOKEN src/cart.js claude-code
// Saída:
//   {"step":"initialize","server":"OmniGet","protocol":"2025-06-18"}
//   {"step":"tools/list","count":49,"has_fs_read":true,"has_agent_delegate":true}
//   {"step":"fs_read","isError":false,"lines":8,"path":"src/cart.js"}
//   {"step":"agent_delegate","isError":false,"agent":"claude-code","answer":"pong"}

// Token de sessão de driver (Central, rodada 3): modo só-lista, sem fs_read.
//   node scripts/mcp-client-smoke.mjs <url> <token> --list [--expect a,b] [--forbid c,d]
//   node scripts/mcp-client-smoke.mjs <url> <token> --expect-unauthorized
// O teste `mcp::tests::session_token_over_http` roda os dois contra um
// servidor de verdade quando OMNIGET_MCP_SMOKE_NODE=1.

const argv = process.argv.slice(2);
const flags = new Map();
const positional = [];
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  if (a === "--list" || a === "--expect-unauthorized") flags.set(a, true);
  else if (a === "--expect" || a === "--forbid") flags.set(a, (argv[++i] ?? "").split(",").filter(Boolean));
  else positional.push(a);
}
const [url, token, file = "README.md", agent] = positional;
if (!url || !token) {
  console.error("usage: mcp-client-smoke.mjs <http://127.0.0.1:PORT/mcp> <token> [file] [agent] | --list [--expect a,b] [--forbid c,d] | --expect-unauthorized");
  process.exit(2);
}

if (flags.get("--expect-unauthorized")) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list", params: {} }),
  });
  console.log(JSON.stringify({ step: "unauthorized", status: res.status }));
  process.exit(res.status === 401 ? 0 : 1);
}

let id = 0;
let session = null;

async function rpc(method, params) {
  const headers = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
    authorization: `Bearer ${token}`,
  };
  if (session) headers["mcp-session-id"] = session;
  const res = await fetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }),
  });
  session = res.headers.get("mcp-session-id") ?? session;
  const text = await res.text();
  if (!res.ok) throw new Error(`${method}: HTTP ${res.status} ${text.slice(0, 200)}`);
  // Streamable HTTP may answer as one SSE event.
  const body = text.startsWith("event:") || text.startsWith("data:")
    ? text.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5)).join("")
    : text;
  const msg = JSON.parse(body);
  if (msg.error) throw new Error(`${method}: ${JSON.stringify(msg.error)}`);
  return msg.result;
}

function payload(result) {
  if (result.structuredContent) return result.structuredContent;
  const text = result.content?.[0]?.text ?? "";
  try {
    return JSON.parse(text);
  } catch {
    return { text };
  }
}

try {
  const init = await rpc("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "omniget-mcp-client-smoke", version: "1" },
  });
  console.log(JSON.stringify({ step: "initialize", server: init.serverInfo?.name, protocol: init.protocolVersion }));

  const list = await rpc("tools/list", {});
  const names = list.tools.map((t) => t.name);
  console.log(JSON.stringify({
    step: "tools/list",
    count: names.length,
    has_fs_read: names.includes("fs_read"),
    has_agent_delegate: names.includes("agent_delegate"),
  }));
  if (flags.get("--list")) {
    const missing = (flags.get("--expect") ?? []).filter((n) => !names.includes(n));
    const leaked = (flags.get("--forbid") ?? []).filter((n) => names.includes(n));
    console.log(JSON.stringify({ step: "scope", missing, leaked, catalog: names.filter((n) => n.startsWith("catalog_")) }));
    process.exit(missing.length || leaked.length ? 1 : 0);
  }

  const read = await rpc("tools/call", { name: "fs_read", arguments: { path: file } });
  const r = payload(read);
  console.log(JSON.stringify({ step: "fs_read", isError: !!read.isError, lines: r.lines, path: r.path ?? r.text }));
  if (read.isError) process.exit(1);

  if (agent) {
    const del = await rpc("tools/call", {
      name: "agent_delegate",
      arguments: { agent_id: agent, task: "Reply with exactly one word: pong" },
    });
    const d = payload(del);
    console.log(JSON.stringify({ step: "agent_delegate", isError: !!del.isError, agent: d.agent, answer: d.answer ?? d.text }));
    if (del.isError) process.exit(1);
  }
} catch (e) {
  console.error(String(e));
  process.exit(1);
}
