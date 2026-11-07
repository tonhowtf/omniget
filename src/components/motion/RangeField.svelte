<script lang="ts">
  let {
    label,
    value = $bindable(),
    min,
    max,
    step = 1,
    unit = "",
    disabled = false,
    id,
  }: { label: string; value: number; min: number; max: number; step?: number; unit?: string; disabled?: boolean; id: string } = $props();

  function commit(raw: string) {
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    value = Math.min(max, Math.max(min, n));
  }
</script>

<div class="rf">
  <label class="rf-label" for={id}>{label}</label>
  <input class="rf-range" type="range" {min} {max} {step} {disabled} bind:value aria-label={label} />
  <span class="rf-num">
    <input
      id={id}
      class="input-number rf-input"
      type="number"
      {min}
      {max}
      {step}
      {disabled}
      value={value}
      onchange={(e) => commit((e.currentTarget as HTMLInputElement).value)}
    />
    {#if unit}<span class="rf-unit">{unit}</span>{/if}
  </span>
</div>

<style>
  .rf {
    display: grid;
    grid-template-columns: minmax(84px, 120px) 1fr auto;
    align-items: center;
    gap: var(--space-2);
  }

  .rf-label {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .rf-range {
    width: 100%;
    accent-color: var(--accent);
  }

  .rf-num {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .rf-input {
    width: 72px;
    font-variant-numeric: tabular-nums;
  }

  .rf-unit {
    min-width: 18px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
