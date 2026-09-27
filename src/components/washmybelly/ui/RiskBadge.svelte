<script lang="ts">
  import { t } from "$lib/i18n";
  import type { Risk } from "$lib/washmybelly/api";

  /// `count` turns the badge into a summary chip ("Can be undone · 12").
  let { risk, count, compact = false }: { risk: Risk; count?: number; compact?: boolean } = $props();
</script>

<span class="wmb-risk wmb-risk-{risk}" class:compact title={compact ? ($t(`washmybelly.risk.${risk}`) as string) : undefined}>
  <span class="dot" aria-hidden="true"></span>
  {#if !compact}<span class="txt">{$t(`washmybelly.risk.${risk}`)}</span>{/if}
  {#if count !== undefined}<span class="n">{count}</span>{/if}
</span>

<style>
  .wmb-risk {
    --tone: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 8px 0 7px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--tone) 14%, transparent);
    color: color-mix(in srgb, var(--tone) 82%, var(--text));
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .wmb-risk.compact {
    padding: 0 6px;
    height: 16px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: var(--radius-full);
    background: var(--tone);
  }
  .n {
    font-variant-numeric: tabular-nums;
    opacity: 0.8;
  }
  .wmb-risk-read { --tone: var(--text-dim); }
  .wmb-risk-reversible { --tone: var(--success); }
  .wmb-risk-destructive { --tone: var(--warning); }
  .wmb-risk-extreme { --tone: var(--danger); }
</style>
