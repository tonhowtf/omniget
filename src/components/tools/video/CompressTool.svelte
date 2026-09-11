<script lang="ts">
  /** Comprimir para um alvo exato de tamanho, com duas passagens do FFmpeg. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Item = {
    input: string;
    output: string | null;
    bytes_before: number;
    bytes_after: number;
    target_bytes: number;
    video_kbps: number;
    audio_kbps: number;
    duration_s: number;
    ok: boolean;
    error: string | null;
  };
  type Result = { items: Item[] };

  let files = $state<string[]>([]);
  let targetMb = $state(10);
  let codec = $state("h264");
  let audioKbps = $state(0);
  let maxHeight = $state(0);
  let fps = $state(0);
  let preset = $state("medium");
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "video-compress") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function saved(item: Item): number {
    if (!item.bytes_before || !item.bytes_after) return 0;
    return Math.round((1 - item.bytes_after / item.bytes_before) * 100);
  }

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_video_compress", {
        opts: {
          inputs: files,
          target_mb: targetMb,
          codec,
          audio_kbps: audioKbps,
          max_height: maxHeight,
          fps,
          preset,
          output_dir: outDir,
          suffix: "",
        },
      });
      const failed = result.items.filter((i) => !i.ok).length;
      showToast(failed ? "info" : "success", `${result.items.length - failed} ${$t("tools.common.done")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.video))]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vcompress.target")}</div><div class="group-row-sub">{$t("tools.vcompress.target_hint")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0.5" step="0.5" bind:value={targetMb} style:width="6em" />
          <span class="dim">MB</span>
          <select class="input" bind:value={codec}><option value="h264">H.264</option><option value="h265">H.265</option><option value="vp9">VP9</option></select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vcompress.height")} <span class="dim">· {$t("tools.vcompress.audio")}</span></div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={maxHeight}><option value={0}>{$t("tools.vcompress.keep")}</option><option value={1080}>1080p</option><option value={720}>720p</option><option value={480}>480p</option><option value={360}>360p</option></select>
          <select class="input" bind:value={audioKbps}><option value={0}>{$t("tools.vcompress.audio_auto")}</option><option value={128}>128k</option><option value={96}>96k</option><option value={64}>64k</option></select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.vcompress.preset")} <span class="dim">· {$t("tools.vcompress.fps")}</span></div><div class="group-row-sub">{$t("tools.vcompress.two_pass")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={preset} disabled={codec === "vp9"}><option value="veryfast">{$t("tools.vcompress.preset_fast")}</option><option value="medium">{$t("tools.vcompress.preset_medium")}</option><option value="slow">{$t("tools.vcompress.preset_slow")}</option></select>
          <select class="input" bind:value={fps}><option value={0}>{$t("tools.vcompress.keep")}</option><option value={60}>60</option><option value={30}>30</option><option value={24}>24</option></select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.vcompress.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      {#each result.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}
              <div class="group-row-sub">
                {fmtBytes(item.bytes_before)} → <strong>{fmtBytes(item.bytes_after)}</strong>
                {#if saved(item) > 0}<span class="dim"> · {saved(item)}% {$t("tools.vcompress.saved")}</span>{/if}
                <span class="dim"> · {$t("tools.vcompress.bitrate", { kbps: item.video_kbps })}</span>
                {#if item.bytes_after > item.target_bytes}<span class="tag tag-danger">{$t("tools.vcompress.over")}</span>{/if}
              </div>
            {:else}
              <div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>
            {/if}
          </div>
          {#if item.ok && item.output}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(item.output!)}>{$t("tools.common.reveal")}</button></div>{/if}
        </div>
      {/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
