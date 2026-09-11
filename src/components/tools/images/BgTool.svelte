<script lang="ts">
  /** Remover fundo de imagem (img-bg): ONNX Runtime + modelo gerido, tudo local. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtBytes, onToolProgress, openUrl, pct, pickDir, pickFile, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Variant = { id: string; label: string; bytes: number; recommended: boolean };
  type Runtime = {
    installed: boolean;
    path: string | null;
    source: string | null;
    version: string | null;
    target_version: string;
    lib_filename: string;
    can_download: boolean;
    variants: Variant[];
  };
  type Model = {
    id: string;
    name: string;
    family: string;
    bytes: number;
    license: string;
    source: string;
    recommended: boolean;
    downloaded: boolean;
    path: string | null;
  };
  type Status = { runtime: Runtime; models: Model[]; models_dir: string | null };
  type Item = { input: string; output: string | null; width: number; height: number; bytes: number; ok: boolean; error: string | null };
  type Result = { model: string; items: Item[]; done: number; failed: number };

  let status = $state<Status | null>(null);
  let variant = $state("");
  let model = $state("u2netp");
  let files = $state<string[]>([]);
  let inputDir = $state("");
  let outDir = $state("");
  let transparent = $state(true);
  let color = $state("#FFFFFF");
  let format = $state("png");
  let alphaThreshold = $state(0);
  let hardEdges = $state(false);
  let maskOnly = $state(false);
  let busy = $state<string | null>(null);
  let progress = $state<Record<string, ToolProgress>>({});
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const current = $derived(status?.models.find((m) => m.id === model) ?? null);
  const ready = $derived(Boolean(status?.runtime.installed && current?.downloaded));
  const hasInput = $derived(files.length > 0 || inputDir.trim().length > 0);

  async function refresh() {
    status = await invoke<Status>("tool_onnx_status", { family: "rembg" });
    if (!variant) variant = status.runtime.variants.find((v) => v.recommended)?.id ?? status.runtime.variants[0]?.id ?? "";
    if (!status.models.some((m) => m.id === model)) model = status.models.find((m) => m.recommended)?.id ?? status.models[0]?.id ?? model;
  }

  onMount(async () => {
    await refresh();
    unlisten = await onToolProgress((p) => {
      progress = { ...progress, [p.id]: p };
    });
  });
  onDestroy(() => unlisten?.());

  async function installRuntime() {
    busy = "runtime";
    try {
      await invoke("tool_onnx_runtime_install", { variant: variant || null });
      showToast("success", $t("tools.common.installed") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      await refresh();
    }
  }

  async function installRuntimeLocal() {
    const path = await pickFile();
    if (!path) return;
    busy = "runtime";
    try {
      await invoke("tool_onnx_runtime_install_local", { path });
      showToast("success", $t("tools.common.installed") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      await refresh();
    }
  }

  async function downloadModel() {
    busy = "model";
    try {
      await invoke("tool_onnx_model_download", { id: model });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      await refresh();
    }
  }

  async function removeModel(id: string) {
    try {
      await invoke("tool_onnx_model_remove", { id });
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      await refresh();
    }
  }

  async function run() {
    if (!hasInput || busy) return;
    busy = "run";
    result = null;
    try {
      result = await invoke<Result>("tool_img_bg", {
        opts: {
          inputs: files,
          input_dir: inputDir,
          model,
          output_dir: outDir,
          suffix: "",
          background: transparent ? "" : color,
          format,
          quality: 92,
          alpha_threshold: alphaThreshold,
          hard_edges: hardEdges,
          mask_only: maskOnly,
        },
      });
      showToast(result.failed ? "info" : "success", `${result.done} ${$t("tools.common.done")}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      await refresh();
    }
  }
</script>

<div class="tool">
  <section>
    <span class="group-label">{$t("tools.imgbg.engine")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.imgbg.runtime")}</div>
          <div class="group-row-sub">
            {#if !status}…
            {:else if status.runtime.installed}
              <span class="mono">{status.runtime.version ?? status.runtime.path}</span>
            {:else if status.runtime.can_download}
              {$t("tools.common.not_installed")} · {$t("tools.imgbg.runtime_note")}
            {:else}
              {$t("tools.imgbg.runtime_no_build")} <span class="mono">{status.runtime.lib_filename}</span>
            {/if}
          </div>
          {#if busy === "runtime" && progress["onnxruntime"]}
            <div class="progress"><div class="progress-fill" style:width="{pct(progress['onnxruntime']) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if status && !status.runtime.installed}
            {#if status.runtime.variants.length > 1}
              <select class="input" bind:value={variant} disabled={busy !== null}>
                {#each status.runtime.variants as v (v.id)}<option value={v.id}>{v.label} · {fmtBytes(v.bytes)}</option>{/each}
              </select>
            {/if}
            {#if status.runtime.can_download}
              <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={installRuntime}>{busy === "runtime" ? $t("tools.common.installing") : $t("tools.common.install")}</button>
            {/if}
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={installRuntimeLocal}>{$t("tools.imgbg.use_local_lib")}</button>
          {/if}
        </div>
      </div>

      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.imgbg.model")}</div>
          <div class="group-row-sub">
            {#if current}
              {fmtBytes(current.bytes)} · {current.license} ·
              <button class="link" type="button" onclick={() => openUrl(current.source)}>{$t("tools.imgbg.model_source")}</button>
              {#if current.downloaded}<span class="tag tag-success">{$t("tools.common.installed")}</span>{/if}
            {/if}
          </div>
          {#if busy === "model" && progress[`onnx-model:${model}`]}
            <div class="progress"><div class="progress-fill" style:width="{pct(progress[`onnx-model:${model}`]) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={model} disabled={busy !== null}>
            {#each status?.models ?? [] as m (m.id)}<option value={m.id}>{m.name} · {fmtBytes(m.bytes)}</option>{/each}
          </select>
          {#if current && !current.downloaded}
            <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={downloadModel}>{busy === "model" ? $t("tools.common.downloading") : $t("tools.common.download")}</button>
          {:else if current}
            <button class="btn btn-ghost btn-sm" type="button" disabled={busy !== null} onclick={() => removeModel(current.id)}>{$t("tools.common.remove")}</button>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{files.length} {$t("tools.common.files")}</div>
          <div class="group-row-sub mono">{files.map(baseName).slice(0, 5).join(", ")}{files.length > 5 ? "…" : ""}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.images))]; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.imgbg.batch_folder")}</div>
          <div class="group-row-sub mono">{inputDir || $t("tools.common.optional")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if inputDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (inputDir = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) inputDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.imgbg.background")}</div>
          <div class="group-row-sub">{transparent ? $t("tools.imgbg.bg_transparent_note") : $t("tools.imgbg.bg_color_note")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={transparent}>
            <option value={true}>{$t("tools.imgbg.bg_transparent")}</option>
            <option value={false}>{$t("tools.imgbg.bg_color")}</option>
          </select>
          {#if !transparent}<input class="input" type="color" bind:value={color} />{/if}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgbg.format")}</div><div class="group-row-sub">{$t("tools.imgbg.format_note")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={format} disabled={transparent && !maskOnly}>
            <option value="png">PNG</option>
            <option value="jpg">JPEG</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgbg.alpha_threshold")} {alphaThreshold === 0 ? $t("tools.imgbg.off") : alphaThreshold}</div><div class="group-row-sub">{$t("tools.imgbg.alpha_note")}</div></div>
        <div class="group-row-trailing"><input type="range" min="0" max="128" step="4" bind:value={alphaThreshold} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgbg.hard_edges")}</div><div class="group-row-sub">{$t("tools.imgbg.hard_edges_note")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={hardEdges} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.imgbg.mask_only")}</div><div class="group-row-sub">{$t("tools.imgbg.mask_only_note")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={maskOnly} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if outDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (outDir = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy === "run"}
            <div class="group-row-sub mono">{progress["img-bg"]?.message ? baseName(progress["img-bg"].message ?? "") : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress['img-bg']) ?? 0}%"></div></div>
          {:else if !ready}
            <div class="group-row-sub">{$t("tools.imgbg.needs_setup")}</div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy !== null || !hasInput || !ready} onclick={run}>{busy === "run" ? $t("tools.common.working") : $t("tools.imgbg.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{$t("tools.common.result")}</span>
      <div class="group">
        {#each result.items as it (it.input)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub mono">{it.output ?? it.input}</div>
              <div class="group-row-sub">
                {#if it.ok}{it.width}×{it.height} · {fmtBytes(it.bytes)}{:else}<span class="tag tag-danger">{$t("tools.common.failed")}</span> {it.error}{/if}
              </div>
            </div>
            {#if it.ok && it.output}
              <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(it.output ?? "")}>{$t("tools.common.reveal")}</button></div>
            {/if}
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .link { background: none; border: 0; padding: 0; color: var(--accent); cursor: pointer; font: inherit; }
</style>
