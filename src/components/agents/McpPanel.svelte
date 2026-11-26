<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { MCP_REGISTRY, blankServer, mcpErrorKey, parseClientConfig, templateOf, type McpRegistryEntry, type McpServerConfig, type McpServerRow } from "$lib/mcp-servers";
  import { getErrorKey, getServers, getTestResult, getTestingId, isLoading, loadServers, removeServer, saveServer, testServer } from "$lib/stores/mcp-servers-store.svelte";
  import SettingGroup from "#components/settings/rebrand/SettingGroup.svelte";
  import AgentLogo from "./AgentLogo.svelte";
  import ServerForm from "./ServerForm.svelte";

  let editing = $state<McpServerConfig | null>(null);
  let pending = $state<McpServerConfig[]>([]);
  let importText = $state("");
  let importFailed = $state(false);
  let confirming = $state<string | null>(null);

  let rows = $derived(getServers());
  let ids = $derived(rows.map((row) => row.config.id));
  let gallery = $derived(MCP_REGISTRY.filter((entry) => !ids.includes(entry.id)));

  onMount(() => void loadServers());

  function edit(row: McpServerRow) {
    editing = structuredClone($state.snapshot(row.config)) as McpServerConfig;
  }

  function install(entry: McpRegistryEntry) {
    editing = templateOf(entry);
  }

  async function save(config: McpServerConfig) {
    if (!(await saveServer(config))) return;
    editing = pending.shift() ?? null;
  }

  function cancel() {
    pending = [];
    editing = null;
  }

  function importPasted() {
    const parsed = parseClientConfig(importText);
    importFailed = parsed.length === 0;
    if (importFailed) return;
    importText = "";
    pending = parsed.slice(1);
    editing = parsed[0];
  }

  function statusOf(row: McpServerRow): string {
    const result = getTestResult(row.config.id);
    if (result && !result.ok) return $t(mcpErrorKey(result.error?.code ?? "")) as string;
    if (row.last_error) return $t(mcpErrorKey(row.last_error.code)) as string;
    if (row.connected || result?.ok) return $t("settings.agents.mcp_ready", { count: result?.tool_count ?? row.tools.length }) as string;
    return !row.config.enabled ? ($t("agents.mcp.disabled") as string) : ($t("settings.agents.mcp_idle") as string);
  }
</script>

<SettingGroup id="mcp" icon="blocks" title={$t("settings.agents.mcp_title") as string} description={$t("settings.agents.mcp_desc") as string}>
  {#if editing}
    {#if pending.length > 0}<p class="hint">{$t("agents.mcp.import_queue", { count: pending.length })}</p>{/if}
    {#key editing.id}
      <ServerForm config={editing} existingIds={ids} onsave={save} oncancel={cancel} />
    {/key}
  {:else}
    {#if getErrorKey()}
      <p class="hint error" role="alert">{$t(getErrorKey()!)}</p>
    {/if}
    {#if isLoading()}<p class="hint" role="status">{$t("agents.mcp.loading")}</p>{/if}
    {#if rows.length > 0}
      <div class="grid" data-setting-key="settings.agents.mcp_title">
        {#each rows as row (row.config.id)}
          <article class="server">
            <AgentLogo id={row.config.id} name={row.config.name} size={22} />
            <div class="text">
              <span class="name"><bdi>{row.config.name}</bdi></span>
              <small><bdi>{statusOf(row)}</bdi></small>
            </div>
            {#if confirming === row.config.id}
              <button type="button" class="btn btn-secondary btn-sm" onclick={() => (confirming = null)}>{$t("settings.agents.keep")}</button>
              <button type="button" class="btn btn-secondary btn-sm danger" onclick={() => { confirming = null; void removeServer(row.config.id); }}>{$t("settings.agents.remove")}</button>
            {:else}
              <button type="button" class="btn btn-secondary btn-sm" disabled={getTestingId() === row.config.id} onclick={() => void testServer(row.config.id)}>{$t("settings.agents.mcp_test")}</button>
              <button type="button" class="btn btn-secondary btn-sm" onclick={() => edit(row)}>{$t("settings.agents.mcp_edit")}</button>
              <button type="button" class="link" onclick={() => (confirming = row.config.id)}>{$t("settings.agents.remove")}</button>
            {/if}
          </article>
        {/each}
      </div>
    {/if}
    <h3>{$t("settings.agents.mcp_gallery")}</h3>
    <div class="gallery">
      {#each gallery as entry (entry.id)}
        <button type="button" class="tile" onclick={() => install(entry)}>
          <AgentLogo id={entry.id} name={entry.name} size={22} />
          <span class="text">
            <span class="name">{entry.name}</span>
            <small>{$t(entry.descKey)}</small>
          </span>
          <span class="plus" aria-hidden="true">+</span>
        </button>
      {/each}
      <button type="button" class="tile dashed" onclick={() => (editing = blankServer())}>
        <span class="text">
          <span class="name">{$t("settings.agents.mcp_custom")}</span>
          <small>{$t("settings.agents.mcp_custom_hint")}</small>
        </span>
        <span class="plus" aria-hidden="true">+</span>
      </button>
    </div>
    <details class="more">
      <summary>{$t("agents.mcp.import")}</summary>
      <p class="hint">{$t("agents.mcp.import_hint")}</p>
      <textarea class="input-text code" rows="3" bind:value={importText} aria-label={$t("agents.mcp.import") as string} placeholder={'{ "mcpServers": { "fetch": { "command": "uvx", "args": ["mcp-server-fetch"] } } }'}></textarea>
      {#if importFailed}<p class="hint error" role="alert">{$t("agents.mcp.import_failed")}</p>{/if}
      <button type="button" class="btn btn-secondary btn-sm" disabled={!importText.trim()} onclick={importPasted}>{$t("agents.mcp.import_button")}</button>
    </details>
  {/if}
</SettingGroup>

<style>
  .grid { display: flex; flex-direction: column; gap: var(--space-2); }
  .gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 240px), 1fr)); gap: var(--space-2); }
  .server, .tile {
    display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
    background: var(--primary);
    color: var(--text);
    font: inherit;
    text-align: start;
  }
  .server { flex-wrap: wrap; }
  .tile { cursor: pointer; }
  .tile:hover { background: var(--surface-hi); border-color: var(--input-border); }
  .tile.dashed { border-style: dashed; background: transparent; border-color: var(--input-border); }
  .text { flex: 1 1 8rem; display: flex; flex-direction: column; min-inline-size: 0; }
  .name { font-size: 13px; line-height: 18px; font-weight: 600; overflow-wrap: anywhere; }
  small { font-size: 12px; line-height: 17px; color: var(--text-muted); overflow-wrap: anywhere; }
  .plus { flex: none; font-size: 18px; line-height: 1; color: var(--text-muted); }
  h3 { margin-block: var(--space-4) var(--space-2); font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); }
  .hint { margin: 0 0 var(--space-2); font-size: 12px; color: var(--text-muted); }
  .error { color: var(--error); }
  .danger { color: var(--error); }
  .link { padding: 0; border: 0; background: none; color: var(--text-muted); font: inherit; font-size: 12px; cursor: pointer; }
  .link:hover { color: var(--error); }
  .more { margin-block-start: var(--space-3); }
  .more > summary { padding-block: var(--space-1); font-size: 13px; color: var(--accent-text, var(--accent)); cursor: pointer; }
  .code { display: block; inline-size: 100%; margin-block-end: var(--space-2); font-family: var(--font-mono); font-size: 11px; resize: vertical; }
</style>
