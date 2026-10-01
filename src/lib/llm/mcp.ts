/**
 * Pure side of the MCP tab: the wire contract of the `llm_mcp_*` commands and
 * every function that turns text into a config (and back) without touching
 * Tauri, the DOM or a timer.
 *
 * Hand-written mirror of `core/mcp/types.rs` (f3-mcp-core), which the commands
 * in `src-tauri/src/commands/llm/mcp.rs` pass through verbatim. Serde shapes
 * that matter:
 *   - `McpTransport` is internally tagged; the tag is `kind`, lowercase
 *     (`"stdio"` or `"http"`), and it sits inside the `transport` field.
 *   - a tool carries `inputSchema`, camelCase, because that is what the MCP
 *     wire calls it; every other field is `snake_case`.
 * A mismatch here is a silent bug, same rule as `$lib/llm/types`.
 */
import type { AgentDef, GrantMode, ToolGrant } from "./types";

// ── Wire contract ───────────────────────────────────────────────────────

type McpTransport =
  | {
      kind: "stdio";
      command: string;
      args: string[];
      env: Record<string, string>;
      cwd?: string | null;
    }
  | { kind: "http"; url: string; headers: Record<string, string> };

export interface McpServerConfig {
  id: string;
  name: string;
  transport: McpTransport;
  enabled: boolean;
  /** Per-call deadline; the backend default is 60 s. */
  timeout_ms?: number | null;
}

/** One tool as the remote server described it (`tools/list`). */
export interface McpToolDef {
  name: string;
  description: string;
  inputSchema?: unknown;
  outputSchema?: unknown;
  title?: string | null;
}

/** Answer of `llm_mcp_test`: one connect + `tools/list`, then disconnect. */
export interface McpTestResult {
  ok: boolean;
  /** `serverInfo.name` as the server reported it, or `null`. */
  server_name?: string | null;
  protocol_version?: string | null;
  /** `false` when the server negotiated a version we have not tested. */
  protocol_known?: boolean;
  tool_count: number;
  elapsed_ms: number;
  /** `ERR_MCP_*` plus a human message, or `null` when `ok`. */
  error?: { code: string; message: string } | null;
}

/** Answer of `llm_mcp_list`: the saved config plus what the UI shows about it. */
export interface McpServerRow {
  config: McpServerConfig;
  /** `tools/list` cached from the last connection; empty when never probed. */
  tools: McpToolDef[];
  connected: boolean;
  last_error?: { code: string; message: string } | null;
}

const MCP_ERROR_KEYS: Record<string, string> = {
  ERR_MCP_SPAWN: "llm.mcp.err.spawn",
  ERR_MCP_PROTO: "llm.mcp.err.proto",
  ERR_MCP_TIMEOUT: "llm.mcp.err.timeout",
  ERR_MCP_HTTP: "llm.mcp.err.http",
  ERR_MCP_TOOL: "llm.mcp.err.tool",
  ERR_MCP_CANCELLED: "llm.mcp.err.cancelled",
  ERR_MCP_CONFIG: "llm.mcp.err.config",
  ERR_MCP_NO_STORE: "llm.mcp.err.no_store",
  ERR_MCP_UNKNOWN_AGENT: "llm.mcp.err.unknown_agent",
};

/** i18n key for a backend error string. Unknown codes get the generic one. */
export function mcpErrorKey(err: unknown): string {
  const text = String(err ?? "");
  for (const [code, key] of Object.entries(MCP_ERROR_KEYS)) {
    if (text.includes(code)) return key;
  }
  return "llm.mcp.err.generic";
}

// ── Validation (same rules as the Rust wrapper) ─────────────────────────

/**
 * Lowercase, digits, dash and underscore, up to 64 characters: the exact rule
 * `core::mcp::types::valid_id` enforces. If these drift, the form accepts a
 * config the registry then refuses.
 */
const ID_RE = /^[a-z0-9_-]{1,64}$/;

/** Turns a display name into a usable id. Never empty. */
export function slugifyId(name: string): string {
  const slug = name
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
  return slug || "server";
}

/**
 * Every problem with a config, as i18n keys, in field order. Empty means the
 * form can be saved. The Rust wrapper refuses the same cases with
 * `ERR_MCP_CONFIG`, so a bad config never reaches the registry.
 */
export function validateServer(config: McpServerConfig): string[] {
  const problems: string[] = [];
  if (!ID_RE.test(config.id)) problems.push("llm.mcp.invalid.id");
  if (!config.name.trim()) problems.push("llm.mcp.invalid.name");
  if (config.transport.kind === "stdio") {
    if (!config.transport.command.trim()) problems.push("llm.mcp.invalid.command");
  } else {
    const url = config.transport.url.trim();
    if (!/^https?:\/\/\S+$/i.test(url)) problems.push("llm.mcp.invalid.url");
  }
  return problems;
}

// ── Text ⇄ config ───────────────────────────────────────────────────────

/**
 * Splits a command line into arguments, honouring single and double quotes so
 * `--root "/Users/me/My Files"` stays one argument. Backslash escapes are left
 * alone: a Windows path is typed far more often than an escape.
 */
export function parseArgs(line: string): string[] {
  const args: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let started = false;
  for (const ch of line) {
    if (quote) {
      if (ch === quote) quote = null;
      else current += ch;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      started = true;
      continue;
    }
    if (/\s/.test(ch)) {
      if (started) args.push(current);
      current = "";
      started = false;
      continue;
    }
    current += ch;
    started = true;
  }
  if (started) args.push(current);
  return args;
}

/** Inverse of `parseArgs` for the form field; quotes what has a space. */
export function formatArgs(args: string[]): string {
  return args.map((a) => (/\s/.test(a) ? `"${a}"` : a)).join(" ");
}

/** `KEY=value` lines into a map. Blank lines and `#` comments are dropped. */
export function parsePairs(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    const colon = line.indexOf(":");
    // A header is typed `Name: value`, an env var `NAME=value`; take whichever
    // separator comes first so one parser serves both fields.
    const at = eq >= 0 && (colon < 0 || eq < colon) ? eq : colon;
    if (at <= 0) continue;
    const key = line.slice(0, at).trim();
    const value = line.slice(at + 1).trim();
    if (key) out[key] = value;
  }
  return out;
}

export function formatPairs(pairs: Record<string, string>, sep = "="): string {
  return Object.entries(pairs)
    .map(([k, v]) => `${k}${sep}${v}`)
    .join("\n");
}

/** True for a header value the backend resolves from the secret store. */
export function isSecretRef(value: string): boolean {
  return value.startsWith("secret:") && value.length > "secret:".length;
}

/**
 * Reads the `mcpServers` block every MCP client uses (Claude Desktop, Claude
 * Code, Cursor, Windsurf), so a server already configured elsewhere can be
 * pasted in instead of retyped. Accepts the bare map too. Returns `[]` when
 * the text is not that shape — the caller shows one error, not a stack.
 */
export function parseClientConfig(text: string): McpServerConfig[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return [];
  }
  if (!parsed || typeof parsed !== "object") return [];
  const root = parsed as Record<string, unknown>;
  const block = (root.mcpServers ?? root.servers ?? root) as Record<string, unknown>;
  if (!block || typeof block !== "object") return [];
  const out: McpServerConfig[] = [];
  for (const [key, value] of Object.entries(block)) {
    if (!value || typeof value !== "object") continue;
    const entry = value as Record<string, unknown>;
    const id = ID_RE.test(key) ? key : slugifyId(key);
    const name = typeof entry.name === "string" ? entry.name : key;
    const url = typeof entry.url === "string" ? entry.url : null;
    const command = typeof entry.command === "string" ? entry.command : null;
    if (url) {
      out.push({
        id,
        name,
        enabled: entry.disabled === true ? false : true,
        transport: { kind: "http", url, headers: stringMap(entry.headers) },
      });
    } else if (command) {
      out.push({
        id,
        name,
        enabled: entry.disabled === true ? false : true,
        transport: {
          kind: "stdio",
          command,
          args: Array.isArray(entry.args) ? entry.args.map((a) => String(a)) : [],
          env: stringMap(entry.env),
          cwd: typeof entry.cwd === "string" ? entry.cwd : null,
        },
      });
    }
  }
  return out;
}

function stringMap(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object") return {};
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
    if (typeof v === "string" || typeof v === "number" || typeof v === "boolean") {
      out[k] = String(v);
    }
  }
  return out;
}

// ── Server half ──────────────────────────────────────────────────────────

/** `abcd…wxyz`, or dots when the token is too short to keep a shape. */
export function maskToken(token: string): string {
  return token.length > 8 ? `${token.slice(0, 4)}…${token.slice(-4)}` : "••••";
}

/** The snippet with the bearer token masked, unless the user asked to see it. */
export function maskSnippet(snippet: string, token: string, reveal: boolean): string {
  if (reveal || !token) return snippet;
  return snippet.replaceAll(token, maskToken(token));
}

/** `name(a, b?)` for one tool row: required first, optional with a `?`. */
export function toolSignature(schema: unknown): string {
  if (!schema || typeof schema !== "object") return "";
  const object = schema as { properties?: Record<string, unknown>; required?: unknown };
  const properties = object.properties;
  if (!properties || typeof properties !== "object") return "";
  const required = Array.isArray(object.required) ? object.required.map(String) : [];
  return Object.keys(properties)
    .map((key) => (required.includes(key) ? key : `${key}?`))
    .join(", ");
}

// ── Grants (AgentDef.tools) ─────────────────────────────────────────────

/** `null` when this agent has no grant for the tool at all. */
export function grantModeOf(
  agent: AgentDef | null | undefined,
  server: string,
  tool: string,
): GrantMode | null {
  const grant = (agent?.tools ?? []).find(
    (g) => g.source === "mcp" && g.server === server && g.tool === tool,
  );
  return grant ? grant.mode : null;
}

/**
 * The tool list of an agent after setting (or clearing, with `null`) one MCP
 * grant. Pure: the caller decides when to save. Same rule as the Rust
 * `apply_grant`, which is what actually writes the roster.
 */
export function applyGrant(
  tools: ToolGrant[] | undefined,
  server: string,
  tool: string,
  mode: GrantMode | null,
): ToolGrant[] {
  const rest = (tools ?? []).filter(
    (g) => !(g.source === "mcp" && g.server === server && g.tool === tool),
  );
  if (mode === null) return rest;
  return [...rest, { source: "mcp", server, tool, mode }];
}

// ── Registry shown in the tab (same five servers as the core fixtures) ───

export interface McpRegistryEntry {
  id: string;
  name: string;
  /** i18n key of the one-line description. */
  descKey: string;
  homepage: string;
  /** Template the "install" button drops into the form. */
  template: McpServerConfig;
  /** Placeholders the user still has to fill: env keys or path arguments. */
  needs: string[];
}

/**
 * The five servers the core records fixtures for, plus Microsoft 365 (the
 * way Meetings hands minutes back to Teams, the calendar and OneDrive). Kept
 * here rather than fetched: the tab must open with zero network, and an
 * entry that cannot be installed offline is not worth a request.
 */
export const MCP_REGISTRY: McpRegistryEntry[] = [
  {
    id: "filesystem",
    name: "Filesystem",
    descKey: "llm.mcp.reg.filesystem",
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem",
    needs: ["<path>"],
    template: {
      id: "filesystem",
      name: "Filesystem",
      enabled: true,
      transport: {
        kind: "stdio",
        command: "npx",
        args: ["-y", "@modelcontextprotocol/server-filesystem", "<path>"],
        env: {},
        cwd: null,
      },
    },
  },
  {
    id: "github",
    name: "GitHub",
    descKey: "llm.mcp.reg.github",
    homepage: "https://github.com/github/github-mcp-server",
    needs: ["GITHUB_PERSONAL_ACCESS_TOKEN"],
    template: {
      id: "github",
      name: "GitHub",
      enabled: true,
      transport: {
        kind: "http",
        url: "https://api.githubcopilot.com/mcp/",
        headers: { Authorization: "secret:github-mcp" },
      },
    },
  },
  {
    id: "fetch",
    name: "Fetch",
    descKey: "llm.mcp.reg.fetch",
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/fetch",
    needs: [],
    template: {
      id: "fetch",
      name: "Fetch",
      enabled: true,
      transport: {
        kind: "stdio",
        command: "uvx",
        args: ["mcp-server-fetch"],
        env: {},
        cwd: null,
      },
    },
  },
  {
    id: "playwright",
    name: "Playwright",
    descKey: "llm.mcp.reg.playwright",
    homepage: "https://github.com/microsoft/playwright-mcp",
    needs: [],
    template: {
      id: "playwright",
      name: "Playwright",
      enabled: true,
      transport: {
        kind: "stdio",
        command: "npx",
        args: ["-y", "@playwright/mcp@latest"],
        env: {},
        cwd: null,
      },
    },
  },
  {
    id: "context7",
    name: "Context7",
    descKey: "llm.mcp.reg.context7",
    homepage: "https://github.com/upstash/context7",
    needs: [],
    template: {
      id: "context7",
      name: "Context7",
      enabled: true,
      transport: { kind: "http", url: "https://mcp.context7.com/mcp", headers: {} },
    },
  },
  {
    // Softeria/ms-365-mcp-server (MIT). `--org-mode` turns on Teams,
    // SharePoint and online meetings; the first call signs in with a device
    // code the agent shows to the person.
    id: "microsoft365",
    name: "Microsoft 365",
    descKey: "llm.mcp.reg.microsoft365",
    homepage: "https://github.com/Softeria/ms-365-mcp-server",
    needs: [],
    template: {
      id: "microsoft365",
      name: "Microsoft 365",
      enabled: true,
      transport: {
        kind: "stdio",
        command: "npx",
        args: ["-y", "@softeria/ms-365-mcp-server", "--org-mode"],
        env: {},
        cwd: null,
      },
    },
  },
];

/** A fresh copy of a registry template, so editing the form never mutates it. */
export function templateOf(entry: McpRegistryEntry): McpServerConfig {
  return structuredClone(entry.template);
}

/** An empty stdio server, for the "add by hand" button. */
export function blankServer(): McpServerConfig {
  return {
    id: "",
    name: "",
    enabled: true,
    transport: { kind: "stdio", command: "", args: [], env: {}, cwd: null },
  };
}
