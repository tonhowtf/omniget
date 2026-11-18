<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatTimecode, parseTimecode } from "$lib/capture/timecode";
  import { length, type Segment } from "$lib/capture/segments";
  import type { Span } from "$lib/capture/types";

  let {
    segments,
    selectedId,
    disabled = false,
    onselect,
    onedit,
    onmove,
    onremove,
  }: {
    segments: Segment[];
    selectedId: string | null;
    disabled?: boolean;
    onselect: (seg: Segment) => void;
    onedit: (id: string, patch: Partial<Span>) => void;
    onmove: (from: number, to: number) => void;
    onremove: (id: string) => void;
  } = $props();

  let invalid = $state<string | null>(null);

  function commit(seg: Segment, field: "start" | "end", input: HTMLInputElement) {
    const value = parseTimecode(input.value);
    const other = field === "start" ? seg.end : seg.start;
    const ok = value !== null && (field === "start" ? value < other : value > other);
    if (!ok) {
      invalid = `${seg.id}:${field}`;
      input.value = formatTimecode(seg[field]);
      return;
    }
    invalid = null;
    onedit(seg.id, { [field]: value });
    const applied = segments.find((s) => s.id === seg.id);
    if (applied) input.value = formatTimecode(applied[field]);
  }

  function key(event: KeyboardEvent, seg: Segment, field: "start" | "end") {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === "Enter") {
      event.preventDefault();
      commit(seg, field, input);
      input.blur();
    } else if (event.key === "Escape") {
      input.value = formatTimecode(seg[field]);
      input.blur();
    }
  }
</script>

<ol class="sl" aria-label={$t("capture.segments")}>
  {#each segments as seg, i (seg.id)}
    <li class="sl-row" class:selected={seg.id === selectedId}>
      <button type="button" class="sl-n" aria-label={$t("capture.go_to_segment", { num: i + 1 })} onclick={() => onselect(seg)}>{i + 1}</button>
      {#each ["start", "end"] as const as field (field)}
        <label class="sl-time">
          <span class="sl-time-label">{$t(field === "start" ? "capture.seg_start" : "capture.seg_end")}</span>
          <input
            class="input sl-input"
            class:error={invalid === `${seg.id}:${field}`}
            type="text"
            spellcheck="false"
            value={formatTimecode(seg[field])}
            {disabled}
            aria-invalid={invalid === `${seg.id}:${field}`}
            onblur={(e) => commit(seg, field, e.currentTarget)}
            onkeydown={(e) => key(e, seg, field)}
          />
        </label>
      {/each}
      <span class="sl-len" title={$t("capture.seg_length")}>{formatTimecode(length(seg))}</span>
      <span class="sl-actions">
        <button type="button" class="btn btn-ghost btn-sm btn-icon" disabled={disabled || i === 0} aria-label={$t("capture.move_up", { num: i + 1 })} onclick={() => onmove(i, i - 1)}>
          <svg viewBox="0 0 12 12" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 7.5 6 4.5l3 3" /></svg>
        </button>
        <button type="button" class="btn btn-ghost btn-sm btn-icon" disabled={disabled || i === segments.length - 1} aria-label={$t("capture.move_down", { num: i + 1 })} onclick={() => onmove(i, i + 1)}>
          <svg viewBox="0 0 12 12" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 4.5 6 7.5l3-3" /></svg>
        </button>
        <button type="button" class="btn btn-ghost btn-sm btn-icon" {disabled} aria-label={$t("capture.remove_segment", { num: i + 1 })} onclick={() => onremove(seg.id)}>
          <svg viewBox="0 0 12 12" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="m3.5 3.5 5 5m0-5-5 5" /></svg>
        </button>
      </span>
    </li>
  {/each}
</ol>

<style>
  .sl {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .sl-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    min-height: var(--row-base);
    padding: var(--space-2) var(--space-3);
  }

  .sl-row + .sl-row {
    border-top: var(--hairline) solid var(--separator);
  }

  .sl-row.selected {
    background: var(--accent-soft);
  }

  .sl-n {
    width: 24px;
    height: 24px;
    border: none;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--accent) 28%, transparent);
    color: var(--text);
    font-size: var(--text-xs);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }

  .sl-n:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .sl-time {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .sl-time-label {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .sl-input {
    width: 104px;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  .sl-len {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .sl-actions {
    display: inline-flex;
    gap: 2px;
    margin-inline-start: auto;
  }
</style>
