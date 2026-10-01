<script lang="ts">
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { fmtDuration, thumbUrl } from "$lib/tube/format";
  import { mplayer, playSong, toggle } from "$lib/tube/music.svelte";
  import { go } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "../views/music/ItemMenu.svelte";
  import { isCurrent, likeSong } from "../views/music/shared.svelte";
  import type { SongItem } from "$lib/tube/types";

  let {
    song,
    variant = "list",
    index = null,
    showArtists = true,
    showAlbum = true,
    onplay,
  }: {
    song: SongItem;
    variant?: "list" | "album" | "pick";
    index?: number | null;
    showArtists?: boolean;
    showAlbum?: boolean;
    onplay?: () => void;
  } = $props();

  let current = $derived(isCurrent(song.id));
  let playing = $derived(current && mplayer.playing);
  let menu = $state<{ x: number; y: number } | null>(null);
  let cover = $derived(variant === "album" ? "" : thumbUrl(song.thumbs, variant === "pick" ? 48 : 40));

  function play() {
    if (current) {
      toggle();
      return;
    }
    if (onplay) onplay();
    else void playSong(song);
  }

  function openMenu(e: MouseEvent) {
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: r.right - 280, y: r.bottom + 4 };
  }

  function contextMenu(e: MouseEvent) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY };
  }

  function rowKey(e: KeyboardEvent) {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      play();
    }
  }
</script>

<div
  class="sr sr--{variant}"
  class:is-current={current}
  role="row"
  tabindex="0"
  aria-current={current ? "true" : undefined}
  ondblclick={play}
  onkeydown={rowKey}
  oncontextmenu={contextMenu}
>
  <div class="sr-lead" role="cell">
    {#if variant === "album"}
      <span class="sr-num">{index ?? ""}</span>
    {:else}
      <span class="sr-cover">
        {#if cover}
          <img src={cover} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
        {:else}
          <span class="sr-cover-ph"><TIcon name="music-note" size={18} /></span>
        {/if}
      </span>
    {/if}
    <button use:agentActions={["click"]} class="sr-play" onclick={play} aria-label={(playing ? $t("tube.music.pause") : $t("tube.music.play_named", { title: song.title })) as string}>
      {#if playing}
        <span class="sr-eq" aria-hidden="true"><i></i><i></i><i></i></span>
        <TIcon name="pause" size={20} class="sr-pause" />
      {:else}
        <TIcon name="play" size={20} />
      {/if}
    </button>
  </div>

  <div class="sr-main" role="cell">
    <span class="sr-title yt-clamp-1" title={song.title}>
      {song.title}
      {#if song.explicit}<span class="sr-e" title={$t("tube.music.explicit") as string}>E</span>{/if}
    </span>
    {#if variant === "pick"}
      <span class="sr-sub yt-clamp-1">
        {#each song.artists as a, i (i)}
          {#if i > 0}, {/if}
          {#if a.id}<button class="sr-link" onclick={() => go({ name: "m_artist", id: a.id })}>{a.name}</button>{:else}{a.name}{/if}
        {/each}
        {#if song.plays_text}<span class="sr-dot"> • </span>{song.plays_text}{/if}
      </span>
    {/if}
  </div>

  {#if variant !== "pick" && showArtists}
    <div class="sr-artists yt-clamp-1" role="cell">
      {#each song.artists as a, i (i)}
        {#if i > 0}, {/if}
        {#if a.id}<button class="sr-link" onclick={() => go({ name: "m_artist", id: a.id })}>{a.name}</button>{:else}<span>{a.name}</span>{/if}
      {/each}
      {#if variant === "album" && song.plays_text}<span class="sr-plays">{song.plays_text}</span>{/if}
    </div>
  {/if}
  {#if variant === "list" && showAlbum}
    <div class="sr-album yt-clamp-1" role="cell">
      {#if song.album}
        <button class="sr-link" onclick={() => song.album && go({ name: "m_album", id: song.album.id })}>{song.album.name}</button>
      {/if}
    </div>
  {/if}

  <div class="sr-end" role="cell">
    {#if variant !== "pick"}
      <button
        class="sr-like"
        class:is-on={song.liked}
        onclick={() => void likeSong(song, !song.liked)}
        aria-pressed={!!song.liked}
        aria-label={(song.liked ? $t("tube.music.unlike") : $t("tube.music.like")) as string}
      >
        <TIcon name={song.liked ? "thumbs-up-fill" : "thumbs-up"} size={18} />
      </button>
    {/if}
    {#if variant !== "pick" && song.duration_s}
      <span class="sr-dur">{fmtDuration(song.duration_s)}</span>
    {/if}
    <button class="sr-more" onclick={openMenu} aria-label={$t("tube.music.more_actions") as string} aria-haspopup="menu" aria-expanded={!!menu}>
      <TIcon name="more-vert" size={20} />
    </button>
  </div>
</div>

{#if menu}
  <ItemMenu item={song} x={menu.x} y={menu.y} onclose={() => (menu = null)} />
{/if}

<style>
  .sr {
    --sr-h: var(--ytm-row-h, 56px);
    display: grid;
    grid-template-columns: auto minmax(0, 2fr) minmax(0, 1.3fr) minmax(0, 1.1fr) auto;
    align-items: center;
    gap: 16px;
    height: var(--sr-h);
    padding: 0 8px;
    border-radius: 4px;
    color: var(--ytm-text, var(--yt-text));
    border-bottom: 1px solid var(--ytm-divider, var(--yt-divider));
    transition: background var(--yt-dur-fast) linear;
    outline: none;
  }
  .sr:hover { background: var(--ytm-row-hover, var(--yt-hover)); }
  .sr:focus-visible { box-shadow: inset 0 0 0 2px var(--yt-link); }
  .sr.is-current { background: var(--ytm-row-playing, var(--yt-tonal)); }
  .sr--album { --sr-h: var(--ytm-list-row-h, 48px); grid-template-columns: 32px minmax(0, 2fr) minmax(0, 1.2fr) auto; }
  .sr--album:not(:has(.sr-artists)) { grid-template-columns: 32px minmax(0, 1fr) auto; }
  .sr--list:not(:has(.sr-album)) { grid-template-columns: auto minmax(0, 2fr) minmax(0, 1.3fr) auto; }
  .sr--list:not(:has(.sr-artists)):not(:has(.sr-album)) { grid-template-columns: auto minmax(0, 1fr) auto; }
  .sr--pick {
    grid-template-columns: 48px minmax(0, 1fr) auto;
    gap: 16px;
    border-bottom: 0;
    padding: 0 4px;
  }

  .sr-lead { position: relative; display: grid; place-items: center; }
  .sr-cover { width: 40px; height: 40px; border-radius: 2px; overflow: hidden; background: var(--yt-skeleton); display: block; }
  .sr--pick .sr-cover { width: 48px; height: 48px; border-radius: 4px; }
  .sr-cover img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .sr-cover-ph { display: grid; place-items: center; width: 100%; height: 100%; color: var(--ytm-text-3, var(--yt-text-2)); }
  .sr-num { width: 32px; text-align: center; font: var(--yt-type-body-strong); color: var(--ytm-text-2, var(--yt-text-2)); }

  .sr-play {
    position: absolute; inset: 0;
    display: grid; place-items: center;
    border: 0; padding: 0; border-radius: 2px;
    color: #fff; background: rgba(0, 0, 0, .5);
    opacity: 0; cursor: pointer;
    transition: opacity var(--yt-dur-fast) linear;
  }
  .sr--album .sr-play { background: transparent; color: var(--ytm-text, var(--yt-text)); }
  .sr:hover .sr-play, .sr-play:focus-visible, .sr.is-current .sr-play { opacity: 1; }
  .sr.is-current:not(:hover) .sr-num { visibility: hidden; }
  .sr-play:focus-visible { outline: 2px solid var(--yt-link); }
  .sr-play :global(.sr-pause) { display: none; }
  .sr:hover .sr-play :global(.sr-pause) { display: block; }
  .sr:hover .sr-eq { display: none; }

  .sr-eq { display: inline-flex; align-items: flex-end; gap: 2px; height: 16px; }
  .sr-eq i { width: 3px; height: 100%; border-radius: 2px; background: currentColor; transform-origin: bottom; animation: sr-eq .9s ease-in-out infinite; }
  .sr-eq i:nth-child(2) { animation-duration: 1.15s; animation-delay: -.3s; }
  .sr-eq i:nth-child(3) { animation-duration: .8s; animation-delay: -.55s; }
  @keyframes sr-eq { 0%, 100% { transform: scaleY(.3); } 50% { transform: scaleY(1); } }
  @media (prefers-reduced-motion: reduce) { .sr-eq i { animation: none; transform: scaleY(.7); } }

  .sr-main { display: flex; flex-direction: column; min-width: 0; }
  .sr-title { font: 500 14px/20px var(--yt-font); color: var(--ytm-text, var(--yt-text)); display: flex; align-items: center; gap: 6px; }
  .sr--pick .sr-title { font-size: 16px; line-height: 22px; display: block; }
  .sr-e {
    display: inline-grid; place-items: center; flex: none;
    width: 14px; height: 14px; border-radius: 2px;
    font: 600 9px/1 var(--yt-font); color: var(--ytm-bg, var(--yt-bg)); background: var(--ytm-text-2, var(--yt-text-2));
  }
  .sr-sub, .sr-artists, .sr-album { font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); }
  .sr-plays { margin-left: 8px; }
  .sr-link {
    border: 0; padding: 0; background: none; font: inherit; color: inherit; cursor: pointer;
  }
  .sr-link:hover { text-decoration: underline; color: var(--ytm-text, var(--yt-text)); }
  .sr-link:focus-visible { outline: 2px solid var(--yt-link); border-radius: 2px; }

  .sr-end { display: flex; align-items: center; justify-content: flex-end; gap: 4px; }
  .sr-dur { min-width: 44px; text-align: right; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); font-variant-numeric: tabular-nums; }
  .sr-like, .sr-more {
    display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%;
    border: 0; padding: 0; background: transparent; color: var(--ytm-text-2, var(--yt-text-2)); cursor: pointer;
    opacity: 0; transition: opacity var(--yt-dur-fast) linear, background var(--yt-dur-fast);
  }
  .sr-like.is-on { opacity: 1; color: var(--ytm-text, var(--yt-text)); }
  .sr:hover .sr-like, .sr:hover .sr-more, .sr:focus-within .sr-like, .sr:focus-within .sr-more, .sr-more[aria-expanded="true"] { opacity: 1; }
  .sr-like:hover, .sr-more:hover { background: var(--ytm-chip, var(--yt-tonal)); color: var(--ytm-text, var(--yt-text)); }
  .sr-like:focus-visible, .sr-more:focus-visible { outline: 2px solid var(--yt-link); opacity: 1; }
  .sr:hover .sr-dur { display: none; }
  .sr--pick .sr-more { opacity: 0; }
  .sr--pick:hover .sr-more { opacity: 1; }

  @container omnitube (max-width: 820px) {
    .sr-album { display: none; }
    .sr--list { grid-template-columns: auto minmax(0, 2fr) minmax(0, 1.3fr) auto; }
  }
  @container omnitube (max-width: 560px) {
    .sr-artists { display: none; }
    .sr--list, .sr--album { grid-template-columns: auto minmax(0, 1fr) auto; }
  }
</style>
