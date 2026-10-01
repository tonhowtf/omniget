<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { fade } from "svelte/transition";
  import { open as openUrl } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { thumbUrl, videoThumb } from "$lib/tube/format";
  import {
    attachVideo,
    cancelNext,
    detachVideo,
    downloadCurrent,
    leaveWatch,
    loadVideo,
    next,
    retry,
    setWatchInfo,
    toggle,
    toggleFullscreen,
    videoElement,
    vplayer,
  } from "$lib/tube/video.svelte";
  import type { WatchInfo } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import Controls from "./Controls.svelte";
  import SkipToast from "./SkipToast.svelte";

  let {
    videoId,
    list = null,
    index = null,
    t: startT = null,
    info = null,
    short = false,
  }: {
    videoId: string;
    list?: string | null;
    index?: number | null;
    t?: number | null;
    info?: WatchInfo | null;
    short?: boolean;
  } = $props();

  let root = $state<HTMLDivElement>();
  let stage = $state<HTMLDivElement>();
  let amb = $state<HTMLCanvasElement>();
  let idle = $state(false);
  let menuOpen = $state(false);
  let overControls = $state(false);
  let firstFrame = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | null = null;

  let showControls = $derived(!vplayer.playing || !idle || menuOpen || overControls || !!vplayer.error);
  let poster = $derived.by(() => {
    const s = vplayer.streams;
    const u = s ? thumbUrl(s.thumbs, 1280) : "";
    return u || videoThumb(videoId, "hqdefault");
  });
  let upNext = $derived(vplayer.upNext[0] ?? null);

  onMount(() => {
    if (stage) attachVideo(stage);
    vplayer.mini = false;
    const v = videoElement();
    const onFrame = () => (firstFrame = true);
    v?.addEventListener("loadeddata", onFrame);
    v?.addEventListener("playing", onFrame);
    if (v && v.readyState >= 2) firstFrame = true;
    return () => {
      v?.removeEventListener("loadeddata", onFrame);
      v?.removeEventListener("playing", onFrame);
    };
  });

  onDestroy(() => {
    if (stage) detachVideo(stage);
    if (idleTimer) clearTimeout(idleTimer);
    leaveWatch();
  });

  let loadedFor = "";
  $effect(() => {
    const id = videoId;
    if (loadedFor === id) return;
    loadedFor = id;
    if (vplayer.videoId === id && !vplayer.error) {
      vplayer.mini = false;
      vplayer.list = list ?? vplayer.list;
      return;
    }
    firstFrame = false;
    void loadVideo(id, { t: startT, list, index, short });
  });

  $effect(() => {
    const i = info;
    if (i && i.id === videoId) untrack(() => setWatchInfo(i));
  });

  function wake() {
    idle = false;
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), 3000);
  }

  function clickStage(e: MouseEvent) {
    if (e.button !== 0) return;
    if (menuOpen) {
      menuOpen = false;
      return;
    }
    toggle();
  }

  function dbl() {
    toggle();
    toggleFullscreen();
  }

  $effect(() => {
    if (!vplayer.ambient || !amb || short) return;
    const c = amb;
    const ctx = c.getContext("2d", { alpha: false });
    if (!ctx) return;
    let alive = true;
    const draw = () => {
      const v = videoElement();
      if (v && v.readyState >= 2) {
        try {
          ctx.drawImage(v, 0, 0, c.width, c.height);
        } catch {
          alive = false;
        }
      }
    };
    draw();
    const id = setInterval(() => {
      if (alive && (vplayer.playing || !firstFrame)) draw();
    }, 400);
    return () => clearInterval(id);
  });

  async function download() {
    try {
      await downloadCurrent(false);
      showToast("success", $t("tube.player.download_queued") as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

<div
  class="vp"
  class:theater={vplayer.theater && !short}
  class:fs={vplayer.fullscreen}
  class:short
  class:idle={!showControls}
  bind:this={root}
  onpointermove={wake}
  onpointerdown={wake}
  onpointerleave={() => (idle = true)}
  role="region"
  aria-label={$t("tube.player.region") as string}
  data-agent-readiness={vplayer.error ? "error" : vplayer.loading ? "loading" : "ready"}
>
  {#if vplayer.ambient && !short && !vplayer.fullscreen}
    <canvas class="amb" class:on={firstFrame} bind:this={amb} width="24" height="14" aria-hidden="true" data-agent-renderer="tube-player"></canvas>
  {/if}
  <div class="frame">
    <div class="stage" bind:this={stage} onclick={clickStage} ondblclick={dbl} role="presentation"></div>

    {#if !firstFrame}
      <img class="poster" src={poster} alt="" decoding="async" referrerpolicy="no-referrer" out:fade={{ duration: 250 }} />
    {/if}

    {#if vplayer.cue}
      <div class="cc" class:lifted={showControls}>
        {#each vplayer.cue.split("\n") as line, i (i)}<span>{line}</span>{/each}
      </div>
    {/if}

    {#if (vplayer.buffering || vplayer.loading) && !vplayer.error}
      <div class="spin" aria-hidden="true" in:fade={{ delay: 300, duration: 150 }}><span></span></div>
    {/if}

    {#key vplayer.bezel.n}
      {#if vplayer.bezel.n > 0}
        <div class="bezel" aria-hidden="true">
          <span class="bz"><TIcon name={vplayer.bezel.icon} size={30} /></span>
          {#if vplayer.bezel.text}<span class="bt">{vplayer.bezel.text}</span>{/if}
        </div>
      {/if}
    {/key}

    {#if vplayer.nextIn != null && upNext}
      <div class="next" transition:fade={{ duration: 200 }}>
        <span class="nl">{$t("tube.player.up_next_in", { count: vplayer.nextIn })}</span>
        <div class="nc">
          <img src={thumbUrl(upNext.thumbs, 240) || videoThumb(upNext.id)} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
          <div class="nm">
            <strong class="yt-clamp-2">{upNext.title}</strong>
            {#if upNext.channel}<span>{upNext.channel.name}</span>{/if}
          </div>
        </div>
        <div class="nb">
          <button class="yt-btn" onclick={cancelNext}>{$t("tube.player.cancel")}</button>
          <button class="yt-btn yt-btn--inverted" onclick={next}>{$t("tube.player.play_now")}</button>
        </div>
      </div>
    {/if}

    {#if vplayer.error}
      <div class="err" role="alert">
        <TIcon name="alert" size={40} />
        <strong>{$t("tube.player.error_title")}</strong>
        <p>{vplayer.error}</p>
        <div class="eb">
          <button class="yt-btn yt-btn--inverted" onclick={retry}><TIcon name="rewind" size={18} />{$t("tube.player.retry")}</button>
          <button class="yt-btn" onclick={() => void openUrl(`https://www.youtube.com/watch?v=${videoId}`)}><TIcon name="external" size={18} />{$t("tube.player.open_browser")}</button>
          <button class="yt-btn" onclick={download}><TIcon name="download" size={18} />{$t("tube.player.download")}</button>
        </div>
      </div>
    {/if}

    <SkipToast lifted={showControls} />
    <Controls show={showControls} bind:menuOpen onhover={(o) => (overControls = o)} />
  </div>
</div>

<style>
  .vp {
    position: relative;
    container: tplayer / inline-size;
    width: 100%;
    isolation: isolate;
  }
  .frame {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 9;
    border-radius: var(--yt-radius-thumb-lg);
    overflow: hidden;
    background: #000;
    color: #fff;
  }
  .vp.idle .frame {
    cursor: none;
  }
  .theater .frame {
    border-radius: 0;
    aspect-ratio: auto;
    height: min(calc(100vh - 169px), 56.25vw);
    min-height: 480px;
  }
  .fs {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: #000;
  }
  .fs .frame {
    width: 100vw;
    height: 100vh;
    max-height: none;
    aspect-ratio: auto;
    border-radius: 0;
  }
  .short .frame {
    aspect-ratio: 9 / 16;
    height: 100%;
    width: auto;
    max-width: 100%;
    margin: 0 auto;
  }
  .stage {
    position: absolute;
    inset: 0;
    display: flex;
  }
  .stage :global(video) {
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: #000;
    display: block;
  }
  .short .stage :global(video) {
    object-fit: cover;
  }
  .poster {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    pointer-events: none;
  }
  .amb {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    z-index: -1;
    transform: scale(1.12, 1.3);
    filter: blur(60px) saturate(1.6);
    opacity: 0;
    transition: opacity 600ms var(--yt-ease-standard);
    pointer-events: none;
  }
  .amb.on {
    opacity: 0.55;
  }
  :global(.omnitube[data-scheme="light"]) .amb,
  .theater .amb {
    display: none;
  }
  .cc {
    position: absolute;
    left: 50%;
    bottom: 5%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    max-width: 80%;
    pointer-events: none;
    transition: bottom 250ms var(--yt-ease-standard);
    z-index: 3;
  }
  .cc.lifted {
    bottom: calc(5% + 64px);
  }
  .cc span {
    padding: 2px 8px;
    background: rgba(8, 8, 8, 0.75);
    color: #fff;
    font: 500 clamp(14px, 2.6cqw, 34px) / 1.35 var(--yt-font);
    text-align: center;
    white-space: pre-wrap;
    border-radius: 2px;
  }
  .spin {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    pointer-events: none;
    z-index: 2;
  }
  .spin span {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    border: 4px solid rgba(255, 255, 255, 0.2);
    border-top-color: #fff;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .bezel {
    position: absolute;
    left: 50%;
    top: 50%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    transform: translate(-50%, -50%);
    pointer-events: none;
    z-index: 3;
  }
  .bz {
    display: grid;
    place-items: center;
    width: 78px;
    height: 78px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    animation: bezel 500ms var(--yt-ease-enter) forwards;
  }
  .bt {
    padding: 2px 10px;
    border-radius: 12px;
    background: rgba(0, 0, 0, 0.5);
    font: 500 14px/20px var(--yt-font);
    animation: bezel-text 700ms var(--yt-ease-enter) forwards;
  }
  @keyframes bezel {
    from {
      opacity: 1;
      transform: scale(1);
    }
    to {
      opacity: 0;
      transform: scale(1.6);
    }
  }
  @keyframes bezel-text {
    0%,
    60% {
      opacity: 1;
    }
    to {
      opacity: 0;
    }
  }
  .next {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    background: rgba(0, 0, 0, 0.72);
    z-index: 5;
  }
  .nl {
    font: 400 16px/22px var(--yt-font);
    color: var(--yt-ov-text-2);
  }
  .nc {
    display: flex;
    gap: 14px;
    align-items: center;
    max-width: min(520px, 90%);
  }
  .nc img {
    width: 168px;
    aspect-ratio: 16 / 9;
    object-fit: cover;
    border-radius: 8px;
    flex: none;
  }
  .nm {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .nm strong {
    font: 500 18px/24px var(--yt-font);
  }
  .nm span {
    color: var(--yt-ov-text-2);
  }
  .nb {
    display: flex;
    gap: 10px;
  }
  .nb .yt-btn:not(.yt-btn--inverted) {
    background: rgba(255, 255, 255, 0.15);
    color: #fff;
  }
  .err {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
    background: rgba(0, 0, 0, 0.85);
    z-index: 5;
  }
  .err strong {
    font: 500 18px/24px var(--yt-font);
  }
  .err p {
    margin: 0 0 8px;
    max-width: 520px;
    color: var(--yt-ov-text-2);
    font: 400 13px/18px var(--yt-font);
    overflow-wrap: anywhere;
  }
  .eb {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .eb .yt-btn:not(.yt-btn--inverted) {
    background: rgba(255, 255, 255, 0.15);
    color: #fff;
  }
  @media (prefers-reduced-motion: reduce) {
    .bz,
    .bt,
    .spin span {
      animation-duration: 0.01ms;
    }
    .amb {
      display: none;
    }
  }
</style>
