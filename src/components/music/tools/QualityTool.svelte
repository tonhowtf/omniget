<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { registerSemantic } from "$lib/native-agent/semantic";
  import { t } from "$lib/i18n";
  import { looksUpscaled, qualityLabel } from "$lib/music/library";
  import { library, trackById } from "$lib/music/library-store.svelte";
  import type { Spectrogram } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";

  let { id = null }: { id?: number | null } = $props();

  let chosen = $state<number | null>(null);
  let spec = $state<Spectrogram | null>(null);
  let loading = $state(false);
  let error = $state("");
  let canvas: HTMLCanvasElement | undefined = $state();
  onMount(() =>
    registerSemantic("music-spectrogram", () => ({ ready: !loading, kind: "spectrogram", trackId: chosen, cutoffHz: spec?.cutoff_hz ?? null })),
  );

  $effect(() => {
    chosen = id;
  });

  let lib = $derived(library());
  let track = $derived(chosen != null ? trackById(chosen) : undefined);
  let suspicious = $derived(lib.tracks.filter((x) => looksUpscaled(x)));
  let unanalyzedLossless = $derived(lib.tracks.filter((x) => !x.analyzed && ["FLAC", "ALAC", "WAV", "AIFF", "WavPack", "APE"].includes(x.codec)).length);

  $effect(() => {
    const tid = chosen;
    spec = null;
    error = "";
    if (tid == null) return;
    loading = true;
    invoke<Spectrogram>("music_spectrogram", { id: tid })
      .then((s) => (spec = s))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  function color(v: number): [number, number, number] {
    const x = v / 255;
    const r = Math.min(255, Math.max(0, 255 * (1.5 * x - 0.1)));
    const g = Math.min(255, Math.max(0, 255 * (1.6 * x * x - 0.2)));
    const b = Math.min(255, Math.max(0, 255 * (x < 0.5 ? 1.4 * x : 1.2 - 1.1 * x)));
    return [r, g, b];
  }

  $effect(() => {
    const c = canvas;
    const s = spec;
    if (!c || !s) return;
    const raw = atob(s.data);
    c.width = s.cols;
    c.height = s.rows;
    const g = c.getContext("2d");
    if (!g) return;
    const img = g.createImageData(s.cols, s.rows);
    for (let i = 0; i < raw.length; i++) {
      const [r, gg, b] = color(raw.charCodeAt(i));
      img.data[i * 4] = r;
      img.data[i * 4 + 1] = gg;
      img.data[i * 4 + 2] = b;
      img.data[i * 4 + 3] = 255;
    }
    g.putImageData(img, 0, 0);
  });

  const TICKS = [2000, 5000, 10000, 15000, 16000, 19000, 20000];
</script>

<div class="qt">
  <p class="lead">{$t("music.quality.lead")}</p>

  {#if track}
    <div class="head">
      <div>
        <b>{track.artist} — {track.title}</b>
        <span class="dim">{qualityLabel(track)}</span>
      </div>
      {#if spec?.cutoff_hz}
        <span class="verdict" class:bad={looksUpscaled({ codec: track.codec, cutoff_hz: spec.cutoff_hz, sample_rate: track.sample_rate })}>
          {$t("music.quality.cutoff", { khz: (spec.cutoff_hz / 1000).toFixed(1) })} ·
          {looksUpscaled({ codec: track.codec, cutoff_hz: spec.cutoff_hz, sample_rate: track.sample_rate }) ? $t("music.quality.fake") : $t("music.quality.ok")}
        </span>
      {/if}
    </div>
    <div class="specwrap">
      {#if loading}<div class="loading"><MusicIcon name="loader" size={20} spin /> {$t("music.quality.rendering")}</div>{/if}
      {#if error}<p class="err">{error}</p>{/if}
      {#if spec}
        <canvas bind:this={canvas} data-agent-renderer="music-spectrogram" aria-label={$t("music.quality.spectrogram") as string}></canvas>
        <div class="axis">
          {#each TICKS.filter((f) => f < spec!.nyquist) as f (f)}
            <span style:bottom="{(f / spec.nyquist) * 100}%">{f / 1000}k</span>
          {/each}
        </div>
        {#if spec.cutoff_hz}<span class="cut" style:bottom="{(spec.cutoff_hz / spec.nyquist) * 100}%"></span>{/if}
      {/if}
    </div>
  {:else}
    <p class="dim">{$t("music.quality.pick")}</p>
  {/if}

  <h3>{$t("music.quality.suspicious", { count: suspicious.length })}</h3>
  {#if unanalyzedLossless}
    <p class="dim small">{$t("music.quality.analyze_first", { count: unanalyzedLossless })}</p>
  {/if}
  <ul class="sus">
    {#each suspicious.slice(0, 200) as x (x.id)}
      <li>
        <button class:on={chosen === x.id} onclick={() => (chosen = x.id)}>
          <span class="n">{x.artist} — {x.title}</span>
          <span class="v">{qualityLabel(x)} · {((x.cutoff_hz ?? 0) / 1000).toFixed(1)} kHz</span>
        </button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .qt {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
  }
  .small {
    font-size: var(--text-xs);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .head div {
    display: flex;
    flex-direction: column;
  }
  .verdict {
    padding: 4px 10px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--success) 14%, transparent);
    color: var(--success);
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .verdict.bad {
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
  }
  .specwrap {
    position: relative;
    height: 260px;
    border-radius: var(--radius-md);
    background: #000;
    overflow: hidden;
  }
  canvas {
    width: 100%;
    height: 100%;
    display: block;
    image-rendering: auto;
  }
  .axis span {
    position: absolute;
    left: 6px;
    transform: translateY(50%);
    font-size: var(--text-caption);
    color: rgba(255, 255, 255, 0.75);
    text-shadow: 0 1px 2px #000;
  }
  .cut {
    position: absolute;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255, 255, 255, 0.7);
    box-shadow: 0 0 6px white;
  }
  .loading {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: rgba(255, 255, 255, 0.8);
  }
  .err {
    color: var(--danger);
    padding: var(--space-3);
  }
  h3 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-md);
  }
  .sus {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 320px;
    overflow: auto;
  }
  .sus button {
    all: unset;
    box-sizing: border-box;
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    padding: 5px 8px;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .sus button:hover,
  .sus button.on {
    background: var(--fill-1);
  }
  .sus button:focus-visible {
    outline: var(--focus-ring);
  }
  .n {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .v {
    color: var(--warning);
    font-size: var(--text-xs);
    white-space: nowrap;
  }
</style>
