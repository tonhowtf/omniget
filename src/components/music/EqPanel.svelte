<script lang="ts">
  import { t } from "$lib/i18n";
  import { EQ_FREQS, EQ_PRESETS, autoPreamp } from "$lib/music/eq";
  import { applyEq, applyReplayGain, applySpeed, prefs, savePrefs, type RgMode, type VisualizerMode } from "$lib/music/player-store.svelte";
  import MusicIcon from "./MusicIcon.svelte";
  import Visualizer from "./Visualizer.svelte";

  function setPreset(id: string) {
    const p = EQ_PRESETS.find((x) => x.id === id);
    if (!p) return;
    prefs.eq = [...p.gains];
    prefs.eqPreset = id;
    prefs.eqPreamp = autoPreamp(p.gains);
    prefs.eqOn = id !== "flat" || prefs.eqOn;
    applyEq();
  }

  function setBand(i: number, v: number) {
    const next = [...prefs.eq];
    next[i] = Math.round(v * 2) / 2;
    prefs.eq = next;
    prefs.eqPreset = "custom";
    applyEq();
  }

  function bandFromPointer(e: PointerEvent, i: number) {
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    const f = 1 - Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
    setBand(i, -12 + f * 24);
  }

  function bandKey(e: KeyboardEvent, i: number) {
    if (e.key === "Home" || e.key === "0") {
      e.preventDefault();
      setBand(i, 0);
      return;
    }
    const d = e.key === "ArrowUp" ? 0.5 : e.key === "ArrowDown" ? -0.5 : 0;
    if (!d) return;
    e.preventDefault();
    setBand(i, Math.max(-12, Math.min(12, prefs.eq[i] + d)));
  }

  const label = (f: number) => (f >= 1000 ? `${f / 1000}k` : String(f));
  const RG: RgMode[] = ["auto", "track", "album", "off"];
  const VIZ: VisualizerMode[] = ["bars", "circle", "wave", "off"];
</script>

<div class="eqp">
  <div class="viz"><Visualizer mode={prefs.visualizer === "off" ? "bars" : prefs.visualizer} bars={40} /></div>

  <section>
    <header>
      <h4>{$t("music.eq.title")}</h4>
      <button class="toggle toggle-sm" class:on={prefs.eqOn} role="switch" aria-checked={prefs.eqOn} aria-label={$t("music.eq.title") as string} onclick={() => { prefs.eqOn = !prefs.eqOn; applyEq(); }}><span class="toggle-knob"></span></button>
    </header>
    <div class="presets">
      {#each EQ_PRESETS as p (p.id)}
        <button class="btn btn-pill btn-sm" class:active={prefs.eqPreset === p.id && prefs.eqOn} onclick={() => setPreset(p.id)}>{$t(`music.eq.preset_${p.id}`)}</button>
      {/each}
    </div>
    <div class="bands" class:off={!prefs.eqOn}>
      {#each EQ_FREQS as f, i (f)}
        <div class="band">
          <span class="db">{prefs.eq[i] > 0 ? "+" : ""}{prefs.eq[i]}</span>
          <div
            class="track"
            role="slider"
            tabindex="0"
            aria-label="{label(f)} Hz"
            aria-valuemin={-12}
            aria-valuemax={12}
            aria-valuenow={prefs.eq[i]}
            onpointerdown={(e) => {
              (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
              bandFromPointer(e, i);
            }}
            onpointermove={(e) => e.buttons && bandFromPointer(e, i)}
            ondblclick={() => setBand(i, 0)}
            onkeydown={(e) => bandKey(e, i)}
          >
            <span class="zero"></span>
            <span
              class="fill"
              style:bottom={prefs.eq[i] >= 0 ? "50%" : `${50 + (prefs.eq[i] / 24) * 100}%`}
              style:height="{(Math.abs(prefs.eq[i]) / 24) * 100}%"
            ></span>
            <span class="knob" style:bottom="{50 + (prefs.eq[i] / 24) * 100}%"></span>
          </div>
          <span class="hz">{label(f)}</span>
        </div>
      {/each}
    </div>
    <label class="row">
      <span>{$t("music.eq.preamp")}</span>
      <input type="range" min="-12" max="6" step="0.5" bind:value={prefs.eqPreamp} oninput={applyEq} />
      <span class="val">{prefs.eqPreamp.toFixed(1)} dB</span>
    </label>
  </section>

  <section>
    <header><h4>{$t("music.rg.title")}</h4></header>
    <div class="mac-segmented seg" role="radiogroup" aria-label={$t("music.rg.title") as string}>
      {#each RG as m (m)}
        <button class="mac-segmented-btn" class:active={prefs.rgMode === m} role="radio" aria-checked={prefs.rgMode === m} onclick={() => { prefs.rgMode = m; applyReplayGain(); }}>{$t(`music.rg.${m}`)}</button>
      {/each}
    </div>
    <p class="hint">{$t(`music.rg.${prefs.rgMode}_hint`)}</p>
    <label class="row">
      <span>{$t("music.rg.preamp")}</span>
      <input type="range" min="-6" max="12" step="0.5" bind:value={prefs.rgPreamp} oninput={applyReplayGain} disabled={prefs.rgMode === "off"} />
      <span class="val">{prefs.rgPreamp > 0 ? "+" : ""}{prefs.rgPreamp.toFixed(1)} dB</span>
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={prefs.preventClip} onchange={applyReplayGain} disabled={prefs.rgMode === "off"} />
      {$t("music.rg.prevent_clip")}
    </label>
  </section>

  <section>
    <header><h4>{$t("music.mix.title")}</h4></header>
    <label class="row">
      <span>{$t("music.mix.crossfade")}</span>
      <input type="range" min="0" max="12" step="1" bind:value={prefs.crossfade} oninput={savePrefs} />
      <span class="val">{prefs.crossfade ? `${prefs.crossfade}s` : $t("music.mix.gapless")}</span>
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={prefs.automix} onchange={savePrefs} />
      <span>{$t("music.mix.automix")} <small>{$t("music.mix.automix_hint")}</small></span>
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={prefs.trimSilence} onchange={savePrefs} />
      <span>{$t("music.mix.trim")} <small>{$t("music.mix.trim_hint")}</small></span>
    </label>
  </section>

  <section>
    <header><h4><MusicIcon name="speed" size={13} /> {$t("music.speed.title")}</h4></header>
    <label class="row">
      <input type="range" min="0.5" max="2" step="0.05" bind:value={prefs.speed} oninput={applySpeed} />
      <span class="val">{prefs.speed.toFixed(2)}×</span>
      {#if prefs.speed !== 1}<button class="btn btn-sm btn-ghost" onclick={() => { prefs.speed = 1; applySpeed(); }}>1×</button>{/if}
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={prefs.keepPitch} onchange={applySpeed} />
      {$t("music.speed.keep_pitch")}
    </label>
  </section>

  <section>
    <header><h4>{$t("music.viz.title")}</h4></header>
    <div class="mac-segmented seg">
      {#each VIZ as v (v)}
        <button class="mac-segmented-btn" class:active={prefs.visualizer === v} onclick={() => { prefs.visualizer = v; savePrefs(); }}>{$t(`music.viz.${v}`)}</button>
      {/each}
    </div>
  </section>
</div>

<style>
  .eqp {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    height: 100%;
    overflow-y: auto;
    padding: var(--space-3);
  }
  .viz {
    height: 56px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    overflow: hidden;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h4 {
    display: flex;
    align-items: center;
    gap: 5px;
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .bands {
    display: grid;
    grid-template-columns: repeat(10, 1fr);
    gap: 4px;
    padding: var(--space-2) 0;
    transition: opacity var(--duration-fast);
  }
  .bands.off {
    opacity: 0.4;
  }
  .band {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .db,
  .hz {
    font-size: var(--text-caption);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .track {
    position: relative;
    width: 100%;
    max-width: 22px;
    height: 120px;
    border-radius: 6px;
    background: repeating-linear-gradient(to top, var(--fill-2) 0 2px, transparent 2px 6px);
    cursor: ns-resize;
    touch-action: none;
  }
  .track:focus-visible {
    outline: var(--focus-ring);
  }
  .zero {
    position: absolute;
    left: 2px;
    right: 2px;
    top: 50%;
    height: 1px;
    background: var(--content-border);
  }
  .fill {
    position: absolute;
    left: 3px;
    right: 3px;
    border-radius: 3px;
    background: var(--accent);
    opacity: 0.8;
  }
  .knob {
    position: absolute;
    left: 50%;
    width: 16px;
    height: 6px;
    margin-bottom: -3px;
    transform: translateX(-50%);
    border-radius: 3px;
    background: var(--text);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .row input[type="range"] {
    flex: 1;
    accent-color: var(--accent);
  }
  .val {
    min-width: 56px;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .check {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .check input {
    accent-color: var(--accent);
    margin-top: 2px;
  }
  .check small {
    display: block;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .seg {
    width: 100%;
  }
</style>
