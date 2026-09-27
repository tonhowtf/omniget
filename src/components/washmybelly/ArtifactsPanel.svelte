<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { Progress, ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, formatBytes, tildify } from "$lib/washmybelly/format";
  import { artifactRows, baseName, pruneSelection, selectedBytes, type ArtifactScan } from "$lib/washmybelly/clean";
  import EmptyState from "./ui/EmptyState.svelte";
  import ScanProgress from "./ui/ScanProgress.svelte";
  import SelectList from "./ui/SelectList.svelte";
  import PlanReview from "./ui/PlanReview.svelte";
  import Icon from "./ui/Icon.svelte";
  import { onProgress } from "./ui/progress";
  import CleanRoots from "./CleanRoots.svelte";

  /// Project artifacts: build output and installed dependencies of code
  /// projects (`target/`, `node_modules`...), grouped by project. The next
  /// build makes them again.
  const AGES = [0, 30, 90, 180] as const;

  let roots = $state<string[]>([]);
  let age = $state<number>(30);
  let phase = $state<"idle" | "scanning" | "done" | "error">("idle");
  let scan = $state<ArtifactScan | null>(null);
  let selected = $state(new Set<string>());
  let progress = $state<Progress | null>(null);
  let error = $state<string | null>(null);
  let reviewOpen = $state(false);
  let receipt = $state<ReceiptSummary | null>(null);
  let stop: (() => void) | null = null;
  onDestroy(() => stop?.());

  let rows = $derived(scan ? artifactRows(scan) : []);
  let chosenBytes = $derived(scan ? selectedBytes(scan.items, selected) : 0);
  let chosenIds = $derived([...selected]);

  async function run() {
    if (phase === "scanning") return;
    phase = "scanning";
    error = null;
    receipt = null;
    progress = null;
    stop?.();
    stop = await onProgress("dev_artifacts_scan", (p) => (progress = p)).catch(() => null);
    try {
      const r = await api.call<ArtifactScan>("wmb_dev_artifacts_scan", {
        ...(roots.length ? { roots } : {}),
        older_than_days: age,
      });
      scan = r;
      selected = new Set(r.items.map((i) => i.id));
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
    const p = scan?.projects.find((x) => x.path === key);
    if (!p) return key ? baseName(key) : ($t("washmybelly.ui.other_group") as string);
    const kinds = p.kinds.join(" + ");
    const when = p.age_days > 0 ? ($t("washmybelly.clean.artifacts.age", { days: p.age_days }) as string) : ($t("washmybelly.clean.artifacts.recent") as string);
    return `${baseName(p.path)} · ${kinds} · ${when}`;
  }

  function ageLabel(d: number): string {
    return d === 0 ? ($t("washmybelly.clean.artifacts.any_age") as string) : ($t("washmybelly.clean.artifacts.days", { days: d }) as string);
  }

  $effect(() => {
    if (!scan) return;
    const next = pruneSelection(selected, rows);
    if (next.size !== selected.size) selected = next;
  });
</script>

<div class="wmb-art">
  <div class="card controls">
    <CleanRoots bind:roots placeholder={$t("washmybelly.clean.artifacts.default_roots") as string} disabled={phase === "scanning"} />
    <div class="row">
      <span class="label">{$t("washmybelly.clean.artifacts.untouched")}</span>
      <div class="segmented" role="radiogroup" aria-label={$t("washmybelly.clean.artifacts.untouched") as string}>
        {#each AGES as d (d)}
          <button type="button" class="segmented-btn" role="radio" aria-checked={age === d} disabled={phase === "scanning"} onclick={() => (age = d)}>{ageLabel(d)}</button>
        {/each}
      </div>
      <span class="spacer"></span>
      <button type="button" class="btn btn-primary btn-sm" disabled={phase === "scanning"} onclick={run}>
        {scan ? $t("washmybelly.clean.scan_again") : $t("washmybelly.clean.artifacts.scan")}
      </button>
    </div>
    <p class="hint"><Icon name="refresh" size={13} /> {$t("washmybelly.clean.artifacts.regenerable")}</p>
  </div>

  {#if phase === "idle"}
    <EmptyState art="searching" compact title={$t("washmybelly.clean.artifacts.idle_title") as string} body={$t("washmybelly.clean.artifacts.idle_body") as string} />
  {:else if phase === "scanning"}
    <div class="card">
      <ScanProgress title={$t("washmybelly.clean.artifacts.scanning") as string} done={progress?.done ?? 0} total={progress?.total ?? 0} message={progress?.message ? tildify(progress.message) : ""} />
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
    <EmptyState art="happy" title={$t("washmybelly.clean.junk.done_title", { size: formatBytes(receipt.bytes_freed) }) as string} body={$t("washmybelly.clean.artifacts.done_body") as string} />
  {:else if scan && scan.items.length === 0}
    <EmptyState art="sleeping" title={$t("washmybelly.clean.artifacts.none_title") as string} body={$t("washmybelly.clean.artifacts.none_body") as string} />
  {:else if scan}
    <div class="head">
      <span class="big">{formatBytes(scan.total_bytes)}</span>
      <span class="dim">{$t("washmybelly.clean.artifacts.found", { count: scan.projects.length })}</span>
    </div>
    {#if scan.skipped_roots.length}
      <p class="hint">{$t("washmybelly.clean.skipped_roots", { folders: scan.skipped_roots.map((r) => tildify(r)).join(", ") })}</p>
    {/if}
    <SelectList items={rows} bind:selected {groupLabel} showRisk={false} />
    <div class="actions">
      <button type="button" class="btn btn-primary" disabled={!selected.size} onclick={() => (reviewOpen = true)}>
        {$t("washmybelly.clean.junk.clean_size", { size: formatBytes(chosenBytes) })}
      </button>
    </div>
  {/if}
</div>

<PlanReview
  kind="dev_artifacts"
  itemIds={chosenIds}
  bind:open={reviewOpen}
  confirmLabel={$t("washmybelly.clean.junk.clean_size", { size: formatBytes(chosenBytes) }) as string}
  onDone={(r) => (receipt = r)}
  onRestored={() => {
    receipt = null;
    run();
  }}
/>

<style>
  .wmb-art {
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
  .hint {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
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
  .actions {
    display: flex;
    justify-content: flex-end;
    position: sticky;
    bottom: 0;
    z-index: 2;
    padding: var(--space-3) 0;
    /* Opaque bar so the button never sits on top of the list's totals. */
    background: var(--pane-bg);
    border-top: 1px solid var(--separator, var(--button-elevated));
  }
</style>
