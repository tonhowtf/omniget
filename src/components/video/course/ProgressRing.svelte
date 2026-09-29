<script lang="ts">
  // Circular progress (after the 21st "Circle Progress" / Activity Ring).
  let { value, size = 44, stroke = 4, label = "" }: { value: number; size?: number; stroke?: number; label?: string } = $props();

  let r = $derived((size - stroke) / 2);
  let c = $derived(2 * Math.PI * r);
  let v = $derived(Math.min(1, Math.max(0, value || 0)));
</script>

<span class="ring" style:width="{size}px" style:height="{size}px" role="img" aria-label={label || `${Math.round(v * 100)}%`}>
  <svg viewBox="0 0 {size} {size}" width={size} height={size} aria-hidden="true">
    <circle class="bg" cx={size / 2} cy={size / 2} {r} stroke-width={stroke} />
    <circle class="fg" class:full={v >= 1} cx={size / 2} cy={size / 2} {r} stroke-width={stroke} stroke-dasharray={c} stroke-dashoffset={c * (1 - v)} />
  </svg>
  <span class="pct" style:font-size="{Math.max(9, size * 0.26)}px">{Math.round(v * 100)}</span>
</span>

<style>
  .ring {
    position: relative;
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
  }
  svg {
    position: absolute;
    inset: 0;
    transform: rotate(-90deg);
  }
  circle {
    fill: none;
  }
  .bg {
    stroke: var(--fill-2);
  }
  .fg {
    stroke: var(--accent);
    stroke-linecap: round;
    transition: stroke-dashoffset var(--duration-slow) var(--ease-out);
  }
  .fg.full {
    stroke: var(--success);
  }
  .pct {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
</style>
