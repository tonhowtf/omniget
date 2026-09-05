<script lang="ts">
  /**
   * Baixar por link: post, foto, vídeo, reel, IGTV, carrossel, story ou
   * highlight. `mode` = "post" (um link), "bulk" (vários) ou "audio"
   * (áudio dos reels).
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";
  import { cancelJob, defaultDownloadOptions, jobId, recall, remember, slugArg, type DownloadResult, type MediaItem } from "$lib/tools/ig.svelte";
  import IgAccountRow from "./IgAccountRow.svelte";
  import IgMediaGrid from "./IgMediaGrid.svelte";

  let { mode = "post" }: { mode?: "post" | "bulk" | "audio" } = $props();
  let url = $state("");
  let bulk = $state("");
  let items = $state<MediaItem[]>([]);
  let busy = $state(false);
  let job = $state("");
  let progress = $state<ToolProgress | null>(null);
  let bulkResult = $state<{ result: DownloadResult; errors: string[]; items: number } | null>(null);
  let dest = $state(recall("dest"));
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (job && p.id === `ig:${job}`) progress = p;
    });
    try {
      const clip = await navigator.clipboard.readText();
      if (mode !== "bulk" && !url && /instagram\.com\//.test(clip)) url = clip.trim();
    } catch {
      /* sem permissão */
    }
  });
  onDestroy(() => unlisten?.());

  async function paste() {
    try {
      const clip = await navigator.clipboard.readText();
      if (mode === "bulk") bulk = bulk ? `${bulk}\n${clip}` : clip;
      else url = clip.trim();
    } catch {
      /* ignore */
    }
  }

  async function fetchItems() {
    if (!url.trim() || busy) return;
    busy = true;
    items = [];
    job = jobId("resolve");
    try {
      items = await invoke<MediaItem[]>("tool_ig_resolve", { slug: slugArg(), url: url.trim(), job });
      if (!items.length) showToast("error", $t("tools.ig.download.nothing") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function runBulk() {
    const urls = bulk.split(/\s+/).map((s) => s.trim()).filter((s) => s.includes("instagram.com"));
    if (!urls.length || busy) return;
    if (!dest) {
      const d = await pickDir();
      if (!d) return;
      dest = d;
    }
    remember("dest", dest);
    busy = true;
    bulkResult = null;
    progress = null;
    job = jobId("bulk");
    const opts = { ...defaultDownloadOptions(), audio_only: mode === "audio" ? "m4a" : defaultDownloadOptions().audio_only, per_user_folder: true };
    try {
      bulkResult = await invoke("tool_ig_download_bulk", { slug: slugArg(), urls, dest, opts, job });
      showToast(bulkResult!.errors.length ? "error" : "success", `${bulkResult!.result.files.length} ${$t("tools.common.files")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  async function loadTxt() {
    const f = await pickFile([{ name: "Text", extensions: ["txt", "csv"] }]);
    if (!f) return;
    try {
      bulk = await invoke<string>("tool_ig_read_text", { path: f });
    } catch (e) {
      showToast("error", errText(e));
    }
  }
</script>

<div class="tool">
  <IgAccountRow />
  {#if mode === "bulk"}
    <section>
      <span class="group-label">{$t("tools.ig.download.bulk_label")}</span>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><textarea class="input" rows="6" bind:value={bulk} placeholder={$t("tools.ig.download.bulk_placeholder")}></textarea></div></div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{dest || $t("tools.common.ask_on_run")}</div></div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            {#if busy}<div class="group-row-sub mono">{progress?.stage ?? ""} {progress?.done ?? 0}{#if progress?.total}/{progress.total}{/if} · {progress?.message ?? ""}</div>{/if}
            {#if bulkResult}
              <div class="group-row-title">{bulkResult.result.files.length} {$t("tools.common.files")} · {bulkResult.items} {$t("tools.ig.grid.items")}{#if bulkResult.errors.length} · {bulkResult.errors.length} {$t("tools.common.failed")}{/if}</div>
              {#each bulkResult.errors as e (e)}<div class="group-row-sub mono">{e}</div>{/each}
            {/if}
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={paste}>{$t("tools.ig.download.paste")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={loadTxt}>{$t("tools.ig.download.from_file")}</button>
            {#if busy}<button class="btn btn-secondary btn-sm" type="button" onclick={() => cancelJob(job)}>{$t("tools.ig.common.cancel")}</button>
            {:else}
              {#if bulkResult}<button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(bulkResult!.result.dest)}>{$t("tools.common.reveal")}</button>{/if}
              <button class="btn btn-primary" type="button" disabled={!bulk.trim()} onclick={runBulk}>{$t("tools.common.download")}</button>
            {/if}
          </div>
        </div>
      </div>
      <p class="group-footer">{$t("tools.ig.download.bulk_hint")}</p>
    </section>
  {:else}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><input class="input" type="url" bind:value={url} placeholder={$t("tools.ig.download.placeholder")} onkeydown={(e) => e.key === "Enter" && fetchItems()} /></div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={paste}>{$t("tools.ig.download.paste")}</button>
            <button class="btn btn-primary" type="button" disabled={busy || !url.trim()} onclick={fetchItems}>{busy ? $t("tools.common.working") : $t("tools.ig.download.fetch")}</button>
          </div>
        </div>
        <div class="group-row"><div class="group-row-sub">{mode === "audio" ? $t("tools.ig.download.audio_hint") : $t("tools.ig.download.hint")}</div></div>
      </div>
    </section>
    <IgMediaGrid {items} jobPrefix="post" audioDefault={mode === "audio" ? "m4a" : ""} />
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  textarea.input { width: 100%; resize: vertical; font-family: var(--font-mono); font-size: var(--text-xs); }
</style>
