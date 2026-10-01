<script lang="ts">
  import { t } from "$lib/i18n";
  import { guardianBand, type GuardianBand } from "$lib/stores/llm-telemetry-store.svelte";

  interface Props {
    label: string;
    used: number;
    resetsAt?: number;
    projection?: string;
    active?: boolean;
    compact?: boolean;
  }

  let { label, used, resetsAt, projection, active = false, compact = false }: Props = $props();

  let ratio = $derived(Number.isFinite(used) ? Math.min(1, Math.max(0, used)) : 0);
  let band = $derived<GuardianBand>(guardianBand(ratio));
  let pct = $derived(Math.round(ratio * 100));

  const GLYPH: Record<GuardianBand, string> = { ok: "●", warn: "▲", critical: "■" };

  let resetIn = $derived.by(() => {
    if (!resetsAt) return "";
    const ms = resetsAt - Date.now();
    if (ms <= 0) return $t("llm.quota.resetting") as string;
    const mins = Math.round(ms / 60_000);
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return h > 0 ? `${h} h ${m} min` : `${m} min`;
  });
</script>

<div class="quota band-{band}" class:active class:compact>
  <div class="head">
    <span class="name" title={label}>{label}</span>
    {#if active}<span class="tag">{$t("llm.quota.active")}</span>{/if}
    <span class="glyph" aria-hidden="true">{GLYPH[band]}</span>
    <span class="pct">{pct}%</span>
  </div>
  <div
    class="track"
    role="meter"
    aria-valuenow={pct}
    aria-valuemin="0"
    aria-valuemax="100"
    aria-label="{label} — {$t(`llm.quota.band_${band}`)}"
  >
    <div class="fill" style:width="{pct}%"></div>
  </div>
  {#if !compact && (resetIn || projection)}
    <div class="foot">
      {#if resetIn}<span>{$t("llm.quota.resets_in")} {resetIn}</span>{/if}
      {#if projection}<span class="proj">{projection}</span>{/if}
    </div>
  {/if}
</div>

<style>
  .quota {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    --meter: var(--accent);
  }
  .quota.band-warn { --meter: var(--warning); }
  .quota.band-critical { --meter: var(--error); }
  .head {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .name {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    font-size: var(--text-caption);
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    padding: 0 6px;
  }
  .glyph {
    margin-left: auto;
    color: var(--meter);
    font-size: 9px;
    line-height: 1;
  }
  .pct {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .track {
    height: 6px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--meter) 18%, var(--surface-hi));
    overflow: hidden;
  }
  .quota.compact .track { height: 4px; }
  .fill {
    height: 100%;
    background: var(--meter);
    border-radius: var(--radius-full);
    transition: width 200ms ease;
  }
  .foot {
    display: flex;
    gap: var(--space-3);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .foot .proj {
    margin-left: auto;
  }
  .quota.active .name {
    font-weight: 600;
  }
  @media (prefers-reduced-motion: reduce) {
    .fill { transition: none; }
  }
</style>
