<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { Progress } from "$lib/washmybelly/api";
  import { errorText, formatBytes } from "$lib/washmybelly/format";
  import { defaultUpdates, managerLabel, managers, updateRows, updatesSummary, type UpdatesList } from "$lib/washmybelly/updates";
  import { pruneSelection } from "$lib/washmybelly/ui";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import SelectList, { type SelectItem } from "./ui/SelectList.svelte";
  import { onProgress } from "./ui/progress";

  let _props: { caps: Caps } = $props();

  const mod = moduleById("packages");

  let list = $state<UpdatesList | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let progress = $state<Progress | null>(null);
  let chosen = $state(new Set<string>());
  let reviewOpen = $state(false);

  let items = $derived(list?.items ?? []);
  let rows = $derived(updateRows(items));
  let groups = $derived(managers(items));
  let sum = $derived(updatesSummary(items, chosen));
  let adminIds = $derived(new Set(items.filter((u) => u.admin).map((u) => u.id)));

  let unlisten: (() => void) | null = null;

  async function load() {
    loading = true;
    error = null;
    progress = null;
    try {
      const r = await api.call<UpdatesList>("wmb_updates_list");
      const first = list === null;
      list = r;
      const fresh = defaultUpdates(r.items ?? []);
      chosen = first ? fresh : pruneSelection(chosen, updateRows(r.items ?? []));
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
      progress = null;
    }
  }

  onMount(async () => {
    unlisten = await onProgress("updates_list", (p) => (progress = p));
    void load();
  });
  onDestroy(() => unlisten?.());

  function groupLabel(key: string): string {
    const k = `washmybelly.updates.manager.${key}`;
    const s = $t(k) as string;
    return s === k ? managerLabel(key) : s;
  }

  function isAdmin(it: SelectItem): boolean {
    return adminIds.has(it.id);
  }

  function afterRun() {
    list = null;
    void load();
  }
</script>

<ModuleHeader title={$t("washmybelly.modules.packages") as string} subtitle={$t("washmybelly.updates.subtitle") as string} tile={mod}>
  {#snippet meta()}
    {#if list && items.length}
      <span class="wmb-up-meta">
        {$t("washmybelly.updates.count", { count: items.length })} · {groups.map((g) => groupLabel(g.manager)).join(", ")}
      </span>
    {/if}
  {/snippet}
  {#snippet actions()}
    <button class="btn btn-secondary" type="button" disabled={loading} onclick={() => void load()}>
      <Icon name="refresh" size={13} />{$t("washmybelly.updates.check")}
    </button>
  {/snippet}
</ModuleHeader>

{#if loading && !list}
  <div class="wmb-up-loading" aria-busy="true">
    <p class="dim">{progress?.message || $t("washmybelly.updates.loading")}</p>
    {#each [0, 1, 2, 3] as i (i)}<span class="skeleton" style:height="44px"></span>{/each}
  </div>
{:else if error && !list}
  <EmptyState art="worried" title={$t("washmybelly.updates.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if items.length === 0}
  <EmptyState art="happy" title={$t("washmybelly.updates.empty_title") as string} body={$t("washmybelly.updates.empty_body") as string} />
{:else}
  {#if error}<p class="wmb-up-error" role="alert">{error}</p>{/if}
  <SelectList items={rows} bind:selected={chosen} showRisk={false} {groupLabel} collapsible={groups.length > 1}>
    {#snippet trailing(it)}
      {#if isAdmin(it)}
        <span class="tag tag-warning" title={$t("washmybelly.updates.admin_hint") as string}><Icon name="lock" size={10} />{$t("washmybelly.updates.admin")}</span>
      {/if}
    {/snippet}
  </SelectList>

  <footer class="wmb-up-foot">
    <span class="dim">
      {#if sum.count}
        {sum.bytes ? $t("washmybelly.updates.selected_size", { count: sum.count, size: formatBytes(sum.bytes) }) : $t("washmybelly.updates.selected", { count: sum.count })}
        {#if sum.admin}· {$t("washmybelly.updates.selected_admin", { count: sum.admin })}{/if}
      {:else}
        {$t("washmybelly.ui.nothing_selected")}
      {/if}
    </span>
    <button class="btn btn-primary" type="button" disabled={!sum.count || loading} onclick={() => (reviewOpen = true)}>
      {$t("washmybelly.updates.update_selected")}
    </button>
  </footer>
{/if}

<PlanReview kind="packages" itemIds={[...chosen]} bind:open={reviewOpen} {groupLabel} onDone={afterRun} />

<style>
  .wmb-up-meta {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .wmb-up-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .wmb-up-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
  .dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .wmb-up-error {
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--danger);
  }
  .wmb-up-foot {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-3);
    padding: var(--space-3) 0 var(--space-1);
    background: linear-gradient(to bottom, transparent, var(--pane-bg, var(--pane)) 30%);
  }
  .wmb-up-foot .dim {
    margin-right: auto;
  }
  .tag :global(svg) {
    margin-right: 2px;
  }
</style>
