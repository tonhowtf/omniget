<script lang="ts">
  import { t } from "$lib/i18n";
  import { CATEGORY_LABEL_KEYS } from "$lib/settings-navigation";
  import type { SettingSearchEntry } from "$lib/settings-search";
  import SettingsEmptyState from "./SettingsEmptyState.svelte";
  import SettingsIcon from "./SettingsIcon.svelte";

  let {
    results,
    onOpen,
    onClear,
  }: {
    results: SettingSearchEntry[];
    onOpen: (entry: SettingSearchEntry) => void;
    onClear: () => void;
  } = $props();
</script>

<section class="settings-results" aria-labelledby="settings-results-title">
  <h1 id="settings-results-title" class="settings-title" tabindex="-1">{$t("settings.rebrand.search.label")}</h1>
  <p class="results-count" role="status">{$t("settings.rebrand.search.results", { count: results.length })}</p>
  {#if results.length === 0}
    <SettingsEmptyState icon="search" title={$t("settings.rebrand.search.empty")} hint={$t("settings.rebrand.search.empty_hint")}>
      {#snippet action()}
        <button type="button" class="btn btn-secondary btn-sm" onclick={onClear}>{$t("settings.rebrand.search.clear")}</button>
      {/snippet}
    </SettingsEmptyState>
  {:else}
    <ul>
      {#each results as entry (entry.key)}
        <li>
          <button type="button" class="settings-result" onclick={() => onOpen(entry)}>
            <span class="result-text">
              <span class="result-label">{$t(entry.key)}</span>
              <span class="result-path">
                {$t(CATEGORY_LABEL_KEYS[entry.category])}{#if entry.sectionKey !== CATEGORY_LABEL_KEYS[entry.category] && entry.sectionKey !== entry.key}{" · "}{$t(entry.sectionKey)}{/if}
              </span>
            </span>
            <SettingsIcon name={entry.href ? "external" : "chevron"} size={18} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .settings-results { min-inline-size: 0; }
  .results-count { margin: 0 0 var(--space-3); font-size: 12px; line-height: 18px; color: var(--text-muted); }
  ul { list-style: none; margin: 0; padding: 0; }
  li + li { border-block-start: 1px solid var(--separator); }
  .settings-result {
    display: flex;
    inline-size: 100%;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-block-size: 44px;
    padding: var(--space-2) var(--space-1);
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: start;
    font: inherit;
    cursor: pointer;
    border-radius: 6px;
  }
  .settings-result:hover { background: var(--surface-hi); }
  .settings-result:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .result-text { display: flex; flex-direction: column; min-inline-size: 0; }
  .result-label { font-size: 13px; line-height: 20px; overflow-wrap: anywhere; }
  .result-path { font-size: 12px; line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
</style>
