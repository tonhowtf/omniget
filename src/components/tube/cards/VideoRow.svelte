<script lang="ts">
  // Horizontal video result. Sizes:
  //   compact  "Up next" column: thumb 168×94 radius 8, title 14/500, meta 12
  //   search   search results: thumb up to 360 (radius 12), title 18/400, channel line with avatar 24, snippet
  //   list     playlists / history: thumb 160×90, title 16/500
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { bestAvatar, fmtCount, fmtDuration, thumbUrl, timeAgo, videoThumb } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { Badge, VideoItem } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import { inview } from "./actions";
  import { dearrow, wantDearrow } from "./dearrow.svelte";
  import { channelHref, openChannel, openWatch, watchHref } from "./util";

  let {
    item,
    size = "search",
    index = null,
    number = null,
    active = false,
    list = null,
    onremove,
    feedbackActions = true,
  }: {
    item: VideoItem;
    size?: "compact" | "search" | "list";
    /** Position inside `list` (for playlist playback). */
    index?: number | null;
    /** Visible number on the left (playlists). */
    number?: number | null;
    active?: boolean;
    list?: string | null;
    onremove?: () => void;
    feedbackActions?: boolean;
  } = $props();

  let prefs = $derived(tubePrefs());
  let da = $derived(prefs.dearrow && !dearrow.original[item.id] ? dearrow.map[item.id] : null);
  let title = $derived(da?.title || item.title);
  let thumbW = $derived(size === "search" ? 360 : size === "compact" ? 168 : 160);
  let thumb = $derived((prefs.dearrow_thumbs && da?.thumb_url) || thumbUrl(item.thumbs, thumbW) || videoThumb(item.id));
  let live = $derived(item.badges.includes("live"));
  let upcoming = $derived(item.badges.includes("upcoming") || item.badges.includes("premiere"));
  let views = $derived(
    prefs.hide_metrics ? "" : item.views_text || (item.views != null ? ($t("tube.card.views", { count: fmtCount(item.views) }) as string) : ""),
  );
  let when = $derived(item.published_text || (item.published_ms ? timeAgo(item.published_ms) : ""));
  let small = $derived(item.badges.filter((b) => b === "new" || b === "hd4k" || b === "cc" || b === "members"));

  let menuOpen = $state(false);
  let menuPoint = $state<{ x: number; y: number } | null>(null);
  let moreBtn = $state<HTMLButtonElement>();

  function open(e: MouseEvent) {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
    e.preventDefault();
    openWatch(item.id, { list, index });
  }

  function badgeLabel(b: Badge): string {
    return $t(`tube.badge.${b}`) as string;
  }
</script>

<article
  class="vrow {size}"
  class:active
  class:menu-open={menuOpen}
  use:inview={(v) => v && prefs.dearrow && wantDearrow(item.id)}
  oncontextmenu={(e) => {
    e.preventDefault();
    menuPoint = { x: e.clientX, y: e.clientY };
    menuOpen = true;
  }}
>
  <a class="cover" href={watchHref(item.id, list)} onclick={open} tabindex="-1" aria-hidden="true"></a>
  {#if number != null}
    <span class="num">{#if active}<TIcon name="play" size={14} />{:else}{number}{/if}</span>
  {/if}
  <div class="yt-thumb">
    <img src={thumb} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
    {#if live}
      <span class="yt-duration is-live">{badgeLabel("live")}</span>
    {:else if upcoming}
      <span class="yt-duration">{badgeLabel("upcoming")}</span>
    {:else if item.duration_s}
      <span class="yt-duration">{fmtDuration(item.duration_s)}</span>
    {/if}
    {#if item.watched_pct}
      <span class="yt-resume"><i style:width="{Math.min(100, item.watched_pct)}%"></i></span>
    {/if}
  </div>
  <div class="meta">
    <h3><a use:agentActions={["click"]} class="title yt-clamp-2" href={watchHref(item.id, list)} onclick={open} title={title}>{title}</a></h3>
    {#if size === "search"}
      {#if views || when}
        <div class="stats">{views}{#if views && when}<span class="dot"> • </span>{/if}{when}</div>
      {/if}
      {#if item.channel}
        <a class="channel big" href={channelHref(item.channel.id)} onclick={(e) => { e.preventDefault(); openChannel(item.channel!.id); }}>
          {#if item.channel.avatar}
            <img class="yt-avatar" src={bestAvatar(item.channel.avatar, 24)} width="24" height="24" alt="" loading="lazy" referrerpolicy="no-referrer" />
          {/if}
          <span class="yt-clamp-1">{item.channel.name}</span>
          {#if item.channel.verified}<TIcon name="verified" size={14} />{/if}
        </a>
      {/if}
      {#if item.description_snippet}
        <p class="snippet yt-clamp-2">{item.description_snippet}</p>
      {/if}
      {#if small.length}
        <div class="badges">{#each small as b (b)}<span class="bdg">{badgeLabel(b)}</span>{/each}</div>
      {/if}
    {:else}
      {#if item.channel}
        <a class="channel" href={channelHref(item.channel.id)} onclick={(e) => { e.preventDefault(); openChannel(item.channel!.id); }}>
          <span class="yt-clamp-1">{item.channel.name}</span>
          {#if item.channel.verified}<TIcon name="verified" size={12} />{/if}
        </a>
      {/if}
      {#if views || when}
        <div class="stats">{views}{#if views && when}<span class="dot"> • </span>{/if}{when}</div>
      {/if}
    {/if}
  </div>
  <div class="side">
    {#if onremove}
      <button class="more" aria-label={$t("tube.card.remove") as string} title={$t("tube.card.remove") as string} onclick={onremove}>
        <TIcon name="close" size={20} />
      </button>
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
  <ItemMenu {item} anchor={moreBtn ?? null} point={menuPoint} {feedbackActions} onclose={() => (menuOpen = false)} />
{/if}

<style>
  .vrow {
    position: relative;
    isolation: isolate;
    display: flex;
    gap: 16px;
    min-width: 0;
  }
  .vrow::before {
    content: "";
    position: absolute;
    inset: -4px -8px;
    z-index: -1;
    border-radius: 12px;
    background: var(--yt-hover);
    opacity: 0;
    transition: opacity var(--yt-dur-fast) linear;
    pointer-events: none;
  }
  .vrow:hover::before,
  .vrow.menu-open::before {
    opacity: 1;
  }
  .vrow.active::before {
    opacity: 1;
    background: var(--yt-tonal);
  }
  .cover {
    position: absolute;
    inset: 0;
    z-index: 1;
  }
  .num {
    align-self: center;
    flex: none;
    width: 24px;
    text-align: center;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
  }
  .yt-thumb {
    flex: none;
  }
  .search .yt-thumb {
    width: min(360px, 45%);
    max-width: 500px;
  }
  .compact {
    gap: 8px;
  }
  .compact .yt-thumb {
    width: 168px;
    border-radius: var(--yt-radius-thumb-md);
  }
  .list .yt-thumb {
    width: 160px;
    border-radius: var(--yt-radius-thumb-md);
  }
  .meta {
    flex: 1;
    min-width: 0;
    padding-right: 32px;
  }
  h3 {
    margin: 0;
  }
  .title {
    position: relative;
    z-index: 2;
    color: var(--yt-text);
    text-decoration: none;
  }
  .title:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: 2px;
    border-radius: 4px;
  }
  .search h3 {
    font: 400 18px/26px var(--yt-font);
  }
  .compact h3 {
    font: var(--yt-type-body-strong);
    margin-bottom: 4px;
  }
  .list h3 {
    font: var(--yt-type-card-title);
    margin-bottom: 4px;
  }
  .stats,
  .channel,
  .snippet {
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .list .stats,
  .list .channel {
    font: var(--yt-type-meta);
  }
  .search .stats {
    margin-top: 2px;
  }
  .channel {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 4px;
    width: fit-content;
    max-width: 100%;
    text-decoration: none;
  }
  .channel:hover {
    color: var(--yt-text);
  }
  .channel.big {
    gap: 8px;
    margin: 12px 0;
  }
  .snippet {
    margin: 0;
  }
  .badges {
    display: flex;
    gap: 4px;
    margin-top: 8px;
  }
  .bdg {
    padding: 1px 4px;
    border-radius: 4px;
    font: 500 12px/18px var(--yt-font);
    color: var(--yt-text-2);
    background: var(--yt-tonal);
  }
  .side {
    position: absolute;
    top: -4px;
    right: -8px;
    z-index: 2;
    display: flex;
  }
  .more {
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
  .vrow:hover .more,
  .vrow:focus-within .more,
  .menu-open .more {
    opacity: 1;
  }
  .more:focus-visible {
    opacity: 1;
    outline: 2px solid var(--yt-link);
  }
  @container omnitube (max-width: 640px) {
    .search .yt-thumb {
      width: 42%;
    }
    .search h3 {
      font: var(--yt-type-body-strong);
    }
    .snippet,
    .badges {
      display: none;
    }
    .channel.big {
      margin: 4px 0;
    }
  }
</style>
