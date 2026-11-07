<script lang="ts">
  import { t, locale } from "$lib/i18n";
  import { formatElapsed, languageLabel } from "$lib/dictation/logic";
  import type { Entry } from "$lib/dictation/types";
  import Icon from "./Icon.svelte";

  let {
    entries,
    loading = false,
    saveHistory,
    oncopy,
    ondelete,
    onclear,
  }: {
    entries: Entry[];
    loading?: boolean;
    saveHistory: boolean;
    oncopy: (entry: Entry) => Promise<boolean>;
    ondelete: (entry: Entry) => void;
    onclear: () => void;
  } = $props();

  let confirming = $state(false);
  let copiedId = $state<string | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;

  function when(ms: number): string {
    const d = new Date(ms);
    const today = new Date();
    const sameDay = d.toDateString() === today.toDateString();
    const loc = $locale ?? undefined;
    return sameDay
      ? d.toLocaleTimeString(loc, { hour: "2-digit", minute: "2-digit" })
      : d.toLocaleString(loc, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  }

  async function copy(entry: Entry) {
    if (copiedTimer) clearTimeout(copiedTimer);
    copiedId = null;
    if (!(await oncopy(entry))) return;
    copiedId = entry.id;
    copiedTimer = setTimeout(() => (copiedId = null), 1600);
  }

  $effect(() => () => {
    if (copiedTimer) clearTimeout(copiedTimer);
  });

  function clearAll() {
    if (!confirming) {
      confirming = true;
      return;
    }
    confirming = false;
    onclear();
  }
</script>

<section class="dc-hist" aria-labelledby="dc-hist-title">
  <header class="dc-hist-head">
    <h2 id="dc-hist-title">{$t("dictation.history")}</h2>
    {#if entries.length > 0}
      <div class="dc-hist-tools">
        {#if confirming}
          <span class="dc-hist-confirm">{$t("dictation.clear_confirm", { count: entries.length })}</span>
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirming = false)}>{$t("dictation.keep")}</button>
        {/if}
        <button type="button" class="btn btn-sm" class:btn-destructive={confirming} class:btn-ghost={!confirming} onclick={clearAll}>
          {confirming ? $t("dictation.clear_yes") : $t("dictation.clear_all")}
        </button>
      </div>
    {/if}
  </header>

  {#if loading}
    <div class="dc-hist-empty"><span class="spinner" aria-hidden="true"></span></div>
  {:else if entries.length === 0}
    <div class="dc-hist-empty">
      <Icon name="clipboard" size={22} />
      <p class="dc-hist-empty-title">{$t("dictation.history_empty")}</p>
      <p>{saveHistory ? $t("dictation.history_empty_body") : $t("dictation.history_off")}</p>
    </div>
  {:else}
    <ul class="dc-hist-list">
      {#each entries as entry (entry.id)}
        <li class="dc-hist-item">
          <p class="dc-hist-text">{entry.text}</p>
          <div class="dc-hist-meta">
            <span>{when(entry.created_at)}</span>
            <span aria-hidden="true">·</span>
            <span>{formatElapsed(entry.duration_ms)}</span>
            {#if entry.language}
              <span aria-hidden="true">·</span>
              <span>{languageLabel(entry.language, $locale)}</span>
            {/if}
            <span aria-hidden="true">·</span>
            <span>{$t(`dictation.hist_${entry.outcome}`)}</span>
            <span class="dc-hist-actions">
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => copy(entry)} aria-label={$t("dictation.copy_entry") as string}>
                <Icon name={copiedId === entry.id ? "done" : "copy"} size={13} />
                <span>{copiedId === entry.id ? $t("dictation.copied") : $t("dictation.copy")}</span>
              </button>
              <button type="button" class="btn btn-ghost btn-sm btn-icon" onclick={() => ondelete(entry)} aria-label={$t("dictation.delete_entry") as string} title={$t("dictation.delete_entry") as string}>
                <Icon name="trash" size={13} />
              </button>
            </span>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .dc-hist {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .dc-hist-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: var(--control-h);
  }

  .dc-hist-head h2 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .dc-hist-tools {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dc-hist-confirm {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .dc-hist-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-dim);
    text-align: center;
  }

  .dc-hist-empty p {
    margin: 0;
    max-width: 360px;
    font-size: var(--text-sm);
  }

  .dc-hist-empty .dc-hist-empty-title {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }

  .dc-hist-list {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .dc-hist-item {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
  }

  .dc-hist-item + .dc-hist-item {
    border-top: var(--hairline) solid var(--content-border);
  }

  .dc-hist-text {
    margin: 0;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .dc-hist-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .dc-hist-actions {
    display: inline-flex;
    gap: var(--space-1);
    margin-left: auto;
  }

  .dc-hist-actions .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
</style>
