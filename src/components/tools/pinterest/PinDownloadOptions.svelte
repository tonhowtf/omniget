<script lang="ts">
  /** Opções de download compartilhadas (pasta, imagens/vídeos, WebP, nomes, sync). */
  import { t } from "$lib/i18n";
  import { pickDir } from "$lib/tools/rt";
  import type { DownloadOptions } from "$lib/tools/pinterest";

  let { opts = $bindable(), sections = true, sync = true }: { opts: DownloadOptions; sections?: boolean; sync?: boolean } = $props();
</script>

<div class="group-row">
  <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{opts.dest || $t("tools.common.ask_on_run")}</div></div>
  <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) opts.dest = d; }}>{$t("tools.common.choose")}</button></div>
</div>
<div class="group-row">
  <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.images")} · {$t("tools.pinterest.videos")}</div></div>
  <div class="group-row-trailing btn-row">
    <label class="chk"><input class="checkbox" type="checkbox" bind:checked={opts.images} /> {$t("tools.pinterest.images")}</label>
    <label class="chk"><input class="checkbox" type="checkbox" bind:checked={opts.videos} /> {$t("tools.pinterest.videos")}</label>
  </div>
</div>
<div class="group-row">
  <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.convert_webp")}</div></div>
  <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={opts.convert_webp} /></div>
</div>
<div class="group-row">
  <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.naming")}</div></div>
  <div class="group-row-trailing">
    <select class="input" bind:value={opts.naming}>
      <option value="title-id">{$t("tools.pinterest.naming_title_id")}</option>
      <option value="title">{$t("tools.pinterest.naming_title")}</option>
      <option value="id">{$t("tools.pinterest.naming_id")}</option>
    </select>
  </div>
</div>
<div class="group-row">
  <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.sidecar")}</div></div>
  <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={opts.sidecar} /></div>
</div>
{#if sync}
  <div class="group-row">
    <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.skip_downloaded")}</div></div>
    <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={opts.skip_downloaded} /></div>
  </div>
{/if}
{#if sections}
  <div class="group-row">
    <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.section_folders")}</div></div>
    <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={opts.section_folders} /></div>
  </div>
{/if}

<style>
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .chk { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--text-sm); }
</style>
