<script lang="ts">
  /** Vídeo → GIF/WebP com paleta em duas passagens. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Item = { input: string; output: string | null; bytes: number; frames: number; ok: boolean; error: string | null };
  type Result = { items: Item[] };

  let files = $state<string[]>([]);
  let format = $state("gif");
  let fps = $state(12);
  let width = $state(480);
  let colors = $state(256);
  let dither = $state("sierra2_4a");
  let quality = $state(75);
  let start = $state(0);
  let duration = $state(0);
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "video-gif") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_video_gif", {
        opts: { inputs: files, start, duration, fps, width, format, dither, max_colors: colors, quality, output_dir: outDir, suffix: "" },
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.gif.format")} <span class="dim">· {$t("tools.gif.size")}</span></div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={format}><option value="gif">GIF</option><option value="webp">WebP</option></select>
          <select class="input" bind:value={width}><option value={0}>{$t("tools.gif.keep")}</option><option value={640}>640</option><option value={480}>480</option><option value={320}>320</option></select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.gif.fps")} {fps}</div><div class="group-row-sub">{format === "gif" ? $t("tools.gif.palette_note") : ""}</div></div>
        <div class="group-row-trailing btn-row"><input type="range" min="5" max="30" step="1" bind:value={fps} /></div>
      </div>
      {#if format === "gif"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.gif.colors")} {colors} <span class="dim">· {$t("tools.gif.dither")}</span></div></div>
          <div class="group-row-trailing btn-row">
            <select class="input" bind:value={colors}><option value={256}>256</option><option value={128}>128</option><option value={64}>64</option><option value={32}>32</option></select>
            <select class="input" bind:value={dither}><option value="sierra2_4a">sierra2_4a</option><option value="bayer">bayer</option><option value="none">{$t("tools.gif.dither_none")}</option></select>
          </div>
        </div>
      {:else}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.gif.quality")} {quality}</div></div>
          <div class="group-row-trailing"><input type="range" min="20" max="100" step="5" bind:value={quality} /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.gif.start")} <span class="dim">· {$t("tools.gif.duration")}</span></div><div class="group-row-sub">{duration ? `${duration}s` : $t("tools.gif.whole")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="number" min="0" step="0.5" bind:value={start} style:width="5em" />
          <input class="input" type="number" min="0" step="0.5" bind:value={duration} style:width="5em" />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.gif.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      {#each result.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}<div class="group-row-sub">{fmtBytes(item.bytes)} <span class="dim">· {item.frames} {$t("tools.gif.frames")}</span></div>
            {:else}<div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> {item.error}</div>{/if}
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
