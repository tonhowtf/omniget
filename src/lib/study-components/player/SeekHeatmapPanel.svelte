<script lang="ts">
  import { studyPlayerSeekHeatmap, type SeekHeatmap } from "$lib/study-bridge";
  import { t } from "$lib/i18n";

  type Props = {
    lessonId: number;
    minSeeks?: number;
  };

  let { lessonId, minSeeks = 10 }: Props = $props();
  let heatmap = $state<SeekHeatmap | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      heatmap = await studyPlayerSeekHeatmap({ lessonId, bucketSecs: 10 });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void lessonId;
    void load();
  });

  const maxBucket = $derived.by(() => {
    if (!heatmap) return 0;
    let m = 0;
    for (const v of heatmap.buckets) if (v > m) m = v;
    return m;
  });

  const showPanel = $derived.by(
    () => heatmap != null && heatmap.total_seeks >= minSeeks && heatmap.duration_ms > 0,
  );
</script>

{#if !loading && !error && showPanel && heatmap}
  <section class="heat" aria-label={$t("study.player.seek_analysis")}>
    <header class="head">
      <span class="eyebrow">Análise</span>
      <h3>Distribuição de retornos</h3>
      <span class="count">{heatmap.total_seeks} seeks</span>
    </header>
    <p class="hint">
      Cada barra representa 10 segundos. Barras altas indicam onde você (ou outros) voltaram mais — provavelmente trecho mais difícil.
    </p>
    <div class="track" role="img" aria-label="Heatmap de seeks">
      <svg viewBox="0 0 {Math.max(heatmap.buckets.length, 1)} 40" preserveAspectRatio="none" width="100%" height="40">
        {#each heatmap.buckets as v, i (i)}
          {@const ratio = maxBucket > 0 ? v / maxBucket : 0}
          {@const h = ratio * 40}
          <rect
            x={i}
            y={40 - h}
            width="0.95"
            height={h}
            fill={ratio > 0 ? "currentColor" : "transparent"}
            opacity={0.2 + ratio * 0.6}
          />
        {/each}
      </svg>
    </div>
  </section>
{/if}

<style>
  .heat {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
    background: color-mix(in oklab, var(--surface) 90%, var(--accent) 4%);
    border: none;
    border-radius: var(--border-radius, 8px);
    margin-top: 16px;
    color: var(--accent);
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    color: inherit;
  }

  .eyebrow {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 600;
    color: var(--accent);
  }

  .head h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--primary, inherit);
    flex: 1 1 auto;
  }

  .count {
    font-size: 12px;
    color: color-mix(in oklab, currentColor 60%, transparent);
    font-variant-numeric: tabular-nums;
  }

  .hint {
    margin: 0;
    font-size: 12px;
    color: color-mix(in oklab, currentColor 60%, transparent);
    line-height: 1.4;
  }

  .track {
    width: 100%;
    border-radius: 4px;
    overflow: hidden;
    background: color-mix(in oklab, currentColor 6%, transparent);
  }

  svg {
    display: block;
  }
</style>
