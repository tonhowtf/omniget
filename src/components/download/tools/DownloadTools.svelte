<script lang="ts">
  import { untrack, type Component } from "svelte";
  import { t } from "$lib/i18n";
  import { DOWNLOAD_TOOLS, findDownloadTool, type DownloadTool } from "$lib/download-tools/registry";

  let { initialTool = null }: { initialTool?: string | null } = $props();

  let openId = $state<string | null>(untrack(() => findDownloadTool(initialTool)?.id ?? null));
  let Panel = $state<Component | null>(null);
  let loadError = $state(false);

  $effect(() => {
    const tool = findDownloadTool(openId);
    Panel = null;
    loadError = false;
    if (!tool) return;
    let cancelled = false;
    tool
      .load()
      .then((m) => {
        if (!cancelled) Panel = m.default;
      })
      .catch(() => {
        if (!cancelled) loadError = true;
      });
    return () => {
      cancelled = true;
    };
  });

  function open(tool: DownloadTool) {
    openId = tool.id;
  }

  const current = $derived(findDownloadTool(openId));
</script>

<div class="dl-tools" data-agent-readiness={openId && !Panel && !loadError ? "loading" : "ready"}>
  {#if current}
    <div class="tool-head">
      <button class="back" onclick={() => (openId = null)}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <polyline points="15 18 9 12 15 6" />
        </svg>
        {$t("downloads.tools.back")}
      </button>
      <h2>{$t(current.titleKey)}</h2>
    </div>
    <div class="tool-body">
      {#if loadError}
        <p class="muted">{$t("common.error")}</p>
      {:else if Panel}
        <Panel />
      {/if}
    </div>
  {:else}
    <p class="lead">{$t("downloads.tools.subtitle")}</p>
    <p class="more-tools">
      {$t("downloads.tools.more_hint")}
      <a href="/superpowers">{$t("downloads.tools.more_link")}</a>
    </p>
    {#if DOWNLOAD_TOOLS.length === 0}
      <p class="muted">{$t("downloads.tools.empty")}</p>
    {:else}
      <ul class="grid">
        {#each DOWNLOAD_TOOLS as tool (tool.id)}
          <li>
            <button class="card" data-tool={tool.id} onclick={() => open(tool)}>
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d={tool.icon} />
              </svg>
              <span class="title">{$t(tool.titleKey)}</span>
              <span class="desc">{$t(tool.descriptionKey)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .dl-tools {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) 0;
  }
  .lead,
  .more-tools,
  .muted {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-2);
  }
  .card {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
    padding: var(--space-3);
    text-align: left;
    border: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }
  .card:hover {
    background: var(--fill-1);
  }
  .card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .card svg {
    color: var(--accent);
  }
  .title {
    font-weight: 600;
    font-size: var(--text-base);
  }
  .desc {
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.4;
  }
  .tool-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .tool-head h2 {
    margin: 0;
    font-size: var(--text-base);
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-full);
    background: none;
    color: var(--accent);
    cursor: pointer;
    font-size: var(--text-sm);
  }
  .back:hover {
    background: var(--accent-soft);
  }
</style>
