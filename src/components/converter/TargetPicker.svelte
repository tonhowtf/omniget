<script lang="ts">
  import { t } from "$lib/i18n";
  import { engineName, groupByCategory, type CommonTarget } from "$lib/converter/plan";

  let {
    targets,
    value,
    disabled = false,
    onpick,
  }: { targets: CommonTarget[]; value: string | null; disabled?: boolean; onpick: (format: string) => void } = $props();

  let groups = $derived(groupByCategory(targets));
  let native = $derived($t("converter.engine_native") as string);
</script>

<div class="cv-picker">
  {#each groups as group (group.category)}
    <div class="cv-picker-group" role="group" aria-labelledby="cv-cat-{group.category}">
      <h3 class="cv-picker-cat" id="cv-cat-{group.category}">{$t(`converter.category.${group.category}`)}</h3>
      <div class="cv-picker-chips">
        {#each group.items as item (item.format)}
          {@const tool = item.missingTool ? engineName(item.missingTool, native) : null}
          <button
            type="button"
            class="cv-chip"
            class:on={value === item.format}
            aria-pressed={value === item.format}
            disabled={disabled || !item.available}
            title={tool
              ? ($t("converter.needs_tool_title", { tool }) as string)
              : item.skipped > 0
                ? ($t("converter.skips_some", { count: item.skipped }) as string)
                : undefined}
            onclick={() => onpick(item.format)}
          >
            {#if value === item.format}
              <svg class="cv-chip-check" viewBox="0 0 12 12" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m2.5 6.3 2.3 2.2 4.7-5" /></svg>
            {/if}
            <span class="cv-chip-label">{item.label}</span>
            {#if tool}
              <span class="cv-chip-why">{$t("converter.needs_tool", { tool })}</span>
            {:else if item.lossy}
              <span class="cv-chip-lossy" title={$t("converter.lossy_title") as string}>{$t("converter.lossy")}</span>
            {/if}
          </button>
        {/each}
      </div>
    </div>
  {/each}
</div>

<style>
  .cv-picker {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4) var(--space-6);
  }

  .cv-picker-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .cv-picker-cat {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .cv-picker-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .cv-chip {
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-1);
    min-height: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 550;
    line-height: var(--control-h);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .cv-chip:hover:not(:disabled):not(.on) {
      background: var(--fill-2);
    }
  }

  .cv-chip:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .cv-chip.on {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
    color: var(--text);
  }

  .cv-chip:disabled {
    cursor: not-allowed;
    color: var(--text-dim);
    background: transparent;
  }

  .cv-chip-check {
    align-self: center;
    color: var(--accent-text);
  }

  .cv-chip-why,
  .cv-chip-lossy {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-dim);
  }

  .cv-chip-lossy {
    font-style: italic;
  }
</style>
