<script lang="ts">
  // Video mini player (spec §2.6): 400×225 in a corner, the same <video>
  // element the watch page used. Hover shows prev/play/next, expand and close;
  // drag it and it snaps to the nearest corner.
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { shellLayout } from "$lib/stores/shell-layout.svelte";
  import {
    attachVideo,
    closeMini,
    detachVideo,
    expandMini,
    hasPrev,
    next,
    prev,
    toggle,
    vplayer,
  } from "$lib/tube/video.svelte";
  import TIcon from "../TIcon.svelte";

  type Corner = "br" | "bl" | "tr" | "tl";
  const LS = "omnitube.mini.corner";

  let stage = $state<HTMLDivElement>();
  let corner = $state<Corner>(readCorner());
  let drag = $state<{ x: number; y: number; dx: number; dy: number; moved: boolean } | null>(null);

  function readCorner(): Corner {
    try {
      const c = localStorage.getItem(LS);
      return c === "bl" || c === "tr" || c === "tl" ? c : "br";
    } catch {
      return "br";
    }
  }

  onMount(() => {
    if (stage) attachVideo(stage);
  });
  onDestroy(() => {
    if (stage) detachVideo(stage);
  });

  let pct = $derived(vplayer.duration > 0 ? Math.min(100, (vplayer.position / vplayer.duration) * 100) : 0);
  let title = $derived(vplayer.info?.title ?? vplayer.streams?.title ?? "");
  let channel = $derived(vplayer.info?.channel?.name ?? vplayer.streams?.author ?? "");
  let bottom = $derived(16 + shellLayout.overlayBottomInset);

  function down(e: PointerEvent) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, dx: 0, dy: 0, moved: false };
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    drag.dx = e.clientX - drag.x;
    drag.dy = e.clientY - drag.y;
    if (Math.abs(drag.dx) + Math.abs(drag.dy) > 6) drag.moved = true;
  }
  function up(e: PointerEvent) {
    if (!drag) return;
    const moved = drag.moved;
    drag = null;
    if (!moved) {
      toggle();
      return;
    }
    const right = e.clientX > window.innerWidth / 2;
    const lower = e.clientY > window.innerHeight / 2;
    corner = `${lower ? "b" : "t"}${right ? "r" : "l"}` as Corner;
    try {
      localStorage.setItem(LS, corner);
    } catch {
      /* storage off */
    }
  }
</script>

<div
  class="mini {corner}"
  class:dragging={drag?.moved}
  style:--bottom="{bottom}px"
  style:transform={drag?.moved ? `translate(${drag.dx}px, ${drag.dy}px)` : undefined}
  role="region"
  aria-label={$t("tube.player.miniplayer") as string}
>
  <div class="vid" role="presentation" onpointerdown={down} onpointermove={move} onpointerup={up} ondblclick={expandMini}>
    <div class="stage" bind:this={stage}></div>
    <div class="ov">
      <div class="top">
        <button class="ib" onclick={expandMini} aria-label={$t("tube.player.expand") as string} title={$t("tube.player.expand") as string}>
          <TIcon name="expand" size={20} />
        </button>
        <button class="ib" onclick={closeMini} aria-label={$t("tube.player.close") as string} title={$t("tube.player.close") as string}>
          <TIcon name="close" size={22} />
        </button>
      </div>
      <div class="mid">
        <button class="ib" onclick={prev} disabled={!hasPrev() && vplayer.position < 3} aria-label={$t("tube.player.prev") as string}><TIcon name="prev" size={24} /></button>
        <button class="ib big" onclick={toggle} aria-label={$t(vplayer.playing ? "tube.player.pause" : "tube.player.play") as string}>
          <TIcon name={vplayer.playing ? "pause" : "play"} size={32} />
        </button>
        <button class="ib" onclick={next} disabled={!vplayer.upNext.length} aria-label={$t("tube.player.next") as string}><TIcon name="next" size={24} /></button>
      </div>
    </div>
    {#if vplayer.buffering && !vplayer.error}<span class="spin" aria-hidden="true"></span>{/if}
    <span class="prog"><i style:width="{pct}%"></i></span>
  </div>
  <button class="info" onclick={expandMini}>
    <span class="t yt-clamp-1">{title}</span>
    <span class="c yt-clamp-1">{channel}</span>
  </button>
</div>

<style>
  .mini {
    position: fixed;
    width: 400px;
    max-width: calc(100vw - 32px);
    border-radius: 12px;
    overflow: hidden;
    background: var(--yt-bg-raised);
    color: var(--yt-text);
    box-shadow:
      0 2px 5px rgba(0, 0, 0, 0.16),
      0 3px 6px rgba(0, 0, 0, 0.2),
      0 16px 40px rgba(0, 0, 0, 0.35);
    z-index: 310;
    pointer-events: auto;
    animation: in 300ms var(--yt-ease-standard);
    transition: transform 300ms var(--yt-ease-standard);
  }
  .mini.dragging {
    transition: none;
    cursor: grabbing;
    opacity: 0.92;
  }
  .br {
    right: 16px;
    bottom: var(--bottom);
  }
  .bl {
    left: 16px;
    bottom: var(--bottom);
  }
  .tr {
    right: 16px;
    top: 72px;
  }
  .tl {
    left: 16px;
    top: 72px;
  }
  @keyframes in {
    from {
      transform: translateY(24px) scale(0.96);
      opacity: 0;
    }
  }
  .vid {
    position: relative;
    aspect-ratio: 16 / 9;
    background: #000;
    cursor: pointer;
    touch-action: none;
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
  }
  .ov {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    background: rgba(0, 0, 0, 0.45);
    opacity: 0;
    transition: opacity var(--yt-dur-base) var(--yt-ease-standard);
  }
  .vid:hover .ov,
  .vid:focus-within .ov {
    opacity: 1;
  }
  .top {
    display: flex;
    justify-content: space-between;
    padding: 6px;
  }
  .mid {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 24px;
    margin-bottom: auto;
    margin-top: -8px;
    flex: 1;
  }
  .ib {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: 50%;
    color: #fff;
    background: transparent;
    cursor: pointer;
    filter: drop-shadow(0 0 2px rgba(0, 0, 0, 0.6));
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .ib.big {
    width: 56px;
    height: 56px;
  }
  .ib:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.15);
  }
  .ib:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .ib:focus-visible,
  .info:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: -2px;
  }
  .prog {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: rgba(255, 255, 255, 0.2);
  }
  .prog i {
    display: block;
    height: 100%;
    background: var(--yt-accent);
    transition: width 250ms linear;
  }
  .spin {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 36px;
    height: 36px;
    margin: -18px 0 0 -18px;
    border-radius: 50%;
    border: 3px solid rgba(255, 255, 255, 0.2);
    border-top-color: #fff;
    animation: spin 0.9s linear infinite;
    pointer-events: none;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 10px 14px 12px;
    border: 0;
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }
  .t {
    font: var(--yt-type-body-strong);
    font-size: 15px;
  }
  .c {
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  @media (prefers-reduced-motion: reduce) {
    .mini {
      animation: none;
      transition: none;
    }
  }
</style>
