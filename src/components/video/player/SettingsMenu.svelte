<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtSpeed, fmtTime } from "$lib/video/format";
  import {
    openExternal,
    player,
    selectSubtitle,
    setAmbient,
    setAudio,
    setSkipSilence,
    setSpeed,
    setSubStyle,
    subtitleDelay,
    toggleAutoPause,
    type SkipSilence,
  } from "$lib/video/player.svelte";
  import VIcon from "./VIcon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  type Tab = "speed" | "subs" | "audio" | "study";
  let tab = $state<Tab>("speed");
  const PRESETS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];
  const SILENCE: SkipSilence[] = ["off", "skip", "speed"];

  let info = $derived(player.info);
  let subs = $derived(info?.subtitles ?? []);
  let audio = $derived(info?.audio_tracks ?? []);
</script>

<div
  class="menu"
  role="dialog"
  tabindex="-1"
  aria-label={$t("video.player.settings") as string}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onclose();
    }
  }}
>
  <div class="tabs" role="tablist">
    {#each ["speed", "subs", "audio", "study"] as const as id (id)}
      <button role="tab" aria-selected={tab === id} class:on={tab === id} onclick={() => (tab = id)}>{$t(`video.player.tab_${id}`)}</button>
    {/each}
  </div>

  <div class="body">
    {#if tab === "speed"}
      <div class="row between">
        <span class="lbl">{$t("video.player.speed")}</span>
        <span class="val">{fmtSpeed(player.speed)}×</span>
      </div>
      <input
        type="range"
        min="0.25"
        max="4"
        step="0.05"
        value={player.speed}
        oninput={(e) => setSpeed(Number((e.currentTarget as HTMLInputElement).value))}
        aria-label={$t("video.player.speed") as string}
      />
      <div class="chips">
        {#each PRESETS as s (s)}
          <button class="chip" class:on={Math.abs(player.speed - s) < 0.001} onclick={() => setSpeed(s)}>{fmtSpeed(s)}×</button>
        {/each}
      </div>
      <p class="hint">{$t("video.player.speed_hint")}</p>
    {:else if tab === "subs"}
      <label class="field">
        <span class="lbl">{$t("video.player.subtitle_main")}</span>
        <select
          value={player.subs.primary ?? ""}
          onchange={(e) => selectSubtitle((e.currentTarget as HTMLSelectElement).value || null, 1)}
        >
          <option value="">{$t("video.player.off")}</option>
          {#each subs as s (s.id)}
            <option value={s.id}>{s.label || s.lang || s.id}{s.forced ? ` · ${$t("video.player.forced")}` : ""}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span class="lbl">{$t("video.player.subtitle_second")}</span>
        <select
          value={player.subs.secondary ?? ""}
          onchange={(e) => selectSubtitle((e.currentTarget as HTMLSelectElement).value || null, 2)}
        >
          <option value="">{$t("video.player.off")}</option>
          {#each subs as s (s.id)}
            <option value={s.id}>{s.label || s.lang || s.id}</option>
          {/each}
        </select>
      </label>
      <div class="row between">
        <span class="lbl">{$t("video.player.subtitle_delay")}</span>
        <div class="stepper">
          <button onclick={() => subtitleDelay(-100)} aria-label={$t("video.player.delay_minus") as string}>−</button>
          <span class="val">{(player.subs.delayMs / 1000).toFixed(1)} s</span>
          <button onclick={() => subtitleDelay(100)} aria-label={$t("video.player.delay_plus") as string}>+</button>
        </div>
      </div>
      <div class="row between">
        <span class="lbl">{$t("video.player.subtitle_size")}</span>
        <div class="stepper">
          <button onclick={() => setSubStyle({ size: player.subs.size - 10 })} aria-label={$t("video.player.smaller") as string}>−</button>
          <span class="val">{player.subs.size}%</span>
          <button onclick={() => setSubStyle({ size: player.subs.size + 10 })} aria-label={$t("video.player.larger") as string}>+</button>
        </div>
      </div>
      <div class="row between">
        <span class="lbl">{$t("video.player.subtitle_position")}</span>
        <input
          class="short"
          type="range"
          min="2"
          max="40"
          value={player.subs.pos}
          oninput={(e) => setSubStyle({ pos: Number((e.currentTarget as HTMLInputElement).value) })}
          aria-label={$t("video.player.subtitle_position") as string}
        />
      </div>
      <label class="row between toggle">
        <span class="lbl">{$t("video.player.subtitle_bg")}</span>
        <input type="checkbox" checked={player.subs.bg} onchange={(e) => setSubStyle({ bg: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
      {#if !subs.length}<p class="hint">{$t("video.player.no_subtitles_hint")}</p>{/if}
    {:else if tab === "audio"}
      {#if audio.length}
        <div class="list">
          {#each audio as a (a.index)}
            <button class="opt" class:on={player.audioIndex === a.index} onclick={() => setAudio(a.index)}>
              <span>{a.label || a.lang || `#${a.index}`}</span>
              <small>{a.codec}</small>
              {#if player.audioIndex === a.index}<VIcon name="check" size={14} />{/if}
            </button>
          {/each}
        </div>
      {:else}
        <p class="hint">{$t("video.player.single_audio")}</p>
      {/if}
      <span class="lbl sep">{$t("video.player.external")}</span>
      <div class="chips">
        <button class="chip" onclick={() => openExternal(null)}><VIcon name="external" size={12} /> {$t("video.player.external_auto")}</button>
        <button class="chip" onclick={() => openExternal("mpv")}>mpv</button>
        <button class="chip" onclick={() => openExternal("iina")}>IINA</button>
        <button class="chip" onclick={() => openExternal("vlc")}>VLC</button>
      </div>
    {:else}
      <span class="lbl">{$t("video.player.skip_silence")}</span>
      <div class="seg" role="radiogroup" aria-label={$t("video.player.skip_silence") as string}>
        {#each SILENCE as m (m)}
          <button role="radio" aria-checked={player.skipSilence === m} class:on={player.skipSilence === m} onclick={() => setSkipSilence(m)}>
            {$t(`video.player.silence_${m}`)}
          </button>
        {/each}
      </div>
      {#if player.skipSilence !== "off"}
        <p class="hint">{$t("video.player.saved", { time: fmtTime(player.savedMs) })}</p>
      {/if}
      <label class="row between toggle">
        <span class="lbl">{$t("video.player.auto_pause")}</span>
        <input type="checkbox" checked={player.autoPause} onchange={toggleAutoPause} />
      </label>
      <p class="hint">{$t("video.player.auto_pause_hint")}</p>
      <label class="row between toggle">
        <span class="lbl">{$t("video.player.ambient")}</span>
        <input type="checkbox" checked={player.ambient} onchange={(e) => setAmbient((e.currentTarget as HTMLInputElement).checked)} />
      </label>
    {/if}
  </div>
</div>

<style>
  .menu {
    position: absolute;
    right: 12px;
    bottom: calc(100% + 10px);
    width: 300px;
    max-height: min(460px, 70vh);
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pl-shade) 62%, transparent);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: 0 0 0 var(--hairline) color-mix(in srgb, var(--pl-ink) 14%, transparent), 0 18px 40px color-mix(in srgb, var(--pl-shade) 45%, transparent);
    color: var(--pl-ink);
    overflow: hidden;
    z-index: 20;
    outline: none;
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin: 8px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--pl-ink) 10%, transparent);
  }
  .tabs button,
  .seg button {
    all: unset;
    flex: 1;
    text-align: center;
    padding: 5px 0;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    cursor: pointer;
    color: color-mix(in srgb, var(--pl-ink) 78%, transparent);
  }
  .tabs button.on,
  .seg button.on {
    background: color-mix(in srgb, var(--pl-ink) 22%, transparent);
    color: var(--pl-ink);
    font-weight: 600;
  }
  .tabs button:focus-visible,
  .seg button:focus-visible,
  .chip:focus-visible,
  .opt:focus-visible,
  .stepper button:focus-visible {
    outline: var(--focus-ring);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 4px 14px 14px;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .between {
    justify-content: space-between;
  }
  .lbl {
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .lbl.sep {
    margin-top: 6px;
  }
  .val {
    font-variant-numeric: tabular-nums;
    font-size: var(--text-sm);
    min-width: 48px;
    text-align: center;
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--pl-ink) 65%, transparent);
  }
  input[type="range"] {
    width: 100%;
    accent-color: var(--accent);
  }
  input.short {
    width: 120px;
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    width: 16px;
    height: 16px;
  }
  .toggle {
    cursor: pointer;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 9px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--pl-ink) 12%, transparent);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .chip:hover {
    background: color-mix(in srgb, var(--pl-ink) 20%, transparent);
  }
  .chip.on {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 650;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  select {
    height: 30px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--pl-ink) 12%, transparent);
    color: var(--pl-ink);
    font: inherit;
    font-size: var(--text-sm);
  }
  select option {
    color: var(--text);
    background: var(--popup-bg);
  }
  .stepper {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .stepper button {
    all: unset;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--pl-ink) 12%, transparent);
    cursor: pointer;
    font-size: var(--text-md);
  }
  .stepper button:hover {
    background: color-mix(in srgb, var(--pl-ink) 22%, transparent);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .opt {
    all: unset;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: var(--text-sm);
  }
  .opt span {
    flex: 1;
  }
  .opt small {
    opacity: 0.6;
  }
  .opt:hover,
  .opt.on {
    background: color-mix(in srgb, var(--pl-ink) 12%, transparent);
  }
  .seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--pl-ink) 10%, transparent);
  }
</style>
