<script lang="ts">
  /**
   * Cron field with readable presets ("Every weekday at 9:00") and a custom
   * expression. `value` is null when `allowNone` and nothing is chosen.
   */
  import { t } from "$lib/i18n";
  import { CRON_PRESETS, presetOf } from "./company-ui";

  let {
    value = $bindable(null),
    allowNone = true,
    id = undefined,
  }: { value?: string | null; allowNone?: boolean; id?: string } = $props();

  let custom = $state(false);
  let choice = $derived(custom ? "custom" : !value ? "none" : presetOf(value) ?? "custom");

  function pick(v: string) {
    if (v === "none") {
      custom = false;
      value = null;
    } else if (v === "custom") {
      custom = true;
      value = value ?? "0 9 * * *";
    } else {
      custom = false;
      value = CRON_PRESETS.find((p) => p.id === v)?.cron ?? value;
    }
  }
</script>

<div class="cron-input">
  <select class="input" {id} value={choice} onchange={(e) => pick(e.currentTarget.value)}>
    {#if allowNone}<option value="none">{$t("company.common.cron.none")}</option>{/if}
    {#each CRON_PRESETS as p (p.id)}
      <option value={p.id}>{$t(`company.common.cron.${p.id}`)}</option>
    {/each}
    <option value="custom">{$t("company.common.cron.custom")}</option>
  </select>
  {#if choice === "custom"}
    <input
      class="input mono"
      type="text"
      spellcheck="false"
      placeholder="0 9 * * 1-5"
      aria-label={$t("company.common.cron.expression")}
      value={value ?? ""}
      oninput={(e) => (value = e.currentTarget.value.trim() || null)}
    />
  {/if}
</div>
{#if choice === "custom"}<p class="cron-hint">{$t("company.common.cron.hint")}</p>{/if}

<style>
  .cron-input { display: flex; gap: var(--space-2); flex-wrap: wrap; }
  .cron-input select { flex: 1; min-width: 180px; }
  .cron-input .mono { font-family: var(--font-mono); width: 160px; }
  .cron-hint { margin: 4px 0 0; font-size: var(--text-xs); color: var(--text-dim); }
</style>
