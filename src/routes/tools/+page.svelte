<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import ToolTile from "$components/tools/ToolTile.svelte";
  import ToolsSearchBar from "$components/tools/ToolsSearchBar.svelte";
  import {
    CATEGORIES,
    buildIndex,
    search,
    toolHref,
    toolsOf,
    type PlatformFilter,
  } from "$lib/tools/catalog";

  // `/tools?q=instagram` abre o hub já filtrado, para a paleta e para links.
  let query = $state(page.url.searchParams.get("q") ?? "");
  let filter = $state<PlatformFilter>("all");

  let index = $derived(buildIndex((k) => $t(k) as string));
  let browsing = $derived(query.trim() === "" && filter === "all");
  let groups = $derived(search(index, query, filter));
  let resultCount = $derived(groups.reduce((n, g) => n + g.tools.length, 0));
  let ready = $derived(index.filter((e) => e.tool.status !== "soon"));
  let categories = $derived([...CATEGORIES].sort((a, b) => a.order - b.order));

  function countLabel(n: number): string {
    return n === 1 ? ($t("tools.hub.tool_count_one") as string) : ($t("tools.hub.tool_count", { count: n }) as string);
  }
</script>

<section class="tools-hub">
  <header class="tools-head">
    <h1>{$t("tools.hub.title")}</h1>
    <p class="tools-subtitle">{$t("tools.hub.subtitle")}</p>
  </header>

  <ToolsSearchBar bind:query bind:filter autofocus resultCount={browsing ? undefined : resultCount} />

  {#if browsing}
    <div class="tools-section">
      <div class="section-header">
        <span class="section-header-title">{$t("tools.hub.categories")}</span>
      </div>
      <div class="tools-grid">
        {#each categories as cat (cat.id)}
          <ToolTile
            href="/tools/{cat.id}"
            label={$t(`tools.categories.${cat.id}.name`)}
            sublabel={countLabel(toolsOf(cat.id).length)}
            icon={cat.icon}
            from={cat.from}
            to={cat.to}
            via={cat.via}
          />
        {/each}
      </div>
    </div>

    <div class="tools-section">
      <div class="section-header">
        <span class="section-header-title">{$t("tools.hub.ready_now")}</span>
      </div>
      <div class="tools-grid">
        {#each ready as entry (entry.tool.id)}
          <ToolTile
            href={toolHref(entry.tool)}
            label={entry.name}
            icon={entry.tool.icon}
            from={entry.tool.from}
            to={entry.tool.to}
            via={entry.tool.via}
            platforms={entry.tool.platforms}
            status={entry.tool.status}
          />
        {/each}
      </div>
    </div>
  {:else if groups.length === 0}
    <div class="tools-empty">
      <img class="empty-state-art" src="/emoji/magnifying_glass_tilted_left.png" alt="" width="96" height="96" />
      <h2>{$t("tools.hub.empty_title")}</h2>
      <p>{$t("tools.hub.empty_desc")}</p>
    </div>
  {:else}
    {#each groups as group (group.category.id)}
      <div class="tools-section">
        <div class="section-header">
          <span class="section-header-title">{$t(`tools.categories.${group.category.id}.name`)}</span>
          <a class="section-header-action" href="/tools/{group.category.id}">{$t("tools.hub.open_category")}</a>
        </div>
        <div class="tools-grid">
          {#each group.tools as entry (entry.tool.id)}
            <ToolTile
              href={toolHref(entry.tool)}
              label={entry.name}
              icon={entry.tool.icon}
              from={entry.tool.from}
              to={entry.tool.to}
              via={entry.tool.via}
              platforms={entry.tool.platforms}
              status={entry.tool.status}
            />
          {/each}
        </div>
      </div>
    {/each}
  {/if}
</section>

<style>
  .tools-hub {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .tools-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .tools-subtitle {
    margin: 0;
    font-size: var(--text-base);
    color: var(--text-muted);
  }

  .tools-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .tools-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(148px, 1fr));
    gap: var(--space-2) var(--space-3);
  }

  .tools-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-4);
    text-align: center;
  }

  .tools-empty h2 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
  }

  .tools-empty p {
    margin: 0;
    font-size: var(--text-base);
    color: var(--text-muted);
  }
</style>
