<script lang="ts">
  import { t } from "$lib/i18n";
  import SettingsField from "./SettingsField.svelte";
  import SettingsSelect from "./SettingsSelect.svelte";
  import type { StudySettings } from "$lib/study-bridge";

  type Props = {
    settings: StudySettings;
    onPatch: (patch: StudySettings) => void;
  };

  let { settings, onPatch }: Props = $props();
  const player = $derived(settings.player ?? {});
  const langOptions = [
    { value: "pt-BR", label: "Português (Brasil)" },
    { value: "pt", label: "Português" },
    { value: "en", label: "English" },
    { value: "es", label: "Español" },
  ];
</script>

<section class="tab">
  <SettingsField
    label={$t("study.settings.audio.default_audio_lang")}
    description={$t("study.settings.audio.default_audio_lang_desc")}
  >
    <SettingsSelect
      value={player.audio_default_lang ?? "pt-BR"}
      options={langOptions}
      onChange={(v) => onPatch({ player: { ...(settings.player ?? {}), audio_default_lang: v } })}
    />
  </SettingsField>

  <SettingsField
    label={$t("study.settings.subtitle.secondary_lang")} description={$t("study.settings.subtitle.secondary_lang_desc")}
  >
    <SettingsSelect
      value={player.audio_secondary_lang ?? "en"}
      options={langOptions}
      onChange={(v) => onPatch({ player: { ...(settings.player ?? {}), audio_secondary_lang: v } })}
    />
  </SettingsField>
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
  }
</style>
