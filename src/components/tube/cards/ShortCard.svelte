<script lang="ts">
  // Shorts card: 9:16 thumb radius 12, two-line title 16/500, views. No avatar.
  import { t } from "$lib/i18n";
  import { thumbUrl, videoThumb } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { ShortItem } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import { openWatch, watchHref } from "./util";

  let { item, feedbackActions = true }: { item: ShortItem; feedbackActions?: boolean } = $props();

  let menuOpen = $state(false);
  let menuPoint = $state<{ x: number; y: number } | null>(null);
  let moreBtn = $state<HTMLButtonElement>();
  let thumb = $derived(thumbUrl(item.thumbs, 240) || videoThumb(item.id, "hqdefault"));

  function open(e: MouseEvent) {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
    e.preventDefault();
    openWatch(item.id);
  }
</script>

<article
  class="short"
  class:menu-open={menuOpen}
  oncontextmenu={(e) => {
    e.preventDefault();
    menuPoint = { x: e.clientX, y: e.clientY };
    menuOpen = true;
  }}
>
  <a class="cover" href={watchHref(item.id)} onclick={open} tabindex="-1" aria-hidden="true"></a>
  <div class="yt-thumb">
    <img src={thumb} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
  </div>
  <div class="meta">
    <h3><a class="title yt-clamp-2" href={watchHref(item.id)} onclick={open} title={item.title}>{item.title}</a></h3>
    {#if item.views_text && !tubePrefs().hide_metrics}<div class="views">{item.views_text}</div>{/if}
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
  .short {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .cover {
    position: absolute;
    inset: 0;
    z-index: 1;
  }
  .yt-thumb {
    aspect-ratio: 9 / 16;
  }
  .yt-thumb > img {
    transition: transform var(--yt-dur-base) var(--yt-ease-standard);
  }
  .meta {
    position: relative;
    padding: 12px 28px 0 0;
  }
  h3 {
    margin: 0 0 4px;
    font: var(--yt-type-card-title);
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
  .views {
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
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
  .short:hover .more,
  .short:focus-within .more,
  .menu-open .more {
    opacity: 1;
  }
  .more:focus-visible {
    opacity: 1;
    outline: 2px solid var(--yt-link);
  }
</style>
