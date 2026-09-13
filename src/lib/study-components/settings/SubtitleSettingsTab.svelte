<script lang="ts">
  import { t } from "$lib/i18n";
  import SettingsField from "./SettingsField.svelte";
  import SettingsSlider from "./SettingsSlider.svelte";
  import SettingsToggle from "./SettingsToggle.svelte";
  import SettingsSelect from "./SettingsSelect.svelte";
  import SettingsColorPicker from "./SettingsColorPicker.svelte";
  import SubtitlePreview from "./SubtitlePreview.svelte";
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
  const langOptions = [
    { value: "pt-BR", label: "Português (Brasil)" },
    { value: "pt", label: "Português" },
    { value: "en", label: "English" },
    { value: "es", label: "Español" },
    { value: "default", label: $t("study.settings.subtitle.default_auto") },
  ];
  const fontOptions = [
    { value: "system", label: "Sistema" },
    { value: "serif", label: "Serif" },
    { value: "sans", label: "Sans-serif" },
  ];
</script>

<section class="tab">
  <div class="preview-block">
    <SubtitlePreview
      size={player.subtitles_size ?? 100}
      textColor={player.subtitles_text_color ?? "#ffffff"}
      backgroundColor={player.subtitles_background_color ?? "#000000"}
      outlineColor={player.subtitles_outline_color ?? "#000000"}
      opacity={player.subtitles_opacity ?? 100}
      font={player.subtitles_font ?? "system"}
      bold={player.subtitles_bold ?? false}
    />
  </div>

  <SettingsField label={$t("study.settings.subtitle.default_lang")} description={$t("study.settings.subtitle.default_lang_desc")}>
    <SettingsSelect
      value={player.subtitles_default_lang ?? "pt-BR"}
      options={langOptions}
      onChange={(v) => setPlayer("subtitles_default_lang", v)}
    />
  </SettingsField>

  <SettingsField label={$t("study.settings.subtitle.secondary_lang")} description={$t("study.settings.subtitle.secondary_lang_desc")}>
    <SettingsSelect
      value={player.subtitles_secondary_lang ?? "en"}
      options={langOptions}
      onChange={(v) => setPlayer("subtitles_secondary_lang", v)}
    />
  </SettingsField>

  <SettingsField label="Tamanho" valueDisplay={`${player.subtitles_size ?? 100}%`}>
    <SettingsSlider
      value={player.subtitles_size ?? 100}
      min={50}
      max={200}
      step={5}
      onChange={(v) => setPlayer("subtitles_size", v)}
    />
  </SettingsField>

  <SettingsField label="Sincronia" description="Compensa atraso na legenda" valueDisplay={`${(player.subtitles_offset_ms ?? 0) / 1000}s`}>
    <SettingsSlider
      value={player.subtitles_offset_ms ?? 0}
      min={-5000}
      max={5000}
      step={100}
      onChange={(v) => setPlayer("subtitles_offset_ms", v)}
    />
  </SettingsField>

  <SettingsField label="Cor do texto">
    <SettingsColorPicker
      value={player.subtitles_text_color ?? "#ffffff"}
      onChange={(v) => setPlayer("subtitles_text_color", v)}
    />
  </SettingsField>

  <SettingsField label="Cor de fundo">
    <SettingsColorPicker
      value={player.subtitles_background_color ?? "#000000"}
      onChange={(v) => setPlayer("subtitles_background_color", v)}
    />
  </SettingsField>

  <SettingsField label="Cor da borda">
    <SettingsColorPicker
      value={player.subtitles_outline_color ?? "#000000"}
      onChange={(v) => setPlayer("subtitles_outline_color", v)}
    />
  </SettingsField>

  <SettingsField label="Opacidade" valueDisplay={`${player.subtitles_opacity ?? 100}%`}>
    <SettingsSlider
      value={player.subtitles_opacity ?? 100}
      min={0}
      max={100}
      step={5}
      onChange={(v) => setPlayer("subtitles_opacity", v)}
    />
  </SettingsField>

  <SettingsField label="Fonte">
    <SettingsSelect
      value={player.subtitles_font ?? "system"}
      options={fontOptions}
      onChange={(v) => setPlayer("subtitles_font", v)}
    />
  </SettingsField>

  <SettingsField label="Negrito">
    <SettingsToggle
      value={player.subtitles_bold ?? false}
      onChange={(v) => setPlayer("subtitles_bold", v)}
      ariaLabel="Negrito"
    />
  </SettingsField>

  <SettingsField
    label="Respeitar estilo do .ass"
    description={$t("study.settings.subtitle.ass_desc")}
  >
    <SettingsToggle
      value={player.ass_subtitles_styling ?? true}
      onChange={(v) => setPlayer("ass_subtitles_styling", v)}
      ariaLabel="Respeitar styling do .ass"
    />
  </SettingsField>
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
  }

  .preview-block {
    margin-bottom: 16px;
  }
</style>
