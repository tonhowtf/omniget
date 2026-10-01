<script lang="ts">
  import { t } from "$lib/i18n";
  import { RUN_MODES, type RunMode } from "$lib/stores/llm-run-mode.svelte";

  let {
    value = $bindable<RunMode>("auto_edits"),
    compact = false,
    disabled = false,
    onchange,
  }: {
    value?: RunMode;
    compact?: boolean;
    disabled?: boolean;
    onchange?: (mode: RunMode) => void;
  } = $props();

  let confirming = $state(false);
  let select = $state<HTMLSelectElement | null>(null);

  function pick(next: RunMode) {
    if (next === value) return;
    if (next === "full_access") {
      confirming = true;
      return;
    }
    confirming = false;
    value = next;
    onchange?.(next);
  }

  function confirmFull() {
    confirming = false;
    value = "full_access";
    onchange?.("full_access");
  }

  function cancelFull() {
    confirming = false;
    if (select) select.value = value;
  }
</script>

<div class="run-mode" class:compact>
  <label class="run-mode-field">
    <span class="field-label">{$t("llm.run_mode.label")}</span>
    <select
      bind:this={select}
      class="input"
      {disabled}
      value={value}
      title={$t(`llm.run_mode.${value}_hint`) as string}
      onchange={(e) => pick(e.currentTarget.value as RunMode)}
    >
      {#each RUN_MODES as mode (mode)}
        <option value={mode}>{$t(`llm.run_mode.${mode}`)}</option>
      {/each}
    </select>
  </label>
  {#if confirming}
    <div class="run-mode-confirm" role="alert">
      <span>{$t("llm.run_mode.full_access_warning")}</span>
      <button type="button" class="button danger" onclick={confirmFull}>{$t("llm.run_mode.full_access_confirm")}</button>
      <button type="button" class="button" onclick={cancelFull}>{$t("llm.run_mode.cancel")}</button>
    </div>
  {:else if !compact}
    <span class="run-mode-hint">{$t(`llm.run_mode.${value}_hint`)}</span>
  {/if}
</div>

<style>
  .run-mode { display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }
  .run-mode.compact { flex-direction: row; align-items: center; flex-wrap: wrap; gap: var(--space-2); }
  .run-mode-field { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .compact .run-mode-field { flex-direction: row; align-items: center; gap: var(--space-2); }
  .compact .field-label { font-size: var(--text-caption); color: var(--text-dim); white-space: nowrap; }
  .compact select { font-size: var(--text-caption); padding: 2px 6px; height: auto; }
  .run-mode-hint { font-size: var(--text-caption); color: var(--text-dim); }
  .run-mode-confirm {
    display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2);
    font-size: var(--text-caption); color: var(--text);
    background: var(--accent-soft); border-radius: var(--radius-md); padding: 6px 10px;
  }
</style>
