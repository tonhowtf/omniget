<script lang="ts">
  import { t } from "$lib/i18n";
  import { contextRatio, guardianBand, type GuardianBand } from "$lib/stores/llm-telemetry-store.svelte";

  interface Props {
    used: number;
    window: number;
    estimated?: boolean;
    compact?: boolean;
  }

  let { used, window: ctxWindow, estimated = false, compact = false }: Props = $props();

  let ratio = $derived(contextRatio(used, ctxWindow));
  let band = $derived<GuardianBand>(guardianBand(ratio));
  let pct = $derived(Math.round(ratio * 100));

  const GLYPH: Record<GuardianBand, string> = { ok: "●", warn: "▲", critical: "■" };

  function k(n: number): string {
    return n >= 1000 ? `${Math.round(n / 1000)}k` : String(Math.round(n));
  }
</script>

{#if ctxWindow > 0}
  <div class="guardian band-{band}" class:compact>
    <div class="head">
      <span class="label">{$t("llm.observatory.guardian")}</span>
      <span class="glyph" aria-hidden="true">{GLYPH[band]}</span>
      <span class="value">{k(used)} / {k(ctxWindow)}</span>
    </div>
    <div
      class="track"
      role="meter"
      aria-valuenow={pct}
      aria-valuemin="0"
      aria-valuemax="100"
      aria-label="{$t('llm.observatory.guardian')} — {$t(`llm.observatory.guardian_${band}`)}"
    >
      <div class="fill" style:width="{pct}%"></div>
    </div>
    {#if !compact}
      <div class="foot">
        <span>{$t(`llm.observatory.guardian_${band}`)}</span>
        <span class="src">{estimated ? $t("llm.observatory.source_estimated") : $t("llm.observatory.source_reported")}</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .guardian {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    --meter: var(--accent);
  }
  .guardian.band-warn { --meter: var(--warning); }
  .guardian.band-critical { --meter: var(--error); }
  .head {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .label { color: var(--text); }
  .glyph {
    margin-left: auto;
    color: var(--meter);
    font-size: 9px;
    line-height: 1;
  }
  .value {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .track {
    height: 6px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--meter) 18%, var(--surface-hi));
    overflow: hidden;
  }
  .guardian.compact .track { height: 4px; }
  .fill {
    height: 100%;
    background: var(--meter);
    border-radius: var(--radius-full);
  }
  .foot {
    display: flex;
    gap: var(--space-3);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .foot .src { margin-left: auto; }
</style>
