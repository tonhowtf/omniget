<script lang="ts">
  import SettingsCanvas from "$components/settings/SettingsCanvas.svelte";
  import AgentsCanvasGroup from "$components/settings/ai/AgentsCanvasGroup.svelte";
  import PresetsGroup from "$components/settings/ai/PresetsGroup.svelte";
  import AccountsPanel from "./AccountsPanel.svelte";
  import LimitsPanel from "./LimitsPanel.svelte";
  import ConnectPanel from "./ConnectPanel.svelte";
  import McpPanel from "./McpPanel.svelte";
  import ApiKeyPanel from "./ApiKeyPanel.svelte";
</script>

<AccountsPanel />
<ApiKeyPanel />
<LimitsPanel />
<AgentsCanvasGroup />
<PresetsGroup />
<ConnectPanel />
<McpPanel />
<SettingsCanvas />
