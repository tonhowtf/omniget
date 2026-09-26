<script lang="ts">
  /**
   * Server half of the MCP tab: OmniGet's own endpoint — on/off, address,
   * bearer token, one snippet per client and the tools it exposes.
   *
   * Same three commands `components/tools/ai/McpTool.svelte` uses (that tool
   * stays where it is); everything pure moved to `$lib/llm/mcp` so both halves
   * mask the token the same way.
   */
  import ServerConnections from "./ServerConnections.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { maskSnippet, maskToken, toolSignature } from "$lib/llm/mcp";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type ToolDef = {
    name: string;
    description: string;
    inputSchema?: unknown;
  };
  type Status = {
    enabled: boolean;
    bridge_enabled: boolean;
    port: number;
    url: string;
    token: string;
    tools: ToolDef[];
    snippets: [string, string][];
  };

  let status = $state<Status | null>(null);
  let busy = $state(false);
  let client = $state(0);
  let reveal = $state(false);
  let selftest = $state("");

  async function refresh() {
    try {
      status = await invoke<Status | null>("tool_mcp_status");
    } catch {
      status = null;
    }
  }

  onMount(refresh);

  async function toggle() {
    if (!status || busy) return;
    busy = true;
    try {
      await invoke("tool_mcp_set_enabled", { enabled: !status.enabled });
      await refresh();
    } catch (error) {
      showToast("error", String(error));
    } finally {
      busy = false;
    }
  }

  async function runSelftest() {
    busy = true;
    selftest = "";
    try {
      selftest = await invoke<string>("tool_mcp_selftest");
    } catch (error) {
      selftest = String(error);
    } finally {
      busy = false;
    }
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    showToast("success", $t("tools.common.copied") as string);
  }

  let serverOn = $derived(status?.enabled === true);
  let snippet = $derived(status?.snippets?.[client]?.[1] ?? "");
  // The Claude Code command never holds the token (argv, shell history).
  let tokenOutsideSnippet = $derived(status?.snippets?.[client]?.[0] === "Claude Code");
</script>

<section class="half">
  <div class="group">
    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t("llm.mcp.server_title")}</div>
        <div class="group-row-sub">
          {status?.enabled ? $t("llm.mcp.server_on") : $t("llm.mcp.server_off")}
          {#if status && !status.bridge_enabled}
            · <span class="danger">{$t("llm.mcp.bridge_off")}</span>
          {/if}
        </div>
      </div>
      <div class="group-row-trailing">
        <button
          class="toggle server-switch"
          class:on={serverOn}
          type="button"
          role="switch"
          aria-checked={serverOn}
          aria-label={$t("llm.mcp.server_title")}
          disabled={!status || busy}
          onclick={toggle}
        >
          <span class="toggle-knob"></span>
        </button>
      </div>
    </div>

    <div class="group-row">
      <div class="group-row-content">
        <div class="group-row-title">{$t("llm.mcp.endpoint")}</div>
        <div class="group-row-sub mono">{status?.url || "—"}</div>
      </div>
      <div class="group-row-trailing row">
        {#if status?.url}
          <button type="button" class="button" onclick={() => copy(status!.url)}>
            {$t("llm.mcp.copy")}
          </button>
        {/if}
        <button
          type="button"
          class="button"
          disabled={busy || !status?.enabled}
          onclick={runSelftest}
        >
          {$t("llm.mcp.selftest")}
        </button>
      </div>
    </div>

    {#if selftest}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub mono">{selftest}</div></div>
      </div>
    {/if}
  </div>

  {#if status}<ServerConnections url={status.url} />{/if}

  {#if status && status.snippets.length > 0}
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("llm.mcp.connect")}</div>
          <div class="group-row-sub">{$t("llm.mcp.connect_hint")}</div>
        </div>
        <div class="group-row-trailing row wrap">
          {#each status.snippets as [name], index (name)}
            <button
              type="button"
              class="button"
              class:primary={client === index}
              onclick={() => (client = index)}
            >
              {name}
            </button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <pre class="code">{maskSnippet(snippet, status.token, reveal)}</pre>
          {#if tokenOutsideSnippet}
            <div class="group-row-sub">{$t("mcp_connections.claude_code_hint")}</div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button type="button" class="button" onclick={() => copy(snippet)}>
            {$t("llm.mcp.copy")}
          </button>
          {#if tokenOutsideSnippet}
            <button type="button" class="button" onclick={() => copy(status!.token)}>
              {$t("mcp_connections.copy_token")}
            </button>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  {#if status && status.tools.length > 0}
    <h2 class="section-title">{$t("llm.mcp.exposed", { count: status.tools.length })}</h2>
    <div class="group">
      {#each status.tools as tool (tool.name)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title mono">{tool.name}</div>
            <div class="group-row-sub">{tool.description}</div>
            {#if toolSignature(tool.inputSchema)}
              <div class="group-row-sub mono">{toolSignature(tool.inputSchema)}</div>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {:else if !status}
    <p class="hint">{$t("llm.mcp.server_unavailable")}</p>
  {/if}
</section>

<style>
  .half {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }

  .row {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .wrap {
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .danger {
    color: var(--danger);
  }

  /* The app's switch (primitives.css .toggle, 38×22). Its on-state is also
     stated here, scoped, so the knob and track always follow aria-checked. */
  .server-switch.on,
  .server-switch[aria-checked="true"] {
    background: var(--accent);
    box-shadow: none;
  }

  .server-switch.on :global(.toggle-knob),
  .server-switch[aria-checked="true"] :global(.toggle-knob) {
    transform: translateX(16px);
  }

  .code {
    margin: 0;
    white-space: pre-wrap;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    max-height: 240px;
    overflow: auto;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  /* At phone width a row of buttons next to a URL squeezes the URL to one
     character per line. Stack the whole row instead. */
  @media (max-width: 720px) {
    .half :global(.group-row) {
      flex-direction: column;
      align-items: stretch;
    }

    .wrap {
      justify-content: flex-start;
    }
  }
</style>
