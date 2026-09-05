<script lang="ts">
  /** Upscale com Real-ESRGAN ncnn Vulkan (estudo 37). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, pct, pickDir, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Status = { installed: boolean; path: string | null; models: string[] };
  type Result = { outputs: string[]; failed: string[] };
  let status = $state<Status | null>(null);
  let files = $state<string[]>([]);
  let model = $state("realesrgan-x4plus");
  let scale = $state(4);
  let format = $state("png");
  let outDir = $state("");
  let busy = $state<string | null>(null);
  let progress = $state<Record<string, ToolProgress>>({});
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  async function refresh() { status = await invoke<Status>("tool_upscale_status"); if (status.models.length && !status.models.includes(model)) model = status.models[0]; }
  onMount(async () => { await refresh(); unlisten = await onToolProgress((p) => { progress = { ...progress, [p.id]: p }; }); });
  onDestroy(() => unlisten?.());

  async function install() {
    busy = "install";
    try { await invoke("tool_upscale_install"); showToast("success", $t("tools.common.installed") as string); }
    catch (e) { showToast("error", errText(e)); } finally { busy = null; await refresh(); }
  }

  async function run() {
    if (!files.length || busy) return;
    busy = "run"; result = null;
    try {
      result = await invoke<Result>("tool_upscale_run", { opts: { inputs: files, model, scale, format, output_dir: outDir, tile_size: 0 } });
      showToast(result.failed.length ? "info" : "success", `${result.outputs.length} ${$t("tools.common.done")}`);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }
</script>

<div class="tool">
  <section>
    <span class="group-label">Real-ESRGAN</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.upscale.engine")}</div><div class="group-row-sub">{#if !status}…{:else if status.installed}<span class="mono">{status.path}</span>{:else}{$t("tools.common.not_installed")} · {$t("tools.upscale.gpu_note")}{/if}</div>
          {#if busy === "install" && progress["realesrgan-ncnn-vulkan"]}<div class="progress"><div class="progress-fill" style:width="{pct(progress['realesrgan-ncnn-vulkan']) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing">{#if status && !status.installed}<button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={install}>{busy === "install" ? $t("tools.common.installing") : $t("tools.common.install")}</button>{/if}</div>
      </div>
    </div>
  </section>
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.images))]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.upscale.model")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={model} disabled={!status?.models.length}>{#each status?.models ?? [] as m (m)}<option value={m}>{m}</option>{/each}</select>
          <select class="input" bind:value={scale}><option value={2}>2×</option><option value={3}>3×</option><option value={4}>4×</option></select>
          <select class="input" bind:value={format}><option value="png">png</option><option value="jpg">jpg</option><option value="webp">webp</option></select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy === "run"}{@const cur = Object.values(progress).find((p) => p.id.startsWith("upscale:") && p.stage !== "done")}<div class="group-row-sub">{cur ? `${baseName(cur.id.slice(8))} ${pct(cur) ?? 0}%` : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(cur) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy !== null || !files.length || !status?.installed} onclick={run}>{busy === "run" ? $t("tools.common.working") : $t("tools.upscale.run")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group">
      {#each result.outputs as o (o)}<div class="group-row"><div class="group-row-content"><div class="group-row-sub mono">{o}</div></div><div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(o)}>{$t("tools.common.reveal")}</button></div></div>{/each}
      {#each result.failed as f (f)}<div class="group-row"><div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> <span class="mono">{f}</span></div></div>{/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
