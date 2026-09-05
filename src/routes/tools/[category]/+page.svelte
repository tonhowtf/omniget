<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import ToolIcon from "$components/tools/ToolIcon.svelte";
  import ToolTile from "$components/tools/ToolTile.svelte";
  import ToolsSearchBar from "$components/tools/ToolsSearchBar.svelte";
  import { buildIndex, categoryById, search, toolHref, type PlatformFilter } from "$lib/tools/catalog";

  let category = $derived(categoryById(page.params.category ?? ""));

  let query = $state("");
  let filter = $state<PlatformFilter>("all");
  let index = $derived(buildIndex((k) => $t(k) as string));
  let group = $derived(category ? search(index, query, filter, category.id)[0] : undefined);
  let tools = $derived(group?.tools ?? []);
</script>

<section class="tools-category">
  <a class="tools-back" href="/tools">
    <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M10 3 5 8l5 5" />
    </svg>
    {$t("tools.hub.title")}
  </a>

  {#if !category}
    <div class="tools-empty">
      <img class="empty-state-art" src="/emoji/warning.png" alt="" width="96" height="96" />
      <h2>{$t("tools.hub.empty_title")}</h2>
    </div>
  {:else}
    <header class="tools-category-head">
      <ToolIcon icon={category.icon} from={category.from} to={category.to} via={category.via} size={72} />
      <div class="tools-category-meta">
        <h1>{$t(`tools.categories.${category.id}.name`)}</h1>
        <p>{$t(`tools.categories.${category.id}.desc`)}</p>
      </div>
    </header>

    <ToolsSearchBar bind:query bind:filter resultCount={query ? tools.length : undefined} />

    {#if tools.length === 0}
      <div class="tools-empty">
        <img class="empty-state-art" src="/emoji/magnifying_glass_tilted_left.png" alt="" width="96" height="96" />
        <h2>{$t("tools.hub.empty_title")}</h2>
        <p>{$t("tools.hub.empty_desc")}</p>
      </div>
    {:else}
      <div class="tools-grid">
        {#each tools as entry (entry.tool.id)}
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
    {/if}
  {/if}
</section>

<style>
  .tools-category {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .tools-back {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    align-self: flex-start;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--accent-hi);
    text-decoration: none;
  }

  .tools-back:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .tools-category-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .tools-category-meta h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .tools-category-meta p {
    margin: 0;
    font-size: var(--text-base);
    color: var(--text-muted);
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
