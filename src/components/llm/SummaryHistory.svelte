<script lang="ts">
  /**
   * Saved AI results (`ai_history_list`), newest first. Reading is local; the
   * only write is the explicit, confirmed "clear" (`ai_history_clear`).
   */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { clearSummaryHistory, loadSummaryHistory, type SummaryHistoryEntry } from "./summaries";

  let { labelledBy, refreshKey = 0 }: { labelledBy?: string; refreshKey?: number } = $props();

  let history = $state<SummaryHistoryEntry[]>([]);
  let loading = $state(true);
  let failed = $state(false);
  let clearing = $state(false);

  async function load() {
    loading = true;
    failed = false;
    try {
      history = await loadSummaryHistory(invoke);
    } catch {
      failed = true;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void refreshKey;
    void load();
  });

  async function clear() {
    if (clearing || !confirm($t("settings.ai.history_clear_confirm") as string)) return;
    clearing = true;
    try {
      await clearSummaryHistory(invoke);
      history = [];
    } catch {
      showToast("error", $t("settings.ai.history_clear_failed"));
    } finally {
      clearing = false;
    }
  }
</script>

<div class="summary-history" aria-labelledby={labelledBy} data-agent-readiness={loading ? "loading" : failed ? "error" : "ready"}>
  {#if history.length > 0 && !failed}
    <div class="actions">
      <button type="button" class="button danger" disabled={clearing || loading} onclick={clear}>{$t("settings.ai.history_clear")}</button>
    </div>
  {/if}
  {#if loading}
    <p class="hint" role="status">{$t("common.loading")}</p>
  {:else if failed}
    <div class="notice" role="alert">
      <p>{$t("settings.ai.history_load_failed")}</p>
      <button type="button" class="button" onclick={load}>{$t("error.remedy.retry")}</button>
    </div>
  {:else if history.length === 0}
    <p class="hint"><bdi>{$t("settings.ai.history_empty")}</bdi></p>
  {:else}
    <ul class="entries">
      {#each history as entry (entry.id)}
        <li>
          <details>
            <summary>{entry.title || entry.url}</summary>
            <p class="body" dir="auto">{entry.content}</p>
          </details>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .summary-history { display: grid; gap: 8px; min-width: 0; }
  .actions { display: flex; justify-content: flex-end; }
  .hint { margin: 0; color: var(--text-muted); font-size: 13px; }
  .notice { margin: 0; font-size: 13px; }
  .entries { list-style: none; margin: 0; padding: 0; }
  .entries li + li { border-top: 1px solid var(--separator); }
  summary { cursor: pointer; font-weight: 600; padding: 8px 0; overflow-wrap: anywhere; }
  summary:focus-visible { outline: var(--focus-ring); }
  .body { white-space: pre-wrap; margin: 4px 0 8px; font-size: 13px; line-height: 1.5; overflow-wrap: anywhere; }
  .danger { color: var(--error); }
</style>
