<script lang="ts">
  import { t } from "$lib/i18n";
  import { bestAvatar } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { ChannelItem } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import SubscribeButton from "./SubscribeButton.svelte";
  import { channelHref, openChannel } from "./util";

  let { item, layout = "row" }: { item: ChannelItem; layout?: "row" | "card" } = $props();

  let menuOpen = $state(false);
  let menuPoint = $state<{ x: number; y: number } | null>(null);
  let hideMetrics = $derived(tubePrefs().hide_metrics);
  let size = $derived(layout === "row" ? 136 : 104);

  function open(e: MouseEvent) {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
    e.preventDefault();
    openChannel(item.id);
  }
</script>

<article
  class="ch {layout}"
  oncontextmenu={(e) => {
    e.preventDefault();
    menuPoint = { x: e.clientX, y: e.clientY };
    menuOpen = true;
  }}
>
  <a class="cover" href={channelHref(item.id)} onclick={open} tabindex="-1" aria-hidden="true"></a>
  <div class="av">
    {#if item.avatar}
      <img class="yt-avatar" src={bestAvatar(item.avatar, size)} width={size} height={size} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
    {:else}
      <span class="yt-avatar ph" style:width="{size}px" style:height="{size}px">{item.name.slice(0, 1)}</span>
    {/if}
  </div>
  <div class="meta">
    <h3>
      <a class="name" href={channelHref(item.id)} onclick={open}>{item.name}</a>
      {#if item.verified}<TIcon name="verified" size={16} label={$t("tube.card.verified") as string} />{/if}
    </h3>
    <div class="sub-line">
      {#if item.handle}<span>{item.handle}</span>{/if}
      {#if item.handle && item.subscribers_text && !hideMetrics}<span aria-hidden="true">•</span>{/if}
      {#if item.subscribers_text && !hideMetrics}<span>{item.subscribers_text}</span>{/if}
    </div>
    {#if layout === "row" && item.description}
      <p class="desc yt-clamp-2">{item.description}</p>
    {/if}
  </div>
  <div class="act">
    <SubscribeButton channel={{ id: item.id, name: item.name, avatar: item.avatar }} subscribed={item.subscribed} size={layout === "card" ? "s" : "m"} />
  </div>
</article>

{#if menuOpen}
  <ItemMenu {item} point={menuPoint} feedbackActions={false} onclose={() => (menuOpen = false)} />
{/if}

<style>
  .ch {
    position: relative;
    isolation: isolate;
    display: flex;
    align-items: center;
    gap: 16px;
    min-width: 0;
  }
  .ch::before {
    content: "";
    position: absolute;
    inset: -8px;
    z-index: -1;
    border-radius: 16px;
    background: var(--yt-hover);
    opacity: 0;
    transition: opacity var(--yt-dur-fast);
    pointer-events: none;
  }
  .ch:hover::before {
    opacity: 1;
  }
  .cover {
    position: absolute;
    inset: 0;
    z-index: 1;
  }
  .row .av {
    display: flex;
    justify-content: center;
    width: min(360px, 45%);
    flex: none;
  }
  .ph {
    display: grid;
    place-items: center;
    font: 500 40px/1 var(--yt-font);
    color: var(--yt-text-2);
    text-transform: uppercase;
  }
  .meta {
    flex: 1;
    min-width: 0;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 4px;
    font: 400 18px/26px var(--yt-font);
    color: var(--yt-text-2);
  }
  .name {
    position: relative;
    z-index: 2;
    color: var(--yt-text);
    text-decoration: none;
  }
  .name:focus-visible {
    outline: 2px solid var(--yt-link);
    border-radius: 4px;
  }
  .sub-line {
    display: flex;
    flex-wrap: wrap;
    gap: 0 4px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .desc {
    margin: 8px 0 0;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .act {
    position: relative;
    z-index: 2;
    flex: none;
  }
  .card {
    flex-direction: column;
    text-align: center;
    gap: 12px;
    padding: 8px 0;
  }
  .card h3 {
    justify-content: center;
    font: var(--yt-type-card-title);
  }
  .card .sub-line {
    justify-content: center;
  }
  .card .meta {
    width: 100%;
  }
  @container omnitube (max-width: 640px) {
    .row .av {
      width: auto;
    }
    .row .av img,
    .row .ph {
      width: 72px !important;
      height: 72px !important;
    }
    .row {
      flex-wrap: wrap;
    }
  }
</style>
