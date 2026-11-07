<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, saveSettings, chooseFolder, toggleBool } from "../settings-helpers";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import SettingStatus from "../rebrand/SettingStatus.svelte";
  import SettingsSaveBar from "../rebrand/SettingsSaveBar.svelte";
  import GeneralChoice from "../rebrand/GeneralChoice.svelte";
  import ManagerLink from "../rebrand/ManagerLink.svelte";
  import MoreOptions from "./MoreOptions.svelte";
  import Toggle from "./Toggle.svelte";
  import { saveMode } from "./download-options";

  let settings = $derived(getSettings());

  type PathLimitInfo = { limit: number; current: number; reserve: number; ok: boolean };
  let pathInfo = $state<PathLimitInfo | null>(null);

  $effect(() => {
    const dir = settings?.download.default_output_dir;
    if (!dir) {
      pathInfo = null;
      return;
    }
    let cancelled = false;
    invoke<PathLimitInfo>("validate_output_path", { outputDir: dir })
      .then((info) => { if (!cancelled) pathInfo = info; })
      .catch(() => { if (!cancelled) pathInfo = null; });
    return () => { cancelled = true; };
  });

  const template = createSettingsDraft({
    id: "filename-template",
    read: (s) => ({ filename_template: s.download.filename_template }),
    toPatch: (d) => ({ download: { filename_template: d.filename_template } }),
  });
  let templateText = $derived(template.value.filename_template ?? "");
  let templateInvalid = $derived(!templateText.trim() || !templateText.includes("%(ext)s"));
  let moreOpen = $state(false);

  $effect(() => {
    if (template.dirty || template.error) moreOpen = true;
  });

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
    void saveSettings({ download: { speed_limit: value } });
  }
</script>

{#snippet folderArt()}
  <path class="art-folder" d="M22 17h12l3 4h21a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H22a2 2 0 0 1-2-2V19a2 2 0 0 1 2-2z" />
  <path class="art-accent" d="M40 7v15M35 17l5 5 5-5" />
{/snippet}

{#snippet askArt()}
  <rect class="art-window" x="14" y="7" width="52" height="36" rx="3" />
  <path class="art-line" d="M14 14h52" />
  <path class="art-folder" d="M20 20h6l2 2h8v7H20z" />
  <path class="art-folder" d="M20 32h6l2 2h8v5H20z" />
  <rect class="art-button" x="44" y="33" width="16" height="6" rx="2" />
  <path class="art-accent" d="M48 20a4 4 0 1 1 5 4c-1 .5-1 1-1 2M52 29h.01" />
{/snippet}

{#if settings}
  {@const d = settings.download}
  <SettingGroup id="files" icon="folder" title={$t("settings.downloads.files.title")}>
    <SettingRow id="dl-default-output-dir" label={$t("settings.download.default_output_dir")} settingKey="settings.download.default_output_dir" status="download.default_output_dir">
      {#snippet control(c)}
        <div class="dir-control">
          <output class="dir-path" id={c.id} aria-describedby={pathInfo && !pathInfo.ok ? "dl-output-path-warning" : c.describedBy} dir="ltr" title={d.default_output_dir}>{d.default_output_dir}</output>
          <button type="button" class="btn btn-secondary btn-sm" onclick={chooseFolder}>{$t("settings.download.choose_folder")}</button>
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
    <div class="choice-block" data-setting-key="settings.download.always_ask_path">
      <GeneralChoice
        name="dl-save-mode"
        label={$t("settings.download.always_ask_path") as string}
        value={saveMode(d.always_ask_path)}
        options={[
          { value: "folder", draw: folderArt, label: $t("settings.downloads.files.save_here") as string, help: $t("settings.downloads.files.save_here_desc") as string },
          { value: "ask", draw: askArt, label: $t("settings.downloads.files.ask") as string, help: $t("settings.downloads.files.ask_desc") as string },
        ]}
        onchange={(v) => saveSettings({ download: { always_ask_path: v === "ask" } })}
      />
      <SettingStatus path="download.always_ask_path" />
    </div>
    <SettingRow id="dl-organize-by-platform" label={$t("settings.download.organize_by_platform")} help={$t("settings.download.organize_by_platform_desc")} settingKey="settings.download.organize_by_platform" status="download.organize_by_platform">
      {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.organize_by_platform} onclick={() => toggleBool("download", "organize_by_platform", d.organize_by_platform)} />{/snippet}
    </SettingRow>
    <SettingRow id="dl-skip-existing" label={$t("settings.download.skip_existing")} help={$t("settings.download.skip_existing_desc")} settingKey="settings.download.skip_existing" status="download.skip_existing">
      {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.skip_existing} onclick={() => toggleBool("download", "skip_existing", d.skip_existing)} />{/snippet}
    </SettingRow>

    <MoreOptions id="files" bind:open={moreOpen}>
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
      <SettingRow id="dl-playlist-number-files" label={$t("settings.download.playlist_number_files")} help={$t("settings.download.playlist_number_files_desc")} settingKey="settings.download.playlist_number_files" status="download.playlist_number_files">
        {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.playlist_number_files} onclick={() => toggleBool("download", "playlist_number_files", d.playlist_number_files)} />{/snippet}
      </SettingRow>
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
      <div data-setting-key="omnibox.adv.templates_title">
        <ManagerLink href={"/?panel=download-advanced"} titleKey={"omnibox.adv.templates_title"} hintKey={"omnibox.adv.templates_empty"} />
      </div>
    </MoreOptions>
  </SettingGroup>
{/if}

<style>
  .dir-control { display: flex; align-items: center; gap: var(--space-2); min-inline-size: 0; inline-size: 100%; justify-content: flex-end; }
  .dir-path {
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .path-warning { display: flex; align-items: center; gap: var(--space-1); margin: 0 0 var(--space-2); font-size: var(--text-sm); line-height: 18px; color: var(--warning, var(--text)); }
  .choice-block { padding-block: var(--space-1) var(--space-2); min-inline-size: 0; }
  .template-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .template-feedback { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-2); min-inline-size: 0; overflow-wrap: anywhere; }
  .template-feedback:empty { display: none; }
  .template-error { color: var(--error); font-size: var(--text-base); line-height: 1.5; }
  .speed-limit { display: flex; gap: var(--space-2); min-inline-size: 0; inline-size: 100%; max-inline-size: 260px; }
  .speed-limit input { flex: 1 1 0; min-inline-size: 4rem; }
  .speed-limit select { flex: 0 0 5.5rem; }
  .art-folder { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); stroke-linejoin: round; }
  .art-window { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .art-line { fill: none; stroke: var(--content-border); }
  .art-button { fill: var(--accent); opacity: 0.8; }
  .art-accent { fill: none; stroke: var(--accent); stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
</style>
