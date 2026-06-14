<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings } from "./settings-helpers";
  import SettingsDrillBack from "./SettingsDrillBack.svelte";
  import SettingsDrillItem from "./SettingsDrillItem.svelte";
  import OutputSection from "./downloads/OutputSection.svelte";
  import QualitySection from "./downloads/QualitySection.svelte";
  import SubtitlesSection from "./downloads/SubtitlesSection.svelte";
  import ClipboardHotkeysSection from "./downloads/ClipboardHotkeysSection.svelte";
  import PlatformOptionsSection from "./downloads/PlatformOptionsSection.svelte";

  let { searchActive = false }: { searchActive?: boolean } = $props();

  type DownloadDrill = "output" | "quality" | "subtitles" | "hotkeys" | "platforms";
  let subView = $state<DownloadDrill | null>(null);

  let settings = $derived(getSettings());

  const DRILL_ITEMS: { id: DownloadDrill; titleKey: string; hintKey: string }[] = [
    { id: "output", titleKey: "settings.download.section_output", hintKey: "settings.download.section_output_desc" },
    { id: "quality", titleKey: "settings.download.section_quality", hintKey: "settings.download.section_quality_desc" },
    { id: "subtitles", titleKey: "settings.download.section_subtitles", hintKey: "settings.download.section_subtitles_desc" },
    { id: "hotkeys", titleKey: "settings.download.section_clipboard_hotkeys", hintKey: "settings.download.section_clipboard_hotkeys_desc" },
    { id: "platforms", titleKey: "settings.download.section_per_platform", hintKey: "settings.download.section_per_platform_desc" },
  ];

  $effect(() => {
    if (searchActive) subView = null;
  });
</script>

{#if settings}
  {#if searchActive}
    <section class="section"><OutputSection /></section>
    <section class="section"><QualitySection /></section>
    <section class="section"><SubtitlesSection /></section>
    <section class="section"><ClipboardHotkeysSection /></section>
    <section class="section"><PlatformOptionsSection /></section>
  {:else if subView === null}
    <nav class="settings-drill-list" aria-label={$t('settings.cat_downloads')}>
      {#each DRILL_ITEMS as item (item.id)}
        <SettingsDrillItem
          title={$t(item.titleKey)}
          hint={$t(item.hintKey)}
          onclick={() => { subView = item.id; }}
        />
      {/each}
    </nav>
  {:else}
    {@const active = DRILL_ITEMS.find((i) => i.id === subView)}
    {#if active}
      <SettingsDrillBack
        title={$t(active.titleKey)}
        hint={$t(active.hintKey)}
        onBack={() => { subView = null; }}
      />
    {/if}
    <section class="section">
      {#if subView === "output"}
        <OutputSection embedded />
      {:else if subView === "quality"}
        <QualitySection embedded />
      {:else if subView === "subtitles"}
        <SubtitlesSection embedded />
      {:else if subView === "hotkeys"}
        <ClipboardHotkeysSection embedded />
      {:else if subView === "platforms"}
        <PlatformOptionsSection embedded />
      {/if}
    </section>
  {/if}
{/if}
