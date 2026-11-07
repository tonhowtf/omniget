<script lang="ts">
  import { t } from "$lib/i18n";
  import { everyN, parseRanges, plural, rangeLabel, type Range } from "$lib/pdf/pages";

  let {
    total,
    busy = false,
    onsplit,
    oncancel,
  }: {
    total: number;
    busy?: boolean;
    onsplit: (ranges: Range[]) => void;
    oncancel: () => void;
  } = $props();

  let mode = $state<"every" | "ranges">("every");
  let every = $state(1);
  let text = $state("");

  let result = $derived.by((): { ranges: Range[]; error: string | null } => {
    if (mode === "every") {
      const n = Number(every);
      if (!Number.isFinite(n) || n < 1) return { ranges: [], error: $t("pdf.split_error_every") as string };
      return { ranges: everyN(total, n), error: null };
    }
    const parsed = parseRanges(text, total);
    if (parsed.ok) return { ranges: parsed.ranges, error: null };
    if (parsed.error === "empty") return { ranges: [], error: null };
    return { ranges: [], error: $t(`pdf.split_error_${parsed.error}`, { max: total }) as string };
  });

  let preview = $derived.by(() => {
    const shown = result.ranges.slice(0, 6).map(rangeLabel).join(", ");
    return result.ranges.length > 6 ? `${shown}, …` : shown;
  });
</script>

<div class="sp" role="group" aria-labelledby="sp-title">
  <h2 id="sp-title" class="sp-title">{$t("pdf.split_title")}</h2>
  <div class="sp-modes">
    <label class="sp-mode">
      <input type="radio" name="sp-mode" value="every" bind:group={mode} disabled={busy} />
      <span>{$t("pdf.split_every_prefix")}</span>
      <input class="input sp-num" type="number" min="1" max={total} bind:value={every} disabled={busy || mode !== "every"} aria-label={$t("pdf.split_every_label") as string} />
      <span>{$t("pdf.split_every_suffix")}</span>
    </label>
    <label class="sp-mode">
      <input type="radio" name="sp-mode" value="ranges" bind:group={mode} disabled={busy} />
      <span>{$t("pdf.split_ranges")}</span>
      <input
        class="input sp-text"
        type="text"
        placeholder={$t("pdf.split_ranges_placeholder") as string}
        bind:value={text}
        disabled={busy || mode !== "ranges"}
        aria-label={$t("pdf.split_ranges") as string}
        aria-invalid={result.error ? "true" : undefined}
      />
    </label>
  </div>
  {#if result.error}
    <p class="sp-error" role="alert">{result.error}</p>
  {:else if result.ranges.length > 0}
    <p class="sp-hint">{plural($t, "pdf.split_preview", result.ranges.length, { ranges: preview })}</p>
  {:else}
    <p class="sp-hint">{$t("pdf.split_ranges_help")}</p>
  {/if}
  <div class="sp-bar">
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={oncancel}>{$t("pdf.cancel")}</button>
    <button type="button" class="btn btn-primary" disabled={busy || result.ranges.length < 1 || !!result.error} onclick={() => onsplit(result.ranges)}>
      {plural($t, "pdf.split_run", result.ranges.length)}
    </button>
  </div>
</div>

<style>
  .sp {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .sp-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
  }

  .sp-modes {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .sp-mode {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .sp-num {
    width: 72px;
  }

  .sp-text {
    flex: 1;
    min-width: 180px;
  }

  .sp-hint,
  .sp-error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .sp-error {
    color: var(--danger, #e5484d);
  }

  .sp-bar {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
