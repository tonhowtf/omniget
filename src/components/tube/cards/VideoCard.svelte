<script lang="ts">
  // Grid card (spec §2.5): 16:9 thumb with radius 12, duration / live badge,
  // watched bar, avatar 36, two-line title 16/500, channel and stats in 14px.
  // Hover paints a soft plate around the card and, after a pause, plays a
  // muted preview (21st HoverPlayCard). DeArrow swaps title/thumb with a
  // small toggle back to the original.
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { bestAvatar, fmtCount, fmtDuration, thumbUrl, timeAgo, videoThumb } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { Badge, StreamSet, VideoItem } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import { inview } from "./actions";
  import { dearrow, toggleOriginal, wantDearrow } from "./dearrow.svelte";
  import { channelHref, openChannel, openWatch, streamsFor, streamUrl, watchHref } from "./util";

  let {
    item,
    showAvatar = true,
    feedbackActions = true,
    list = null,
    index = null,
  }: {
    item: VideoItem;
    showAvatar?: boolean;
    feedbackActions?: boolean;
    /** Playlist context, when the card belongs to one. */
    list?: string | null;
    index?: number | null;
  } = $props();

  let prefs = $derived(tubePrefs());
  let da = $derived(prefs.dearrow && !dearrow.original[item.id] ? dearrow.map[item.id] : null);
  let hasDa = $derived(prefs.dearrow && !!dearrow.map[item.id] && !!(dearrow.map[item.id]?.title || dearrow.map[item.id]?.thumb_url));
  let title = $derived(da?.title || item.title);
  let thumb = $derived(
    (prefs.dearrow_thumbs && da?.thumb_url) || thumbUrl(item.thumbs, 360) || videoThumb(item.id, "hqdefault"),
  );
  let live = $derived(item.badges.includes("live"));
  let upcoming = $derived(item.badges.includes("upcoming") || item.badges.includes("premiere"));
  let views = $derived(
    prefs.hide_metrics ? "" : item.views_text || (item.views != null ? ($t("tube.card.views", { count: fmtCount(item.views) }) as string) : ""),
  );
  let when = $derived(item.published_text || (item.published_ms ? timeAgo(item.published_ms) : ""));
  let small = $derived(item.badges.filter((b) => b === "new" || b === "hd4k" || b === "cc" || b === "members" || b === "auto_dubbed"));

  let menuOpen = $state(false);
  let menuPoint = $state<{ x: number; y: number } | null>(null);
  let moreBtn = $state<HTMLButtonElement>();

  // hover preview
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;
  let hovering = false;
  let previewSrc = $state<string | null>(null);
  let previewOn = $state(false);
  let midThumb = $state(false);
  let pvPos = $state(0);
  let pvDur = $state(0);

  function pickPreview(set: StreamSet): number | null {
    const fs = set.formats.filter((f) => (f.kind === "muxed" || f.kind === "video") && (f.height ?? 0) > 0 && (f.height ?? 0) <= 480);
    fs.sort((a, b) => {
      const ma = a.kind === "muxed" ? 0 : 1;
      const mb = b.kind === "muxed" ? 0 : 1;
      const ca = a.codecs.startsWith("avc1") ? 0 : 1;
      const cb = b.codecs.startsWith("avc1") ? 0 : 1;
      return ma - mb || ca - cb || (b.height ?? 0) - (a.height ?? 0);
    });
    return fs[0]?.itag ?? null;
  }

  async function startPreview() {
    try {
      const set = await streamsFor(item.id);
      if (!hovering) return;
      const itag = set.is_live ? null : pickPreview(set);
      if (itag == null) {
        midThumb = true;
        return;
      }
      previewSrc = streamUrl(set, itag);
    } catch {
      if (hovering) midThumb = true;
    }
  }

  function enter() {
    hovering = true;
    if (!prefs.preview_on_hover || live || upcoming || menuOpen) return;
    hoverTimer = setTimeout(startPreview, 600);
  }

  function leave() {
    hovering = false;
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = null;
    previewSrc = null;
    previewOn = false;
    midThumb = false;
  }

  function open(e: MouseEvent) {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
    e.preventDefault();
    leave();
    openWatch(item.id, { list, index });
  }

  function openMenu(e?: MouseEvent) {
    leave();
    menuPoint = e && e.type === "contextmenu" ? { x: e.clientX, y: e.clientY } : null;
    menuOpen = true;
  }

  function badgeLabel(b: Badge): string {
    return $t(`tube.badge.${b}`) as string;
  }
</script>

<article
  class="vcard"
  class:menu-open={menuOpen}
  use:inview={(v) => v && prefs.dearrow && wantDearrow(item.id)}
  onmouseenter={enter}
  onmouseleave={leave}
  oncontextmenu={(e) => {
    e.preventDefault();
    openMenu(e);
  }}
>
  <a class="cover" href={watchHref(item.id, list)} onclick={open} tabindex="-1" aria-hidden="true"></a>
  <div class="yt-thumb">
    <img
      src={midThumb ? `https://i.ytimg.com/vi/${item.id}/hq2.jpg` : thumb}
      alt=""
      loading="lazy"
      decoding="async"
      referrerpolicy="no-referrer"
    />
    {#if previewSrc}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        class="preview"
        class:on={previewOn}
        src={previewSrc}
        muted
        autoplay
        playsinline
        preload="auto"
        onplaying={() => (previewOn = true)}
        ontimeupdate={(e) => {
          pvPos = e.currentTarget.currentTime;
          pvDur = e.currentTarget.duration || item.duration_s || 0;
        }}
      ></video>
      {#if previewOn}
        <span class="pv-remaining">{fmtDuration(Math.max(0, pvDur - pvPos))}</span>
        <span class="pv-bar"><i style:width="{pvDur ? (pvPos / pvDur) * 100 : 0}%"></i></span>
      {/if}
    {/if}
    {#if !previewOn}
      {#if live}
        <span class="yt-duration is-live"><TIcon name="live" size={12} />{badgeLabel("live")}</span>
      {:else if upcoming}
        <span class="yt-duration">{badgeLabel(item.badges.includes("premiere") ? "premiere" : "upcoming")}</span>
      {:else if item.duration_s}
        <span class="yt-duration">{fmtDuration(item.duration_s)}</span>
      {/if}
      {#if item.watched_pct}
        <span class="yt-resume"><i style:width="{Math.min(100, item.watched_pct)}%"></i></span>
      {/if}
    {/if}
  </div>

  <div class="details">
    {#if showAvatar && item.channel}
      <a class="avatar" href={channelHref(item.channel.id)} onclick={(e) => { e.preventDefault(); openChannel(item.channel!.id); }} tabindex="-1" aria-hidden="true">
        {#if item.channel.avatar}
          <img class="yt-avatar" src={bestAvatar(item.channel.avatar, 36)} width="36" height="36" alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
        {:else}
          <span class="yt-avatar ph">{item.channel.name.slice(0, 1)}</span>
        {/if}
      </a>
    {/if}
    <div class="meta">
      <h3>
        <a use:agentActions={["click"]} class="title yt-clamp-2" href={watchHref(item.id, list)} onclick={open} title={title}>{title}</a>
      </h3>
      {#if item.channel}
        <a class="channel" href={channelHref(item.channel.id)} onclick={(e) => { e.preventDefault(); openChannel(item.channel!.id); }}>
          <span class="yt-clamp-1">{item.channel.name}</span>
          {#if item.channel.verified}<TIcon name="verified" size={14} label={$t("tube.card.verified") as string} />{/if}
        </a>
      {/if}
      {#if views || when}
        <div class="stats">
          {#if views}<span>{views}</span>{/if}
          {#if views && when}<span class="dot" aria-hidden="true">•</span>{/if}
          {#if when}<span>{when}</span>{/if}
        </div>
      {/if}
      {#if small.length}
        <div class="badges">
          {#each small as b (b)}<span class="bdg">{badgeLabel(b)}</span>{/each}
        </div>
      {/if}
    </div>
    <div class="side">
      <button
        bind:this={moreBtn}
        class="more"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label={$t("tube.card.actions") as string}
        onclick={() => openMenu()}
      >
        <TIcon name="more-vert" size={20} />
      </button>
      {#if hasDa}
        <button
          class="da"
          class:orig={dearrow.original[item.id]}
          aria-pressed={!!dearrow.original[item.id]}
          title={$t(dearrow.original[item.id] ? "tube.card.dearrow_on" : "tube.card.dearrow_off") as string}
          aria-label={$t(dearrow.original[item.id] ? "tube.card.dearrow_on" : "tube.card.dearrow_off") as string}
          onclick={() => toggleOriginal(item.id)}
        >
          <img src="/icons/tube/dearrow-32.webp" width="18" height="18" alt="" />
        </button>
      {/if}
    </div>
  </div>
</article>

{#if menuOpen}
  <ItemMenu {item} anchor={moreBtn ?? null} point={menuPoint} {feedbackActions} onclose={() => (menuOpen = false)} />
{/if}

<style>
  .vcard {
    position: relative;
    isolation: isolate;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .vcard::before {
    content: "";
    position: absolute;
    inset: -8px;
    z-index: -1;
    border-radius: 16px;
    background: var(--yt-hover);
    opacity: 0;
    transition: opacity var(--yt-dur-fast) linear;
    pointer-events: none;
  }
  .vcard:hover::before,
  .vcard.menu-open::before {
    opacity: 1;
  }
  .yt-thumb > img {
    transition: opacity var(--yt-dur-preview) var(--yt-ease-standard);
  }
  .yt-duration {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    z-index: 1;
  }
  .yt-resume {
    z-index: 1;
  }
  .preview {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    background: #000;
    opacity: 0;
    transition: opacity var(--yt-dur-preview) var(--yt-ease-standard);
  }
  .preview.on {
    opacity: 1;
  }
  .pv-remaining {
    position: absolute;
    right: 8px;
    bottom: 10px;
    padding: 1px 4px;
    border-radius: 4px;
    font: 500 12px/18px var(--yt-font);
    color: #fff;
    background: var(--yt-ov-badge);
  }
  .pv-bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: var(--yt-ov-track);
  }
  .pv-bar > i {
    display: block;
    height: 100%;
    background: var(--yt-accent);
  }
  .details {
    position: relative;
    display: flex;
    gap: 12px;
    margin-top: 12px;
    padding-right: 24px;
    min-width: 0;
  }
  .avatar {
    position: relative;
    z-index: 2;
    flex: none;
    width: 36px;
    height: 36px;
  }
  .yt-avatar {
    display: block;
    width: 36px;
    height: 36px;
  }
  .ph {
    display: grid;
    place-items: center;
    font: 500 16px/1 var(--yt-font);
    color: var(--yt-text-2);
    text-transform: uppercase;
  }
  .meta {
    flex: 1;
    min-width: 0;
  }
  h3 {
    margin: 0 0 4px;
    font: var(--yt-type-card-title);
  }
  .title {
    color: var(--yt-text);
    text-decoration: none;
  }
  .cover {
    position: absolute;
    inset: 0;
    z-index: 1;
  }
  .title {
    position: relative;
    z-index: 2;
  }
  .title:focus-visible {
    outline: none;
  }
  .vcard:has(.title:focus-visible)::before {
    opacity: 1;
    box-shadow: 0 0 0 2px var(--yt-link);
  }
  .channel {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    width: fit-content;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
    text-decoration: none;
  }
  .channel:hover {
    color: var(--yt-text);
  }
  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: 0 4px;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
  }
  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 6px;
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
    top: -6px;
    right: -14px;
    z-index: 2;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .more,
  .da {
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
  .da {
    width: 32px;
    height: 32px;
  }
  .da.orig img {
    filter: grayscale(1);
    opacity: 0.7;
  }
  .more:hover,
  .da:hover {
    background: var(--yt-tonal);
  }
  .vcard:hover .more,
  .vcard:hover .da,
  .vcard:focus-within .more,
  .vcard:focus-within .da,
  .menu-open .more {
    opacity: 1;
  }
  .more:focus-visible,
  .da:focus-visible,
  .channel:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: 1px;
    opacity: 1;
  }
  @media (hover: none) {
    .more {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .preview,
    .yt-thumb > img {
      transition: none;
    }
  }
</style>
