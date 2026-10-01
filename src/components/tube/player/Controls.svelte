<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtDuration } from "$lib/tube/format";
  import { patchTubePrefs, tubePrefs } from "$lib/tube/prefs.svelte";
  import {
    expandMini,
    hasPrev,
    jumpToHighlight,
    next,
    prev,
    seek,
    setVolume,
    toggle,
    toggleCaptions,
    toggleFullscreen,
    toggleMute,
    togglePip,
    toggleTheater,
    vplayer,
  } from "$lib/tube/video.svelte";
  import { back } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import Scrubber from "./Scrubber.svelte";
  import SettingsMenu from "./SettingsMenu.svelte";

  let {
    show = true,
    menuOpen = $bindable(false),
    onhover,
  }: { show?: boolean; menuOpen?: boolean; onhover?: (over: boolean) => void } = $props();

  let remaining = $state(false);
  let volOpen = $state(false);
  let volDrag = $state(false);
  let volEl = $state<HTMLDivElement>();

  let live = $derived(!!vplayer.streams?.is_live);
  let dur = $derived(vplayer.duration);
  let chapters = $derived([...(vplayer.info?.chapters ?? [])].sort((a, b) => a.start_s - b.start_s));
  let chapter = $derived.by(() => {
    let c = null as (typeof chapters)[number] | null;
    for (const x of chapters) if (x.start_s <= vplayer.position + 0.25) c = x;
    return c;
  });
  let timeText = $derived.by(() => {
    if (remaining && dur > 0) {
      const left = (dur - vplayer.position) / (vplayer.rate || 1);
      return `-${fmtDuration(left)}`;
    }
    return `${fmtDuration(vplayer.position)} / ${fmtDuration(dur)}`;
  });
  let volIcon = $derived(vplayer.muted || vplayer.volume === 0 ? "mute" : vplayer.volume < 0.5 ? "volume-low" : "volume");
  let prefs = $derived(tubePrefs());
  let showHighlight = $derived(!!vplayer.sbHighlight && vplayer.position < vplayer.sbHighlight.start_s - 1);

  function nextChapter() {
    const n = chapters.find((c) => c.start_s > vplayer.position + 0.5);
    if (n) seek(n.start_s);
  }

  function volAt(x: number) {
    if (!volEl) return;
    const r = volEl.getBoundingClientRect();
    setVolume(Math.min(1, Math.max(0, (x - r.left) / r.width)));
  }
  function volDown(e: PointerEvent) {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    volDrag = true;
    volAt(e.clientX);
  }
  function volKey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft" || e.key === "ArrowDown") setVolume(vplayer.volume - 0.05);
    else if (e.key === "ArrowRight" || e.key === "ArrowUp") setVolume(vplayer.volume + 0.05);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function miniplayer() {
    if (vplayer.mini) expandMini();
    else back();
  }
</script>

<div class="ctl" class:show class:short={vplayer.short} role="group" onpointerenter={() => onhover?.(true)} onpointerleave={() => onhover?.(false)}>
  {#if !live}<Scrubber compact={vplayer.short} />{/if}
  <div class="row">
    <div class="side">
      <button class="pill play" onclick={toggle} aria-label={$t(vplayer.playing ? "tube.player.pause" : "tube.player.play") as string} title="{$t(vplayer.playing ? 'tube.player.pause' : 'tube.player.play')} (k)">
        <TIcon name={vplayer.ended ? "rewind" : vplayer.playing ? "pause" : "play"} size={24} />
      </button>
      {#if !vplayer.short}
        <div class="pill pair">
          <button class="ib" onclick={prev} disabled={!hasPrev() && vplayer.position < 3} aria-label={$t("tube.player.prev") as string} title="{$t('tube.player.prev')} (Shift+P)">
            <TIcon name="prev" size={20} />
          </button>
          <button class="ib" onclick={next} disabled={!vplayer.upNext.length} aria-label={$t("tube.player.next") as string} title="{$t('tube.player.next')} (Shift+N)">
            <TIcon name="next" size={20} />
          </button>
        </div>
      {/if}
      {#if live}
        <button class="pill time live" onclick={() => seek(dur)}>
          <i class="dot"></i>{$t("tube.player.live")}
        </button>
      {:else}
        <button class="pill time" onclick={() => (remaining = !remaining)} aria-label={$t("tube.player.time_toggle") as string}>
          {timeText}
        </button>
      {/if}
      {#if chapter && !vplayer.short}
        <button class="pill chap" onclick={nextChapter} title={$t("tube.player.next_chapter") as string}>
          <span class="bullet">•</span><span class="ctext">{chapter.title}</span><TIcon name="chevron-right" size={18} />
        </button>
      {/if}
      {#if showHighlight}
        <button class="pill hl" onclick={jumpToHighlight}>
          <i class="dot" style:background="var(--sb-poi_highlight)"></i>{$t("tube.player.sb.highlight")}
        </button>
      {/if}
    </div>

    <div class="pill right">
      <div
        class="vol"
        class:open={volOpen || volDrag}
        role="group"
        onpointerenter={() => (volOpen = true)}
        onpointerleave={() => (volOpen = false)}
      >
        <button class="ib" onclick={toggleMute} aria-label={$t(vplayer.muted ? "tube.player.unmute" : "tube.player.mute") as string} title="{$t(vplayer.muted ? 'tube.player.unmute' : 'tube.player.mute')} (m)">
          <TIcon name={volIcon} size={22} />
        </button>
        <div
          class="vtrack"
          bind:this={volEl}
          role="slider"
          tabindex="0"
          aria-label={$t("tube.player.volume") as string}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round((vplayer.muted ? 0 : vplayer.volume) * 100)}
          onpointerdown={volDown}
          onpointermove={(e) => volDrag && volAt(e.clientX)}
          onpointerup={() => (volDrag = false)}
          onfocus={() => (volOpen = true)}
          onblur={() => (volOpen = false)}
          onkeydown={volKey}
        >
          <span class="vfill" style:width="{(vplayer.muted ? 0 : vplayer.volume) * 100}%"></span>
          <span class="vknob" style:left="{(vplayer.muted ? 0 : vplayer.volume) * 100}%"></span>
        </div>
      </div>
      {#if !vplayer.short}
        <button
          class="ib auto"
          role="switch"
          aria-checked={prefs.autoplay_next}
          onclick={() => void patchTubePrefs({ autoplay_next: !prefs.autoplay_next })}
          aria-label={$t("tube.player.autoplay") as string}
          title={$t(prefs.autoplay_next ? "tube.player.autoplay_on" : "tube.player.autoplay_off") as string}
        >
          <span class="tog" class:on={prefs.autoplay_next}><i><TIcon name={prefs.autoplay_next ? "play" : "pause"} size={10} /></i></span>
        </button>
      {/if}
      {#if vplayer.streams?.captions.length}
        <button class="ib cc" class:on={!!vplayer.captions} onclick={toggleCaptions} aria-pressed={!!vplayer.captions} aria-label={$t("tube.player.captions") as string} title="{$t('tube.player.captions')} (c)">
          <TIcon name={vplayer.captions ? "captions-fill" : "captions"} size={22} />
        </button>
      {/if}
      <button class="ib gear" class:spin={menuOpen} onclick={() => (menuOpen = !menuOpen)} aria-expanded={menuOpen} aria-haspopup="menu" aria-label={$t("tube.player.settings") as string} title={$t("tube.player.settings") as string}>
        <TIcon name="settings" size={22} />
      </button>
      {#if !vplayer.short}
        <button class="ib" onclick={miniplayer} aria-label={$t("tube.player.miniplayer") as string} title="{$t('tube.player.miniplayer')} (i)">
          <TIcon name="miniplayer" size={22} />
        </button>
        <button class="ib" onclick={togglePip} aria-label={$t("tube.player.pip") as string} title={$t("tube.player.pip") as string}>
          <TIcon name="pip" size={22} />
        </button>
        {#if !vplayer.fullscreen}
          <button class="ib" class:on={vplayer.theater} onclick={toggleTheater} aria-pressed={vplayer.theater} aria-label={$t("tube.player.theater") as string} title="{$t('tube.player.theater')} (t)">
            <TIcon name="theater" size={22} />
          </button>
        {/if}
      {/if}
      <button class="ib" onclick={toggleFullscreen} aria-label={$t(vplayer.fullscreen ? "tube.player.exit_fullscreen" : "tube.player.fullscreen") as string} title="{$t('tube.player.fullscreen')} (f)">
        <TIcon name={vplayer.fullscreen ? "fullscreen-exit" : "fullscreen"} size={22} />
      </button>
    </div>
  </div>
  {#if menuOpen}<SettingsMenu onclose={() => (menuOpen = false)} />{/if}
</div>

<style>
  .ctl {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    padding: 0 12px 12px;
    color: var(--yt-ov-text);
    opacity: 0;
    transition: opacity 250ms var(--yt-ease-standard);
    pointer-events: none;
    z-index: 4;
  }
  .ctl.show {
    opacity: 1;
    pointer-events: auto;
  }
  .ctl::before {
    content: "";
    position: absolute;
    inset: -40px 0 0;
    background: linear-gradient(transparent, rgba(0, 0, 0, 0.35));
    pointer-events: none;
    z-index: -1;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 6px;
    min-width: 0;
  }
  .side {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .pill {
    position: relative;
    display: inline-flex;
    align-items: center;
    height: 48px;
    border: 0;
    border-radius: 28px;
    padding: 0;
    color: inherit;
    background: var(--yt-ov-pill);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    text-shadow: 0 0 2px #000;
    font: 500 14px/20px var(--yt-font);
    white-space: nowrap;
    flex: none;
  }
  button.pill {
    cursor: pointer;
    padding: 0 16px;
  }
  button.pill::before,
  .ib::before {
    content: "";
    position: absolute;
    inset: 4px;
    border-radius: inherit;
    background: transparent;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
    pointer-events: none;
  }
  button.pill:hover::before,
  .ib:hover:not(:disabled)::before {
    background: var(--yt-ov-pill-hover);
  }
  button.pill:active::before,
  .ib:active:not(:disabled)::before {
    background: var(--yt-ov-pill-press);
  }
  button.pill:focus-visible,
  .ib:focus-visible {
    outline: 2px solid #fff;
    outline-offset: -2px;
  }
  .play {
    width: 56px;
    height: 56px;
    padding: 0 !important;
    justify-content: center;
    border-radius: 50%;
  }
  .play :global(svg) {
    filter: drop-shadow(0 0 1px rgba(0, 0, 0, 0.8));
  }
  .ib {
    position: relative;
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: inherit;
    cursor: pointer;
    flex: none;
  }
  .ib:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .pair .ib:first-child {
    border-radius: 28px 0 0 28px;
  }
  .pair .ib:last-child {
    border-radius: 0 28px 28px 0;
  }
  .time {
    font-variant-numeric: tabular-nums;
  }
  .live {
    gap: 8px;
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--yt-live-red);
    margin-right: 6px;
  }
  .live .dot {
    margin: 0;
  }
  .chap {
    gap: 6px;
    min-width: 0;
    flex: 0 1 auto;
  }
  .chap .ctext {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bullet {
    opacity: 0.7;
  }
  .hl {
    gap: 2px;
  }
  .right {
    gap: 0;
    padding: 0 4px;
  }
  .vol {
    display: flex;
    align-items: center;
  }
  .vtrack {
    position: relative;
    width: 0;
    height: 20px;
    margin-right: 0;
    overflow: hidden;
    cursor: pointer;
    outline: none;
    transition:
      width var(--yt-dur-base) var(--yt-ease-standard),
      margin var(--yt-dur-base) var(--yt-ease-standard);
    touch-action: none;
  }
  .vtrack::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 3px;
    margin-top: -1.5px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.3);
  }
  .vol.open .vtrack {
    width: 72px;
    margin-right: 10px;
    overflow: visible;
  }
  .vfill {
    position: absolute;
    left: 0;
    top: 50%;
    height: 3px;
    margin-top: -1.5px;
    border-radius: 2px;
    background: #fff;
  }
  .vknob {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    margin: -6px 0 0 -6px;
    border-radius: 50%;
    background: #fff;
  }
  .tog {
    position: relative;
    display: block;
    width: 30px;
    height: 18px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.3);
    transition: background var(--yt-dur-base);
  }
  .tog i {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: #fff;
    color: #000;
    transition: transform var(--yt-dur-base) var(--yt-ease-standard);
  }
  .tog.on {
    background: rgba(255, 255, 255, 0.55);
  }
  .tog.on i {
    transform: translateX(12px);
  }
  .cc.on::after {
    content: "";
    position: absolute;
    left: 14px;
    right: 14px;
    bottom: 9px;
    height: 2px;
    border-radius: 1px;
    background: var(--yt-accent);
  }
  .gear :global(svg) {
    transition: transform 250ms var(--yt-ease-standard);
  }
  .gear.spin :global(svg) {
    transform: rotate(30deg);
  }
  .ib.on:not(.cc) {
    color: #fff;
  }
  .short .row {
    margin-top: 2px;
  }
  .short .play {
    width: 48px;
    height: 48px;
  }

  @container tplayer (max-width: 720px) {
    .chap,
    .hl {
      display: none;
    }
  }
  @container tplayer (max-width: 560px) {
    .pair,
    .auto {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ctl,
    .vtrack,
    .tog i,
    .gear :global(svg) {
      transition: none;
    }
  }
</style>
