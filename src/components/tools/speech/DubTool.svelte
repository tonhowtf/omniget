<script lang="ts">
  /** Dublagem a partir de uma legenda: Edge TTS por cue + atempo + mux (estudo 06). */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pct, pickFile, reveal, FILTERS, type ToolProgress } from "$lib/tools/rt";

  type Voice = { short_name: string; gender: string; locale: string };
  type Result = { audio_path: string; video_path: string | null; cues: number; sped_up: number };

  let srt = $state("");
  let video = $state("");
  let voices = $state<Voice[]>([]);
  let locale = $state("pt-BR");
  let voice = $state("pt-BR-AntonioNeural");
  let maxSpeed = $state(1.3);
  let keepOriginal = $state(0.15);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  let locales = $derived([...new Set(voices.map((v) => v.locale))]);
  let voicesOfLocale = $derived(voices.filter((v) => v.locale === locale));

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "dub") progress = p;
    });
    try {
      voices = await invoke<Voice[]>("tool_tts_voices");
      if (!voices.some((v) => v.short_name === voice)) voice = voices.find((v) => v.locale === locale)?.short_name ?? voices[0]?.short_name ?? "";
    } catch (e) {
      showToast("error", errText(e));
    }
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!srt || !voice || busy) return;
    busy = true;
    result = null;
    try {
      result = await invoke<Result>("tool_dub", {
        opts: { srt_path: srt, video_path: video, voice, rate: "+0%", max_speed: maxSpeed, output_dir: "", keep_original_volume: video ? keepOriginal : 0 },
      });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }

  function stageLabel(p: ToolProgress | null): string {
    if (!p) return "…";
    if (p.stage === "synthesize") return `${$t("tools.dub.synth")} ${p.done}/${p.total ?? "?"}`;
    if (p.stage === "mix") return $t("tools.dub.mix") as string;
    if (p.stage === "mux") return $t("tools.dub.mux") as string;
    return p.stage;
  }
</script>

<div class="tool">
  <div class="notice notice-info"><div class="notice-text">{$t("tools.dub.intro")}</div></div>
  <section>
    <span class="group-label">{$t("tools.srtt.input")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.dub.srt")}</div><div class="group-row-sub mono">{srt || $t("tools.dub.srt_hint")}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.subtitles); if (f) srt = f; }}>{$t("tools.common.choose")}</button></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.dub.video")}</div><div class="group-row-sub mono">{video || $t("tools.common.optional")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if video}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (video = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(FILTERS.video); if (f) video = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
    </div>
  </section>
  <section>
    <span class="group-label">{$t("tools.tts.voice")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tts.language")}</div></div>
        <div class="group-row-trailing"><select class="input" bind:value={locale} onchange={() => (voice = voicesOfLocale[0]?.short_name ?? "")}>{#each locales as l (l)}<option value={l}>{l}</option>{/each}</select></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tts.voice")}</div></div>
        <div class="group-row-trailing"><select class="input" bind:value={voice}>{#each voicesOfLocale as v (v.short_name)}<option value={v.short_name}>{v.short_name.replace(`${v.locale}-`, "").replace("Neural", "")} · {v.gender}</option>{/each}</select></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.dub.max_speed")} <span class="dim">{maxSpeed.toFixed(2)}×</span></div><div class="group-row-sub">{$t("tools.dub.max_speed_hint")}</div></div>
        <div class="group-row-trailing"><input type="range" min="1" max="2" step="0.05" bind:value={maxSpeed} /></div>
      </div>
      {#if video}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.dub.keep_original")} <span class="dim">{Math.round(keepOriginal * 100)}%</span></div></div>
          <div class="group-row-trailing"><input type="range" min="0" max="1" step="0.05" bind:value={keepOriginal} /></div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub">{stageLabel(progress)}{#if progress?.message} · {progress.message}{/if}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !srt || !voice} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.dub.run")}</button></div>
      </div>
    </div>
  </section>
  {#if result}
    <section>
      <span class="group-label">{$t("tools.common.result")}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.cues} {$t("tools.whisper.cues")} · {result.sped_up} {$t("tools.dub.sped_up")}</div>
            <div class="group-row-sub mono">{result.video_path ?? result.audio_path}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.video_path ?? result!.audio_path)}>{$t("tools.common.reveal")}</button></div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
