<script lang="ts">
  import { onMount } from "svelte";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { categoryParent, categoryTitle, counts, filterItems, groupByCategory, isExtreme, presetIds, pruneSelection, type Preset, type PrivacyCatalog, type PrivacyItem } from "$lib/washmybelly/privacy";
  import EmptyState from "./ui/EmptyState.svelte";
  import ConfirmDialog from "./ui/ConfirmDialog.svelte";
  import Icon from "./ui/Icon.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import PrivacyItemRow from "./PrivacyItemRow.svelte";

  /// Settings tab: the privacy.sexy catalog grouped by category, each item
  /// with the state read from the system. Presets apply only recommended,
  /// readable, not-yet-applied items; items that weaken security stay
  /// hidden behind their own warning and never join a preset.
  let { privacySourceUrl = "" }: { privacySourceUrl?: string } = $props();

  let catalog = $state<PrivacyCatalog | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let recommendedOnly = $state(false);
  let showExtreme = $state(false);
  let extremeAsk = $state(false);
  let query = $state("");
  let selected = $state(new Set<string>());
  let reviewIds = $state<string[]>([]);
  let reviewOpen = $state(false);
  let reviewTitle = $state<string | undefined>(undefined);

  let items = $derived(catalog?.items ?? []);
  let visible = $derived(filterItems(items, { recommendedOnly, showExtreme, query }));
  let groups = $derived(groupByCategory(visible));
  let tally = $derived(counts(items));
  let standardIds = $derived(presetIds(items, "standard"));
  let strictIds = $derived(presetIds(items, "strict"));
  let extremeCount = $derived(items.filter(isExtreme).length);
  let sourceUrl = $derived(catalog?.source_url || privacySourceUrl);

  async function load() {
    loading = true;
    error = null;
    try {
      catalog = await api.call<PrivacyCatalog>("wmb_privacy_catalog", {});
      selected = pruneSelection(selected, catalog.items ?? []);
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  function select(it: PrivacyItem, on: boolean) {
    const next = new Set(selected);
    if (on) next.add(it.id);
    else next.delete(it.id);
    selected = next;
  }

  function review(ids: string[], title: string) {
    if (!ids.length) return;
    reviewIds = ids;
    reviewTitle = title;
    reviewOpen = true;
  }

  function applyPreset(p: Preset) {
    const ids = p === "standard" ? standardIds : strictIds;
    review(ids, $t(p === "standard" ? "washmybelly.privacy.review_standard" : "washmybelly.privacy.review_strict", { count: ids.length }) as string);
  }

  function applyOne(it: PrivacyItem) {
    review([it.id], $t("washmybelly.privacy.review_one", { name: it.name }) as string);
  }

  function applySelected() {
    review([...selected], $t("washmybelly.privacy.review_selected", { count: selected.size }) as string);
  }

  function afterDone() {
    selected = new Set();
    void load();
  }
</script>

{#if loading && !catalog}
  <div class="ps-loading" aria-busy="true">
    <p class="dim">{$t("washmybelly.privacy.loading")}</p>
    {#each [0, 1, 2, 3] as i (i)}<span class="skeleton" style:height="48px"></span>{/each}
  </div>
{:else if error && !catalog}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if items.length === 0}
  <EmptyState art="happy" title={$t("washmybelly.privacy.empty_title") as string} body={$t("washmybelly.privacy.empty_body") as string} />
{:else}
  <div class="ps-summary">
    <span class="ps-count">{$t("washmybelly.privacy.summary", { applied: tally.applied, pending: tally.pending })}</span>
    <div class="ps-presets">
      <button class="btn btn-primary" type="button" disabled={!standardIds.length || reviewOpen} onclick={() => applyPreset("standard")}>
        {$t("washmybelly.privacy.apply_standard", { count: standardIds.length })}
      </button>
      <button class="btn btn-secondary" type="button" disabled={!strictIds.length || reviewOpen} onclick={() => applyPreset("strict")}>
        {$t("washmybelly.privacy.apply_strict", { count: strictIds.length })}
      </button>
    </div>
  </div>
  <p class="ps-hint">{$t("washmybelly.privacy.presets_hint")}</p>

  <div class="ps-bar">
    <input class="input ps-search" type="search" autocomplete="off" spellcheck="false" placeholder={$t("washmybelly.privacy.search") as string} bind:value={query} />
    <label class="ps-filter">
      <button type="button" class="toggle" class:on={recommendedOnly} role="switch" aria-checked={recommendedOnly} aria-label={$t("washmybelly.privacy.recommended_only") as string} onclick={() => (recommendedOnly = !recommendedOnly)}>
        <span class="toggle-knob"></span>
      </button>
      <span>{$t("washmybelly.privacy.recommended_only")}</span>
    </label>
    {#if selected.size}
      <button class="btn btn-primary" type="button" onclick={applySelected}>{$t("washmybelly.privacy.apply_selected", { count: selected.size })}</button>
    {/if}
    <button class="btn btn-ghost btn-icon" type="button" disabled={loading} title={$t("washmybelly.privacy.refresh") as string} aria-label={$t("washmybelly.privacy.refresh") as string} onclick={() => void load()}>
      <Icon name="refresh" size={13} />
    </button>
  </div>

  {#if groups.length === 0}
    <p class="dim none">{$t("washmybelly.privacy.no_match")}</p>
  {/if}
  {#each groups as g (g.category)}
    <span class="group-label">
      {#if categoryParent(g.category)}<span class="ps-parent">{categoryParent(g.category)} /</span>{/if}
      {categoryTitle(g.category)}
      <span class="ps-gcount">{$t("washmybelly.privacy.group_count", { applied: g.applied, count: g.items.length })}</span>
    </span>
    <div class="group" role="list">
      {#each g.items as it (it.id)}
        <PrivacyItemRow item={it} selected={selected.has(it.id)} onSelect={select} onApply={applyOne} />
      {/each}
    </div>
  {/each}

  {#if extremeCount > 0}
    <div class="notice notice-warning ps-extreme">
      <Icon name="warning" size={16} />
      <div class="notice-body">
        <span class="notice-title">{$t("washmybelly.privacy.extreme_title", { count: extremeCount })}</span>
        <span class="notice-text">{$t("washmybelly.privacy.extreme_body")}</span>
      </div>
      {#if showExtreme}
        <button class="btn btn-sm btn-secondary" type="button" onclick={() => (showExtreme = false)}>{$t("washmybelly.privacy.extreme_hide")}</button>
      {:else}
        <button class="btn btn-sm btn-secondary" type="button" onclick={() => (extremeAsk = true)}>{$t("washmybelly.privacy.extreme_show")}</button>
      {/if}
    </div>
  {/if}

  <p class="group-footer ps-license">
    {$t("washmybelly.privacy.license", { license: catalog?.license ?? "AGPL-3.0" })}
    {#if sourceUrl}
      <button class="link" type="button" onclick={() => void openExternal(sourceUrl)}>{$t("washmybelly.privacy.source_link")}</button>
    {/if}
  </p>
{/if}

<ConfirmDialog
  bind:open={extremeAsk}
  title={$t("washmybelly.privacy.extreme_confirm_title") as string}
  body={$t("washmybelly.privacy.extreme_confirm_body") as string}
  confirmLabel={$t("washmybelly.privacy.extreme_confirm") as string}
  destructive
  onConfirm={() => (showExtreme = true)}
/>

<PlanReview kind="privacy" itemIds={reviewIds} bind:open={reviewOpen} title={reviewTitle} onDone={afterDone} onRestored={afterDone} />

<style>
  .ps-summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .ps-count {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .ps-presets {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .ps-hint {
    margin: var(--space-2) 0 var(--space-4);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .ps-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .ps-search {
    flex: 1 1 200px;
    min-width: 0;
  }
  .ps-filter {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  .ps-parent {
    font-weight: 400;
    color: var(--text-dim);
  }
  .ps-gcount {
    font-weight: 400;
    margin-left: var(--space-1);
  }
  .group {
    margin-bottom: var(--space-5);
  }
  .ps-extreme {
    align-items: center;
    margin-bottom: var(--space-4);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .none {
    padding: var(--space-4) var(--space-2);
  }
  .ps-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .ps-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
</style>
