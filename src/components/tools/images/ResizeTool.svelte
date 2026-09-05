<script lang="ts">
  /** Redimensionar em lote com FFmpeg (estudo 29). */
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, pickDir, pickFiles, reveal, FILTERS } from "$lib/tools/rt";

  type Result = { outputs: string[]; failed: string[] };
  let files = $state<string[]>([]);
  let mode = $state("width");
  let value = $state(1920);
  let value2 = $state(1080);
  let format = $state("");
  let quality = $state(90);
  let outDir = $state("");
  let busy = $state(false);
  let result = $state<Result | null>(null);

  async function run() {
    if (!files.length || busy) return;
    busy = true; result = null;
    try {
      result = await invoke<Result>("tool_resize", { opts: { inputs: files, mode, value, value2, format, quality, output_dir: outDir, suffix: "" } });
      showToast(result.failed.length ? "info" : "success", `${result.outputs.length} ${$t("tools.common.done")}`);
    } catch (e) { showToast("error", errText(e)); } finally { busy = false; }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.images))]; }}>{$t("tools.common.add")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.resize.mode")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={mode}><option value="width">{$t("tools.resize.width")}</option><option value="height">{$t("tools.resize.height")}</option><option value="fit">{$t("tools.resize.fit")}</option><option value="percent">%</option></select>
          <input class="input" type="number" min="1" bind:value={value} style:width="6em" />
          {#if mode === "fit"}<span>×</span><input class="input" type="number" min="1" bind:value={value2} style:width="6em" />{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.resize.format")} <span class="dim">· {$t("tools.resize.quality")} {quality}</span></div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={format}><option value="">{$t("tools.resize.keep")}</option><option value="jpg">jpg</option><option value="png">png</option><option value="webp">webp</option></select>
          <input type="range" min="40" max="100" step="5" bind:value={quality} />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"></div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.resize.run")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section><div class="group">
      {#if result.outputs.length}<div class="group-row"><div class="group-row-content"><div class="group-row-title">{result.outputs.length} {$t("tools.common.done")}</div><div class="group-row-sub mono">{result.outputs[0]}</div></div><div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.outputs[0])}>{$t("tools.common.reveal")}</button></div></div>{/if}
      {#each result.failed as f (f)}<div class="group-row"><div class="group-row-sub"><span class="tag tag-danger">{$t("tools.common.failed")}</span> <span class="mono">{f}</span></div></div>{/each}
    </div></section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
