<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import {
    getSettings,
    updateSettings,
    chooseFolder,
    toggleBool,
  } from "../settings-helpers";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import SettingsSaveBar from "../rebrand/SettingsSaveBar.svelte";

  type ControlArgs = { id: string; describedBy: string | undefined };

  let { part = "files" }: { part?: "files" | "output" } = $props();

  let settings = $derived(getSettings());

  type PathLimitInfo = { limit: number; current: number; reserve: number; ok: boolean };
  let pathInfo = $state<PathLimitInfo | null>(null);

  $effect(() => {
    if (part !== "files") return;
    const dir = settings?.download.default_output_dir;
    if (!dir) {
      pathInfo = null;
      return;
    }
    let cancelled = false;
    invoke<PathLimitInfo>("validate_output_path", { outputDir: dir })
      .then((info) => {
        if (!cancelled) pathInfo = info;
      })
      .catch(() => {
        if (!cancelled) pathInfo = null;
      });
    return () => {
      cancelled = true;
    };
  });

  const template = createSettingsDraft({
    id: "filename-template",
    read: (s) => ({ filename_template: s.download.filename_template }),
    toPatch: (d) => ({ download: { filename_template: d.filename_template } }),
  });
  let templateText = $derived(template.value.filename_template ?? "");
  let templateInvalid = $derived(!templateText.trim() || !templateText.includes("%(ext)s"));

  function saveTemplate() {
    if (templateInvalid) return;
    void template.save();
  }

  function previewTemplate(value: string): string {
    return value
      .replace("%(title).200s", "My Video Title")
      .replace("%(title)s", "My Video Title")
      .replace("%(id)s", "dQw4w9WgXcQ")
      .replace("%(ext)s", "mp4")
      .replace("%(uploader)s", "Channel Name")
      .replace("%(upload_date)s", "20260217")
      .replace("%(resolution)s", "1920x1080")
      .replace("%(fps)s", "30")
      .replace("%(duration)s", "212");
  }

  let speedNum = $state<number | null>(null);
  let speedUnit = $state<"K" | "M">("M");

  $effect(() => {
    const raw = settings?.download.speed_limit?.trim() ?? "";
    const m = raw.match(/^(\d+(?:\.\d+)?)([KM])?$/i);
    if (m) {
      speedNum = Number(m[1]);
      speedUnit = (m[2]?.toUpperCase() as "K" | "M") ?? "M";
    } else {
      speedNum = null;
    }
  });

  function applySpeedLimit() {
    const value = speedNum && speedNum > 0 ? `${speedNum}${speedUnit}` : "";
    updateSettings({ download: { speed_limit: value } });
  }

  function changeMusicAudioFormat(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { music_audio_format: value } });
  }
</script>

{#snippet sw(c: ControlArgs, label: string, on: boolean, onclick: () => void)}
  <button type="button" class="toggle" class:on id={c.id} aria-label={label} aria-describedby={c.describedBy} role="switch" aria-checked={on} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  {@const d = settings.download}
  {#if part === "files"}
    <SettingGroup id="files" title={$t("settings.rebrand.downloads.output")} description={$t("settings.download.section_output_desc")}>
      <SettingRow id="dl-default-output-dir" label={$t("settings.download.default_output_dir")} settingKey="settings.download.default_output_dir" status="download.default_output_dir">
        {#snippet control(c)}
          <div class="dir-control">
            <output class="dir-path" id={c.id} aria-describedby={pathInfo && !pathInfo.ok ? "dl-output-path-warning" : c.describedBy} dir="ltr" title={d.default_output_dir}>{d.default_output_dir}</output>
            <button type="button" class="button" onclick={chooseFolder}>{$t("settings.download.choose_folder")}</button>
          </div>
        {/snippet}
      </SettingRow>
      {#if pathInfo && !pathInfo.ok}
        <p class="path-warning" id="dl-output-path-warning" role="status">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          {$t("settings.download.path_too_long", { current: pathInfo.current, limit: pathInfo.limit })}
        </p>
      {/if}
      <SettingRow id="dl-always-ask-path" label={$t("settings.download.always_ask_path")} help={$t("settings.download.always_ask_path_desc")} settingKey="settings.download.always_ask_path" status="download.always_ask_path">
        {#snippet control(c)}{@render sw(c, $t("settings.download.always_ask_path"), d.always_ask_path, () => toggleBool("download", "always_ask_path", d.always_ask_path))}{/snippet}
      </SettingRow>
      <SettingRow id="dl-skip-existing" label={$t("settings.download.skip_existing")} help={$t("settings.download.skip_existing_desc")} settingKey="settings.download.skip_existing" status="download.skip_existing">
        {#snippet control(c)}{@render sw(c, $t("settings.download.skip_existing"), d.skip_existing, () => toggleBool("download", "skip_existing", d.skip_existing))}{/snippet}
      </SettingRow>
    </SettingGroup>
  {:else}
    <SettingGroup id="output" title={$t("settings.download.section_output")} description={$t("settings.download.section_output_desc")} advanced>
      <SettingRow id="dl-organize-by-platform" label={$t("settings.download.organize_by_platform")} help={$t("settings.download.organize_by_platform_desc")} settingKey="settings.download.organize_by_platform" status="download.organize_by_platform">
        {#snippet control(c)}{@render sw(c, $t("settings.download.organize_by_platform"), d.organize_by_platform, () => toggleBool("download", "organize_by_platform", d.organize_by_platform))}{/snippet}
      </SettingRow>

      <form class="template-form" onsubmit={(e) => { e.preventDefault(); saveTemplate(); }}>
        <SettingRow id="dl-filename-template" label={$t("settings.download.filename_template")} help={$t("settings.download.filename_template_desc")} settingKey="settings.download.filename_template">
          {#snippet control(c)}
            <input
              id={c.id}
              type="text"
              class="input-template"
              dir="ltr"
              bind:value={template.value.filename_template}
              spellcheck="false"
              aria-invalid={templateInvalid}
              aria-describedby={[c.describedBy, "dl-filename-template-feedback"].filter(Boolean).join(" ")}
            />
          {/snippet}
        </SettingRow>
        <div id="dl-filename-template-feedback" class="template-feedback">
          {#if templateInvalid}
            <span class="template-error" role="alert">{$t("settings.download.filename_template_invalid")}</span>
          {:else if templateText}
            <span class="setting-path" dir="ltr">{$t("settings.download.filename_template_preview", { preview: previewTemplate(templateText) })}</span>
          {/if}
        </div>
        <SettingsSaveBar dirty={template.dirty} saving={template.saving} error={template.error} onSave={saveTemplate} onDiscard={() => template.discard()} />
      </form>

      <SettingRow id="dl-speed-limit" label={$t("settings.download.speed_limit")} help={$t("settings.download.speed_limit_desc")} settingKey="settings.download.speed_limit" status="download.speed_limit">
        {#snippet control(c)}
          <div class="speed-limit">
            <input
              id={c.id}
              type="number"
              class="downloads-speed-input"
              min="0"
              step="1"
              inputmode="numeric"
              placeholder={$t("settings.download.speed_limit_unlimited") as string}
              bind:value={speedNum}
              onchange={applySpeedLimit}
              aria-describedby={c.describedBy}
            />
            <select class="select" bind:value={speedUnit} onchange={applySpeedLimit} aria-label={`${$t("settings.download.speed_limit")}: KB/s, MB/s`}>
              <option value="K">KB/s</option>
              <option value="M">MB/s</option>
            </select>
          </div>
        {/snippet}
      </SettingRow>
      <SettingRow id="dl-music-audio-format" label={$t("settings.download.music_audio_format")} help={$t("settings.download.music_audio_format_desc")} settingKey="settings.download.music_audio_format" status="download.music_audio_format">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.music_audio_format} onchange={changeMusicAudioFormat}>
            <option value="m4a">M4A (AAC)</option>
            <option value="mp3">MP3</option>
            <option value="flac">FLAC (lossless)</option>
            <option value="opus">Opus</option>
            <option value="wav">WAV</option>
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="dl-embed-metadata" label={$t("settings.download.embed_metadata")} help={$t("settings.download.embed_metadata_desc")} settingKey="settings.download.embed_metadata" status="download.embed_metadata">
        {#snippet control(c)}{@render sw(c, $t("settings.download.embed_metadata"), d.embed_metadata, () => toggleBool("download", "embed_metadata", d.embed_metadata))}{/snippet}
      </SettingRow>
      <SettingRow id="dl-embed-thumbnail" label={$t("settings.download.embed_thumbnail")} help={$t("settings.download.embed_thumbnail_desc")} settingKey="settings.download.embed_thumbnail" status="download.embed_thumbnail">
        {#snippet control(c)}{@render sw(c, $t("settings.download.embed_thumbnail"), d.embed_thumbnail, () => toggleBool("download", "embed_thumbnail", d.embed_thumbnail))}{/snippet}
      </SettingRow>
      <SettingRow id="dl-write-nfo-sidecar" label={$t("settings.download.write_nfo_sidecar")} help={$t("settings.download.write_nfo_sidecar_desc")} settingKey="settings.download.write_nfo_sidecar" status="download.write_nfo_sidecar">
        {#snippet control(c)}{@render sw(c, $t("settings.download.write_nfo_sidecar"), d.write_nfo_sidecar, () => toggleBool("download", "write_nfo_sidecar", d.write_nfo_sidecar))}{/snippet}
      </SettingRow>
    </SettingGroup>
  {/if}
{/if}

<style>
  .dir-control { display: flex; align-items: center; gap: var(--space-2); min-inline-size: 0; inline-size: 100%; justify-content: flex-end; }
  .dir-path {
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--text-muted);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .path-warning { display: flex; align-items: center; gap: var(--space-1); margin: 0 0 var(--space-2); font-size: 12px; line-height: 18px; color: var(--warning, var(--text)); }
  .template-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .template-feedback { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; min-inline-size: 0; overflow-wrap: anywhere; }
  .template-feedback:empty { display: none; }
  .template-error { color: var(--error); font-size: 13px; line-height: 1.5; }
  .speed-limit { display: flex; gap: var(--space-2); min-inline-size: 0; inline-size: 100%; max-inline-size: 260px; }
  .speed-limit input { flex: 1 1 0; min-inline-size: 4rem; }
  .speed-limit select { flex: 0 0 5.5rem; }
</style>
