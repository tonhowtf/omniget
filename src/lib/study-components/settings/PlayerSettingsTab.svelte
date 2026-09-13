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

  function setPlayer<K extends keyof NonNullable<StudySettings["player"]>>(
    key: K,
    value: NonNullable<StudySettings["player"]>[K],
  ) {
    onPatch({ player: { ...(settings.player ?? {}), [key]: value } });
  }

  const player = $derived(settings.player ?? {});
</script>

<section class="tab">
  <SettingsField
    label={$t("study.settings.player.completion_limit")}
    description={$t("study.settings.player.completion_limit_desc")}
    valueDisplay={`${Math.round((player.completion_threshold ?? 0.95) * 100)}%`}
  >
    <SettingsSlider
      value={Math.round((player.completion_threshold ?? 0.95) * 100)}
      min={50}
      max={100}
      step={1}
      onChange={(v) => setPlayer("completion_threshold", v / 100)}
    />
  </SettingsField>

  <SettingsField
    label={$t("study.settings.player.long_seek")}
    description="Tempo do salto com J/L (atalho)"
    valueDisplay={`${(player.seek_step_long_ms ?? 10000) / 1000}s`}
  >
    <SettingsSlider
      value={player.seek_step_long_ms ?? 10000}
      min={1000}
      max={30000}
      step={1000}
      onChange={(v) => setPlayer("seek_step_long_ms", v)}
    />
  </SettingsField>

  <SettingsField
    label={$t("study.settings.player.short_seek")}
    description="Tempo do salto com Shift+J/L"
    valueDisplay={`${(player.seek_step_short_ms ?? 3000) / 1000}s`}
  >
    <SettingsSlider
      value={player.seek_step_short_ms ?? 3000}
      min={1000}
      max={10000}
      step={500}
      onChange={(v) => setPlayer("seek_step_short_ms", v)}
    />
  </SettingsField>

  <SettingsField
    label="Pausar ao minimizar"
    description={$t("study.settings.player.pause_blur_desc")}
  >
    <SettingsToggle
      value={player.pause_on_minimize ?? false}
      onChange={(v) => setPlayer("pause_on_minimize", v)}
      ariaLabel="Pausar ao minimizar"
    />
  </SettingsField>

  <SettingsField
    label="ESC sai do fullscreen"
    description="Quando ativo, ESC sai do fullscreen. Quando inativo, ESC fecha a aula"
  >
    <SettingsToggle
      value={player.esc_exit_fullscreen ?? true}
      onChange={(v) => setPlayer("esc_exit_fullscreen", v)}
      ariaLabel="ESC sai do fullscreen"
    />
  </SettingsField>

  <SettingsField
    label="Intensidade do blur do hero"
    description={$t("study.settings.player.hero_blur_desc")}
    valueDisplay={`${player.hero_blur_intensity ?? 40}`}
  >
    <SettingsSlider
      value={player.hero_blur_intensity ?? 40}
      min={0}
      max={100}
      step={5}
      onChange={(v) => setPlayer("hero_blur_intensity", v)}
    />
  </SettingsField>

  <SettingsField
    label="Gerar miniaturas automaticamente"
    description="Cria sprite VTT pra preview na barra de progresso (consome disco)"
  >
    <SettingsToggle
      value={player.thumbnails_auto_generate ?? false}
      onChange={(v) => setPlayer("thumbnails_auto_generate", v)}
      ariaLabel="Gerar miniaturas automaticamente"
    />
  </SettingsField>
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
  }
</style>
