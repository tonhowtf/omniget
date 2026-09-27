<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { desktopDir, documentDir, downloadDir, pictureDir } from "@tauri-apps/api/path";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { Progress, ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, formatBytes, tildify } from "$lib/washmybelly/format";
  import {
    cloneIdsFor,
    defaultDupSelection,
    dupPlanIds,
    dupSelectedBytes,
    dupSelectedCount,
    initialKeeps,
    setKeep,
    toggleCopy,
    unclonableCount,
    type DupScan,
  } from "$lib/washmybelly/clean";
  import type { Caps } from "./registry";
  import EmptyState from "./ui/EmptyState.svelte";
  import ScanProgress from "./ui/ScanProgress.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import Icon from "./ui/Icon.svelte";
  import { onProgress } from "./ui/progress";
  import CleanRoots from "./CleanRoots.svelte";
  import DuplicatesGroup from "./DuplicatesGroup.svelte";

  /// Exact duplicates in the folders the person picks. Each group keeps
  /// one file; the copies go to the Trash, or, on an APFS disk, become
  /// clones of the kept file that take no extra space.
  let { caps }: { caps: Caps } = $props();

  const SIZES = [
    { kb: 64, key: "washmybelly.clean.dupes.min_64k" },
    { kb: 1024, key: "washmybelly.clean.dupes.min_1m" },
    { kb: 10240, key: "washmybelly.clean.dupes.min_10m" },
    { kb: 102400, key: "washmybelly.clean.dupes.min_100m" },
  ] as const;
  const PAGE = 60;

  let roots = $state<string[]>([]);
  let minKb = $state(1024);
  let phase = $state<"idle" | "scanning" | "done" | "error">("idle");
  let scan = $state<DupScan | null>(null);
  let keeps = $state(new Map<string, string>());
  let selected = $state(new Set<string>());
  let progress = $state<Progress | null>(null);
  let error = $state<string | null>(null);
  let limit = $state(PAGE);
  let mode = $state<"trash" | "clone">("trash");
  let reviewOpen = $state(false);
  let receipt = $state<ReceiptSummary | null>(null);
  let stop: (() => void) | null = null;
  onDestroy(() => stop?.());

  onMount(async () => {
    const found = await Promise.allSettled([downloadDir(), documentDir(), desktopDir(), pictureDir()]);
    if (roots.length) return;
    roots = found.flatMap((r) => (r.status === "fulfilled" && r.value ? [r.value.replace(/[\\/]$/, "")] : []));
  });

  let groups = $derived(scan?.groups ?? []);
  let visible = $derived(groups.slice(0, limit));
  let count = $derived(dupSelectedCount(groups, selected));
  let bytes = $derived(dupSelectedBytes(groups, selected));
  let trashPlan = $derived(dupPlanIds(groups, selected));
  let cloneIds = $derived(cloneIdsFor(groups, keeps, selected));
  let unclonable = $derived(unclonableCount(groups, keeps, selected));
  let canClone = $derived(caps.os === "macos" && !!scan?.clone_plan_id);
  let planIds = $derived(mode === "clone" ? cloneIds : trashPlan.ids);

  async function run() {
    if (phase === "scanning" || !roots.length) return;
    phase = "scanning";
    error = null;
    receipt = null;
    progress = null;
    limit = PAGE;
    stop?.();
    stop = await onProgress("find_duplicates", (p) => (progress = p)).catch(() => null);
    try {
      const r = await api.call<DupScan>("wmb_find_duplicates", { roots, min_kb: minKb });
      scan = r;
      keeps = initialKeeps(r.groups);
      selected = defaultDupSelection(r.groups, keeps);
      phase = "done";
    } catch (e) {
      error = errorText(e);
      phase = "error";
    } finally {
      stop?.();
      stop = null;
    }
  }

  function keep(hash: string, path: string) {
    const r = setKeep(groups, keeps, selected, hash, path);
    keeps = r.keeps;
    selected = r.selected;
  }

  function review(m: "trash" | "clone") {
    mode = m;
    reviewOpen = true;
  }

  const STAGES = ["walk", "partial_hash", "full_hash"];
</script>

<div class="wmb-dup">
  <div class="card controls">
    <CleanRoots bind:roots disabled={phase === "scanning"} placeholder={$t("washmybelly.clean.dupes.no_roots") as string} />
    <div class="row">
      <span class="label">{$t("washmybelly.clean.dupes.min_size")}</span>
      <div class="segmented" role="radiogroup" aria-label={$t("washmybelly.clean.dupes.min_size") as string}>
        {#each SIZES as s (s.kb)}
          <button type="button" class="segmented-btn" role="radio" aria-checked={minKb === s.kb} disabled={phase === "scanning"} onclick={() => (minKb = s.kb)}>{$t(s.key)}</button>
        {/each}
      </div>
      <span class="spacer"></span>
      <button type="button" class="btn btn-primary btn-sm" disabled={phase === "scanning" || !roots.length} onclick={run}>
        {scan ? $t("washmybelly.clean.scan_again") : $t("washmybelly.clean.dupes.scan")}
      </button>
    </div>
  </div>

  {#if phase === "idle"}
    <EmptyState art="searching" compact title={$t("washmybelly.clean.dupes.idle_title") as string} body={$t("washmybelly.clean.dupes.idle_body") as string} />
  {:else if phase === "scanning"}
    <div class="card">
      <ScanProgress
        title={$t("washmybelly.clean.dupes.scanning") as string}
        stages={STAGES.map((s) => $t(`washmybelly.clean.dupes.stage_${s}`) as string)}
        current={Math.max(0, STAGES.indexOf(progress?.stage ?? "walk"))}
        done={progress?.done ?? 0}
        total={progress?.total ?? 0}
        message={progress?.message ? tildify(progress.message) : ""}
      />
    </div>
  {:else if phase === "error"}
    <div class="notice notice-danger" role="alert">
      <Icon name="error" size={18} />
      <div class="notice-body">
        <span class="notice-title">{$t("washmybelly.clean.scan_failed")}</span>
        <span class="notice-text">{error}</span>
      </div>
    </div>
  {:else if receipt}
    <EmptyState
      art="happy"
      title={(receipt.bytes_freed > 0
        ? $t("washmybelly.clean.junk.done_title", { size: formatBytes(receipt.bytes_freed) })
        : $t("washmybelly.clean.dupes.done_title", { count: receipt.done })) as string}
      body={(mode === "clone" ? $t("washmybelly.clean.dupes.done_clone_body") : $t("washmybelly.clean.dupes.done_trash_body")) as string}
    />
  {:else if scan && groups.length === 0}
    <EmptyState art="sleeping" title={$t("washmybelly.clean.dupes.none_title") as string} body={$t("washmybelly.clean.dupes.none_body", { count: scan.scanned_files }) as string} />
  {:else if scan}
    <div class="head">
      <span class="big">{formatBytes(scan.total_bytes)}</span>
      <span class="dim">{$t("washmybelly.clean.dupes.found", { count: scan.groups_total, files: scan.scanned_files.toLocaleString() })}</span>
    </div>
    {#if scan.skipped_roots.length}
      <p class="hint">{$t("washmybelly.clean.skipped_roots", { folders: scan.skipped_roots.map((r) => tildify(r)).join(", ") })}</p>
    {/if}

    <div class="groups">
      {#each visible as g (g.hash)}
        <DuplicatesGroup
          group={g}
          keep={keeps.get(g.hash) ?? ""}
          {selected}
          onKeep={(p) => keep(g.hash, p)}
          onToggle={(p) => (selected = toggleCopy(selected, p))}
        />
      {/each}
    </div>
    {#if groups.length > limit}
      <button type="button" class="btn btn-secondary btn-sm more" onclick={() => (limit += PAGE)}>
        {$t("washmybelly.clean.dupes.show_more", { count: groups.length - limit })}
      </button>
    {/if}

    {#if trashPlan.heldBack > 0}
      <p class="hint"><Icon name="info" size={13} /> {$t("washmybelly.clean.dupes.held_back", { count: trashPlan.heldBack })}</p>
    {/if}

    <div class="actions">
      <span class="dim">{$t("washmybelly.ui.selected_summary", { count, size: formatBytes(bytes) })}</span>
      {#if canClone}
        <button
          type="button"
          class="btn btn-secondary"
          disabled={!cloneIds.length}
          title={unclonable ? ($t("washmybelly.clean.dupes.clone_skip", { count: unclonable }) as string) : undefined}
          onclick={() => review("clone")}
        >
          {$t("washmybelly.clean.dupes.clone")}
        </button>
      {/if}
      <button type="button" class="btn btn-primary" disabled={!trashPlan.ids.length} onclick={() => review("trash")}>
        {$t("washmybelly.clean.dupes.to_trash", { count: trashPlan.ids.length })}
      </button>
    </div>
    {#if canClone}
      <p class="hint clone-hint">{$t("washmybelly.clean.dupes.clone_hint")}</p>
    {/if}
  {/if}
</div>

<PlanReview
  kind="duplicates"
  itemIds={planIds}
  bind:open={reviewOpen}
  title={mode === "clone" ? ($t("washmybelly.clean.dupes.clone_title") as string) : undefined}
  confirmLabel={(mode === "clone" ? $t("washmybelly.clean.dupes.clone_confirm") : $t("washmybelly.clean.dupes.trash_confirm")) as string}
  onDone={(r) => (receipt = r)}
  onRestored={() => {
    receipt = null;
    run();
  }}
/>

<style>
  .wmb-dup {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .card {
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    padding: var(--space-3) var(--space-4);
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .label,
  .dim {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .spacer {
    flex: 1;
  }
  .head {
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
  .groups {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .more {
    align-self: center;
  }
  .hint {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .clone-hint {
    justify-content: flex-end;
    margin-top: calc(-1 * var(--space-3));
    text-align: right;
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    position: sticky;
    bottom: 0;
    z-index: 2;
    padding: var(--space-3) 0;
    background: var(--pane-bg);
    border-top: 1px solid var(--separator, var(--button-elevated));
  }
  .actions .dim {
    margin-right: auto;
  }
</style>
