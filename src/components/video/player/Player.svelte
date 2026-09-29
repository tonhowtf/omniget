<script lang="ts">
  // Full-window player inside the app. Layers (after limeplay's Player
  // Layout): ambient light, <video>, subtitles, top bar, status, controls,
  // sheets. The side panel sits next to the stage and pushes it.
  import { t } from "$lib/i18n";
  import { fmtTime } from "$lib/video/format";
  import { actionFor } from "$lib/video/keys.svelte";
  import {
    abLoop,
    attach,
    closePlayer,
    cycleSkipSilence,
    cycleSubtitle,
    detach,
    frameStep,
    holdFast,
    jumpLine,
    nextLineNow,
    openExternal,
    pause,
    player,
    playNext,
    playPrev,
    seekBy,
    seekPercent,
    setFullscreen,
    setSpeed,
    setVolume,
    startCard,
    startNote,
    subtitleDelay,
    toggle,
    toggleAutoPause,
    toggleFullscreen,
    toggleMute,
    togglePanel,
    togglePip,
    tryFallback,
  } from "$lib/video/player.svelte";
  import CardComposer from "$components/video/study/CardComposer.svelte";
  import CastPicker from "$components/video/views/CastPicker.svelte";
  import Controls from "./Controls.svelte";
  import InfoPanel from "./InfoPanel.svelte";
  import NextUp from "./NextUp.svelte";
  import ShortcutSheet from "./ShortcutSheet.svelte";
  import SidePanel from "./SidePanel.svelte";
  import SubtitleOverlay from "./SubtitleOverlay.svelte";
  import VIcon from "./VIcon.svelte";

  let video: HTMLVideoElement | undefined = $state();
  let canvas: HTMLCanvasElement | undefined = $state();
  let menuOpen = $state(false);
  let castOpen = $state(false);
  let awake = $state(true);
  let wakeTimer: ReturnType<typeof setTimeout> | null = null;
  let shiftTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    const v = video;
    if (!v) return;
    attach(v);
    return () => detach(v);
  });

  let info = $derived(player.info);
  let title = $derived(info?.title || player.pendingTitle);
  let controlsVisible = $derived(awake || !player.playing || menuOpen || player.showInfo || player.nextIn != null);

  function wake() {
    awake = true;
    if (wakeTimer) clearTimeout(wakeTimer);
    wakeTimer = setTimeout(() => (awake = false), 2600);
  }

  // ambient light: a tiny copy of the frame, blurred behind the picture
  $effect(() => {
    if (!player.ambient || !canvas || !video) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const v = video;
    const c = canvas;
    const id = setInterval(() => {
      if (v.readyState >= 2 && !v.paused) {
        try {
          ctx.drawImage(v, 0, 0, c.width, c.height);
        } catch {
          /* frame not ready */
        }
      }
    }, 200);
    try {
      if (v.readyState >= 2) ctx.drawImage(v, 0, 0, c.width, c.height);
    } catch {
      /* ignore */
    }
    return () => clearInterval(id);
  });

  function typing(e: KeyboardEvent): boolean {
    const el = e.target as HTMLElement | null;
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
  }

  function keydown(e: KeyboardEvent) {
    if (!player.open || player.showKeys || player.cardDraft || castOpen) return;
    if (typing(e) || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "Shift") {
      if (!e.repeat && !shiftTimer) shiftTimer = setTimeout(() => holdFast(true), 280);
      return;
    }
    if (shiftTimer) {
      clearTimeout(shiftTimer);
      shiftTimer = null;
    }
    if (player.hold) holdFast(false);
    if (e.key === "Escape") {
      e.preventDefault();
      if (menuOpen) menuOpen = false;
      else if (player.showInfo) player.showInfo = false;
      else if (player.noteDraft) player.noteDraft = null;
      else if (player.fullscreen) void setFullscreen(false);
      else closePlayer();
      return;
    }
    if (/^[0-9]$/.test(e.key)) {
      seekPercent(Number(e.key) / 10);
      return;
    }
    const action = actionFor(e.key);
    if (!action) return;
    e.preventDefault();
    wake();
    switch (action) {
      case "toggle":
      case "toggle_alt":
        toggle();
        break;
      case "back10":
        seekBy(-10);
        break;
      case "fwd10":
        seekBy(10);
        break;
      case "back5":
        seekBy(-5);
        break;
      case "fwd5":
        seekBy(5);
        break;
      case "vol_up":
        setVolume(player.volume + 0.05);
        break;
      case "vol_down":
        setVolume(player.volume - 0.05);
        break;
      case "mute":
        toggleMute();
        break;
      case "fullscreen":
        toggleFullscreen();
        break;
      case "pip":
        void togglePip();
        break;
      case "frame_back":
        frameStep(-1);
        break;
      case "frame_fwd":
        frameStep(1);
        break;
      case "speed_down":
        setSpeed(player.speed - 0.05);
        break;
      case "speed_up":
        setSpeed(player.speed + 0.05);
        break;
      case "speed_reset":
        setSpeed(1);
        break;
      case "sub_cycle":
        cycleSubtitle();
        break;
      case "sub_delay_minus":
        subtitleDelay(-100);
        break;
      case "sub_delay_plus":
        subtitleDelay(100);
        break;
      case "sub_next_now":
        nextLineNow();
        break;
      case "ab_loop":
        abLoop();
        break;
      case "line_prev":
        jumpLine(-1);
        break;
      case "line_next":
        jumpLine(1);
        break;
      case "line_repeat":
        jumpLine(0);
        break;
      case "auto_pause":
        toggleAutoPause();
        break;
      case "skip_silence":
        cycleSkipSilence();
        break;
      case "note":
        startNote();
        break;
      case "card":
        startCard();
        break;
      case "panel":
        togglePanel();
        break;
      case "info":
        player.showInfo = !player.showInfo;
        break;
      case "next":
        playNext();
        break;
      case "prev":
        playPrev();
        break;
      case "shortcuts":
        player.showKeys = true;
        break;
    }
  }

  function keyup(e: KeyboardEvent) {
    if (e.key !== "Shift") return;
    if (shiftTimer) clearTimeout(shiftTimer);
    shiftTimer = null;
    holdFast(false);
  }
</script>

<svelte:window onkeydown={keydown} onkeyup={keyup} onblur={() => holdFast(false)} />

{#if player.open}
  <div class="pl" class:with-panel={!!player.panel} class:idle={!controlsVisible} role="region" aria-label={$t("video.player.label") as string}>
    <div class="stage" role="presentation" onpointermove={wake} onpointerdown={wake}>
      {#if player.ambient}<canvas class="amb" bind:this={canvas} width="32" height="18" aria-hidden="true"></canvas>{/if}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={video} playsinline preload="auto" onclick={toggle} ondblclick={toggleFullscreen}></video>

      <SubtitleOverlay lifted={controlsVisible} />

      <div class="top" class:hide={!controlsVisible}>
        <button class="ic" onclick={closePlayer} aria-label={$t("video.player.close") as string} title={$t("video.player.close") as string}>
          <VIcon name="back" size={18} />
        </button>
        <div class="tt">
          <strong>{title}</strong>
          {#if info?.subtitle}<span>{info.subtitle}</span>{/if}
        </div>
        {#if info && info.mode !== "direct"}
          <button class="chip" onclick={() => (player.showInfo = !player.showInfo)} title={info.reason}>
            {$t(`video.player.mode_${info.mode}`)}
          </button>
        {/if}
        {#if info && !info.is_live}
          <button
            class="ic"
            onclick={() => {
              pause();
              castOpen = true;
            }}
            aria-label={$t("video.player.cast") as string}
            title={$t("video.player.cast") as string}
          >
            <img src="/icons/video/cast-32.webp" srcset="/icons/video/cast-64.webp 2x" alt="" width="18" height="18" />
          </button>
        {/if}
        <button class="ic" onclick={() => (player.showKeys = true)} aria-label={$t("video.keys.title") as string} title={$t("video.keys.title") as string}>
          <VIcon name="keyboard" size={18} />
        </button>
      </div>

      {#if player.loading || (player.buffering && !player.error)}
        <div class="center" aria-live="polite">
          <VIcon name="loader" size={34} spin />
          {#if player.loading && title}<span>{title}</span>{/if}
        </div>
      {/if}

      {#if player.error}
        <div class="center err" role="alert">
          <p>{player.error}</p>
          <div class="err-acts">
            {#if info && (info.source.kind === "local" || info.source.kind === "url") && info.mode !== "transcode"}
              <button class="btn btn-sm" onclick={() => tryFallback()}>{$t("video.player.try_next_mode")}</button>
            {/if}
            {#if info}<button class="btn btn-sm" onclick={() => openExternal(null)}>{$t("video.player.open_external")}</button>{/if}
            <button class="btn btn-sm" onclick={closePlayer}>{$t("video.player.close")}</button>
          </div>
        </div>
      {/if}

      {#if player.osd}
        {#key player.osd.id}
          <div class="osd" role="status">{player.osd.text}</div>
        {/key}
      {/if}

      {#if player.showInfo}<InfoPanel onclose={() => (player.showInfo = false)} />{/if}
      <NextUp />

      {#if player.ended && !player.info?.next && player.nextIn == null}
        <div class="center done">
          <button class="big" onclick={toggle} aria-label={$t("video.player.replay") as string}><VIcon name="replay" size={28} /></button>
          <span>{$t("video.player.finished", { time: fmtTime(player.duration * 1000) })}</span>
        </div>
      {/if}

      <div class="bottom" class:hide={!controlsVisible}>
        <Controls bind:menuOpen />
      </div>

      {#if player.showKeys}<ShortcutSheet onclose={() => (player.showKeys = false)} />{/if}
      <CardComposer />
      {#if castOpen && info}<CastPicker source={info.source} startMs={Math.round(player.position * 1000)} onclose={() => (castOpen = false)} />{/if}
    </div>

    {#if player.panel}<SidePanel />{/if}
  </div>
{/if}

<style>
  .pl {
    --pl-ink: #fff;
    --pl-shade: #000;
    position: fixed;
    inset: 0;
    z-index: 900;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    background: var(--pl-shade);
    color: var(--pl-ink);
  }
  .pl.with-panel {
    grid-template-columns: minmax(0, 1fr) clamp(300px, 28vw, 400px);
  }
  .stage {
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--pl-shade);
  }
  .idle .stage {
    cursor: none;
  }
  video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: transparent;
    z-index: 1;
  }
  .amb {
    position: absolute;
    inset: -8%;
    width: 116%;
    height: 116%;
    filter: blur(70px) saturate(1.4);
    opacity: 0.55;
    z-index: 0;
    pointer-events: none;
  }
  .top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px 30px;
    padding-top: max(14px, var(--safe-area-top));
    background: linear-gradient(to bottom, color-mix(in srgb, var(--pl-shade) 65%, transparent), transparent);
    z-index: 10;
    transition: opacity var(--duration-slow) var(--ease-out);
  }
  .bottom {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    width: min(96%, 1180px);
    z-index: 10;
    transition:
      opacity var(--duration-slow) var(--ease-out),
      transform var(--duration-slow) var(--ease-out);
  }
  .hide {
    opacity: 0;
    pointer-events: none;
  }
  .bottom.hide {
    transform: translate(-50%, 12px);
  }
  .tt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .tt strong {
    font-family: var(--font-display);
    font-size: var(--text-md);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tt span {
    font-size: var(--text-xs);
    opacity: 0.75;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ic {
    all: unset;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    cursor: pointer;
    background: color-mix(in srgb, var(--pl-ink) 10%, transparent);
  }
  .ic:hover {
    background: color-mix(in srgb, var(--pl-ink) 20%, transparent);
  }
  .ic:focus-visible,
  .chip:focus-visible,
  .big:focus-visible {
    outline: var(--focus-ring);
  }
  .chip {
    all: unset;
    padding: 3px 10px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--blue) 40%, transparent);
    font-size: var(--text-xs);
    font-weight: 650;
    cursor: pointer;
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    pointer-events: none;
    z-index: 5;
    font-size: var(--text-sm);
  }
  .center.err {
    pointer-events: auto;
    background: color-mix(in srgb, var(--pl-shade) 55%, transparent);
  }
  .center.err p {
    max-width: 520px;
    margin: 0;
    padding: 0 20px;
    text-align: center;
    overflow-wrap: anywhere;
  }
  .err-acts {
    display: flex;
    gap: 8px;
  }
  .center.done {
    pointer-events: auto;
    background: color-mix(in srgb, var(--pl-shade) 35%, transparent);
  }
  .big {
    all: unset;
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--pl-ink) 18%, transparent);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    cursor: pointer;
  }
  .osd {
    position: absolute;
    top: 72px;
    left: 50%;
    transform: translateX(-50%);
    padding: 7px 14px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--pl-shade) 62%, transparent);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    font-size: var(--text-sm);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    z-index: 20;
    pointer-events: none;
    animation: osd-in var(--duration-base) var(--ease-out);
  }
  @keyframes osd-in {
    from {
      opacity: 0;
      transform: translate(-50%, -6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .osd {
      animation: none;
    }
    .bottom,
    .top {
      transition: none;
    }
  }
  @media (max-width: 860px) {
    .pl.with-panel {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 1fr) 45%;
    }
  }
</style>
