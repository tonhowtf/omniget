<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { Progress, ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, formatBytes } from "$lib/washmybelly/format";
  import {
    categoryKey,
    categorySlices,
    defaultJunkSelection,
    junkRows,
    pruneSelection,
    ruleLabelKey,
    runningReason,
    selectedByCategory,
    selectedBytes,
    type JunkItem,
    type JunkScan,
  } from "$lib/washmybelly/clean";
  import type { Caps } from "./registry";
  import EmptyState from "./ui/EmptyState.svelte";
  import ScanProgress from "./ui/ScanProgress.svelte";
  import SelectList from "./ui/SelectList.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import Icon from "./ui/Icon.svelte";
  import { onProgress } from "./ui/progress";
  import JunkSummary from "./JunkSummary.svelte";

  let { caps }: { caps: Caps } = $props();

  type Phase = "idle" | "scanning" | "done" | "error";
  let phase = $state<Phase>("idle");
  let scan = $state<JunkScan | null>(null);
  let selected = $state(new Set<string>());
  let progress = $state<Progress | null>(null);
  let error = $state<string | null>(null);
  let filter = $state<string | null>(null);
  let reviewOpen = $state(false);
  let receipt = $state<ReceiptSummary | null>(null);
  let stop: (() => void) | null = null;

  onDestroy(() => stop?.());

  function tr(key: string, fallback: string, params?: Record<string, unknown>): string {
    const s = $t(key, params) as string;
    return s === key ? fallback : s;
  }

  function isPath(p: string): boolean {
    return p.startsWith("/") || p.startsWith("~") || /^[A-Za-z]:\\/.test(p) || p.startsWith("\\\\");
  }

  function itemLabel(it: JunkItem): string {
    return isPath(it.path) ? tr(ruleLabelKey(it.rule_id), it.label) : it.label;
  }

  function itemHint(it: JunkItem): string | null {
    const r = runningReason(it.running);
    if (r?.kind === "close") return $t("washmybelly.clean.junk.close_app", { app: r.app }) as string;
    if (r?.kind === "in_use") return $t("washmybelly.clean.junk.in_use", { who: r.who }) as string;
    if (r?.kind === "lock") return $t("washmybelly.clean.junk.close_owner") as string;
    if (it.category === "privacy") return $t("washmybelly.clean.junk.privacy_hint") as string;
    if (it.category === "trash") return $t("washmybelly.clean.junk.trash_hint") as string;
    return null;
  }

  let rows = $derived(scan ? junkRows(scan.items, itemLabel, itemHint) : []);
  let shown = $derived(filter ? rows.filter((r) => r.group === filter) : rows);
  let slices = $derived(scan ? categorySlices(scan) : []);
  let perCategory = $derived(scan ? selectedByCategory(scan.items, selected) : new Map<string, number>());
  let chosenBytes = $derived(scan ? selectedBytes(scan.items, selected) : 0);
  let chosenIds = $derived([...selected]);
  let privacyChosen = $derived(!!scan?.items.some((i) => i.category === "privacy" && selected.has(i.id)));
  let adminChosen = $derived(!!scan?.items.some((i) => i.admin && selected.has(i.id)));
  let showFda = $derived(caps.os === "macos" && !!scan?.needs_full_disk_access);

  async function run() {
    if (phase === "scanning") return;
    phase = "scanning";
    error = null;
    receipt = null;
    filter = null;
    progress = null;
    stop?.();
    stop = await onProgress("junk_scan", (p) => (progress = p)).catch(() => null);
    try {
      const r = await api.call<JunkScan>("wmb_junk_scan", {});
      scan = r;
      selected = defaultJunkSelection(r.items);
      phase = "done";
    } catch (e) {
      error = errorText(e);
      phase = "error";
    } finally {
      stop?.();
      stop = null;
    }
  }

  function groupLabel(key: string): string {
    return tr(categoryKey(key), key.replace(/_/g, " "));
  }

  async function openFda() {
    try {
      await api.ui("security.open_fda_settings");
    } catch (e) {
      error = errorText(e);
    }
  }

  function done(r: ReceiptSummary) {
    receipt = r;
  }

  function restored() {
    receipt = null;
    scan = null;
    phase = "idle";
  }

  $effect(() => {
    if (!scan) return;
    const next = pruneSelection(selected, rows);
    if (next.size !== selected.size) selected = next;
  });
</script>

<div class="wmb-junk">
  {#if phase === "idle"}
    <EmptyState art="searching" title={$t("washmybelly.clean.junk.idle_title") as string} body={$t("washmybelly.clean.junk.idle_body") as string}>
      {#snippet action()}
        <button type="button" class="btn btn-primary" onclick={run}>{$t("washmybelly.clean.junk.scan")}</button>
      {/snippet}
    </EmptyState>
  {:else if phase === "scanning"}
    <div class="card">
      <ScanProgress
        title={$t("washmybelly.clean.junk.scanning") as string}
        done={progress?.done ?? 0}
        total={progress?.total ?? 0}
        message={progress?.message ?? ""}
      />
    </div>
  {:else if phase === "error"}
    <div class="notice notice-danger" role="alert">
      <Icon name="error" size={18} />
      <div class="notice-body">
        <span class="notice-title">{$t("washmybelly.clean.scan_failed")}</span>
        <span class="notice-text">{error}</span>
      </div>
      <button type="button" class="btn btn-secondary btn-sm" onclick={run}>{$t("washmybelly.retry")}</button>
    </div>
  {:else if receipt}
    <EmptyState
      art="happy"
      title={$t("washmybelly.clean.junk.done_title", { size: formatBytes(receipt.bytes_freed) }) as string}
      body={$t("washmybelly.clean.junk.done_body") as string}
    >
      {#snippet action()}
        <button type="button" class="btn btn-secondary" onclick={run}>{$t("washmybelly.clean.scan_again")}</button>
      {/snippet}
    </EmptyState>
  {:else if scan && scan.items.length === 0}
    <EmptyState art="sleeping" title={$t("washmybelly.clean.junk.none_title") as string} body={$t("washmybelly.clean.junk.none_body") as string}>
      {#snippet action()}
        <button type="button" class="btn btn-secondary" onclick={run}>{$t("washmybelly.clean.scan_again")}</button>
      {/snippet}
    </EmptyState>
  {:else if scan}
    <div class="head">
      <div class="total">
        <span class="big">{formatBytes(scan.total_bytes)}</span>
        <span class="dim">{$t("washmybelly.clean.junk.found", { count: scan.items.length })}</span>
      </div>
      <button type="button" class="btn btn-ghost btn-sm" onclick={run}>{$t("washmybelly.clean.scan_again")}</button>
    </div>

    <JunkSummary {slices} selected={perCategory} onPick={(id) => (filter = filter === id ? null : id)} />

    {#if showFda}
      <div class="notice notice-info">
        <Icon name="lock" size={18} />
        <div class="notice-body">
          <span class="notice-title">{$t("washmybelly.clean.junk.fda_title")}</span>
          <span class="notice-text">{$t("washmybelly.clean.junk.fda_body")}</span>
        </div>
        <button type="button" class="btn btn-secondary btn-sm" onclick={openFda}>{$t("washmybelly.ui.open_fda")}</button>
      </div>
    {/if}

    {#if filter}
      <div class="filter">
        <span>{$t("washmybelly.clean.junk.showing", { category: groupLabel(filter) })}</span>
        <button type="button" class="link" onclick={() => (filter = null)}>{$t("washmybelly.clean.junk.show_all")}</button>
      </div>
    {/if}

    <SelectList items={shown} bind:selected {groupLabel} showRisk={false}>
      {#snippet trailing(item)}
        {@const it = scan?.items.find((x) => x.id === item.id)}
        {#if it?.admin}<span class="tag tag-warning">{$t("washmybelly.clean.junk.admin_badge")}</span>{/if}
        {#if it?.category === "privacy"}<span class="tag tag-danger">{$t("washmybelly.clean.junk.for_good")}</span>{/if}
      {/snippet}
    </SelectList>

    {#if privacyChosen}
      <p class="note warn"><Icon name="warning" size={14} /> {$t("washmybelly.clean.junk.privacy_note")}</p>
    {/if}
    {#if adminChosen}
      <p class="note"><Icon name="lock" size={14} /> {$t("washmybelly.plan.needs_admin_body")}</p>
    {/if}
    {#if scan.protected_paths > 0}
      <p class="note">{$t("washmybelly.clean.junk.protected", { count: scan.protected_paths })}</p>
    {/if}

    <div class="actions">
      <button type="button" class="btn btn-primary" disabled={!selected.size} onclick={() => (reviewOpen = true)}>
        {$t("washmybelly.clean.junk.clean_size", { size: formatBytes(chosenBytes) })}
      </button>
    </div>
  {/if}
</div>

<PlanReview
  kind="cleanup"
  itemIds={chosenIds}
  bind:open={reviewOpen}
  {groupLabel}
  confirmLabel={$t("washmybelly.clean.junk.clean_size", { size: formatBytes(chosenBytes) }) as string}
  onDone={done}
  onRestored={restored}
/>

<style>
  .wmb-junk {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .card {
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    padding: var(--space-4);
  }
  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .total {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .big {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .filter {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
    margin-bottom: calc(-1 * var(--space-2));
  }
  .link {
    all: unset;
    cursor: pointer;
    color: var(--accent-text, var(--accent));
  }
  .link:focus-visible {
    outline: var(--focus-ring);
    border-radius: 4px;
  }
  .note {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: calc(-1 * var(--space-2)) 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .note.warn {
    color: var(--warning);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    position: sticky;
    bottom: 0;
    z-index: 2;
    padding: var(--space-3) 0;
    background: var(--pane-bg);
    border-top: 1px solid var(--separator, var(--button-elevated));
  }
</style>
