<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import {
    appliedCount,
    clearGroup,
    extremePicks,
    orderedCategories,
    pickedItemIds,
    pickedNeedRestart,
    prunePicks,
    rowsFor,
    togglePick,
    type TweakGroup,
    type TweakItem,
    type TweakList,
  } from "$lib/washmybelly/tweaks";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import ConfirmDialog from "./ui/ConfirmDialog.svelte";
  import TweakRow from "./TweakRow.svelte";
  import TweakGroupCard from "./TweakGroupCard.svelte";

  /// Windows tweaks, by category. The state of each one is read from
  /// Windows every time; nothing changes until the picked items go through
  /// the plan review, and the receipt can put the old values back. Exclusive
  /// choices (power plan, taskbar search, IPv6...) are radio cards. Memory
  /// integrity asks first, because it trades security for FPS.
  let { caps }: { caps: Caps } = $props();

  const mod = moduleById("tweaks");

  let list = $state<TweakList | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let category = $state<string | null>(null);
  let query = $state("");
  let picked = $state(new Set<string>());
  let reviewIds = $state<string[]>([]);
  let reviewOpen = $state(false);
  let reviewTitle = $state<string | undefined>(undefined);
  let warnOpen = $state(false);

  let categories = $derived(orderedCategories(list));
  let rows = $derived(rowsFor(list, category, query));
  let ids = $derived(pickedItemIds(list, picked));
  let extreme = $derived(extremePicks(list, picked));
  let restart = $derived(pickedNeedRestart(list, picked));
  let applied = $derived(appliedCount(list));

  async function load() {
    error = null;
    loading = true;
    try {
      const r = await api.call<TweakList>("wmb_tweaks_list", {});
      list = r;
      picked = prunePicks(r, picked);
      if (category && !r.categories.includes(category)) category = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  function pick(it: TweakItem) {
    if (list) picked = togglePick(list, picked, it);
  }

  function clear(g: TweakGroup) {
    if (list) picked = clearGroup(list, picked, g.id);
  }

  function openReview() {
    reviewIds = ids;
    reviewTitle = $t("washmybelly.tweaks.review_many", { count: ids.length }) as string;
    reviewOpen = true;
  }

  function apply() {
    if (!ids.length) return;
    if (extreme.length) warnOpen = true;
    else openReview();
  }

  function revert(it: TweakItem) {
    if (!it.default_item_id) return;
    reviewIds = [it.default_item_id];
    reviewTitle = $t("washmybelly.tweaks.review_default", { name: it.label }) as string;
    reviewOpen = true;
  }

  function afterDone() {
    picked = new Set();
    void load();
  }

  function categoryLabel(c: string): string {
    const k = `washmybelly.tweaks.category.${c}`;
    const s = $t(k) as string;
    return s === k ? c : s;
  }
</script>

<ModuleHeader title={$t("washmybelly.modules.tweaks") as string} subtitle={$t("washmybelly.tweaks.subtitle") as string} tile={mod}>
  {#snippet meta()}
    {#if list}
      <span class="tw-meta">{$t("washmybelly.tweaks.meta", { count: applied, build: list.build })}</span>
    {/if}
  {/snippet}
  {#snippet actions()}
    <button class="btn btn-secondary" type="button" disabled={loading} onclick={() => void load()}>
      <Icon name="refresh" size={13} />{$t("washmybelly.tweaks.refresh")}
    </button>
  {/snippet}
</ModuleHeader>

{#if caps.os !== "windows"}
  <EmptyState art="sleeping" title={$t("washmybelly.tweaks.windows_only") as string} />
{:else if loading && !list}
  <div class="tw-loading" aria-busy="true">
    <p class="dim">{$t("washmybelly.tweaks.loading")}</p>
    {#each [0, 1, 2, 3, 4] as i (i)}<span class="skeleton" style:height="56px"></span>{/each}
  </div>
{:else if error && !list}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if list}
  <div class="tw-cats" role="tablist" aria-label={$t("washmybelly.tweaks.categories") as string}>
    <button type="button" role="tab" class="tw-cat" class:active={category === null} aria-selected={category === null} onclick={() => (category = null)}>
      {$t("washmybelly.tweaks.all")}
    </button>
    {#each categories as c (c)}
      <button type="button" role="tab" class="tw-cat" class:active={category === c} aria-selected={category === c} onclick={() => (category = c)}>
        {categoryLabel(c)}
        {#if appliedCount(list, c)}<span class="tw-cat-n">{appliedCount(list, c)}</span>{/if}
      </button>
    {/each}
  </div>

  <div class="tw-bar">
    <input class="input tw-search" type="search" autocomplete="off" spellcheck="false" placeholder={$t("washmybelly.tweaks.search") as string} bind:value={query} />
  </div>

  <p class="tw-note"><Icon name="info" size={12} />{$t(list.restore_point ? "washmybelly.tweaks.note_restore_point" : "washmybelly.tweaks.note_undo")}</p>

  {#if rows.length === 0}
    <p class="dim none">{$t("washmybelly.tweaks.no_match")}</p>
  {:else}
    <div class="group tw-list" role="list">
      {#each rows as r (r.kind === "group" ? r.group.id : r.item.id)}
        {#if r.kind === "group"}
          <TweakGroupCard group={r.group} items={r.items} {picked} busy={reviewOpen} onPick={pick} onClear={clear} />
        {:else}
          <TweakRow item={r.item} picked={picked.has(r.item.id)} busy={reviewOpen} onPick={pick} onRevert={revert} />
        {/if}
      {/each}
    </div>
  {/if}

  {#if ids.length}
    <div class="tw-apply" role="region" aria-label={$t("washmybelly.tweaks.picked", { count: ids.length }) as string}>
      <span class="tw-apply-text">
        {$t("washmybelly.tweaks.picked", { count: ids.length })}
        {#if restart}<span class="dim">· {$t("washmybelly.tweaks.restart_after")}</span>{/if}
      </span>
      <button class="btn btn-secondary" type="button" onclick={() => (picked = new Set())}>{$t("washmybelly.tweaks.clear")}</button>
      <button class="btn btn-primary" type="button" onclick={apply}>{$t("washmybelly.tweaks.apply", { count: ids.length })}</button>
    </div>
  {/if}
{/if}

<ConfirmDialog
  bind:open={warnOpen}
  title={$t("washmybelly.tweaks.hvci_title") as string}
  body={$t("washmybelly.tweaks.hvci_body") as string}
  confirmLabel={$t("washmybelly.tweaks.hvci_confirm") as string}
  destructive
  tone="danger"
  onConfirm={() => {
    // Opens the review once this dialog has closed.
    queueMicrotask(openReview);
  }}
>
  <ul class="tw-extreme">
    {#each extreme as it (it.id)}<li>{it.label}</li>{/each}
  </ul>
</ConfirmDialog>

<PlanReview kind="tweaks" itemIds={reviewIds} bind:open={reviewOpen} title={reviewTitle} groupLabel={categoryLabel} onDone={afterDone} onRestored={afterDone} />

<style>
  .tw-meta {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .tw-cats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-bottom: var(--space-3);
  }
  .tw-cat {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
  }
  .tw-cat.active {
    background: var(--accent);
    color: var(--accent-contrast, #fff);
  }
  .tw-cat:focus-visible {
    outline: var(--focus-ring);
  }
  .tw-cat-n {
    font-size: var(--text-caption);
    font-weight: 600;
    opacity: 0.8;
  }
  .tw-bar {
    display: flex;
    gap: var(--space-3);
    margin-bottom: var(--space-2);
  }
  .tw-search {
    flex: 1 1 200px;
    min-width: 0;
  }
  .tw-note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .tw-list {
    margin-bottom: var(--space-6, 48px);
  }
  .tw-apply {
    position: sticky;
    bottom: var(--space-3);
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface-hi);
    box-shadow: 0 6px 24px rgba(var(--shadow-ink), var(--elev-alpha-2, 0.18)), inset 0 0 0 var(--hairline) var(--content-border);
  }
  .tw-apply-text {
    flex: 1;
    font-size: var(--text-sm);
    font-weight: 500;
  }
  .tw-extreme {
    margin: 0;
    padding-left: var(--space-4);
    text-align: left;
    font-size: var(--text-sm);
  }
  .dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .none {
    padding: var(--space-4) var(--space-2);
  }
  .tw-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .tw-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
</style>
