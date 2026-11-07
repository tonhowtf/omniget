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

  let settings = $derived(getSettings());
</script>

{#if settings}
  <div class="settings-downloads">
    <OutputSection />
    <QualitySection />
    <SubtitlesSection />
    <ClipboardHotkeysSection />
    <SettingsCookies />
    <PlatformOptionsSection />
    <SettingsChannels />
    <SettingsRules />
  </div>
{:else}
  <p class="settings-downloads-loading" role="status" aria-busy="true">{$t("settings.rebrand.state.loading")}</p>
{/if}

<style>
  .settings-downloads { display: flex; flex-direction: column; min-inline-size: 0; }
  .settings-downloads-loading { margin: 0; font-size: var(--text-base); color: var(--text-muted); }
</style>
