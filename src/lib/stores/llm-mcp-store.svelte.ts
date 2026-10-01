import { invoke } from "@tauri-apps/api/core";
import type { AgentDef, GrantMode } from "$lib/llm/types";
import {
  applyGrant,
  mcpErrorKey,
  type McpServerConfig,
  type McpServerRow,
  type McpTestResult,
  type McpToolDef,
} from "$lib/llm/mcp";

let rows = $state<McpServerRow[]>([]);
let loading = $state(false);
let available = $state(true);
let errorKey = $state<string | null>(null);
let testing = $state<string | null>(null);
let results = $state<Record<string, McpTestResult>>({});

let loadedOnce = false;
let inFlight: Promise<void> | null = null;

export function getServers(): McpServerRow[] {
  return rows;
}

export function getServer(id: string | null | undefined): McpServerRow | null {
  if (!id) return null;
  return rows.find((r) => r.config.id === id) ?? null;
}

export function isLoading(): boolean {
  return loading;
}

export function isAvailable(): boolean {
  return available;
}

export function getErrorKey(): string | null {
  return errorKey;
}

export function getTestingId(): string | null {
  return testing;
}

export function getTestResult(id: string): McpTestResult | null {
  return results[id] ?? null;
}

export function getTools(id: string): McpToolDef[] {
  return getServer(id)?.tools ?? [];
}

function toRows(answer: unknown): McpServerRow[] | null {
  if (Array.isArray(answer)) return answer as McpServerRow[];
  if (answer && typeof answer === "object" && Array.isArray((answer as { servers?: unknown }).servers)) {
    return (answer as { servers: McpServerRow[] }).servers;
  }
  return null;
}

export function loadServers(force = false): Promise<void> {
  if (inFlight) return inFlight;
  if (loadedOnce && !force) return Promise.resolve();
  loading = true;
  errorKey = null;
  inFlight = invoke<unknown>("llm_mcp_list")
    .then((answer) => {
      const list = toRows(answer);
      if (list) {
        rows = list;
        available = true;
        return;
      }
      available = false;
      errorKey = mcpErrorKey("ERR_NO_ANSWER");
    })
    .catch((err) => {
      available = false;
      errorKey = mcpErrorKey(err);
    })
    .finally(() => {
      loading = false;
      loadedOnce = true;
      inFlight = null;
    });
  return inFlight;
}

export async function saveServer(config: McpServerConfig): Promise<boolean> {
  errorKey = null;
  const existing = rows.some((r) => r.config.id === config.id);
  const nextRows = existing
    ? rows.map((r) => (r.config.id === config.id ? { ...r, config } : r))
    : [...rows, { config, tools: [], connected: false, last_error: null }];
  try {
    const answer = await invoke<unknown>("llm_mcp_upsert", { config });
    const list = toRows(answer);
    rows = list ?? nextRows;
    available = true;
    return true;
  } catch (err) {
    available = false;
    errorKey = mcpErrorKey(err);
    return false;
  }
}

export async function removeServer(id: string): Promise<boolean> {
  errorKey = null;
  const nextRows = rows.filter((r) => r.config.id !== id);
  try {
    const answer = await invoke<unknown>("llm_mcp_remove", { id });
    const list = toRows(answer);
    rows = list ?? nextRows;
    delete results[id];
    return true;
  } catch (err) {
    available = false;
    errorKey = mcpErrorKey(err);
    return false;
  }
}

export async function testServer(id: string): Promise<McpTestResult | null> {
  if (testing) return null;
  errorKey = null;
  testing = id;
  const startedAt = Date.now();
  try {
    const result = await invoke<McpTestResult | null>("llm_mcp_test", { id });
    if (!result) {
      errorKey = mcpErrorKey("ERR_NO_ANSWER");
      return null;
    }
    results = { ...results, [id]: result };
    const row = rows.find((r) => r.config.id === id);
    if (row) {
      row.connected = result.ok;
      row.last_error = result.error ?? null;
    }
    if (result.ok) await loadTools(id, true);
    return result;
  } catch (err) {
    const result: McpTestResult = {
      ok: false,
      tool_count: 0,
      elapsed_ms: Date.now() - startedAt,
      error: { code: String(err ?? ""), message: String(err ?? "") },
    };
    results = { ...results, [id]: result };
    const row = rows.find((r) => r.config.id === id);
    if (row) {
      row.connected = false;
      row.last_error = result.error ?? null;
    }
    errorKey = mcpErrorKey(err);
    available = false;
    return result;
  } finally {
    testing = null;
  }
}

export async function loadTools(id: string, force = false): Promise<McpToolDef[]> {
  const row = rows.find((r) => r.config.id === id);
  if (!row) return [];
  if (row.tools.length > 0 && !force) return row.tools;
  try {
    const answer = await invoke<McpToolDef[] | null>("llm_mcp_tools", { id });
    if (Array.isArray(answer)) {
      row.tools = answer;
      row.connected = true;
      row.last_error = null;
    }
    return row.tools;
  } catch (err) {
    row.last_error = { code: String(err ?? ""), message: String(err ?? "") };
    errorKey = mcpErrorKey(err);
    return row.tools;
  }
}

export async function setGrant(
  agent: AgentDef,
  server: string,
  tool: string,
  mode: GrantMode | null,
): Promise<boolean> {
  errorKey = null;
  const next = applyGrant(agent.tools, server, tool, mode);
  try {
    await invoke("llm_mcp_grant", {
      agentId: agent.id,
      grant: { source: "mcp", server, tool, mode },
    });
    agent.tools = next;
    return true;
  } catch (err) {
    errorKey = mcpErrorKey(err);
    return false;
  }
}

export function resetMcpStore(): void {
  rows = [];
  loading = false;
  available = true;
  errorKey = null;
  testing = null;
  results = {};
  loadedOnce = false;
  inFlight = null;
}
