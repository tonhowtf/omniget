<script lang="ts">
  /**
   * Transcrever com whisper.cpp local (estudo 01). Três blocos: motor
   * (binário), modelos (download do Hugging Face) e a transcrição em si.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtBytes, onToolProgress, pct, pickDir, pickFile, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type ModelInfo = { id: string; label: string; size_mb: number; note: string; installed: boolean; path: string | null; size_bytes: number };
  type Status = {
    installed: boolean; path: string | null; source: string; version: string | null; variants: string[];
    can_install: boolean; install_hint: string; models: ModelInfo[]; models_dir: string | null;
  };
  type Cue = { start_ms: number; end_ms: number; text: string };
  type Result = { language: string; cues: Cue[]; text: string; srt_path: string; vtt_path: string; txt_path: string; seconds: number };

  let status = $state<Status | null>(null);
  let busy = $state<string | null>(null);
  let progress = $state<Record<string, ToolProgress>>({});
  let variant = $state("cpu");
  let input = $state("");
  let model = $state("");
  let language = $state("auto");
  let translate = $state(false);
  let maxLen = $state(0);
  let prompt = $state("");
  let outputDir = $state("");
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  let installedModels = $derived(status?.models.filter((m) => m.installed) ?? []);
  let canRun = $derived(!!status?.installed && !!model && !!input && busy === null);

  async function refresh() {
    try {
      status = await invoke<Status>("tool_whisper_status");
      if (!model) model = installedModels.find((m) => m.id.startsWith("large-v3-turbo"))?.id ?? installedModels[0]?.id ?? "";
      if (status.variants.length && !status.variants.includes(variant)) variant = status.variants[0];
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function run(id: string, fn: () => Promise<unknown>, ok?: string) {
    if (busy) return;
    busy = id;
    try {
      await fn();
      if (ok) showToast("success", ok);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
      await refresh();
    }
  }

  const install = () => run("install", () => invoke("tool_whisper_install", { variant }), $t("tools.common.installed") as string);
  const download = (id: string) => run(`model:${id}`, () => invoke("tool_whisper_model_download", { id }));
  const remove = (id: string) => run(`rm:${id}`, () => invoke("tool_whisper_model_remove", { id }));

  async function transcribe() {
    result = null;
    await run("transcribe", async () => {
      result = await invoke<Result>("tool_whisper_transcribe", {
        opts: { input, model, language, translate, max_len: maxLen, prompt, output_dir: outputDir, threads: 0 },
      });
    }, $t("tools.common.done") as string);
  }

  async function chooseInput() {
    const f = await pickFile(FILTERS.media);
    if (f) input = f;
  }
  async function chooseOut() {
    const d = await pickDir();
    if (d) outputDir = d;
  }

  onMount(async () => {
    await refresh();
    unlisten = await onToolProgress((p) => {
      progress = { ...progress, [p.id]: p };
    });
  });
  onDestroy(() => unlisten?.());

  const LANGS = ["auto", "pt", "en", "es", "fr", "de", "it", "ja", "ko", "zh", "ru", "ar", "hi"];
</script>

<div class="tool">
  <section>
    <span class="group-label">{$t("tools.whisper.engine")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">whisper.cpp</div>
          <div class="group-row-sub">
            {#if !status}…{:else if status.installed}
              {status.version ?? "?"} · {status.source} · <span class="mono">{status.path}</span>
            {:else}
              {$t("tools.common.not_installed")}{#if status.install_hint} · <span class="mono">{status.install_hint}</span>{/if}
            {/if}
          </div>
          {#if progress["whisper-cli"] && busy === "install"}
            <div class="progress"><div class="progress-fill" style:width="{pct(progress['whisper-cli']) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing btn-row">
          {#if status?.can_install}
            {#if status.variants.length > 1}
              <select class="input" bind:value={variant} disabled={busy !== null}>
                {#each status.variants as v (v)}<option value={v}>{v === "cuda" ? "NVIDIA (cuBLAS)" : "CPU"}</option>{/each}
              </select>
            {/if}
            <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null} onclick={install}>
              {busy === "install" ? $t("tools.common.installing") : status?.installed ? $t("tools.common.update") : $t("tools.common.install")}
            </button>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.whisper.models")}</span>
    <div class="group">
      {#each status?.models ?? [] as m (m.id)}
        {@const p = progress[`whisper-model:${m.id}`]}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{m.label} <span class="dim">· {m.installed ? fmtBytes(m.size_bytes) : `~${m.size_mb} MB`}</span></div>
            <div class="group-row-sub">{m.note}</div>
            {#if busy === `model:${m.id}` && p}
              <div class="progress"><div class="progress-fill" style:width="{pct(p) ?? 0}%"></div></div>
            {/if}
          </div>
          <div class="group-row-trailing">
            {#if m.installed}
              <button class="btn btn-ghost btn-sm" type="button" disabled={busy !== null} onclick={() => remove(m.id)}>{$t("tools.common.remove")}</button>
            {:else}
              <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null} onclick={() => download(m.id)}>
                {busy === `model:${m.id}` ? $t("tools.common.downloading") : $t("tools.common.download")}
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.whisper.transcribe")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.file")}</div>
          <div class="group-row-sub mono">{input || $t("tools.whisper.file_hint")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={chooseInput}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.whisper.model")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={model} disabled={installedModels.length === 0}>
            {#each installedModels as m (m.id)}<option value={m.id}>{m.label}</option>{/each}
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.whisper.language")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={language}>{#each LANGS as l (l)}<option value={l}>{l}</option>{/each}</select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.whisper.max_len")}</div>
          <div class="group-row-sub">{$t("tools.whisper.max_len_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="number" min="0" max="120" bind:value={maxLen} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.whisper.prompt")}</div>
          <div class="group-row-sub">{$t("tools.whisper.prompt_hint")}</div>
        </div>
        <div class="group-row-trailing"><input class="input" type="text" bind:value={prompt} placeholder="Ex.: React, useEffect, Tailwind" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.whisper.translate")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={translate} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{outputDir || $t("tools.common.same_folder")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={chooseOut}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy === "transcribe"}
            {@const p = progress[`transcribe:${input}`]}
            <div class="group-row-sub">{p?.stage === "convert" ? $t("tools.whisper.converting") : $t("tools.whisper.working")} {pct(p) ?? 0}%</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(p) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={!canRun} onclick={transcribe}>{$t("tools.whisper.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{$t("tools.common.result")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.cues.length} {$t("tools.whisper.cues")} · {result.language} · {result.seconds.toFixed(0)}s</div>
            <div class="group-row-sub mono">{result.srt_path}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.srt_path)}>{$t("tools.common.reveal")}</button>
            <a class="btn btn-ghost btn-sm" href="/tools/youtube/yt-workshop">{$t("tools.whisper.open_workshop")}</a>
          </div>
        </div>
        <div class="group-row"><pre class="transcript">{result.text}</pre></div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .progress { margin-top: var(--space-2); }
  .transcript { margin: 0; max-height: 240px; overflow: auto; white-space: pre-wrap; font-size: var(--text-sm); color: var(--text-muted); width: 100%; }
</style>
