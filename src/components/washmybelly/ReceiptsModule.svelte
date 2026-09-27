<script lang="ts">
  import { onMount } from "svelte";
  import { t, locale } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, formatBytes, formatDay, splitBytes } from "$lib/washmybelly/format";
  import { groupByDay } from "$lib/washmybelly/ui";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import StatTile from "./ui/StatTile.svelte";
  import ConfirmDialog from "./ui/ConfirmDialog.svelte";
  import Icon from "./ui/Icon.svelte";
  import ReceiptRow from "./ReceiptRow.svelte";

  /// History: every executed plan, newest first, grouped by day. Each
  /// receipt opens to show what happened per item and can be restored.
  let _props: { caps: Caps } = $props();

  const mod = moduleById("receipts");

  let receipts = $state<ReceiptSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let expanded = $state<string | null>(null);
  let confirmOpen = $state(false);
  let target = $state<ReceiptSummary | null>(null);
  let restoreResults = $state<Record<string, { label: string; ok: boolean; detail: string }[]>>({});

  let days = $derived(groupByDay(receipts));
  let totalFreed = $derived(receipts.reduce((n, r) => n + (r.bytes_freed || 0), 0));
  let undoable = $derived(receipts.filter((r) => r.restorable && !r.restored_at).length);
  let freed = $derived(splitBytes(totalFreed));

  async function load() {
    error = null;
    try {
      const r = await api.call<{ receipts: ReceiptSummary[] }>("wmb_receipts", { limit: 100 });
      receipts = r.receipts ?? [];
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  function askRestore(r: ReceiptSummary) {
    target = r;
    confirmOpen = true;
  }

  async function doRestore() {
    if (!target) return;
    const id = target.receipt_id;
    const r = await api.restore(id);
    restoreResults = { ...restoreResults, [id]: r.items };
    expanded = id;
    await load();
  }

  function dayLabel(bucket: string, ts: number): string {
    if (bucket === "today") return $t("washmybelly.receipts.today") as string;
    if (bucket === "yesterday") return $t("washmybelly.receipts.yesterday") as string;
    return formatDay(ts, $locale ?? undefined);
  }

  onMount(() => {
    void load();
  });
</script>

<ModuleHeader title={$t("washmybelly.modules.receipts") as string} subtitle={$t("washmybelly.receipts.subtitle") as string} tile={mod}>
  {#snippet actions()}
    <button class="btn btn-secondary" disabled={loading} onclick={() => { loading = true; void load(); }}>
      <Icon name="refresh" size={13} />{$t("washmybelly.receipts.refresh")}
    </button>
  {/snippet}
</ModuleHeader>

{#if loading && receipts.length === 0}
  <div class="wmb-rc-tiles">
    {#each [0, 1, 2] as i (i)}<span class="skeleton" style:height="70px" style:border-radius="var(--radius-lg)"></span>{/each}
  </div>
  <div class="skeleton" style:height="180px" style:border-radius="var(--radius-lg)"></div>
{:else if error}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" onclick={() => { loading = true; void load(); }}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if receipts.length === 0}
  <EmptyState art="sleeping" title={$t("washmybelly.receipts.empty_title") as string} body={$t("washmybelly.receipts.empty_body") as string} />
{:else}
  <div class="wmb-rc-tiles">
    <StatTile label={$t("washmybelly.receipts.stat_freed") as string} value={freed.value} unit={freed.unit} tone="good" />
    <StatTile label={$t("washmybelly.receipts.stat_runs") as string} value={String(receipts.length)} />
    <StatTile label={$t("washmybelly.receipts.stat_undoable") as string} value={String(undoable)} sub={$t("washmybelly.receipts.stat_undoable_sub") as string} />
  </div>

  {#each days as day (day.key)}
    <h3 class="group-label wmb-rc-day">{dayLabel(day.bucket, day.ts)}</h3>
    <ul class="group wmb-rc-list">
      {#each day.items as r (r.receipt_id)}
        <ReceiptRow
          receipt={r}
          expanded={expanded === r.receipt_id}
          onToggle={() => (expanded = expanded === r.receipt_id ? null : r.receipt_id)}
          onRestore={askRestore}
          restoreLines={restoreResults[r.receipt_id] ?? null}
        />
      {/each}
    </ul>
  {/each}
{/if}

<ConfirmDialog
  bind:open={confirmOpen}
  title={$t("washmybelly.receipts.confirm_title") as string}
  body={target ? ($t("washmybelly.receipts.confirm_body", { count: target.done, size: formatBytes(target.bytes_freed) }) as string) : ""}
  confirmLabel={$t("washmybelly.receipts.restore") as string}
  tone="info"
  onConfirm={doRestore}
/>

<style>
  .wmb-rc-tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: var(--space-3);
    margin-bottom: var(--space-5);
  }
  .wmb-rc-day {
    margin-top: var(--space-5);
  }
  .wmb-rc-day:first-of-type {
    margin-top: 0;
  }
  .wmb-rc-list {
    margin: 0;
    padding: 0;
  }
</style>
