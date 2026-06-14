<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings, toggleBool } from "../settings-helpers";

  let { embedded = false }: { embedded?: boolean } = $props();

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

  function setSbMode(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { sponsorblock_mode: value } });
  }

  function setBilibiliDanmakuFormat(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { bilibili_danmaku_format: value } });
  }

  function setBilibiliContainer(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { bilibili_container: value } });
  }

  function setBilibiliCoverFormat(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { bilibili_cover_format: value } });
  }

  let namingTemplatesOpen = $state(false);
  let cdnOpen = $state(false);

  async function setBilibiliNamingTemplate(field: string, value: string) {
    await updateSettings({ download: { [field]: value } });
  }

  async function setBilibiliCdnHosts(e: Event) {
    const value = (e.target as HTMLTextAreaElement).value;
    await updateSettings({ download: { bilibili_cdn_hosts: value } });
  }
</script>

{#if settings}
  {#if !embedded}
    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.download.section_per_platform')}</h5>
      <p class="settings-section-hint">{$t('settings.download.section_per_platform_desc')}</p>
    </div>
  {/if}

  <p class="settings-subsection-head">{$t('settings.download.youtube_specific')}</p>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.youtube_sponsorblock')}</span>
          <span class="setting-path">{$t('settings.download.youtube_sponsorblock_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.youtube_sponsorblock} onclick={() => toggleBool("download", "youtube_sponsorblock", settings.download.youtube_sponsorblock)} role="switch" aria-checked={settings.download.youtube_sponsorblock} aria-label={$t('settings.download.youtube_sponsorblock') as string}><span class="toggle-knob"></span></button>
      </div>
      {#if settings.download.youtube_sponsorblock}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.sb_mode')}</span>
            <span class="setting-path">{$t('settings.download.sb_mode_desc')}</span>
          </div>
          <select class="select" value={settings.download.sponsorblock_mode} onchange={setSbMode}>
            <option value="remove">{$t('settings.download.sb_mode_remove')}</option>
            <option value="mark">{$t('settings.download.sb_mode_mark')}</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.sb_categories')}</span>
            <span class="setting-path">{$t('settings.download.sb_categories_desc')}</span>
          </div>
        </div>
        <div class="sb-chips">
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
      {/if}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.split_by_chapters')}</span>
          <span class="setting-path">{$t('settings.download.split_by_chapters_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.split_by_chapters} onclick={() => toggleBool("download", "split_by_chapters", settings.download.split_by_chapters)} role="switch" aria-checked={settings.download.split_by_chapters} aria-label={$t('settings.download.split_by_chapters') as string}><span class="toggle-knob"></span></button>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.live_from_start')}</span>
          <span class="setting-path">{$t('settings.download.live_from_start_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.live_from_start} onclick={() => toggleBool("download", "live_from_start", settings.download.live_from_start)} role="switch" aria-checked={settings.download.live_from_start} aria-label={$t('settings.download.live_from_start') as string}><span class="toggle-knob"></span></button>
      </div>
    </div>

    <p class="settings-subsection-head">{$t('settings.download.bilibili_section')}</p>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.bilibili_container_label')}</span>
          <span class="setting-path">{$t('settings.download.bilibili_container_desc')}</span>
        </div>
        <select class="select" value={settings.download.bilibili_container} onchange={setBilibiliContainer}>
          <option value="mp4">MP4</option>
          <option value="mkv">MKV</option>
        </select>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.bilibili_danmaku_label')}</span>
          <span class="setting-path">{$t('settings.download.bilibili_danmaku_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.bilibili_danmaku_enabled} onclick={() => toggleBool("download", "bilibili_danmaku_enabled", settings.download.bilibili_danmaku_enabled)} role="switch" aria-checked={settings.download.bilibili_danmaku_enabled} aria-label={$t('settings.download.bilibili_danmaku_label') as string}><span class="toggle-knob"></span></button>
      </div>
      {#if settings.download.bilibili_danmaku_enabled}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.bilibili_danmaku_format_label')}</span>
            <span class="setting-path">{$t('settings.download.bilibili_danmaku_format_desc')}</span>
          </div>
          <select class="select" value={settings.download.bilibili_danmaku_format} onchange={setBilibiliDanmakuFormat}>
            <option value="xml">XML</option>
            <option value="ass">ASS</option>
            <option value="json">JSON</option>
          </select>
        </div>
      {/if}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.bilibili_nfo_label')}</span>
          <span class="setting-path">{$t('settings.download.bilibili_nfo_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.bilibili_nfo_enabled} onclick={() => toggleBool("download", "bilibili_nfo_enabled", settings.download.bilibili_nfo_enabled)} role="switch" aria-checked={settings.download.bilibili_nfo_enabled} aria-label={$t('settings.download.bilibili_nfo_label') as string}><span class="toggle-knob"></span></button>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.bilibili_cover_sidecar_label')}</span>
          <span class="setting-path">{$t('settings.download.bilibili_cover_sidecar_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.bilibili_cover_sidecar} onclick={() => toggleBool("download", "bilibili_cover_sidecar", settings.download.bilibili_cover_sidecar)} role="switch" aria-checked={settings.download.bilibili_cover_sidecar} aria-label={$t('settings.download.bilibili_cover_sidecar_label') as string}><span class="toggle-knob"></span></button>
      </div>
      {#if settings.download.bilibili_cover_sidecar}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.bilibili_cover_format_label')}</span>
            <span class="setting-path">{$t('settings.download.bilibili_cover_format_desc')}</span>
          </div>
          <select class="select" value={settings.download.bilibili_cover_format} onchange={setBilibiliCoverFormat}>
            <option value="jpg">JPG</option>
            <option value="png">PNG</option>
            <option value="webp">WebP</option>
            <option value="avif">AVIF</option>
          </select>
        </div>
      {/if}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.bilibili_naming_label')}</span>
          <span class="setting-path">{$t('settings.download.bilibili_naming_desc')}</span>
        </div>
        <button type="button" class="ghost-btn" onclick={() => (namingTemplatesOpen = !namingTemplatesOpen)}>
          {namingTemplatesOpen ? $t('settings.download.bilibili_naming_hide') : $t('settings.download.bilibili_naming_edit')}
        </button>
      </div>
      {#if namingTemplatesOpen}
        <div class="naming-block">
          <label class="naming-row">
            <span class="naming-label">{$t('settings.download.bilibili_naming_video_label')}</span>
            <input class="naming-input" type="text" value={settings.download.bilibili_naming_video} oninput={(e) => setBilibiliNamingTemplate('bilibili_naming_video', (e.target as HTMLInputElement).value)} />
          </label>
          <label class="naming-row">
            <span class="naming-label">{$t('settings.download.bilibili_naming_multi_part_label')}</span>
            <input class="naming-input" type="text" value={settings.download.bilibili_naming_multi_part} oninput={(e) => setBilibiliNamingTemplate('bilibili_naming_multi_part', (e.target as HTMLInputElement).value)} />
          </label>
          <label class="naming-row">
            <span class="naming-label">{$t('settings.download.bilibili_naming_bangumi_label')}</span>
            <input class="naming-input" type="text" value={settings.download.bilibili_naming_bangumi} oninput={(e) => setBilibiliNamingTemplate('bilibili_naming_bangumi', (e.target as HTMLInputElement).value)} />
          </label>
          <label class="naming-row">
            <span class="naming-label">{$t('settings.download.bilibili_naming_cheese_label')}</span>
            <input class="naming-input" type="text" value={settings.download.bilibili_naming_cheese} oninput={(e) => setBilibiliNamingTemplate('bilibili_naming_cheese', (e.target as HTMLInputElement).value)} />
          </label>
          <label class="naming-row">
            <span class="naming-label">{$t('settings.download.bilibili_naming_collection_label')}</span>
            <input class="naming-input" type="text" value={settings.download.bilibili_naming_collection} oninput={(e) => setBilibiliNamingTemplate('bilibili_naming_collection', (e.target as HTMLInputElement).value)} />
          </label>
          <p class="naming-help">{$t('settings.download.bilibili_naming_help')}</p>
        </div>
      {/if}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.bilibili_cdn_label')}</span>
          <span class="setting-path">{$t('settings.download.bilibili_cdn_desc')}</span>
        </div>
        <button type="button" class="ghost-btn" onclick={() => (cdnOpen = !cdnOpen)}>
          {cdnOpen ? $t('settings.download.bilibili_naming_hide') : $t('settings.download.bilibili_naming_edit')}
        </button>
      </div>
      {#if cdnOpen}
        <div class="naming-block">
          <label class="naming-row">
            <span class="naming-label">{$t('settings.download.bilibili_cdn_hosts_label')}</span>
            <textarea class="naming-input" rows="3" value={settings.download.bilibili_cdn_hosts} oninput={setBilibiliCdnHosts} placeholder={$t('settings.download.bilibili_cdn_hosts_placeholder') as string}></textarea>
          </label>
          <div class="setting-row" style="padding: 4px 0;">
            <div class="setting-col">
              <span class="setting-label">{$t('settings.download.bilibili_cdn_prefer_label')}</span>
              <span class="setting-path">{$t('settings.download.bilibili_cdn_prefer_desc')}</span>
            </div>
            <button class="toggle" class:on={settings.download.bilibili_cdn_prefer_alternatives} onclick={() => toggleBool("download", "bilibili_cdn_prefer_alternatives", settings.download.bilibili_cdn_prefer_alternatives)} role="switch" aria-checked={settings.download.bilibili_cdn_prefer_alternatives} aria-label={$t('settings.download.bilibili_cdn_prefer_label') as string}><span class="toggle-knob"></span></button>
          </div>
        </div>
      {/if}
    </div>
{/if}
