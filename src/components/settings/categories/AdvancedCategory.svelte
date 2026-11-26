<script lang="ts">
  import { t } from "$lib/i18n";
  import type { ResetScope } from "$lib/stores/settings-v3-types";
  import SettingGroup from "#components/settings/rebrand/SettingGroup.svelte";
  import SettingsNetwork from "#components/settings/SettingsNetwork.svelte";
  import SpeedSettings from "#components/settings/advanced/SpeedSettings.svelte";
  import TorrentSettings from "#components/settings/advanced/TorrentSettings.svelte";
  import ComponentsSettings from "#components/settings/advanced/ComponentsSettings.svelte";
  import TroubleshootingSettings from "#components/settings/advanced/TroubleshootingSettings.svelte";

  let { askReset }: { askReset: (scope: ResetScope) => void } = $props();
</script>

<SettingGroup id="connection" icon="network" title={$t("settings.advanced.connection_title")} description={$t("settings.advanced.connection_desc")}>
  <SettingsNetwork />
</SettingGroup>
<SettingGroup id="speed" icon="speed" title={$t("settings.advanced.speed_title")} description={$t("settings.advanced.speed_desc")}>
  <SpeedSettings />
</SettingGroup>
<SettingGroup id="torrent" icon="magnet" title={$t("settings.advanced.torrent_title")} description={$t("settings.advanced.torrent_desc")}>
  <TorrentSettings />
</SettingGroup>
<SettingGroup id="dependencies" icon="blocks" title={$t("settings.advanced.components_title")} description={$t("settings.advanced.components_desc")}>
  <ComponentsSettings />
</SettingGroup>
<SettingGroup id="troubleshooting" icon="wrench" title={$t("settings.advanced.troubleshooting_title")} description={$t("settings.advanced.troubleshooting_desc")}>
  <TroubleshootingSettings {askReset} />
</SettingGroup>
