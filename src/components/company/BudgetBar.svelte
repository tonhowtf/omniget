<script lang="ts">
  /** Usage against a limit: green, orange past the warning, red at the limit. */
  let { pct, warn = 80 }: { pct: number; warn?: number } = $props();
  let clamped = $derived(Math.max(0, Math.min(100, pct)));
  let tone = $derived(pct >= 100 ? "var(--red)" : pct >= warn ? "var(--orange)" : "var(--green)");
</script>

<div class="bar" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(clamped)}>
  <div class="fill" style:width="{clamped}%" style:background={tone}></div>
  <div class="warn" style:left="{Math.min(100, warn)}%"></div>
</div>

<style>
  .bar {
    position: relative;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: var(--radius-full);
    transition: width var(--duration-base) var(--ease-out);
  }
  .warn {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--text-faint);
  }
</style>
