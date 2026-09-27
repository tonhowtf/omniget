<script lang="ts">
  import { healthTone } from "$lib/washmybelly/ui";

  /// Activity-ring style score (0–100). `label` goes under the number.
  let { score, size = 120, label = "" }: { score: number; size?: number; label?: string } = $props();

  let stroke = $derived(Math.max(6, Math.round(size / 11)));
  let r = $derived(size / 2 - stroke / 2 - 1);
  let c = $derived(2 * Math.PI * r);
  let clamped = $derived(Math.max(0, Math.min(100, Number.isFinite(score) ? score : 0)));
  let tone = $derived(healthTone(clamped));
</script>

<div class="wmb-ring tone-{tone}" style:width="{size}px" style:height="{size}px" role="img" aria-label={`${Math.round(clamped)}/100${label ? ` ${label}` : ""}`}>
  <svg width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true">
    <circle class="track" cx={size / 2} cy={size / 2} {r} fill="none" stroke-width={stroke} />
    <circle
      class="value"
      cx={size / 2}
      cy={size / 2}
      {r}
      fill="none"
      stroke-width={stroke}
      stroke-linecap="round"
      stroke-dasharray={c}
      stroke-dashoffset={c * (1 - clamped / 100)}
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  </svg>
  <span class="center">
    <span class="n" style:font-size="{Math.round(size / 3.6)}px">{Math.round(clamped)}</span>
    {#if label}<span class="lbl" style:font-size="{Math.max(10, Math.round(size / 11))}px">{label}</span>{/if}
  </span>
</div>

<style>
  .wmb-ring {
    --tone: var(--success);
    position: relative;
    flex-shrink: 0;
  }
  .tone-fair { --tone: var(--warning); }
  .tone-poor { --tone: var(--orange); }
  .tone-bad { --tone: var(--danger); }
  .track {
    stroke: color-mix(in srgb, var(--tone) 16%, transparent);
  }
  .value {
    stroke: var(--tone);
    transition: stroke-dashoffset var(--duration-slow) var(--ease-out);
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    line-height: 1;
  }
  .n {
    font-family: var(--font-display);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .lbl {
    margin-top: 3px;
    font-weight: 600;
    color: var(--text-dim);
  }
  @media (prefers-reduced-motion: reduce) {
    .value { transition: none; }
  }
</style>
