<script lang="ts">
  import { t } from "$lib/i18n";
  import type { CostRow } from "$lib/company/rpc";
  import { usd } from "./company-ui";
  import { tokens } from "./company-format";

  let { rows }: { rows: CostRow[] } = $props();

  let sorted = $derived([...rows].sort((a, b) => b.cost_usd - a.cost_usd || b.tokens - a.tokens));
  let max = $derived(Math.max(0, ...sorted.map((r) => r.cost_usd)));
</script>

{#if sorted.length === 0}
  <p class="hint">{$t("company.costs.no_rows")}</p>
{:else}
  <div class="group">
    {#each sorted as r (r.key ?? "none")}
      <div class="group-row cost-row">
        <span class="name">{r.name || r.key || $t("company.costs.unattributed")}</span>
        <div class="track"><div class="fill" style:width="{max > 0 ? (r.cost_usd / max) * 100 : 0}%"></div></div>
        <span class="num money">{$t("company.common.cost", { amount: usd(r.cost_usd) })}</span>
        <span class="num">{tokens(r.tokens)}</span>
        <span class="num">{$t("company.costs.events", { count: r.events })}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .cost-row {
    min-height: var(--row-compact);
    gap: var(--space-3);
  }
  .name {
    width: 180px;
    flex-shrink: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }
  .track {
    flex: 1;
    min-width: 60px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: var(--radius-full);
    background: var(--accent);
  }
  .num {
    flex-shrink: 0;
    min-width: 64px;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .money {
    color: var(--text);
    min-width: 84px;
  }
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
