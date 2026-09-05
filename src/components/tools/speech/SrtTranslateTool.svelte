<script lang="ts">
  /** Traduzir legendas em lote: IA configurada ou servidor LibreTranslate (estudos 08, 12). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pct, pickFile, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Out = { output_path: string; cues: number; failed: number };

  let srt = $state("");
  let kind = $state<"llm" | "libre_translate">("llm");
  let baseUrl = $state("http://localhost:5000");
  let apiKey = $state("");
  let source = $state("auto");
  let target = $state("pt");
  let context = $state("");
  let bilingual = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Out | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "translate") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function choose() {
    const f = await pickFile(FILTERS.subtitles);
    if (f) srt = f;
  }

  async function run() {
    if (!srt || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      const translator = kind === "llm" ? { kind: "llm" } : { kind: "libre_translate", base_url: baseUrl, api_key: apiKey };
      result = await invoke<Out>("tool_srt_translate", {
        srtPath: srt,
        opts: { translator, source_lang: source === "auto" ? "" : source, target_lang: target, context, batch_size: 25 },
        bilingual,
        outputPath: null,
      });
      showToast(result.failed ? "info" : "success", result.failed ? `${result.failed} ${$t("tools.srtt.failed_lines")}` : ($t("tools.common.done") as string));
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  const LANGS = ["pt", "en", "es", "fr", "de", "it", "ja", "ko", "zh", "ru", "ar", "hi", "nl", "pl", "tr"];
</script>

<div class="tool">
  <section>
    <span class="group-label">{$t("tools.srtt.input")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.srtt.file")}</div>
          <div class="group-row-sub mono">{srt || $t("tools.srtt.file_hint")}</div>
        </div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={choose}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.srtt.from_to")}</div></div>
        <div class="group-row-trailing btn-row">
          <select class="input" bind:value={source}><option value="auto">auto</option>{#each LANGS as l (l)}<option value={l}>{l}</option>{/each}</select>
          <span>→</span>
          <select class="input" bind:value={target}>{#each LANGS as l (l)}<option value={l}>{l}</option>{/each}</select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.srtt.bilingual")}</div><div class="group-row-sub">{$t("tools.srtt.bilingual_hint")}</div></div>
        <div class="group-row-trailing"><input class="checkbox" type="checkbox" bind:checked={bilingual} /></div>
      </div>
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.srtt.engine")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.srtt.provider")}</div></div>
        <div class="group-row-trailing">
          <div class="segmented">
            <button class="segmented-btn" class:active={kind === "llm"} type="button" onclick={() => (kind = "llm")}>{$t("tools.srtt.llm")}</button>
            <button class="segmented-btn" class:active={kind === "libre_translate"} type="button" onclick={() => (kind = "libre_translate")}>LibreTranslate</button>
          </div>
        </div>
      </div>
      {#if kind === "llm"}
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{$t("tools.srtt.context")}</div>
            <div class="group-row-sub">{$t("tools.srtt.context_hint")}</div>
          </div>
          <div class="group-row-trailing"><input class="input" type="text" bind:value={context} placeholder={$t("tools.srtt.context_placeholder")} /></div>
        </div>
        <div class="group-row"><div class="group-row-sub">{$t("tools.srtt.llm_note")} <a href="/settings">{$t("tools.srtt.open_settings")}</a></div></div>
      {:else}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.srtt.server")}</div></div>
          <div class="group-row-trailing"><input class="input" type="url" bind:value={baseUrl} /></div>
        </div>
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">API key</div><div class="group-row-sub">{$t("tools.common.optional")}</div></div>
          <div class="group-row-trailing"><input class="input" type="password" bind:value={apiKey} /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub">{progress ? `${progress.done}/${progress.total ?? "?"}` : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !srt} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.srtt.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <span class="group-label">{$t("tools.common.result")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.cues} {$t("tools.whisper.cues")}{#if result.failed} · {result.failed} {$t("tools.srtt.failed_lines")}{/if}</div>
            <div class="group-row-sub mono">{result.output_path}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.output_path)}>{$t("tools.common.reveal")}</button></div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .segmented-btn.active { background: var(--surface-hi); color: var(--text); }
</style>
