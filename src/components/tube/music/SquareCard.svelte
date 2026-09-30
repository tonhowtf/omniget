<script lang="ts">
  // YT Music carousel card: square art (circle for artists, 16:9 for music
  // videos), floating play button bottom-right on hover, ⋮ in the top gradient,
  // two-line title and subtitle. Clicking the art opens the item.
  import { t } from "$lib/i18n";
  import { bestAvatar, thumbUrl } from "$lib/tube/format";
  import { mplayer, toggle } from "$lib/tube/music.svelte";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "../views/music/ItemMenu.svelte";
  import { ctxOf, isCurrent, isPlayingCtx, openItem, playItem, subtitleOf, thumbsOf, titleOf } from "../views/music/shared.svelte";
  import type { Item } from "$lib/tube/types";

  let {
    item,
    width = 180,
    /** Fill the grid cell instead of a fixed width (library/search grids). */
    fluid = false,
    subtitle,
  }: { item: Item; width?: number; fluid?: boolean; subtitle?: string } = $props();

  let round = $derived(item.kind === "artist" || item.kind === "channel");
  let wide = $derived(item.kind === "video" || (item.kind === "song" && item.is_video));
  let w = $derived(wide ? Math.round(width * 1.78) : width);
  let src = $derived(round ? bestAvatar(thumbsOf(item), width) : thumbUrl(thumbsOf(item), w));
  let ctx = $derived(ctxOf(item));
  let activeSong = $derived((item.kind === "song" || item.kind === "video") && isCurrent(item.id));
  let active = $derived(activeSong || (!!ctx && mplayer.context === ctx));
  let playing = $derived(activeSong ? mplayer.playing : isPlayingCtx(ctx));
  let liked = $derived(item.kind === "playlist" && (item.id === "LM" || item.id === "VLLM"));
  let busy = $state(false);
  let menu = $state<{ x: number; y: number } | null>(null);
  let sub = $derived(subtitle ?? subtitleOf(item));
  let title = $derived(titleOf(item));

  async function play(e: MouseEvent) {
    e.stopPropagation();
    if (active) {
      toggle();
      return;
    }
    busy = true;
    try {
      await playItem(item);
    } finally {
      busy = false;
    }
  }

  function openMenu(e: MouseEvent) {
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: r.left, y: r.bottom + 4 };
  }

  function contextMenu(e: MouseEvent) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY };
  }
</script>

<div class="sq" class:round class:wide class:fluid class:active style:--sq-w="{w}px" oncontextmenu={contextMenu} role="group" aria-label={title}>
  <div class="sq-art">
    <button class="sq-open" onclick={() => openItem(item)} aria-label={$t("tube.music.open_named", { title }) as string}>
      {#if liked}
        <span class="sq-ph sq-liked"><TIcon name="thumbs-up-fill" size={56} /></span>
      {:else if src}
        <img src={src} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
      {:else}
        <span class="sq-ph"><TIcon name={round ? "artist" : item.kind === "playlist" ? "playlist" : "album"} size={40} /></span>
      {/if}
    </button>
    {#if !round}
      <span class="sq-shade" aria-hidden="true"></span>
      <button class="sq-more" onclick={openMenu} aria-label={$t("tube.music.more_actions") as string} aria-haspopup="menu" aria-expanded={!!menu}>
        <TIcon name="more-vert" size={20} />
      </button>
    {/if}
    <button
      class="sq-play"
      class:is-on={active}
      onclick={play}
      aria-label={(playing ? $t("tube.music.pause") : $t("tube.music.play_named", { title })) as string}
    >
      {#if busy}
        <TIcon name="loader" size={22} class="sq-spin" />
      {:else if playing}
        <span class="sq-eq" aria-hidden="true"><i></i><i></i><i></i></span>
        <TIcon name="pause" size={22} class="sq-pause" />
      {:else}
        <TIcon name="play" size={22} />
      {/if}
    </button>
  </div>
  <div class="sq-text">
    <button class="sq-title yt-clamp-2" onclick={() => openItem(item)} {title}>{title}</button>
    {#if sub}<p class="sq-sub yt-clamp-2" title={sub}>{sub}</p>{/if}
  </div>
</div>

{#if menu}
  <ItemMenu {item} x={menu.x} y={menu.y} onclose={() => (menu = null)} />
{/if}

<style>
  .sq { width: var(--sq-w); flex: none; scroll-snap-align: start; color: var(--ytm-text, var(--yt-text)); }
  .sq.fluid { width: auto; min-width: 0; }
  .sq-art {
    position: relative;
    aspect-ratio: 1;
    border-radius: var(--ytm-card-radius, 8px);
    overflow: hidden;
    background: var(--yt-skeleton);
  }
  .sq.wide .sq-art { aspect-ratio: 16 / 9; }
  .sq.round .sq-art { border-radius: 50%; }
  .sq-open { all: unset; display: block; width: 100%; height: 100%; cursor: pointer; }
  .sq-open:focus-visible { outline: 2px solid var(--yt-link); outline-offset: -2px; }
  .sq-open img { width: 100%; height: 100%; object-fit: cover; display: block; transition: transform var(--yt-dur-preview) var(--yt-ease-standard); }
  .sq-liked { background: linear-gradient(135deg, #623cea, #3ea6ff); color: #fff !important; }
  .sq-ph { display: grid; place-items: center; width: 100%; height: 100%; color: var(--ytm-text-3, var(--yt-text-2)); }

  .sq-shade {
    position: absolute; inset: 0; pointer-events: none;
    background: linear-gradient(rgba(0, 0, 0, .5), transparent 50%), linear-gradient(transparent 60%, rgba(0, 0, 0, .35));
    opacity: 0; transition: opacity var(--yt-dur-base) var(--yt-ease-standard);
  }
  .sq-more {
    position: absolute; top: 4px; right: 4px;
    display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%;
    border: 0; padding: 0; background: transparent; color: #fff; cursor: pointer;
    opacity: 0; transition: opacity var(--yt-dur-base), background var(--yt-dur-fast);
  }
  .sq-more:hover { background: rgba(255, 255, 255, .15); }
  .sq-play {
    position: absolute; right: 12px; bottom: 12px;
    display: grid; place-items: center; width: 44px; height: 44px; border-radius: 50%;
    border: 0; padding: 0 0 0 2px; cursor: pointer;
    color: #fff; background: rgba(0, 0, 0, .55);
    backdrop-filter: blur(4px);
    opacity: 0; transform: scale(.9);
    transition: opacity var(--yt-dur-base) var(--yt-ease-standard), transform var(--yt-dur-base) var(--yt-ease-standard), background var(--yt-dur-base), color var(--yt-dur-base);
  }
  .sq.round .sq-play { right: 50%; bottom: 50%; transform: translate(50%, 50%) scale(.9); }
  .sq-play:hover, .sq-play:focus-visible { background: #fff; color: #000; transform: scale(1.1); }
  .sq.round .sq-play:hover, .sq.round .sq-play:focus-visible { transform: translate(50%, 50%) scale(1.1); }
  .sq-play:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .sq-play.is-on { opacity: 1; transform: scale(1); background: #fff; color: #000; padding: 0; }
  .sq.round .sq-play.is-on { transform: translate(50%, 50%) scale(1); }
  .sq-play :global(.sq-pause) { display: none; }
  .sq-play:hover :global(.sq-pause) { display: block; }
  .sq-play:hover .sq-eq { display: none; }
  .sq-play :global(.sq-spin) { animation: sq-spin 1s linear infinite; }

  .sq:hover .sq-shade, .sq:focus-within .sq-shade, .sq.active .sq-shade { opacity: 1; }
  .sq:hover .sq-more, .sq:focus-within .sq-more, .sq-more[aria-expanded="true"] { opacity: 1; }
  .sq:hover .sq-play, .sq:focus-within .sq-play { opacity: 1; transform: scale(1); }
  .sq.round:hover .sq-play, .sq.round:focus-within .sq-play { transform: translate(50%, 50%) scale(1); }
  .sq:hover .sq-open img { transform: scale(1.03); }

  .sq-eq { display: inline-flex; align-items: flex-end; gap: 2px; height: 16px; }
  .sq-eq i { width: 3px; height: 100%; border-radius: 2px; background: currentColor; transform-origin: bottom; animation: sq-eq .9s ease-in-out infinite; }
  .sq-eq i:nth-child(2) { animation-duration: 1.15s; animation-delay: -.3s; }
  .sq-eq i:nth-child(3) { animation-duration: .8s; animation-delay: -.55s; }
  @keyframes sq-eq { 0%, 100% { transform: scaleY(.3); } 50% { transform: scaleY(1); } }
  @keyframes sq-spin { to { transform: rotate(360deg); } }

  .sq-text { padding-top: 8px; }
  .sq.round .sq-text { text-align: center; }
  .sq-title {
    all: unset; cursor: pointer;
    font: 500 16px/22px var(--yt-font); color: var(--ytm-text, var(--yt-text));
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
    word-break: break-word;
  }
  .sq-title:hover { text-decoration: underline; }
  .sq-title:focus-visible { outline: 2px solid var(--yt-link); border-radius: 2px; }
  .sq-sub { margin: 2px 0 0; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); word-break: break-word; }

  @media (prefers-reduced-motion: reduce) {
    .sq-eq i, .sq-play :global(.sq-spin) { animation: none; }
    .sq-open img, .sq-play { transition: none; }
    .sq:hover .sq-open img { transform: none; }
  }
</style>
