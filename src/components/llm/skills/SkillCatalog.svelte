<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    catalogPress,
    isInstalled,
    isUnlicensed,
    scanOf,
    type SkillCatalogEntry,
    type SkillManifest,
  } from "$lib/stores/llm-skills-store.svelte";
  import SkillScanBadge from "./SkillScanBadge.svelte";

  let {
    entries = [],
    installed = [],
    busy = null,
    oninstall,
  }: {
    entries?: SkillCatalogEntry[];
    installed?: SkillManifest[];
    busy?: string | null;
    oninstall?: (entry: SkillCatalogEntry) => void;
  } = $props();

  let filter = $state("");
  let confirming = $state<string | null>(null);

  function press(entry: SkillCatalogEntry) {
    const step = catalogPress(entry, confirming);
    confirming = step.confirming;
    if (step.install) oninstall?.(entry);
  }

  let shown = $derived(
    filter.trim() === ""
      ? entries
      : entries.filter((e) => {
          const needle = filter.trim().toLowerCase();
          return (
            e.name.toLowerCase().includes(needle) ||
            e.description.toLowerCase().includes(needle)
          );
        }),
  );
</script>

<section class="catalog">
  <header class="head">
    <div>
      <h2 class="title">{$t("llm.skills.catalog_title")}</h2>
      <p class="lede">{$t("llm.skills.catalog_lede")}</p>
    </div>
    <input
      class="input filter"
      type="search"
      bind:value={filter}
      placeholder={$t("llm.skills.catalog_filter")}
      aria-label={$t("llm.skills.catalog_filter")}
    />
  </header>

  {#if shown.length === 0}
    <p class="empty">{$t("llm.skills.catalog_empty")}</p>
  {:else}
    <ul class="grid">
      {#each shown as entry (entry.name)}
        {@const done = isInstalled(installed, entry)}
        {@const scan = scanOf(installed, entry.name)}
        <li class="entry">
          <div class="entry-head">
            <h3 class="name">{entry.name}</h3>
            {#if scan}
              <SkillScanBadge {scan} />
            {/if}
            {#if done}
              <span class="badge">{$t("llm.skills.installed")}</span>
            {:else if isUnlicensed(entry)}
              <span class="badge warn" title={$t("llm.skills.catalog.unlicensed")}>
                {$t("llm.skills.unlicensed_badge")}
              </span>
            {/if}
          </div>
          <p class="description">{entry.description}</p>
          {#if confirming === entry.name}
            <p class="warn-line" role="status">{$t("llm.skills.catalog.unlicensed")}</p>
          {/if}
          <div class="foot">
            {#if entry.html_url ?? entry.repo_url}
              <a
                class="link"
                href={entry.html_url ?? entry.repo_url}
                target="_blank"
                rel="noreferrer"
              >
                {$t("llm.skills.catalog_source")}
              </a>
            {/if}
            <button
              type="button"
              class="button"
              disabled={done || busy !== null}
              onclick={() => press(entry)}
            >
              {#if busy === entry.name}
                {$t("llm.skills.installing")}
              {:else if confirming === entry.name}
                {$t("llm.skills.scan_install_confirm")}
              {:else}
                {$t("llm.skills.install")}
              {/if}
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .catalog {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .lede {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .filter {
    width: 220px;
    max-width: 100%;
  }

  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-3);
  }

  .entry {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .entry-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .name {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .badge {
    padding: 1px var(--space-2);
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-size: var(--text-xs);
    white-space: nowrap;
  }

  .badge.warn {
    background: color-mix(in srgb, var(--warning, var(--danger)) 16%, transparent);
    color: var(--warning, var(--danger));
  }

  .warn-line {
    margin: 0;
    font-size: var(--text-xs);
    line-height: var(--leading-base);
    color: var(--warning, var(--text-muted));
  }

  .description {
    margin: 0;
    flex: 1;
    font-size: var(--text-sm);
    line-height: var(--leading-base);
    color: var(--text-muted);
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .link {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
