<script lang="ts">
  /** Texto para voz com o Edge TTS reimplementado em Rust (estudo 09). */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, fmtMs, openPath, reveal, saveAs, FILTERS } from "$lib/tools/rt";

  type Voice = { short_name: string; gender: string; locale: string; friendly_name: string };
  type Result = { audio_path: string; srt_path: string; words: number; duration_ms: number };

  let voices = $state<Voice[]>([]);
  let locale = $state("pt-BR");
  let voice = $state("pt-BR-FranciscaNeural");
  let text = $state("");
  let rate = $state(0);
  let pitch = $state(0);
  let busy = $state(false);
  let result = $state<Result | null>(null);

  let locales = $derived([...new Set(voices.map((v) => v.locale))]);
  let voicesOfLocale = $derived(voices.filter((v) => v.locale === locale));

  onMount(async () => {
    try {
      voices = await invoke<Voice[]>("tool_tts_voices");
      if (!voices.some((v) => v.locale === locale)) locale = voices[0]?.locale ?? "";
      if (!voicesOfLocale.some((v) => v.short_name === voice)) voice = voicesOfLocale[0]?.short_name ?? "";
    } catch (e) {
      showToast("error", errText(e));
    }
  });

  function onLocale() {
    voice = voices.find((v) => v.locale === locale)?.short_name ?? "";
  }

  function sign(n: number, unit: string): string {
    return `${n >= 0 ? "+" : ""}${n}${unit}`;
  }

  async function speak() {
    if (!text.trim() || !voice || busy) return;
    const out = await saveAs("narracao.mp3", FILTERS.audio);
    if (!out) return;
    busy = true;
    result = null;
    try {
      result = await invoke<Result>("tool_tts_speak", {
        opts: { text, voice, rate: sign(rate, "%"), pitch: sign(pitch, "Hz"), volume: "+0%" },
        outputPath: out,
      });
      showToast("success", $t("tools.common.done") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <span class="group-label">{$t("tools.tts.voice")}</span>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tts.language")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={locale} onchange={onLocale} disabled={voices.length === 0}>
            {#each locales as l (l)}<option value={l}>{l}</option>{/each}
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tts.voice")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={voice} disabled={voicesOfLocale.length === 0}>
            {#each voicesOfLocale as v (v.short_name)}<option value={v.short_name}>{v.short_name.replace(`${v.locale}-`, "").replace("Neural", "")} · {v.gender}</option>{/each}
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tts.rate")} <span class="dim">{sign(rate, "%")}</span></div></div>
        <div class="group-row-trailing"><input type="range" min="-50" max="100" step="5" bind:value={rate} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.tts.pitch")} <span class="dim">{sign(pitch, "Hz")}</span></div></div>
        <div class="group-row-trailing"><input type="range" min="-50" max="50" step="5" bind:value={pitch} /></div>
      </div>
    </div>
  </section>

  <section>
    <span class="group-label">{$t("tools.tts.text")}</span>
    <div class="group">
      <div class="group-row">
        <textarea class="input text" rows="8" bind:value={text} placeholder={$t("tools.tts.placeholder")}></textarea>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-sub">{text.length} {$t("tools.tts.chars")} · {$t("tools.tts.free_note")}</div></div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !text.trim() || !voice} onclick={speak}>
            {busy ? $t("tools.common.working") : $t("tools.tts.run")}
          </button>
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
            <div class="group-row-title">{fmtMs(result.duration_ms)} · {result.words} {$t("tools.tts.words")}</div>
            <div class="group-row-sub mono">{result.audio_path}</div>
            <div class="group-row-sub mono">{result.srt_path}</div>
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-primary btn-sm" type="button" onclick={() => openPath(result!.audio_path)}>{$t("tools.common.play")}</button>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.audio_path)}>{$t("tools.common.reveal")}</button>
          </div>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .text { width: 100%; resize: vertical; font-family: inherit; }
</style>
