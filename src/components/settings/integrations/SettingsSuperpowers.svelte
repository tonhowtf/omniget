<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, saveSettings } from "$components/settings/settings-helpers";
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import SettingRow from "$components/settings/rebrand/SettingRow.svelte";
  import { POWER_IDS, powerEnabled, powerPatch } from "./superpowers";

  let settings = $derived(getSettings());
</script>

{#if settings}
  <SettingGroup id="superpowers" icon="blocks" title={$t("nav.superpowers")} description={$t("settings.integrations.superpowers.desc")}>
    {#each POWER_IDS as power (power)}
      {@const on = powerEnabled(settings, power)}
      <SettingRow
        id={`setting-power-${power}`}
        label={$t(`settings.integrations.superpowers.${power}`)}
        help={$t(`settings.integrations.superpowers.${power}_desc`)}
        settingKey={`settings.integrations.superpowers.${power}`}
        status={`${power}.enabled`}
      >
        {#snippet control({ id, describedBy })}
          <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy}
            onclick={() => saveSettings(powerPatch(settings, power, !on))}><span class="toggle-knob"></span></button>
        {/snippet}
      </SettingRow>
    {/each}
  </SettingGroup>
{/if}
