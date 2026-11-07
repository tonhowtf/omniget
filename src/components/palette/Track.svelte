<script lang="ts">
  let {
    label,
    name,
    value,
    min,
    max,
    step,
    gradient,
    checker = false,
    digits = 3,
    unit = "",
    scale = 1,
    onchange,
  }: {
    label: string;
    name: string;
    value: number;
    min: number;
    max: number;
    step: number;
    gradient: string;
    checker?: boolean;
    digits?: number;
    unit?: string;
    scale?: number;
    onchange: (value: number) => void;
  } = $props();

  let shown = $derived(Number((value * scale).toFixed(digits)));

  function fromRange(e: Event) {
    onchange(Number((e.currentTarget as HTMLInputElement).value));
  }

  function fromNumber(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(v)) onchange(Math.min(max, Math.max(min, v / scale)));
  }
</script>

<div class="pl-track">
  <span class="pl-track-label" aria-hidden="true">{label}</span>
  <div class="pl-track-rail" class:checker style:--rail={gradient}>
    <input type="range" {min} {max} {step} {value} aria-label={name} aria-valuetext={`${shown}${unit}`} oninput={fromRange} />
  </div>
  <span class="pl-track-num">
    <input class="input" type="number" min={min * scale} max={max * scale} step={step * scale} value={shown} aria-label={name} onchange={fromNumber} />
    {#if unit}<span class="pl-track-unit" aria-hidden="true">{unit}</span>{/if}
  </span>
</div>

<style>
  .pl-track {
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr) 96px;
    align-items: center;
    gap: var(--space-3);
  }

  .pl-track-label {
    font-size: var(--text-sm);
    font-weight: 650;
    color: var(--text-muted);
    font-family: var(--font-mono);
  }

  .pl-track-rail {
    position: relative;
    height: 22px;
    border-radius: var(--radius-full);
    background: var(--rail);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .pl-track-rail.checker {
    background:
      var(--rail),
      repeating-conic-gradient(var(--surface-hi) 0% 25%, var(--surface) 0% 50%) 0 0 / 10px 10px;
  }

  .pl-track-rail input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    background: transparent;
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
  }

  .pl-track-rail input::-webkit-slider-runnable-track {
    height: 22px;
    background: transparent;
  }

  .pl-track-rail input::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px;
    height: 22px;
    border-radius: var(--radius-sm);
    background: transparent;
    box-shadow: 0 0 0 2px var(--secondary), 0 0 0 3px rgba(var(--shadow-ink), var(--elev-alpha-3));
  }

  .pl-track-rail input::-moz-range-thumb {
    width: 14px;
    height: 22px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    box-shadow: 0 0 0 2px var(--secondary), 0 0 0 3px rgba(var(--shadow-ink), var(--elev-alpha-3));
  }

  .pl-track-rail input:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
    border-radius: var(--radius-full);
  }

  .pl-track-num {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .pl-track-num .input {
    width: 100%;
    min-width: 0;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .pl-track-unit {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
