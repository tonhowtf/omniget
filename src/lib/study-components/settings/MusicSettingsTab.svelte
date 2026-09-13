<script lang="ts">
  import { t } from "$lib/i18n";
  import SettingsField from "./SettingsField.svelte";
  import SettingsToggle from "./SettingsToggle.svelte";
  import type { StudySettings } from "$lib/study-bridge";

  type Props = {
    settings: StudySettings;
    onPatch: (patch: StudySettings) => void;
  };

  let { settings, onPatch }: Props = $props();

  const ALL_CATEGORIES = [
    { key: "sponsor", label: "Patrocinado" },
    { key: "selfpromo", label: $t("study.settings.music.selfpromo") },
    { key: "intro", label: "Intro" },
    { key: "outro", label: "Encerramento" },
    { key: "interaction", label: $t("study.settings.music.interaction") },
    { key: "preview", label: $t("study.settings.music.preview") },
    { key: "music_offtopic", label: $t("study.settings.music.music_offtopic") },
    { key: "filler", label: $t("study.settings.music.filler") },
  ];

  const music = $derived(((settings as unknown as { music?: Record<string, unknown> }).music ?? {}) as Record<string, unknown>);

  const enabled = $derived(Boolean(music.sponsorblock_enabled ?? true));
  const autoSkip = $derived(Boolean(music.sponsorblock_auto_skip ?? false));
  const categories = $derived(
    Array.isArray(music.sponsorblock_categories)
      ? (music.sponsorblock_categories as string[])
      : ["sponsor", "selfpromo"],
  );

  function setMusic(key: string, value: unknown) {
    const next: Record<string, unknown> = { ...music, [key]: value };
    onPatch({ music: next } as unknown as StudySettings);
  }

  function toggleCategory(cat: string) {
    const has = categories.includes(cat);
    const next = has ? categories.filter((c) => c !== cat) : [...categories, cat];
    setMusic("sponsorblock_categories", next);
  }
</script>

<section class="tab">
  <SettingsField
    label="SponsorBlock"
    description="Pula trechos patrocinados detectados pela comunidade. A request usa hash-prefix do video ID, preservando privacidade."
  >
    <SettingsToggle
      value={enabled}
      onChange={(v) => setMusic("sponsorblock_enabled", v)}
      ariaLabel="SponsorBlock"
    />
  </SettingsField>

  {#if enabled}
    <SettingsField
      label="Pular automaticamente"
      description={$t("study.settings.music.autoskip_desc")}
    >
      <SettingsToggle
        value={autoSkip}
        onChange={(v) => setMusic("sponsorblock_auto_skip", v)}
        ariaLabel="Pular automaticamente"
      />
    </SettingsField>

    <SettingsField
      label="Categorias monitoradas"
      description={$t("study.settings.music.segment_types_desc")}
    >
      <div class="categories">
        {#each ALL_CATEGORIES as cat (cat.key)}
          <label class="cat" class:active={categories.includes(cat.key)}>
            <input
              type="checkbox"
              checked={categories.includes(cat.key)}
              onchange={() => toggleCategory(cat.key)}
            />
            <span>{cat.label}</span>
          </label>
        {/each}
      </div>
    </SettingsField>
  {/if}
</section>

<style>
  .tab {
    display: flex;
    flex-direction: column;
  }
  .categories {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 8px;
  }
  .cat {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid color-mix(in oklab, var(--text) 14%, transparent);
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
    user-select: none;
  }
  .cat.active {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
  .cat input {
    accent-color: var(--accent);
  }
</style>
