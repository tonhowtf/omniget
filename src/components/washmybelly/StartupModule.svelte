<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { disableIds, filterItems, groupItems, noteCodes, type StartupItem, type StartupList } from "$lib/washmybelly/startup";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import StartupRow from "./StartupRow.svelte";

  /// Startup: what opens at login or runs in the background, by type.
  /// Each switch turns one item off or on through a one-item plan (nothing
  /// is deleted, the receipt can undo it); several can be picked and turned
  /// off together. System items are shown read only with the reason.
  let { caps }: { caps: Caps } = $props();

  const mod = moduleById("startup");

  let items = $state<StartupItem[]>([]);
  let notes = $state<string[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let thirdOnly = $state(true);
  let query = $state("");
  let selected = $state(new Set<string>());
  let reviewIds = $state<string[]>([]);
  let reviewOpen = $state(false);
  let reviewTitle = $state<string | undefined>(undefined);
  let loadedSystem = $state(false);

  let visible = $derived(filterItems(items, { thirdPartyOnly: thirdOnly, query }));
  let groups = $derived(groupItems(visible));
  let chosen = $derived(disableIds(items, selected));
  let onCount = $derived(items.filter((i) => i.enabled && i.third_party).length);
  let settingsOnly = $derived(items.some((i) => i.reason === "settings_only") && caps.os !== "linux");
  let noteList = $derived(noteCodes(notes));

  async function load() {
    error = null;
    loading = true;
    try {
      const r = await api.call<StartupList>("wmb_startup_list", { include_system: !thirdOnly });
      items = r.items ?? [];
      notes = r.notes ?? [];
      loadedSystem = !thirdOnly;
      const ids = new Set(items.map((i) => i.id));
      selected = new Set([...selected].filter((id) => ids.has(id)));
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  // System items are only fetched when the filter asks for them.
  $effect(() => {
    if (!thirdOnly && !loadedSystem && !loading) void load();
  });

  function toggle(it: StartupItem) {
    reviewIds = [it.id];
    reviewTitle = $t(it.enabled ? "washmybelly.startup.review_off" : "washmybelly.startup.review_on", { name: it.name }) as string;
    reviewOpen = true;
  }

  function disableSelected() {
    if (!chosen.length) return;
    reviewIds = chosen;
    reviewTitle = $t("washmybelly.startup.review_many", { count: chosen.length }) as string;
    reviewOpen = true;
  }

  function select(it: StartupItem, on: boolean) {
    const next = new Set(selected);
    if (on) next.add(it.id);
    else next.delete(it.id);
    selected = next;
  }

  function afterDone() {
    selected = new Set();
    void load();
  }

  async function openSettings() {
    try {
      await api.ui("startup.open_settings");
    } catch (e) {
      error = errorText(e);
    }
  }

  function groupLabel(key: string): string {
    const k = `washmybelly.startup.kind.${key}`;
    const s = $t(k) as string;
    return s === k ? key : s;
  }
</script>

<ModuleHeader title={$t("washmybelly.modules.startup") as string} subtitle={$t("washmybelly.startup.subtitle") as string} tile={mod}>
  {#snippet meta()}
    {#if !loading && items.length}
      <span class="st-meta">{$t("washmybelly.startup.on_count", { count: onCount })}</span>
    {/if}
  {/snippet}
  {#snippet actions()}
    <button class="btn btn-secondary" type="button" disabled={loading} onclick={() => void load()}>
      <Icon name="refresh" size={13} />{$t("washmybelly.startup.refresh")}
    </button>
  {/snippet}
</ModuleHeader>

{#if loading && items.length === 0}
  <div class="st-loading" aria-busy="true">
    <p class="dim">{$t("washmybelly.startup.loading")}</p>
    {#each [0, 1, 2, 3, 4] as i (i)}<span class="skeleton" style:height="52px"></span>{/each}
  </div>
{:else if error && items.length === 0}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else}
  <div class="st-bar">
    <input
      class="input st-search"
      type="search"
      autocomplete="off"
      spellcheck="false"
      placeholder={$t("washmybelly.startup.search") as string}
      bind:value={query}
    />
    <label class="st-filter">
      <button type="button" class="toggle" class:on={thirdOnly} role="switch" aria-checked={thirdOnly} aria-label={$t("washmybelly.startup.third_only") as string} onclick={() => (thirdOnly = !thirdOnly)}>
        <span class="toggle-knob"></span>
      </button>
      <span>{$t("washmybelly.startup.third_only")}</span>
    </label>
    {#if chosen.length}
      <button class="btn btn-primary" type="button" onclick={disableSelected}>
        {$t("washmybelly.startup.disable_selected", { count: chosen.length })}
      </button>
    {/if}
  </div>

  {#if noteList.length}
    <div class="st-notes">
      {#each noteList as n (n)}
        <p class="st-note"><Icon name="info" size={12} />{$t(`washmybelly.startup.note.${n}`)}</p>
      {/each}
    </div>
  {/if}

  {#if groups.length === 0}
    {#if query.trim()}
      <p class="dim none">{$t("washmybelly.startup.no_match")}</p>
    {:else}
      <EmptyState art="sleeping" title={$t("washmybelly.startup.empty_title") as string} body={$t("washmybelly.startup.empty_body") as string} />
    {/if}
  {:else}
    {#each groups as g (g.group)}
      <span class="group-label">
        {$t(`washmybelly.startup.group.${g.group}`)}
        <span class="st-count">{$t("washmybelly.startup.group_count", { count: g.items.length, on: g.on })}</span>
      </span>
      <div class="group" role="list">
        {#each g.items as it (it.id)}
          <StartupRow item={it} selected={selected.has(it.id)} busy={reviewOpen} onToggle={toggle} onSelect={select} />
        {/each}
      </div>
    {/each}
    {#if settingsOnly}
      <p class="group-footer">
        {$t("washmybelly.startup.settings_hint")}
        <button class="link" type="button" onclick={() => void openSettings()}>{$t("washmybelly.startup.open_settings")}</button>
      </p>
    {/if}
  {/if}
{/if}

<PlanReview kind="startup" itemIds={reviewIds} bind:open={reviewOpen} title={reviewTitle} {groupLabel} onDone={afterDone} onRestored={afterDone} />

<style>
  .st-meta {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .st-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .st-search {
    flex: 1 1 200px;
    min-width: 0;
  }
  .st-filter {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  .st-notes {
    margin-bottom: var(--space-3);
  }
  .st-note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .st-count {
    font-weight: 400;
    margin-left: var(--space-1);
  }
  .group {
    margin-bottom: var(--space-5);
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
  .st-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .st-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
</style>
