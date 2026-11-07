<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, toggleBool, changeNumber } from "../settings-helpers";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import AdvancedDisclosure from "./AdvancedDisclosure.svelte";

  type ToggleField = "torrent_auto_trackers" | "torrent_stop_seeding" | "torrent_upnp";

  let settings = $derived(getSettings());
</script>

{#snippet toggleRow(field: ToggleField)}
  {@const on = settings?.advanced[field] ?? false}
  <SettingRow id={`setting-advanced-${field}`} label={$t(`settings.advanced.${field}`)} help={$t(`settings.advanced.${field}_desc`)}
    settingKey={`settings.advanced.${field}`} status={`advanced.${field}`}>
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy}
        onclick={() => toggleBool("advanced", field, on)}><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>
{/snippet}

{#if settings}
  {@render toggleRow("torrent_auto_trackers")}
  {@render toggleRow("torrent_stop_seeding")}

  <AdvancedDisclosure id="settings-torrent">
    {@render toggleRow("torrent_upnp")}
    <SettingRow id="setting-advanced-torrent_listen_port" label={$t("settings.advanced.torrent_listen_port")} help={$t("settings.advanced.torrent_listen_port_desc")}
      settingKey="settings.advanced.torrent_listen_port" status="advanced.torrent_listen_port">
      {#snippet control({ id, describedBy })}
        <input {id} aria-describedby={describedBy} type="number" class="input-number" dir="ltr" min="1024" max="65525"
          value={settings.advanced.torrent_listen_port} onchange={(e) => changeNumber("advanced", "torrent_listen_port", e)} />
      {/snippet}
    </SettingRow>
  </AdvancedDisclosure>
{/if}
