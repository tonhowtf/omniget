<script lang="ts">
  import { t } from "$lib/i18n";
  import SettingsField from "./SettingsField.svelte";
  import SettingsSlider from "./SettingsSlider.svelte";
  import SettingsToggle from "./SettingsToggle.svelte";
  import type { StudySettings } from "$lib/study-bridge";

  type Props = {
    settings: StudySettings;
    onPatch: (patch: StudySettings) => void;
  };

  let { settings, onPatch }: Props = $props();
  const player = $derived(settings.player ?? {});

  function setPlayer<K extends keyof NonNullable<StudySettings["player"]>>(
    key: K,
    value: NonNullable<StudySettings["player"]>[K],
  ) {
    onPatch({ player: { ...(settings.player ?? {}), [key]: value } });
  }
</script>

<section class="tab">
  <SettingsField
    label={$t("study.settings.behavior.autoplay_next")}
    description={$t("study.settings.behavior.autoplay_next_desc")}
  >
    <SettingsToggle
      value={player.binge_watching ?? true}
      onChange={(v) => setPlayer("binge_watching", v)}
      ariaLabel="Auto-play"
    />
  </SettingsField>

  <SettingsField
    label="Tempo do countdown"
    description="Quantos segundos o aviso de auto-play aparece antes de pular"
    valueDisplay={`${(player.next_video_notification_ms ?? 5000) / 1000}s`}
  >
    <SettingsSlider
      value={player.next_video_notification_ms ?? 5000}
      min={1000}
      max={15000}
      step={500}
      onChange={(v) => setPlayer("next_video_notification_ms", v)}
    />
  </SettingsField>

  <SettingsField
    label={$t("study.settings.behavior.collect_seeks")}
    description={$t("study.settings.behavior.collect_seeks_desc")}
  >
    <SettingsToggle
      value={player.collect_seek_logs ?? true}
      onChange={(v) => setPlayer("collect_seek_logs", v)}
      ariaLabel={$t("study.settings.behavior.collect_seeks_aria")}
    />
  </SettingsField>
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
  }
</style>
