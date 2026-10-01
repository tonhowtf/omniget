<script lang="ts">
  import { t } from "$lib/i18n";
  import QuotaMeter from "$components/llm/QuotaMeter.svelte";
  import {
    exhaustsAt,
    humanDuration,
    type UsageWindow,
  } from "$lib/stores/llm-accounts-store.svelte";

  interface Props {
    windows: UsageWindow[];
    available: boolean;
    active?: boolean;
    compact?: boolean;
    nowMs?: number;
  }

  let { windows, available, active = false, compact = false, nowMs = Date.now() }: Props = $props();

  function projection(w: UsageWindow): string | undefined {
    const at = exhaustsAt(w, nowMs);
    if (!at) return undefined;
    const left = humanDuration(at - nowMs);
    return left ? `${$t("llm.accounts.exhausts_in")} ${left}` : undefined;
  }

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
    return String(n);
  }
</script>

{#if !available || windows.length === 0}
  <p class="no-quota">{$t("llm.accounts.quota_unknown")}</p>
{:else}
  <div class="windows">
    {#each windows as w (w.window)}
      <div class="window">
        {#if w.used === null}
          <div class="tokens">
            <span class="tokens-label">{$t(`llm.accounts.window_${w.window}`)}</span>
            <span class="tokens-value">{formatTokens(w.used_tokens)}</span>
          </div>
        {:else}
          <QuotaMeter
            label={$t(`llm.accounts.window_${w.window}`) as string}
            used={w.used}
            resetsAt={w.resets_at_ms ?? undefined}
            projection={projection(w)}
            active={active && w.window === "five_hour"}
            {compact}
          />
        {/if}
        <span class="source" class:estimated={w.source === "estimated"}>
          {$t(`llm.accounts.source_${w.source}`)}
          {#if w.used === null}· {$t("llm.accounts.no_plan_ceiling")}{/if}
        </span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .windows {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .window {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .source {
    align-self: flex-start;
    font-size: var(--text-caption);
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    padding: 0 6px;
  }
  .source.estimated {
    border-style: dashed;
  }
  .tokens {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .tokens-label {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tokens-value {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .no-quota {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
