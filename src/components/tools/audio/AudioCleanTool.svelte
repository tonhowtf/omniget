<script lang="ts">
  /** Normalizar loudness (duas passagens) e tirar ruído. `mode` escolhe qual. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, pct, pickDir, pickFile, pickFiles, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  let { mode = "loudness" }: { mode?: "loudness" | "denoise" } = $props();

  type Loudness = { input_i: number; input_tp: number; input_lra: number; input_thresh: number; target_offset: number };
  type Item = { input: string; output: string | null; measured: Loudness | null; target_i: number; gain_db: number; filter: string; ok: boolean; error: string | null };
  type Result = { items: Item[] };

  let files = $state<string[]>([]);
  let target = $state("podcast");
  let denoiseMode = $state("fft");
  let strength = $state(50);
  let rnnn = $state("");
  let outDir = $state("");
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "audio-clean") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!files.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_audio_clean", {
        opts: {
          inputs: files,
          mode,
          target,
          denoise_mode: denoiseMode,
          strength,
          rnnn_path: rnnn,
          audio_format: "",
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
        <div class="group-row-content"><div class="group-row-title">{files.length} {$t("tools.common.files")}</div><div class="group-row-sub">{files.length ? files.map(baseName).slice(0, 4).join(", ") : $t("tools.aclean.video_kept")}</div></div>
        <div class="group-row-trailing btn-row">{#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}<button class="btn btn-secondary btn-sm" type="button" onclick={async () => { files = [...files, ...(await pickFiles(FILTERS.media))]; }}>{$t("tools.common.add")}</button></div>
      </div>

      {#if mode === "loudness"}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.aclean.target")}</div><div class="group-row-sub">{$t("tools.aclean.two_pass")}</div></div>
          <div class="group-row-trailing">
            <select class="input" bind:value={target}>
              <option value="streaming">{$t("tools.aclean.target_streaming")}</option>
              <option value="podcast">{$t("tools.aclean.target_podcast")}</option>
              <option value="broadcast">{$t("tools.aclean.target_broadcast")}</option>
            </select>
          </div>
        </div>
      {:else}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.aclean.method")}</div>{#if denoiseMode === "rnnoise" && !rnnn}<div class="group-row-sub">{$t("tools.aclean.no_model")}</div>{/if}</div>
          <div class="group-row-trailing">
            <select class="input" bind:value={denoiseMode}>
              <option value="fft">{$t("tools.aclean.method_fft")}</option>
              <option value="voice">{$t("tools.aclean.method_voice")}</option>
              <option value="nlm">{$t("tools.aclean.method_nlm")}</option>
              <option value="rnnoise">{$t("tools.aclean.method_rnnoise")}</option>
            </select>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.aclean.strength")} {strength}</div></div>
          <div class="group-row-trailing"><input type="range" min="10" max="100" step="5" bind:value={strength} /></div>
        </div>
        {#if denoiseMode === "rnnoise"}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{$t("tools.aclean.model")}</div><div class="group-row-sub mono">{rnnn ? baseName(rnnn) : "—"}</div></div>
            <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "RNNoise", extensions: ["rnnn"] }]); if (f) rnnn = f; }}>{$t("tools.common.choose")}</button></div>
          </div>
        {/if}
      {/if}

      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.common.output_folder")}</div><div class="group-row-sub mono">{outDir || $t("tools.common.same_folder")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div><div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !files.length} onclick={run}>{busy ? $t("tools.common.working") : mode === "loudness" ? $t("tools.aclean.run_loudness") : $t("tools.aclean.run_denoise")}</button></div>
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
                {#if item.measured}
                  {$t("tools.aclean.measured")} {item.measured.input_i.toFixed(1)} LUFS → {item.target_i} LUFS
                  <span class="dim"> · {$t("tools.aclean.gain")} {item.gain_db > 0 ? "+" : ""}{item.gain_db.toFixed(1)} dB</span>
                {:else}
                  <span class="mono">{item.filter}</span>
                {/if}
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
