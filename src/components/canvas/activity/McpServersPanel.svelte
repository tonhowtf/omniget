<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { isTerminalKind } from "$lib/canvas/model/types";

  interface Server {
    id: string;
    name: string;
    transport: "stdio" | "http";
    target: string;
    enabled: boolean;
    connected: boolean;
    toolCount: number | null;
  }

  interface TestResult {
    ok: boolean;
    tool_count: number;
    elapsed_ms: number;
    error: { code: string; message: string } | null;
  }

  let { nodeId = null }: { nodeId?: string | null } = $props();

  const engine = useEngine();
  let servers = $state<Server[]>([]);
  let loaded = $state(false);
  let failure = $state<string | null>(null);
  let busy = $state<Record<string, boolean>>({});
  let tests = $state<Record<string, TestResult>>({});
  let touched = $state(false);

  const node = $derived(nodeId ? engine.store.node(nodeId) : undefined);
  const terminal = $derived(!!node && isTerminalKind(node.kind));
  const agent = $derived(String(node?.data.agent ?? "shell"));
  const receives = $derived(agent === "claude" || agent === "codex");
  const attached = $derived(
    Array.isArray(node?.data.mcpServers) ? (node!.data.mcpServers as unknown[]).filter((x): x is string => typeof x === "string") : [],
  );

  async function refresh() {
    try {
      servers = await invoke<Server[]>("canvas_mcp_servers");
      failure = null;
    } catch (e) {
      failure = String(e);
    } finally {
      loaded = true;
    }
  }

  async function setEnabled(s: Server, on: boolean) {
    busy = { ...busy, [s.id]: true };
    try {
      servers = await invoke<Server[]>("canvas_mcp_server_set", { id: s.id, enabled: on });
    } catch (e) {
      failure = String(e);
    } finally {
      busy = { ...busy, [s.id]: false };
    }
  }

  async function test(s: Server) {
    busy = { ...busy, [s.id]: true };
    try {
      tests = { ...tests, [s.id]: await invoke<TestResult>("canvas_mcp_server_test", { id: s.id }) };
      await refresh();
    } catch (e) {
      failure = String(e);
    } finally {
      busy = { ...busy, [s.id]: false };
    }
  }

  function toggleAttach(id: string, on: boolean) {
    if (!node) return;
    const next = on ? [...attached.filter((x) => x !== id), id] : attached.filter((x) => x !== id);
    engine.commit([{ op: "update_node", id: node.id, patch: { data: { mcpServers: next.length ? next : null } } as never }]);
    touched = true;
  }

  function status(s: Server): string {
    if (!s.enabled) return $t("canvas.mcpServers.off", { default: "Off" });
    if (s.connected) return $t("canvas.mcpServers.connected", { default: "Connected" });
    return $t("canvas.mcpServers.idle", { default: "Not connected" });
  }

  onMount(() => {
    void refresh();
  });
</script>

<div class="mcp">
  {#if node && terminal}
    <div class="node">
      <div class="nodeHead">
        {$t("canvas.mcpServers.forNode", { default: "Access for {{name}}", name: node.title || agent })}
      </div>
      {#if !receives}
        <p class="hint">{$t("canvas.mcpServers.onlyAgents", { default: "Only Claude and Codex terminals receive MCP servers." })}</p>
      {/if}
      {#if touched}
        <p class="hint">
          {$t("canvas.mcpServers.restartHint", { default: "Applies at the next start of this terminal." })}
          <button class="link" onclick={() => { engine.requestRestart(node!.id); touched = false; }}>
            {$t("canvas.mcpServers.restartNow", { default: "Restart now" })}
          </button>
        </p>
      {/if}
    </div>
  {/if}

  {#if failure}
    <p class="error" role="alert">{failure}</p>
  {/if}

  {#if loaded && servers.length === 0 && !failure}
    <p class="hint">{$t("canvas.mcpServers.empty", { default: "No MCP servers yet. Add them in AI & agents → Extra tools; they show up here to turn on, test and give to a terminal." })}</p>
  {/if}

  <ul>
    {#each servers as s (s.id)}
      <li class:off={!s.enabled}>
        <span class="dot" class:live={s.connected} class:disabled={!s.enabled} title={status(s)} aria-hidden="true"></span>
        <div class="info">
          <div class="name">{s.name || s.id}</div>
          <div class="meta">
            {s.transport} · {s.target || s.id}
            {#if s.toolCount != null} · {$t("canvas.mcpServers.tools", { default: "{{count}} tools", count: s.toolCount })}{/if}
            · {status(s)}
          </div>
          {#if tests[s.id]}
            <div class="meta" class:bad={!tests[s.id].ok}>
              {tests[s.id].ok
                ? $t("canvas.mcpServers.testOk", { default: "OK: {{count}} tools in {{ms}} ms", count: tests[s.id].tool_count, ms: tests[s.id].elapsed_ms })
                : tests[s.id].error?.message ?? $t("canvas.mcpServers.testFailed", { default: "Test failed" })}
            </div>
          {/if}
        </div>
        <div class="actions">
          {#if node && terminal}
            <label class="pick" title={$t("canvas.mcpServers.give", { default: "Give to this terminal" })}>
              <input
                type="checkbox"
                checked={attached.includes(s.id)}
                disabled={!s.enabled}
                onchange={(e) => toggleAttach(s.id, (e.currentTarget as HTMLInputElement).checked)}
              />
              <span>{$t("canvas.mcpServers.use", { default: "Use" })}</span>
            </label>
          {/if}
          <button class="small" disabled={busy[s.id]} onclick={() => void test(s)}>{$t("canvas.mcpServers.test", { default: "Test" })}</button>
          <label class="pick">
            <input type="checkbox" checked={s.enabled} disabled={busy[s.id]} onchange={(e) => void setEnabled(s, (e.currentTarget as HTMLInputElement).checked)} />
            <span>{$t("canvas.mcpServers.on", { default: "On" })}</span>
          </label>
        </div>
      </li>
    {/each}
  </ul>
</div>

<style>
  .mcp {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    overflow-y: auto;
  }
  .node {
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .nodeHead {
    font-weight: 600;
  }
  .hint {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .error {
    margin: 0;
    font-size: 12px;
    color: var(--danger, #e5484d);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  li {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px;
    border-radius: 9px;
  }
  li:hover {
    background: color-mix(in srgb, var(--text) 5%, transparent);
  }
  li.off .name {
    color: var(--text-muted);
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--text) 35%, transparent);
  }
  .dot.live {
    background: #21c08b;
  }
  .dot.disabled {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--text) 35%, transparent);
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font-size: 11px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .meta.bad {
    color: var(--danger, #e5484d);
  }
  .actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pick {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 12px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .small,
  .link {
    font: inherit;
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 2px 8px;
    color: var(--text);
    background: transparent;
    cursor: pointer;
  }
  .link {
    border: 0;
    padding: 0 2px;
    color: var(--accent);
  }
  .small:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
