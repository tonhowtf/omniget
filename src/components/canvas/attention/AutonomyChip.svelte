<script lang="ts">
  import { t } from "$lib/i18n";
  import { autonomyOf } from "$lib/canvas/attention/autonomy";

  let { command, compact = false }: { command: readonly string[]; compact?: boolean } = $props();

  let found = $derived(autonomyOf(command));
</script>

{#if found}
  {@const label = $t(`canvas.autonomy.${found.level}`) as string}
  <span class="chip" class:compact data-level={found.level} role="img" title={`${label}. ${$t("canvas.autonomy.hint", { flag: found.flag })}`} aria-label={label}>
    <span class="dot" aria-hidden="true"></span>
    {#if !compact}<span class="label" aria-hidden="true">{label}</span>{/if}
  </span>
{/if}

<style>
  .chip {
    flex: none;
    max-width: 170px;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 18px;
    padding: 0 6px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-muted);
  }
  .chip[data-level="auto_edits"] .dot {
    background: var(--agent-working);
  }
  .chip[data-level="full"] {
    color: var(--text);
    border-color: color-mix(in srgb, var(--agent-blocked) 55%, transparent);
  }
  .chip[data-level="full"] .dot {
    background: var(--agent-blocked);
  }
  .chip.compact {
    padding: 0 5px;
  }
  .chip.compact[data-level="full"] .dot {
    width: 8px;
    height: 8px;
    border-radius: 0;
    transform: rotate(45deg);
  }
</style>
