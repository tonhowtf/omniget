<script lang="ts">
  import { t } from "$lib/i18n";
  import SettingRow from "../rebrand/SettingRow.svelte";

  let {
    id,
    label,
    help,
    note,
    settingKey,
    status,
    value,
    systemValue,
    systemValues,
    systemLabel,
    onsave,
  }: {
    id: string;
    label: string;
    help: string;
    note?: string;
    settingKey: string;
    status: string;
    value: string;
    systemValue: string;
    systemValues: readonly string[];
    systemLabel: string;
    onsave: (value: string) => Promise<boolean>;
  } = $props();

  let isSystem = $derived(systemValues.includes(value.trim()));
  let customOpen = $state(false);
  let showCustom = $derived(customOpen || !isSystem);
  let input = $state<HTMLInputElement | null>(null);

  async function onSelect(e: Event) {
    const select = e.target as HTMLSelectElement;
    if (select.value === "custom") {
      customOpen = true;
      queueMicrotask(() => input?.focus());
      return;
    }
    customOpen = false;
    if (!isSystem && !(await onsave(systemValue))) select.value = "custom";
  }

  async function onCustom(e: Event) {
    const field = e.target as HTMLInputElement;
    const next = field.value.trim();
    if (!next) {
      field.value = isSystem ? "" : value;
      return;
    }
    if (next === value) return;
    if (!(await onsave(next))) field.value = isSystem ? "" : value;
  }
</script>

<SettingRow {id} {label} help={note ? `${help} ${note}` : help} {settingKey} {status}>
  {#snippet control({ id: controlId, describedBy })}
    <select id={controlId} class="select" aria-describedby={describedBy} value={showCustom ? "custom" : "system"} onchange={onSelect}>
      <option value="system">{systemLabel}</option>
      <option value="custom">{$t("settings.appearance.fonts.custom")}</option>
    </select>
  {/snippet}
</SettingRow>
{#if showCustom}
  <SettingRow id={`${id}-name`} label={$t("settings.appearance.fonts.name")} help={$t("settings.appearance.fonts.name_hint")}>
    {#snippet control({ id: controlId, describedBy })}
      <input bind:this={input} id={controlId} type="text" class="input-text" dir="ltr" spellcheck="false" autocomplete="off"
        aria-describedby={describedBy} value={isSystem ? "" : value} onchange={onCustom} />
    {/snippet}
  </SettingRow>
{/if}
