<script lang="ts">
  import { onMount } from "svelte";
  import { desktopDir, documentDir, downloadDir } from "@tauri-apps/api/path";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, shortenPath, tildify } from "$lib/washmybelly/format";
  import { baseName, emptyRows, pruneSelection, type EmptyScan } from "$lib/washmybelly/clean";
  import EmptyState from "./ui/EmptyState.svelte";
  import SelectList from "./ui/SelectList.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import Icon from "./ui/Icon.svelte";
  import CleanRoots from "./CleanRoots.svelte";

  let roots = $state<string[]>([]);
  let phase = $state<"idle" | "scanning" | "done" | "error">("idle");
  let scan = $state<EmptyScan | null>(null);
  let selected = $state(new Set<string>());
  let error = $state<string | null>(null);
  let reviewOpen = $state(false);
  let receipt = $state<ReceiptSummary | null>(null);
  let showLinks = $state(false);

  onMount(async () => {
    const found = await Promise.allSettled([downloadDir(), documentDir(), desktopDir()]);
    if (roots.length) return;
    roots = found.flatMap((r) => (r.status === "fulfilled" && r.value ? [r.value.replace(/[\\/]$/, "")] : []));
  });

  let rows = $derived(scan ? emptyRows(scan.items) : []);
  let chosenIds = $derived([...selected]);

  async function run() {
    if (phase === "scanning" || !roots.length) return;
    phase = "scanning";
    error = null;
    receipt = null;
    try {
      const r = await api.call<EmptyScan>("wmb_find_empty_folders", { roots });
      scan = r;
      selected = new Set(r.items.map((i) => i.id));
      phase = "done";
    } catch (e) {
      error = errorText(e);
      phase = "error";
    }
  }

  function groupLabel(key: string): string {
    return tildify(key);
  }

  $effect(() => {
    if (!scan) return;
    const next = pruneSelection(selected, rows);
    if (next.size !== selected.size) selected = next;
  });
</script>

<div class="wmb-empty">
  <div class="card controls">
    <CleanRoots bind:roots disabled={phase === "scanning"} placeholder={$t("washmybelly.clean.dupes.no_roots") as string} />
    <div class="row">
      <span class="spacer"></span>
      <button type="button" class="btn btn-primary btn-sm" disabled={phase === "scanning" || !roots.length} onclick={run}>
        {scan ? $t("washmybelly.clean.scan_again") : $t("washmybelly.clean.empty.scan")}
      </button>
    </div>
  </div>

  {#if phase === "idle"}
    <EmptyState art="searching" compact title={$t("washmybelly.clean.empty.idle_title") as string} body={$t("washmybelly.clean.empty.idle_body") as string} />
  {:else if phase === "scanning"}
    <div class="card scanning" role="status" aria-live="polite">
      <span class="spinner" aria-hidden="true"></span>
      {$t("washmybelly.clean.empty.scanning")}
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
    <EmptyState art="happy" title={$t("washmybelly.clean.empty.done_title", { count: receipt.done }) as string} body={$t("washmybelly.clean.dupes.done_trash_body") as string} />
  {:else if scan}
    {#if scan.items.length === 0}
      <EmptyState art="sleeping" title={$t("washmybelly.clean.empty.none_title") as string} />
    {:else}
      <div class="head">
        <span class="big">{scan.items.length.toLocaleString()}</span>
        <span class="dim">{$t("washmybelly.clean.empty.found", { count: scan.items.length })}</span>
      </div>
      <SelectList items={rows} bind:selected {groupLabel} showRisk={false} />
      <div class="actions">
        <button type="button" class="btn btn-primary" disabled={!selected.size} onclick={() => (reviewOpen = true)}>
          {$t("washmybelly.clean.dupes.to_trash", { count: selected.size })}
        </button>
      </div>
    {/if}

    {#if scan.broken_links.length}
      <section class="links">
        <button type="button" class="disclosure" aria-expanded={showLinks} onclick={() => (showLinks = !showLinks)}>
          <span class="chev" class:open={showLinks}><Icon name="chevron" size={12} /></span>
          {$t("washmybelly.clean.empty.broken_links", { count: scan.broken_links.length })}
        </button>
        <p class="hint">{$t("washmybelly.clean.empty.broken_hint")}</p>
        {#if showLinks}
          <ul class="group">
            {#each scan.broken_links as l (l.path)}
              <li class="group-row">
                <div class="group-row-content">
                  <span class="group-row-title" title={l.path}>{baseName(l.path)}</span>
                  <span class="group-row-sub" title={l.target}>{shortenPath(l.path, 60)} → {shortenPath(l.target, 48)}</span>
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}
  {/if}
</div>

<PlanReview
  kind="empty_folders"
  itemIds={chosenIds}
  bind:open={reviewOpen}
  confirmLabel={$t("washmybelly.clean.dupes.trash_confirm") as string}
  onDone={(r) => (receipt = r)}
  onRestored={() => {
    receipt = null;
    run();
  }}
/>

<style>
  .wmb-empty {
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
  }
  .spacer {
    flex: 1;
  }
  .scanning {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  .big {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }
  .dim,
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
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
  .links {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .links ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .disclosure {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
  }
  .disclosure:focus-visible {
    outline: var(--focus-ring);
    border-radius: 4px;
  }
  .chev {
    display: inline-flex;
    transition: transform var(--duration-fast, 0.15s) var(--ease-out, ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .group-row-title,
  .group-row-sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
