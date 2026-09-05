<script lang="ts">
  /** .m3u8/.mpd → MP4 pelo FFmpeg, com Referer/cookie (estudos 34/54). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtSecs, onToolProgress, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";

  type Result = { path: string; seconds: number };
  let url = $state("");
  let dest = $state("");
  let name = $state("video");
  let referer = $state("");
  let cookie = $state("");
  let ua = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => { unlisten = await onToolProgress((p) => { if (p.id.startsWith("manifest:")) progress = p; }); });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!url.trim() || busy) return;
    if (!dest) { const d = await pickDir(); if (!d) return; dest = d; }
    busy = true; result = null; progress = null;
    try {
      result = await invoke<Result>("tool_manifest_download", { opts: { url, dest_dir: dest, file_name: name || "video", referer, user_agent: ua, cookie, extra_headers: [] } });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
</script>

<div class="tool">
  <div class="notice notice-info"><div class="notice-text">{$t("tools.manifest.intro")}</div></div>
  <section>
    <div class="group">
      <div class="group-row"><div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder="https://…/playlist.m3u8 · https://…/manifest.mpd" /></div></div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
        <div class="group-row-trailing btn-row"><input class="input" type="text" bind:value={name} placeholder="nome.mp4" style:width="12em" /><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">Referer</div><div class="group-row-sub">{$t("tools.manifest.referer_hint")}</div></div>
        <div class="group-row-trailing"><input class="input" type="url" bind:value={referer} placeholder="https://plataforma.com/aula/…" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">Cookie</div><div class="group-row-sub">{$t("tools.common.optional")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" bind:value={cookie} placeholder="sessao=abc; token=…" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">User-Agent</div><div class="group-row-sub">{$t("tools.common.optional")}</div></div>
        <div class="group-row-trailing"><input class="input mono" type="text" bind:value={ua} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub">{progress?.message ?? "…"}</div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.common.download")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group"><div class="group-row">
      <div class="group-row-content"><div class="group-row-title">{fmtSecs(result.seconds)}</div><div class="group-row-sub mono">{result.path}</div></div>
      <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.path)}>{$t("tools.common.reveal")}</button></div>
    </div></div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
