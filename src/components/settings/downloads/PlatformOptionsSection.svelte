<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings, toggleBool } from "../settings-helpers";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import SettingsSaveBar from "../rebrand/SettingsSaveBar.svelte";

  type ControlArgs = { id: string; describedBy: string | undefined };

  let settings = $derived(getSettings());

  const SB_CATEGORIES = [
    "sponsor",
    "selfpromo",
    "interaction",
    "intro",
    "outro",
    "preview",
    "filler",
    "music_offtopic",
  ] as const;

  const NAMING_FIELDS = [
    { field: "bilibili_naming_video", labelKey: "settings.download.bilibili_naming_video_label" },
    { field: "bilibili_naming_multi_part", labelKey: "settings.download.bilibili_naming_multi_part_label" },
    { field: "bilibili_naming_bangumi", labelKey: "settings.download.bilibili_naming_bangumi_label" },
    { field: "bilibili_naming_cheese", labelKey: "settings.download.bilibili_naming_cheese_label" },
    { field: "bilibili_naming_collection", labelKey: "settings.download.bilibili_naming_collection_label" },
  ] as const;

  function sbHas(cat: string): boolean {
    return settings?.download.sponsorblock_categories?.includes(cat) ?? false;
  }

  function toggleSbCategory(cat: string) {
    const current = settings?.download.sponsorblock_categories ?? [];
    const next = current.includes(cat)
      ? current.filter((c) => c !== cat)
      : [...current, cat];
    updateSettings({ download: { sponsorblock_categories: next } });
  }

  function setDownloadField(field: string, e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { [field]: value } });
  }

  let namingTemplatesOpen = $state(false);
  let cdnOpen = $state(false);

  // Naming templates and CDN hosts are free text: one draft for the Bilibili
  // text fields, saved with "Save changes" instead of on every keystroke.
  const bilibiliText = createSettingsDraft({
    id: "bilibili-text",
    read: (s) => ({
      bilibili_naming_video: s.download.bilibili_naming_video,
      bilibili_naming_multi_part: s.download.bilibili_naming_multi_part,
      bilibili_naming_bangumi: s.download.bilibili_naming_bangumi,
      bilibili_naming_cheese: s.download.bilibili_naming_cheese,
      bilibili_naming_collection: s.download.bilibili_naming_collection,
      bilibili_cdn_hosts: s.download.bilibili_cdn_hosts,
    }),
    toPatch: (d) => ({ download: { ...d } }),
  });

  // A draft must stay reachable: reopen the editors that hold unsaved text.
  $effect(() => {
    if (!bilibiliText.dirty) return;
    const saved = settings?.download;
    if (!saved) return;
    if (NAMING_FIELDS.some(({ field }) => bilibiliText.value[field] !== saved[field])) namingTemplatesOpen = true;
    if (bilibiliText.value.bilibili_cdn_hosts !== saved.bilibili_cdn_hosts) cdnOpen = true;
  });
</script>

{#snippet sw(c: ControlArgs, label: string, on: boolean, onclick: () => void)}
  <button type="button" class="toggle" class:on id={c.id} aria-label={label} aria-describedby={c.describedBy} role="switch" aria-checked={on} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  {@const d = settings.download}
  <SettingGroup id="sites" title={$t("settings.download.section_per_platform")} description={$t("settings.download.section_per_platform_desc")} advanced>
    <h3 class="platform-head" id="dl-youtube-head">{$t("settings.download.youtube_specific")}</h3>
    <SettingRow id="dl-youtube-sponsorblock" label={$t("settings.download.youtube_sponsorblock")} help={$t("settings.download.youtube_sponsorblock_desc")} settingKey="settings.download.youtube_sponsorblock" status="download.youtube_sponsorblock">
      {#snippet control(c)}{@render sw(c, $t("settings.download.youtube_sponsorblock"), d.youtube_sponsorblock, () => toggleBool("download", "youtube_sponsorblock", d.youtube_sponsorblock))}{/snippet}
    </SettingRow>
    {#if d.youtube_sponsorblock}
      <SettingRow id="dl-sb-mode" label={$t("settings.download.sb_mode")} help={$t("settings.download.sb_mode_desc")} settingKey="settings.download.sb_mode" status="download.sponsorblock_mode">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.sponsorblock_mode} onchange={(e) => setDownloadField("sponsorblock_mode", e)}>
            <option value="remove">{$t("settings.download.sb_mode_remove")}</option>
            <option value="mark">{$t("settings.download.sb_mode_mark")}</option>
          </select>
        {/snippet}
      </SettingRow>
      <div class="sb-categories" data-setting-key="settings.download.sb_categories">
        <p class="sb-categories-label" id="dl-sb-categories-label">{$t("settings.download.sb_categories")}</p>
        <p class="sb-categories-help" id="dl-sb-categories-help">{$t("settings.download.sb_categories_desc")}</p>
        <div class="sb-chips" role="group" aria-labelledby="dl-sb-categories-label" aria-describedby="dl-sb-categories-help">
          {#each SB_CATEGORIES as cat (cat)}
            <button
              type="button"
              class="sb-chip"
              class:on={sbHas(cat)}
              onclick={() => toggleSbCategory(cat)}
              aria-pressed={sbHas(cat)}
            >
              {$t(`settings.download.sb_cat_${cat}`)}
            </button>
          {/each}
        </div>
      </div>
    {/if}
    <SettingRow id="dl-split-by-chapters" label={$t("settings.download.split_by_chapters")} help={$t("settings.download.split_by_chapters_desc")} settingKey="settings.download.split_by_chapters" status="download.split_by_chapters">
      {#snippet control(c)}{@render sw(c, $t("settings.download.split_by_chapters"), d.split_by_chapters, () => toggleBool("download", "split_by_chapters", d.split_by_chapters))}{/snippet}
    </SettingRow>
    <SettingRow id="dl-live-from-start" label={$t("settings.download.live_from_start")} help={$t("settings.download.live_from_start_desc")} settingKey="settings.download.live_from_start" status="download.live_from_start">
      {#snippet control(c)}{@render sw(c, $t("settings.download.live_from_start"), d.live_from_start, () => toggleBool("download", "live_from_start", d.live_from_start))}{/snippet}
    </SettingRow>

    <h3 class="platform-head" id="dl-bilibili-head">{$t("settings.download.bilibili_section")}</h3>
    <SettingRow id="dl-bilibili-container" label={$t("settings.download.bilibili_container_label")} help={$t("settings.download.bilibili_container_desc")} settingKey="settings.download.bilibili_container_label" status="download.bilibili_container">
      {#snippet control(c)}
        <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.bilibili_container} onchange={(e) => setDownloadField("bilibili_container", e)}>
          <option value="mp4">MP4</option>
          <option value="mkv">MKV</option>
        </select>
      {/snippet}
    </SettingRow>
    <SettingRow id="dl-bilibili-danmaku" label={$t("settings.download.bilibili_danmaku_label")} help={$t("settings.download.bilibili_danmaku_desc")} settingKey="settings.download.bilibili_danmaku_label" status="download.bilibili_danmaku_enabled">
      {#snippet control(c)}{@render sw(c, $t("settings.download.bilibili_danmaku_label"), d.bilibili_danmaku_enabled, () => toggleBool("download", "bilibili_danmaku_enabled", d.bilibili_danmaku_enabled))}{/snippet}
    </SettingRow>
    {#if d.bilibili_danmaku_enabled}
      <SettingRow id="dl-bilibili-danmaku-format" label={$t("settings.download.bilibili_danmaku_format_label")} help={$t("settings.download.bilibili_danmaku_format_desc")} settingKey="settings.download.bilibili_danmaku_format_label" status="download.bilibili_danmaku_format">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.bilibili_danmaku_format} onchange={(e) => setDownloadField("bilibili_danmaku_format", e)}>
            <option value="xml">XML</option>
            <option value="ass">ASS</option>
            <option value="json">JSON</option>
          </select>
        {/snippet}
      </SettingRow>
    {/if}
    <SettingRow id="dl-bilibili-nfo" label={$t("settings.download.bilibili_nfo_label")} help={$t("settings.download.bilibili_nfo_desc")} settingKey="settings.download.bilibili_nfo_label" status="download.bilibili_nfo_enabled">
      {#snippet control(c)}{@render sw(c, $t("settings.download.bilibili_nfo_label"), d.bilibili_nfo_enabled, () => toggleBool("download", "bilibili_nfo_enabled", d.bilibili_nfo_enabled))}{/snippet}
    </SettingRow>
    <SettingRow id="dl-bilibili-cover-sidecar" label={$t("settings.download.bilibili_cover_sidecar_label")} help={$t("settings.download.bilibili_cover_sidecar_desc")} settingKey="settings.download.bilibili_cover_sidecar_label" status="download.bilibili_cover_sidecar">
      {#snippet control(c)}{@render sw(c, $t("settings.download.bilibili_cover_sidecar_label"), d.bilibili_cover_sidecar, () => toggleBool("download", "bilibili_cover_sidecar", d.bilibili_cover_sidecar))}{/snippet}
    </SettingRow>
    {#if d.bilibili_cover_sidecar}
      <SettingRow id="dl-bilibili-cover-format" label={$t("settings.download.bilibili_cover_format_label")} help={$t("settings.download.bilibili_cover_format_desc")} settingKey="settings.download.bilibili_cover_format_label" status="download.bilibili_cover_format">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.bilibili_cover_format} onchange={(e) => setDownloadField("bilibili_cover_format", e)}>
            <option value="jpg">JPG</option>
            <option value="png">PNG</option>
            <option value="webp">WebP</option>
            <option value="avif">AVIF</option>
          </select>
        {/snippet}
      </SettingRow>
    {/if}

    <form class="bilibili-text-form" onsubmit={(e) => { e.preventDefault(); void bilibiliText.save(); }}>
      <SettingRow id="dl-bilibili-naming-toggle" label={$t("settings.download.bilibili_naming_label")} help={$t("settings.download.bilibili_naming_desc")} settingKey="settings.download.bilibili_naming_label">
        {#snippet control(c)}
          <button type="button" class="ghost-btn" id={c.id} aria-describedby={c.describedBy} aria-expanded={namingTemplatesOpen} aria-controls="dl-bilibili-naming-block" onclick={() => (namingTemplatesOpen = !namingTemplatesOpen)}>
            {namingTemplatesOpen ? $t("settings.download.bilibili_naming_hide") : $t("settings.download.bilibili_naming_edit")}
          </button>
        {/snippet}
      </SettingRow>
      <div class="naming-block" id="dl-bilibili-naming-block" hidden={!namingTemplatesOpen}>
        {#each NAMING_FIELDS as item (item.field)}
          <label class="naming-row" for={`dl-${item.field}`}>
            <span class="naming-label">{$t(item.labelKey)}</span>
          </label>
          <input class="naming-input" id={`dl-${item.field}`} type="text" dir="ltr" spellcheck="false" bind:value={bilibiliText.value[item.field]} aria-describedby="dl-bilibili-naming-help" />
        {/each}
        <p class="naming-help" id="dl-bilibili-naming-help">{$t("settings.download.bilibili_naming_help")}</p>
      </div>

      <SettingRow id="dl-bilibili-cdn-toggle" label={$t("settings.download.bilibili_cdn_label")} help={$t("settings.download.bilibili_cdn_desc")} settingKey="settings.download.bilibili_cdn_label">
        {#snippet control(c)}
          <button type="button" class="ghost-btn" id={c.id} aria-describedby={c.describedBy} aria-expanded={cdnOpen} aria-controls="dl-bilibili-cdn-block" onclick={() => (cdnOpen = !cdnOpen)}>
            {cdnOpen ? $t("settings.download.bilibili_naming_hide") : $t("settings.download.bilibili_naming_edit")}
          </button>
        {/snippet}
      </SettingRow>
      <div class="naming-block" id="dl-bilibili-cdn-block" hidden={!cdnOpen}>
        <label class="naming-row" for="dl-bilibili-cdn-hosts">
          <span class="naming-label">{$t("settings.download.bilibili_cdn_hosts_label")}</span>
        </label>
        <textarea class="naming-input" id="dl-bilibili-cdn-hosts" rows="3" dir="ltr" spellcheck="false" bind:value={bilibiliText.value.bilibili_cdn_hosts} placeholder={$t("settings.download.bilibili_cdn_hosts_placeholder") as string}></textarea>
        <SettingRow id="dl-bilibili-cdn-prefer" label={$t("settings.download.bilibili_cdn_prefer_label")} help={$t("settings.download.bilibili_cdn_prefer_desc")} settingKey="settings.download.bilibili_cdn_prefer_label" status="download.bilibili_cdn_prefer_alternatives">
          {#snippet control(c)}{@render sw(c, $t("settings.download.bilibili_cdn_prefer_label"), d.bilibili_cdn_prefer_alternatives, () => toggleBool("download", "bilibili_cdn_prefer_alternatives", d.bilibili_cdn_prefer_alternatives))}{/snippet}
        </SettingRow>
      </div>
      <SettingsSaveBar dirty={bilibiliText.dirty} saving={bilibiliText.saving} error={bilibiliText.error} onSave={() => { void bilibiliText.save(); }} onDiscard={() => bilibiliText.discard()} />
    </form>
  </SettingGroup>
{/if}

<style>
  .platform-head { margin: var(--space-3) 0 0; font-size: 12px; line-height: 18px; font-weight: 600; color: var(--text-muted); }
  .platform-head:first-child { margin-block-start: 0; }
  .sb-categories { padding-block: var(--space-2); min-inline-size: 0; }
  .sb-categories-label { margin: 0; font-size: 13px; line-height: 20px; color: var(--text); }
  .sb-categories-help { margin: 2px 0 var(--space-2); font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .bilibili-text-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .naming-block { display: flex; flex-direction: column; gap: var(--space-1); padding-block: var(--space-2); min-inline-size: 0; }
  .naming-block[hidden] { display: none; }
  .naming-block .naming-input { inline-size: 100%; min-inline-size: 0; box-sizing: border-box; margin-block-end: var(--space-2); }
</style>
