<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    SCAN_RISK_THRESHOLD,
    scanBadgeKey,
    scanBadgeTone,
    scanScore,
    type SkillScan,
  } from "$lib/stores/llm-skills-store.svelte";

  let { scan = null }: { scan?: SkillScan | null } = $props();

  let score = $derived(scanScore(scan));
  let tone = $derived(scanBadgeTone(scan));
  let label = $derived($t(scanBadgeKey(scan)));
  let title = $derived(
    scan?.status === "scanned"
      ? $t("llm.skills.scan_score_hint", { score: String(score), threshold: String(SCAN_RISK_THRESHOLD) })
      : scan?.status === "failed"
        ? $t("llm.skills.scan_failed_hint", { reason: scan.reason })
        : $t("llm.skills.scan_none_hint"),
  );
</script>

<span class="scan {tone}" {title}>
  {label}{#if score !== null}<span class="score">{score}</span>{/if}
</span>

<style>
  .scan {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 1px var(--space-2);
    border-radius: var(--radius-full);
    border: var(--hairline) solid var(--separator);
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }

  .scan.warn {
    border-color: transparent;
    background: color-mix(in srgb, var(--warning, var(--danger)) 16%, transparent);
    color: var(--warning, var(--danger));
  }

  .scan.risk {
    border-color: transparent;
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
    font-weight: 600;
  }

  .score {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
</style>
