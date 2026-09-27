<script lang="ts">
  /**
   * Guard seal for a component card: level + score. Colour only for warn and
   * block; an "ok" seal stays neutral and never says "safe" — it says the
   * static checks found nothing that warns. No report = "not scanned".
   *
   * Clicking it (when `onclick` is given) opens the report dialog.
   */
  import { t } from "$lib/i18n";
  import type { GuardLevel, GuardReport } from "$lib/central/guard";

  let {
    report = null,
    compact = false,
    onclick,
  }: {
    report?: GuardReport | null;
    compact?: boolean;
    onclick?: () => void;
  } = $props();

  let level = $derived<GuardLevel | "none">(report ? report.level : "none");
  let label = $derived(
    level === "block"
      ? $t("llm.central.guard.level_block")
      : level === "warn"
        ? $t("llm.central.guard.level_warn")
        : level === "ok"
          ? $t("llm.central.guard.level_ok")
          : $t("llm.central.guard.not_scanned"),
  );
  let title = $derived(
    report
      ? $t("llm.central.guard.badge_hint", {
          score: String(report.score),
          critical: String(report.counts.critical ?? 0),
          high: String(report.counts.high ?? 0),
          medium: String(report.counts.medium ?? 0),
        })
      : $t("llm.central.guard.not_scanned_hint"),
  );
</script>

{#if onclick}
  <button type="button" class="seal {level}" class:compact {title} onclick={() => onclick?.()}>
    <span class="dot" aria-hidden="true"></span>
    {#if !compact}<span class="label">{label}</span>{/if}
    {#if report}<span class="score">{report.score}</span>{/if}
  </button>
{:else}
  <span class="seal {level}" class:compact {title}>
    <span class="dot" aria-hidden="true"></span>
    {#if !compact}<span class="label">{label}</span>{/if}
    {#if report}<span class="score">{report.score}</span>{/if}
  </span>
{/if}

<style>
  .seal {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 1px var(--space-2);
    border-radius: var(--radius-full);
    border: var(--hairline) solid var(--separator);
    background: transparent;
    font: inherit;
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
    line-height: 1.6;
  }

  button.seal {
    cursor: pointer;
  }

  button.seal:hover {
    border-color: var(--content-border);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.55;
  }

  .seal.none .dot {
    background: transparent;
    box-shadow: inset 0 0 0 1px currentColor;
  }

  .seal.warn {
    border-color: transparent;
    background: color-mix(in srgb, var(--warning, var(--danger)) 16%, transparent);
    color: var(--warning, var(--danger));
  }

  .seal.block {
    border-color: transparent;
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
    font-weight: 600;
  }

  .seal.warn .dot,
  .seal.block .dot {
    opacity: 1;
  }

  .score {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .compact {
    padding: 1px var(--space-1);
  }
</style>
