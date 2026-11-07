<script lang="ts">
  import { t } from "$lib/i18n";
  import { fileName } from "$lib/capture/record";
  import { formatBytes, formatTimecode } from "$lib/capture/timecode";
  import type { EntryKind, HistoryEntry } from "$lib/capture/types";

  let {
    entries,
    loading = false,
    error = null,
    onopen,
    onreveal,
    onremove,
    onretry,
  }: {
    entries: HistoryEntry[];
    loading?: boolean;
    error?: string | null;
    onopen: (entry: HistoryEntry) => void;
    onreveal: (entry: HistoryEntry) => void;
    onremove: (entry: HistoryEntry) => void;
    onretry: () => void;
  } = $props();

  type Filter = "all" | EntryKind;
  let filter = $state<Filter>("all");
  const FILTERS: Filter[] = ["all", "recording", "export", "snapshot"];
  let shown = $derived(filter === "all" ? entries : entries.filter((e) => e.kind === filter));

  function when(ms: number): string {
    try {
      return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
    } catch {
      return "";
    }
  }
</script>

<section class="hist" aria-labelledby="cap-hist-title">
  <div class="hist-head">
    <h2 id="cap-hist-title">{$t("capture.history")}</h2>
    {#if entries.length}
      <div class="mac-segmented" role="radiogroup" aria-label={$t("capture.history_filter")}>
        {#each FILTERS as f (f)}
          <button type="button" role="radio" aria-checked={filter === f} class="mac-segmented-btn" class:active={filter === f} onclick={() => (filter = f)}>{$t(`capture.filter_${f}`)}</button>
        {/each}
      </div>
    {/if}
  </div>

  {#if loading && entries.length === 0}
    <p class="hist-empty"><span class="spinner hist-spin" aria-hidden="true"></span>{$t("capture.history_loading")}</p>
  {:else if error}
    <p class="hist-empty">
      {$t("capture.history_error")}
      <button type="button" class="btn btn-ghost btn-sm" onclick={onretry}>{$t("capture.try_again")}</button>
    </p>
  {:else if entries.length === 0}
    <p class="hist-empty">{$t("capture.history_empty")}</p>
  {:else if shown.length === 0}
    <p class="hist-empty">{$t("capture.history_filter_empty")}</p>
  {:else}
    <ul class="hist-list">
      {#each shown as entry (entry.id)}
        <li class="hist-row" class:missing={!entry.exists}>
          <span class="hist-kind hist-kind-{entry.kind}">{$t(`capture.kind_${entry.kind}`)}</span>
          <span class="hist-copy">
            <span class="hist-name" title={entry.path}>{fileName(entry.path)}</span>
            <span class="hist-meta">
              {when(entry.created_at)}{entry.duration ? ` · ${formatTimecode(entry.duration, false)}` : ""}{entry.size_bytes ? ` · ${formatBytes(entry.size_bytes)}` : ""}{entry.exists ? "" : ` · ${$t("capture.file_missing")}`}
            </span>
          </span>
          <span class="hist-actions">
            {#if entry.kind !== "snapshot"}
              <button type="button" class="btn btn-ghost btn-sm" disabled={!entry.exists} title={entry.exists ? undefined : $t("capture.file_missing_hint")} onclick={() => onopen(entry)}>{$t("capture.open_in_cutter_short")}</button>
            {/if}
            <button type="button" class="btn btn-ghost btn-sm" disabled={!entry.exists} title={entry.exists ? undefined : $t("capture.file_missing_hint")} onclick={() => onreveal(entry)}>{$t("capture.reveal")}</button>
            <button type="button" class="btn btn-ghost btn-sm" aria-label={$t("capture.remove_from_list_named", { name: fileName(entry.path) })} title={$t("capture.remove_from_list_hint")} onclick={() => onremove(entry)}>{$t("capture.remove_from_list")}</button>
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .hist {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .hist-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .hist-head h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .hist-empty {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .hist-spin {
    width: 12px;
    height: 12px;
  }

  .hist-list {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .hist-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--row-base);
    padding: var(--space-2) var(--space-3);
  }

  .hist-row + .hist-row {
    border-top: var(--hairline) solid var(--separator);
  }

  .hist-kind {
    flex: none;
    min-width: 72px;
    padding: 2px var(--space-2);
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    text-align: center;
    color: var(--text-muted);
    background: var(--surface-mut);
  }

  .hist-kind-recording {
    color: var(--text);
    background: color-mix(in srgb, var(--error) 18%, transparent);
  }

  .hist-kind-export {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }

  .hist-copy {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .hist-name {
    font-size: var(--text-sm);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hist-row.missing .hist-name {
    color: var(--text-dim);
    text-decoration: line-through;
  }

  .hist-meta {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .hist-actions {
    display: inline-flex;
    flex: none;
    gap: 2px;
  }
</style>
