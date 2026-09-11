<script lang="ts">
  /** Restaurar (cintilação/ruído/banding) e estabilizar. `mode` escolhe a cara. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  let { mode = "restore" }: { mode?: "restore" | "stabilize" } = $props();

  type Item = { input: string; output: string | null; filter: string; bytes_before: number; bytes_after: number; stabilized: boolean; ok: boolean; error: string | null };
  type Result = { items: Item[] };

  let files = $state<string[]>([]);
  let deflicker = $state(false);
  let deband = $state(false);
  let denoise = $state("none");
  let sharpen = $state(false);
  let shakiness = $state(5);
  let smoothing = $state(10);
  let crf = $state(20);
  let preset = $state("medium");
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  let stabilize = $derived(mode === "stabilize");
  let nothing = $derived(!stabilize && !deflicker && !deband && !sharpen && denoise === "none");

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "video-restore") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy || nothing) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_video_restore", {
        opts: { inputs: files, deflicker, deband, denoise, sharpen, stabilize, shakiness, smoothing, crf, preset, output_dir: outDir, suffix: "" },
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
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub">{files.length ? files.map(baseName).slice(0, 4).join(", ") : $t("tools.restore.audio_kept")}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.video))]; }}>{$t("tools.common.add")}</button></div>
      </div>

      {#if stabilize}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.restore.shakiness")} {shakiness} <span class="dim">· {$t("tools.restore.smoothing")} {smoothing}</span></div><div class="group-row-sub">{$t("tools.restore.two_pass")}</div></div>
          <div class="group-row-trailing btn-row">
            <input type="range" min="1" max="10" step="1" bind:value={shakiness} />
            <input type="range" min="2" max="40" step="1" bind:value={smoothing} />
          </div>
        </div>
      {/if}

      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.restore.denoise")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={denoise}>
            <option value="none">{$t("tools.restore.denoise_none")}</option>
            <option value="hqdn3d">{$t("tools.restore.denoise_hqdn3d")}</option>
            <option value="nlmeans">{$t("tools.restore.denoise_nlmeans")}</option>
            <option value="atadenoise">{$t("tools.restore.denoise_atadenoise")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.restore.deflicker")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={deflicker} type="button" role="switch" aria-checked={deflicker} aria-label={$t("tools.restore.deflicker")} onclick={() => (deflicker = !deflicker)}><span class="toggle-knob"></span></button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.restore.deband")}</div></div>
        <div class="group-row-trailing"><button class="toggle" class:on={deband} type="button" role="switch" aria-checked={deband} aria-label={$t("tools.restore.deband")} onclick={() => (deband = !deband)}><span class="toggle-knob"></span></button></div>
      </div>
      {#if !stabilize}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.restore.sharpen")}</div></div>
          <div class="group-row-trailing"><button class="toggle" class:on={sharpen} type="button" role="switch" aria-checked={sharpen} aria-label={$t("tools.restore.sharpen")} onclick={() => (sharpen = !sharpen)}><span class="toggle-knob"></span></button></div>
        </div>
      {/if}

      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.restore.quality")} CRF {crf} <span class="dim">· {$t("tools.restore.effort")}</span></div></div>
        <div class="group-row-trailing btn-row">
          <input type="range" min="14" max="30" step="1" bind:value={crf} />
          <select class="input" bind:value={preset}><option value="veryfast">veryfast</option><option value="medium">medium</option><option value="slow">slow</option></select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{:else if nothing}<div class="group-row-sub">{$t("tools.restore.nothing")}</div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length || nothing} onclick={run}>{busy ? $t("tools.common.working") : stabilize ? $t("tools.restore.run_stabilize") : $t("tools.restore.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section><div class="group">
      {#each result.items as item (item.input)}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{baseName(item.input)}</div>
            {#if item.ok}<div class="group-row-sub">{fmtBytes(item.bytes_before)} → {fmtBytes(item.bytes_after)}</div><div class="group-row-sub mono">{item.filter}</div>
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
