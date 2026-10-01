import type { AgentDef, GrantMode, ToolGrant } from "./types";

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
  timeout_ms?: number | null;
}

export interface McpToolDef {
  name: string;
  description: string;
  inputSchema?: unknown;
  outputSchema?: unknown;
  title?: string | null;
}

export interface McpTestResult {
  ok: boolean;
  server_name?: string | null;
  protocol_version?: string | null;
  protocol_known?: boolean;
  tool_count: number;
  elapsed_ms: number;
  error?: { code: string; message: string } | null;
}

export interface McpServerRow {
  config: McpServerConfig;
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

export function mcpErrorKey(err: unknown): string {
  const text = String(err ?? "");
  for (const [code, key] of Object.entries(MCP_ERROR_KEYS)) {
    if (text.includes(code)) return key;
  }
  return "llm.mcp.err.generic";
}

const ID_RE = /^[a-z0-9_-]{1,64}$/;

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

export function formatArgs(args: string[]): string {
  return args.map((a) => (/\s/.test(a) ? `"${a}"` : a)).join(" ");
}

export function parsePairs(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    const colon = line.indexOf(":");
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

export function isSecretRef(value: string): boolean {
  return value.startsWith("secret:") && value.length > "secret:".length;
}

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

export function maskToken(token: string): string {
  return token.length > 8 ? `${token.slice(0, 4)}…${token.slice(-4)}` : "••••";
}

export function maskSnippet(snippet: string, token: string, reveal: boolean): string {
  if (reveal || !token) return snippet;
  return snippet.replaceAll(token, maskToken(token));
}

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

export interface McpRegistryEntry {
  id: string;
  name: string;
  descKey: string;
  homepage: string;
  template: McpServerConfig;
  needs: string[];
}

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

export function templateOf(entry: McpRegistryEntry): McpServerConfig {
  return structuredClone(entry.template);
}

export function blankServer(): McpServerConfig {
  return {
    id: "",
    name: "",
    enabled: true,
    transport: { kind: "stdio", command: "", args: [], env: {}, cwd: null },
  };
}
