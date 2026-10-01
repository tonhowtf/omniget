<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings } from "./settings-helpers";
  import OutputSection from "./downloads/OutputSection.svelte";
  import QualitySection from "./downloads/QualitySection.svelte";
  import SubtitlesSection from "./downloads/SubtitlesSection.svelte";
  import ClipboardHotkeysSection from "./downloads/ClipboardHotkeysSection.svelte";
  import PlatformOptionsSection from "./downloads/PlatformOptionsSection.svelte";
  import SettingsCookies from "./SettingsCookies.svelte";
  import SettingsChannels from "./SettingsChannels.svelte";
  import SettingsRules from "./SettingsRules.svelte";

  let {}: { searchActive?: boolean; section?: string } = $props();

  let settings = $derived(getSettings());
</script>

{#if settings}
  <div class="settings-downloads">
    <OutputSection part="files" />
    <QualitySection part="quality" />
    <SubtitlesSection />
    <ClipboardHotkeysSection part="clipboard" />

    <OutputSection part="output" />
    <QualitySection part="presets" />
    <ClipboardHotkeysSection part="hotkeys" />
    <SettingsCookies />
    <SettingsChannels />
    <SettingsRules />
    <PlatformOptionsSection />
  </div>
{:else}
  <p class="settings-downloads-loading" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
{/if}

<style>
  .settings-downloads { display: flex; flex-direction: column; min-inline-size: 0; }
  .settings-downloads-loading { margin: 0; font-size: 13px; color: var(--text-muted); }
</style>
