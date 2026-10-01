<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtClock, fmtSpeed, fmtTime } from "$lib/video/format";
  import {
    player,
    playNext,
    playPrev,
    seekBy,
    setVolume,
    toggle,
    toggleFullscreen,
    toggleMute,
    togglePanel,
    togglePip,
    clearAb,
  } from "$lib/video/player.svelte";
  import Scrubber from "./Scrubber.svelte";
  import SettingsMenu from "./SettingsMenu.svelte";
  import VIcon from "./VIcon.svelte";

  let { menuOpen = $bindable(false) }: { menuOpen?: boolean } = $props();

  let info = $derived(player.info);
  let live = $derived(!!info?.is_live);
  let rate = $derived(player.hold ? Math.max(2, player.speed) : player.speed);
  let remainingMs = $derived(Math.max(0, (player.duration - player.position) * 1000) / (rate || 1));
  let endsAt = $derived(Date.now() + remainingMs);
  let volIcon = $derived(player.muted || player.volume === 0 ? "mute" : player.volume < 0.5 ? "volume-low" : "volume");
</script>

<div class="bar">
  {#if !live}<Scrubber />{/if}
  <div class="row">
    <div class="grp">
      <button class="ic big" onclick={toggle} aria-label={player.playing ? ($t("video.player.pause") as string) : ($t("video.player.play") as string)}>
        <VIcon name={player.ended ? "replay" : player.playing ? "pause" : "play"} size={22} />
      </button>
      {#if !live}
        <button class="ic" onclick={() => seekBy(-10)} aria-label={$t("video.player.back10") as string} title={$t("video.player.back10") as string}>
          <VIcon name="back10" size={18} />
        </button>
        <button class="ic" onclick={() => seekBy(10)} aria-label={$t("video.player.fwd10") as string} title={$t("video.player.fwd10") as string}>
          <VIcon name="fwd10" size={18} />
        </button>
      {/if}
      {#if info?.prev}
        <button class="ic" onclick={playPrev} aria-label={$t("video.player.prev") as string} title={$t("video.player.prev") as string}><VIcon name="prev" size={16} /></button>
      {/if}
      {#if info?.next}
        <button class="ic" onclick={playNext} aria-label={$t("video.player.next") as string} title={$t("video.player.next") as string}><VIcon name="next" size={16} /></button>
      {/if}
      <div class="vol">
        <button class="ic" onclick={toggleMute} aria-label={$t("video.player.mute") as string}><VIcon name={volIcon} size={18} /></button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={player.muted ? 0 : player.volume}
          oninput={(e) => setVolume(Number((e.currentTarget as HTMLInputElement).value))}
          aria-label={$t("video.player.volume") as string}
        />
      </div>
      {#if live}
        <span class="live">{$t("video.player.live")}</span>
      {:else}
        <span class="time">
          {fmtTime(player.position * 1000)} <span class="dim">/ {fmtTime(player.duration * 1000)}</span>
        </span>
      {/if}
    </div>

    <div class="grp mid">
      {#if !live && player.duration > 0}
        <span class="rem" title={$t("video.player.remaining_hint") as string}>
          −{fmtTime(remainingMs)} · {$t("video.player.ends_at", { time: fmtClock(endsAt) })}
        </span>
      {/if}
      {#if rate !== 1}
        <span class="badge-speed" class:hold={player.hold}>{fmtSpeed(rate)}×</span>
      {/if}
      {#if player.skipSilence !== "off"}
        <span class="pill" title={$t("video.player.skip_silence") as string}><VIcon name="wave" size={12} /> {$t("video.player.saved_short", { time: fmtTime(player.savedMs) })}</span>
      {/if}
      {#if player.autoPause}
        <span class="pill"><VIcon name="pause_line" size={12} /> {$t("video.player.auto_pause_short")}</span>
      {/if}
      {#if player.ab.a != null}
        <button class="pill ab" onclick={clearAb} title={$t("video.player.ab_clear") as string}>
          <VIcon name="abloop" size={12} />
          {fmtTime(player.ab.a * 1000)}{player.ab.b != null ? `–${fmtTime(player.ab.b * 1000)}` : "…"}
        </button>
      {/if}
    </div>

    <div class="grp right">
      <div class="menu-anchor">
        <button
          class="ic"
          class:on={menuOpen}
          onclick={() => (menuOpen = !menuOpen)}
          aria-label={$t("video.player.settings") as string}
          aria-expanded={menuOpen}
          title={$t("video.player.settings") as string}
        >
          <VIcon name="settings" size={18} />
        </button>
        {#if menuOpen}<SettingsMenu onclose={() => (menuOpen = false)} />{/if}
      </div>
      <button class="ic" onclick={() => (player.showInfo = !player.showInfo)} class:on={player.showInfo} aria-label={$t("video.player.why_title") as string} title={$t("video.player.why_title") as string}>
        <VIcon name="info" size={18} />
      </button>
      <button class="ic" onclick={() => togglePanel()} class:on={!!player.panel} aria-label={$t("video.panel.toggle") as string} title={$t("video.panel.toggle") as string}>
        <VIcon name="panel" size={18} />
      </button>
      <button class="ic" onclick={togglePip} aria-label={$t("video.player.pip") as string} title={$t("video.player.pip") as string}>
        <VIcon name="pip" size={18} />
      </button>
      <button class="ic" onclick={toggleFullscreen} aria-label={$t("video.player.fullscreen") as string} title={$t("video.player.fullscreen") as string}>
        <VIcon name={player.fullscreen ? "shrink" : "expand"} size={18} />
      </button>
    </div>
  </div>
</div>

<style>
  .bar {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 14px 10px;
    border-radius: var(--radius-2xl);
    background: color-mix(in srgb, var(--pl-shade) 42%, transparent);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: 0 0 0 var(--hairline) color-mix(in srgb, var(--pl-ink) 12%, transparent), 0 12px 30px color-mix(in srgb, var(--pl-shade) 35%, transparent);
    color: var(--pl-ink);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
  }
  .grp {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
  }
  .grp.mid {
    flex: 1;
    justify-content: center;
    gap: 8px;
    overflow: hidden;
  }
  .ic {
    all: unset;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    cursor: pointer;
    color: var(--pl-ink);
    transition: background var(--duration-fast);
  }
  .ic.big {
    width: 40px;
    height: 40px;
  }
  .ic:hover,
  .ic.on {
    background: color-mix(in srgb, var(--pl-ink) 16%, transparent);
  }
  .ic:focus-visible,
  .pill.ab:focus-visible {
    outline: var(--focus-ring);
  }
  .vol {
    display: flex;
    align-items: center;
  }
  .vol input {
    width: 0;
    opacity: 0;
    accent-color: var(--pl-ink);
    transition:
      width var(--duration-base) var(--ease-out),
      opacity var(--duration-base);
  }
  .vol:hover input,
  .vol:focus-within input {
    width: 84px;
    opacity: 1;
  }
  .time {
    margin-left: 6px;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .dim {
    opacity: 0.65;
  }
  .rem {
    font-size: var(--text-xs);
    opacity: 0.8;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .badge-speed {
    padding: 2px 8px;
    border-radius: var(--radius-full);
    background: var(--accent);
    color: var(--on-accent);
    font-size: var(--text-xs);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .badge-speed.hold {
    background: var(--pl-ink);
    color: var(--pl-shade);
  }
  .pill {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--pl-ink) 14%, transparent);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .pill.ab {
    cursor: pointer;
    background: color-mix(in srgb, var(--purple) 45%, transparent);
  }
  .live {
    margin-left: 8px;
    padding: 2px 8px;
    border-radius: var(--radius-xs);
    background: var(--error);
    color: var(--on-error);
    font-size: var(--text-xs);
    font-weight: 750;
    letter-spacing: var(--track-caps);
  }
  .menu-anchor {
    position: relative;
  }
  .menu-anchor :global(.menu) {
    right: -8px;
  }
  @media (max-width: 720px) {
    .grp.mid {
      display: none;
    }
  }
</style>
