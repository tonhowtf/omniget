<script lang="ts">
  // Playlist / mix card. The thumb carries the "stack" (two strips above it)
  // and a count badge, as on YouTube. `row` renders it like a search result.
  import { t } from "$lib/i18n";
  import { thumbUrl, videoThumb } from "$lib/tube/format";
  import type { PlaylistItem } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import { channelHref, openChannel, openPlaylist, openWatch, playlistHref, watchHref } from "./util";

  let { item, layout = "card" }: { item: PlaylistItem; layout?: "card" | "row" | "compact" } = $props();

  let menuOpen = $state(false);
  let menuPoint = $state<{ x: number; y: number } | null>(null);
  let moreBtn = $state<HTMLButtonElement>();
  let thumb = $derived(thumbUrl(item.thumbs, layout === "compact" ? 168 : 360) || (item.first_video_id ? videoThumb(item.first_video_id) : ""));
  let href = $derived(item.is_mix && item.first_video_id ? watchHref(item.first_video_id, item.id) : playlistHref(item.id));

  function open(e: MouseEvent) {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
    e.preventDefault();
    if (item.is_mix && item.first_video_id) openWatch(item.first_video_id, { list: item.id, index: 0 });
    else openPlaylist(item.id);
  }
</script>

<article
  class="pcard {layout}"
  class:menu-open={menuOpen}
  oncontextmenu={(e) => {
    e.preventDefault();
    menuPoint = { x: e.clientX, y: e.clientY };
    menuOpen = true;
  }}
>
  <a class="cover" {href} onclick={open} tabindex="-1" aria-hidden="true"></a>
  <div class="stack">
    <div class="yt-thumb">
      {#if thumb}<img src={thumb} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />{/if}
      <span class="count">
        <TIcon name={item.is_mix ? "radio" : "playlist"} size={16} />
        {item.is_mix ? $t("tube.card.mix") : item.count_text ?? ""}
      </span>
      <span class="hover-play"><TIcon name="play" size={20} />{$t("tube.menu.play_all")}</span>
    </div>
  </div>
  <div class="meta">
    <h3><a class="title yt-clamp-2" {href} onclick={open} title={item.title}>{item.title}</a></h3>
    {#if item.channel}
      <a class="channel" href={channelHref(item.channel.id)} onclick={(e) => { e.preventDefault(); openChannel(item.channel!.id); }}>
        <span class="yt-clamp-1">{item.channel.name}</span>
        {#if item.channel.verified}<TIcon name="verified" size={14} />{/if}
      </a>
    {/if}
    {#if layout !== "compact"}
      <a class="full" href={playlistHref(item.id)} onclick={(e) => { e.preventDefault(); openPlaylist(item.id); }}>
        {$t("tube.card.view_playlist")}
      </a>
    {/if}
    <button
      bind:this={moreBtn}
      class="more"
      aria-haspopup="menu"
      aria-expanded={menuOpen}
      aria-label={$t("tube.card.actions") as string}
      onclick={() => {
        menuPoint = null;
        menuOpen = true;
      }}
    >
      <TIcon name="more-vert" size={20} />
    </button>
  </div>
</article>

{#if menuOpen}
  <ItemMenu {item} anchor={moreBtn ?? null} point={menuPoint} feedbackActions={false} onclose={() => (menuOpen = false)} />
{/if}

<style>
  .pcard {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .pcard.row,
  .pcard.compact {
    flex-direction: row;
    gap: 16px;
  }
  .pcard.compact {
    gap: 8px;
  }
  .cover {
    position: absolute;
    inset: 0;
    z-index: 1;
  }
  .stack {
    position: relative;
    padding-top: 8px;
    flex: none;
  }
  .row .stack {
    width: min(360px, 45%);
  }
  .compact .stack {
    width: 168px;
    padding-top: 6px;
  }
  .stack::before,
  .stack::after {
    content: "";
    position: absolute;
    left: 50%;
    border-radius: 8px 8px 0 0;
    background: var(--yt-text-2);
    transform: translateX(-50%);
  }
  .stack::before {
    top: 0;
    width: calc(100% - 24px);
    height: 4px;
    opacity: 0.35;
  }
  .stack::after {
    top: 4px;
    width: calc(100% - 12px);
    height: 4px;
    opacity: 0.6;
  }
  .compact .stack::before {
    width: calc(100% - 16px);
    height: 3px;
  }
  .compact .stack::after {
    top: 3px;
    width: calc(100% - 8px);
    height: 3px;
  }
  .compact .yt-thumb {
    border-radius: var(--yt-radius-thumb-md);
  }
  .count {
    position: absolute;
    right: 8px;
    bottom: 8px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border-radius: 4px;
    font: 500 12px/18px var(--yt-font);
    color: #fff;
    background: var(--yt-ov-badge);
  }
  .hover-play {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font: var(--yt-type-body-strong);
    color: #fff;
    background: var(--yt-ov-heavy);
    opacity: 0;
    transition: opacity var(--yt-dur-base) var(--yt-ease-standard);
  }
  .pcard:hover .hover-play {
    opacity: 0.85;
  }
  .meta {
    position: relative;
    flex: 1;
    min-width: 0;
    padding: 12px 28px 0 0;
  }
  .row .meta,
  .compact .meta {
    padding-top: 6px;
  }
  h3 {
    margin: 0 0 4px;
    font: var(--yt-type-card-title);
  }
  .row h3 {
    font: 400 18px/26px var(--yt-font);
  }
  .compact h3 {
    font: var(--yt-type-body-strong);
  }
  .title {
    position: relative;
    z-index: 2;
    color: var(--yt-text);
    text-decoration: none;
  }
  .title:focus-visible {
    outline: 2px solid var(--yt-link);
    border-radius: 4px;
  }
  .channel,
  .full {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 4px;
    width: fit-content;
    max-width: 100%;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
    text-decoration: none;
  }
  .compact .channel {
    font: var(--yt-type-caption);
  }
  .channel:hover,
  .full:hover {
    color: var(--yt-text);
  }
  .full {
    margin-top: 2px;
  }
  .more {
    position: absolute;
    top: 4px;
    right: -12px;
    z-index: 2;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 0;
    border-radius: 50%;
    color: var(--yt-text);
    background: transparent;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--yt-dur-fast), background var(--yt-dur-fast);
  }
  .more:hover {
    background: var(--yt-tonal);
  }
  .pcard:hover .more,
  .pcard:focus-within .more,
  .menu-open .more {
    opacity: 1;
  }
  .more:focus-visible {
    opacity: 1;
    outline: 2px solid var(--yt-link);
  }
</style>
