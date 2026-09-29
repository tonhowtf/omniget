/**
 * State of `/llm/mcp`: the external MCP servers, what each one exposes and the
 * per-agent grants.
 *
 * Three rules, same shape as `llm-store.svelte.ts`:
 *
 * 1. **Nothing runs at rest.** No listener, no timer, no polling. Reading the
 *    saved list is a file read; a *connection* only happens when the user
 *    presses "test" or opens a server's tool table.
 * 2. **`tools/list` is cached.** Once a server answered, its tools stay in the
 *    store until the user tests again, so opening the grant table twice costs
 *    one connection, not two.
 * 3. **A failure is shown, never dressed as data.** A list the backend cannot
 *    give stays empty with the error on screen (U4: the example servers of
 *    the old demo mode are gone; the screenshot harness has its own fixture).
 */
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

// ── State ───────────────────────────────────────────────────────────────

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

/** False once the saved list could not be read or a change was refused. */
export function isAvailable(): boolean {
  return available;
}

export function getErrorKey(): string | null {
  return errorKey;
}

/** Id of the server being probed right now, or `null`. One at a time. */
export function getTestingId(): string | null {
  return testing;
}

export function getTestResult(id: string): McpTestResult | null {
  return results[id] ?? null;
}

export function getTools(id: string): McpToolDef[] {
  return getServer(id)?.tools ?? [];
}

/** Normalises both shapes the backend may answer: a list or `{ servers }`. */
function toRows(answer: unknown): McpServerRow[] | null {
  if (Array.isArray(answer)) return answer as McpServerRow[];
  if (answer && typeof answer === "object" && Array.isArray((answer as { servers?: unknown }).servers)) {
    return (answer as { servers: McpServerRow[] }).servers;
  }
  return null;
}

/**
 * Reads the saved servers. No connection is opened: this is the config file,
 * not the network. Loads once per session unless `force`. Never throws.
 */
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
      // No list at all: a backend that does not answer.
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

/** Creates or replaces one server. Returns false when the backend refused. */
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

/**
 * The one command that opens a connection: `initialize` + `tools/list`, then
 * the client is dropped. Only ever called from the "test" button.
 */
export async function testServer(id: string): Promise<McpTestResult | null> {
  if (testing) return null;
  errorKey = null;
  testing = id;
  const startedAt = Date.now();
  try {
    const result = await invoke<McpTestResult | null>("llm_mcp_test", { id });
    if (!result) {
      // No answer: keep whatever the row already showed, and say so.
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

/**
 * Tools of one server. Cached: with tools already in the row nothing is sent
 * unless `force`. Called when the grant table opens, never on page load.
 */
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

/**
 * Writes one grant into `AgentDef.tools`; `null` clears it.
 *
 * `llm_mcp_grant` is the narrow command — it touches one grant and leaves the
 * rest of the agent alone; the local copy follows the same pure `applyGrant`.
 */
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

/** Test seam: drops every bit of state. Nothing to cancel — nothing runs. */
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
