import { invoke } from "@tauri-apps/api/core";
import {
  mcpErrorKey,
  type McpServerConfig,
  type McpServerRow,
  type McpTestResult,
} from "$lib/mcp-servers";

let rows = $state<McpServerRow[]>([]);
let loading = $state(false);
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

export function getErrorKey(): string | null {
  return errorKey;
}

export function getTestingId(): string | null {
  return testing;
}

export function getTestResult(id: string): McpTestResult | null {
  return results[id] ?? null;
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
        return;
      }
      errorKey = mcpErrorKey("ERR_NO_ANSWER");
    })
    .catch((err) => {
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
    return true;
  } catch (err) {
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
    return result;
  } finally {
    testing = null;
  }
}
