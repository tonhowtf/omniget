<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  /**
   * MCP tab, two halves.
   *
   * **Server** — OmniGet's own endpoint, the one other agents connect to.
   * **Client** — the external servers OmniGet connects to: add by stdio or
   * Streamable HTTP, test, see the discovered tools and grant them per agent.
   *
   * Budget: opening the tab reads the saved config and nothing else. No
   * connection, no process, no timer until a button is pressed.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    MCP_REGISTRY,
    blankServer,
    parseClientConfig,
    templateOf,
    type McpRegistryEntry,
    type McpServerConfig,
    type McpServerRow,
  } from "$lib/llm/mcp";
  import type { AgentDef, GrantMode } from "$lib/llm/types";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import {
    getErrorKey,
    getServer,
    getServers,
    getTestingId,
    getTestResult,
    isDemo,
    isLoading,
    loadServers,
    loadTools,
    removeServer,
    saveServer,
    setGrant,
    testServer,
  } from "$lib/stores/llm-mcp-store.svelte";
  import ClientList from "$components/llm/mcp/ClientList.svelte";
  import McpMarketplaceCard from "$components/llm/mcp/McpMarketplaceCard.svelte";
  import ServerForm from "$components/llm/mcp/ServerForm.svelte";
  import ServerHalf from "$components/llm/mcp/ServerHalf.svelte";
  import ToolGrantTable from "$components/llm/mcp/ToolGrantTable.svelte";

  type View = { kind: "list" } | { kind: "form"; config: McpServerConfig } | { kind: "grants"; id: string };

  let half = $state<"server" | "client">("client");
  let view = $state<View>({ kind: "list" });
  let importText = $state("");
  /** Servers from one pasted config still waiting to be reviewed. */
  let pending = $state<McpServerConfig[]>([]);
  let importError = $state(false);
  let grantBusy = $state(false);

  let rows = $derived(getServers());
  let agents = $derived(getAgents());
  let ids = $derived(rows.map((r) => r.config.id));
  let grantRow = $derived<McpServerRow | null>(
    view.kind === "grants" ? getServer(view.id) : null,
  );

  onMount(() => {
    void loadServers();
    void loadRoster();
  });

  function edit(row: McpServerRow) {
    view = { kind: "form", config: structuredClone($state.snapshot(row.config)) as McpServerConfig };
  }

  function install(entry: McpRegistryEntry) {
    half = "client";
    view = { kind: "form", config: templateOf(entry) };
  }

  async function save(config: McpServerConfig) {
    const ok = await saveServer(config);
    if (!ok) return;
    // A pasted config may carry several servers: review them one at a time
    // instead of writing the lot behind the user's back.
    const next = pending.shift();
    view = next ? { kind: "form", config: next } : { kind: "list" };
  }

  function cancelForm() {
    pending = [];
    view = { kind: "list" };
  }

  async function openGrants(row: McpServerRow) {
    view = { kind: "grants", id: row.config.id };
    // Cached: a second visit costs no connection.
    await loadTools(row.config.id);
  }

  async function grant(agent: AgentDef, tool: string, mode: GrantMode | null) {
    if (view.kind !== "grants") return;
    grantBusy = true;
    try {
      await setGrant(agent, view.id, tool, mode);
    } finally {
      grantBusy = false;
    }
  }

  function importPasted() {
    const parsed = parseClientConfig(importText);
    if (parsed.length === 0) {
      importError = true;
      return;
    }
    importError = false;
    importText = "";
    pending = parsed.slice(1);
    view = { kind: "form", config: parsed[0] };
  }
</script>

<svelte:head><title>{$t("llm.tab.mcp")}</title></svelte:head>

<div class="page page-wide mcp-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.mcp.title")}</h1>
      <p class="page-lede">{$t("llm.mcp.lede")}</p>
    </div>
    <div class="mac-segmented" role="tablist">
      <button
        type="button"
        class="mac-segmented-btn"
        class:active={half === "client"}
        role="tab"
        aria-selected={half === "client"}
        onclick={() => (half = "client")}
      >
        {$t("llm.mcp.half_client")}
      </button>
      <button
        type="button"
        class="mac-segmented-btn"
        class:active={half === "server"}
        role="tab"
        aria-selected={half === "server"}
        onclick={() => (half = "server")}
      >
        {$t("llm.mcp.half_server")}
      </button>
    </div>
  </header>
  <!-- Each half has its own intro: the client sentence is not repeated on "Your endpoint". -->
  <SurfaceGuide text={$t(half === "server" ? "llm.surface.mcp_server_hint" : "llm.surface.mcp_hint")} href="/help?article=mcp#guide" />

  {#if half === "server"}
    <ServerHalf />
  {:else if view.kind === "form"}
    {#if pending.length > 0}
      <p class="hint">{$t("llm.mcp.import_queue", { count: pending.length })}</p>
    {/if}
    {#key view.config.id}
      <ServerForm
        config={view.config}
        existingIds={ids}
        onsave={save}
        oncancel={cancelForm}
      />
    {/key}
  {:else if view.kind === "grants"}
    <section class="grants">
      <div class="grants-head">
        <h2 class="section-title">
          {$t("llm.mcp.grants_for", { server: grantRow?.config.name ?? view.id })}
        </h2>
        <button type="button" class="button" onclick={() => (view = { kind: "list" })}>
          {$t("llm.mcp.back")}
        </button>
      </div>
      <p class="hint">{$t("llm.mcp.grants_hint")}</p>
      {#if grantRow}
        <ToolGrantTable row={grantRow} {agents} busy={grantBusy} onset={grant} />
      {/if}
    </section>
  {:else}
    {#if isDemo()}
      <p class="notice notice-info" role="status">{$t("llm.mcp.demo")}</p>
    {/if}
    {#if getErrorKey()}
      <p class="notice notice-danger" role="alert">{$t(getErrorKey()!)}</p>
      <button type="button" class="button" disabled={isLoading()} onclick={() => void loadServers(true)}>{$t("llm.surface.retry")}</button>
    {/if}

    <section class="stack">
      <div class="section-header">
        <h2 class="section-header-title">{$t("llm.mcp.servers")}</h2>
        <button
          type="button"
          class="button primary"
          onclick={() => (view = { kind: "form", config: blankServer() })}
        >
          {$t("llm.mcp.add")}
        </button>
      </div>

      {#if isLoading()}
        <p class="hint">{$t("llm.mcp.loading")}</p>
      {:else}
        <ClientList
          {rows}
          testingId={getTestingId()}
          resultOf={getTestResult}
          onedit={edit}
          ontest={(id) => void testServer(id)}
          ongrants={openGrants}
          onremove={(id) => void removeServer(id)}
        />
      {/if}
    </section>

    <details class="import-settings"><summary>{$t("llm.surface.advanced")} · {$t("llm.mcp.import")}</summary>
    <section class="stack">
      <h2 class="section-header-title">{$t("llm.mcp.import")}</h2>
      <p class="hint">{$t("llm.mcp.import_hint")}</p>
      <textarea
        class="input mono"
        rows="3"
        bind:value={importText}
        aria-label={$t("llm.mcp.import")}
        placeholder={'{ "mcpServers": { "fetch": { "command": "uvx", "args": ["mcp-server-fetch"] } } }'}
      ></textarea>
      {#if importError}
        <p class="field-error">{$t("llm.mcp.import_failed")}</p>
      {/if}
      <div>
        <button type="button" class="button" disabled={!importText.trim()} onclick={importPasted}>
          {$t("llm.mcp.import_button")}
        </button>
      </div>
    </section>

    </details>

    <section class="stack">
      <h2 class="section-header-title">{$t("llm.mcp.registry")}</h2>
      <p class="hint">{$t("llm.mcp.registry_hint")}</p>
      <div class="cards">
        {#each MCP_REGISTRY as entry (entry.id)}
          <McpMarketplaceCard {entry} installed={ids.includes(entry.id)} oninstall={install} />
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .import-settings { margin:24px 0; } .import-settings summary { cursor:pointer; padding:14px 0; font-weight:600; color:var(--text); }
  /* Block layout: as a flex column the groups shrink to the viewport and clip
     their own rows instead of letting the page scroll. Same as the roster. */
  .mcp-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  /* The segmented control has to be able to drop below the title on a narrow
     window instead of pushing the page into a horizontal scroll. */
  .mcp-page .page-head {
    flex-wrap: wrap;
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-5);
  }

  .grants,
  .grants-head {
    display: flex;
    gap: var(--space-3);
  }

  .grants {
    flex-direction: column;
  }

  .grants-head {
    align-items: center;
    justify-content: space-between;
  }

  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 240px), 1fr));
    gap: var(--space-3);
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .notice {
    margin: 0 0 var(--space-4);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    resize: vertical;
  }
</style>
