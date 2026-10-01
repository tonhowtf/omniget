<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { bestAvatar } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { WatchInfo } from "$lib/tube/types";
  import type { Route } from "$lib/tube/ui.svelte";
  import { seek, vplayer } from "$lib/tube/video.svelte";
  import TIcon from "../TIcon.svelte";
  import Popup from "../cards/Popup.svelte";
  import SaveDialog from "../cards/SaveDialog.svelte";
  import SkeletonGrid from "../cards/SkeletonGrid.svelte";
  import State from "../cards/State.svelte";
  import SubscribeButton from "../cards/SubscribeButton.svelte";
  import UndoPill from "../cards/UndoPill.svelte";
  import { blockChannel } from "../cards/feedback.svelte";
  import { channelHref, copyText, errText, openChannel, openExternal, watchUrl } from "../cards/util";
  import VideoPlayer from "../player/VideoPlayer.svelte";
  import Chapters from "./Chapters.svelte";
  import Comments from "./Comments.svelte";
  import Description from "./Description.svelte";
  import DownloadMenu from "./DownloadMenu.svelte";
  import LikeButtons from "./LikeButtons.svelte";
  import PlaylistPanel from "./PlaylistPanel.svelte";
  import UpNext from "./UpNext.svelte";

  let { route }: { route: Extract<Route, { name: "watch" }> } = $props();

  let info = $state<WatchInfo | null>(null);
  let error = $state<string | null>(null);
  let dlOpen = $state(false);
  let saveOpen = $state(false);
  let moreOpen = $state(false);
  let dlBtn = $state<HTMLButtonElement>();
  let saveBtn = $state<HTMLButtonElement>();
  let moreBtn = $state<HTMLButtonElement>();
  let playerBox = $state<HTMLDivElement>();
  let gen = 0;

  async function load() {
    const g = ++gen;
    error = null;
    if (info && info.id !== route.id) info = null;
    try {
      const w = await api.watch(route.id, route.list ?? null, route.index ?? null);
      if (g === gen) info = w;
    } catch (e) {
      if (g === gen) error = errText(e);
    }
  }

  $effect(() => {
    void route.id;
    void route.list;
    void route.index;
    untrack(() => {
      dlOpen = saveOpen = moreOpen = false;
      void load();
    });
  });

  let prefs = $derived(tubePrefs());
  let cur = $derived(info && info.id === route.id ? info : null);
  let hasSide = $derived(!prefs.hide_related || !!cur?.playlist);
  let ch = $derived(cur?.channel ?? null);
  let title = $derived(cur?.title ?? vplayer.streams?.title ?? "");

  function onseek(s: number) {
    seek(s);
    playerBox?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }

  function share() {
    void copyText(watchUrl(route.id), $t("tube.menu.link_copied") as string);
  }
</script>

<div
  class="watch"
  class:theater={vplayer.theater}
  class:single={!hasSide}
  data-agent-readiness={!cur && !error ? "loading" : error && !cur ? "error" : "ready"}
  data-agent-surface="omnitube-watch"
>
  <div class="player" bind:this={playerBox}>
    <VideoPlayer videoId={route.id} list={route.list ?? null} index={route.index ?? null} t={route.t ?? null} info={cur} />
  </div>

  <div class="info">
    {#if cur}
      <h1 class="title">
        {#if cur.is_live}<span class="live">{$t("tube.badge.live")}</span>{/if}
        {title}
      </h1>
      <div class="owner-row">
        {#if ch}
          <div class="owner">
            <a class="av" href={channelHref(ch.id)} onclick={(e) => { e.preventDefault(); openChannel(ch!.id); }} aria-label={ch.name}>
              {#if ch.avatar}
                <img class="yt-avatar" src={bestAvatar(ch.avatar, 40)} width="40" height="40" alt="" referrerpolicy="no-referrer" />
              {:else}
                <span class="yt-avatar ph">{ch.name.slice(0, 1)}</span>
              {/if}
            </a>
            <div class="on">
              <a class="name" href={channelHref(ch.id)} onclick={(e) => { e.preventDefault(); openChannel(ch!.id); }}>
                <span class="yt-clamp-1">{ch.name}</span>
                {#if ch.verified}<TIcon name="verified" size={14} label={$t("tube.card.verified") as string} />{/if}
              </a>
              {#if ch.subscribers_text && !prefs.hide_metrics}<div class="subs">{ch.subscribers_text}</div>{/if}
            </div>
            <SubscribeButton channel={{ id: ch.id, name: ch.name, avatar: ch.avatar }} subscribed={ch.subscribed} />
          </div>
        {/if}
        <div class="actions">
          <LikeButtons info={cur} />
          <button class="yt-btn" onclick={share}><TIcon name="share-arrow" size={22} /><span class="lbl">{$t("tube.watch.share")}</span></button>
          <button bind:this={dlBtn} class="yt-btn" aria-haspopup="menu" aria-expanded={dlOpen} onclick={() => (dlOpen = !dlOpen)}>
            <TIcon name="download" size={22} /><span class="lbl">{$t("tube.watch.download")}</span>
          </button>
          <button bind:this={saveBtn} class="yt-btn" aria-haspopup="dialog" aria-expanded={saveOpen} onclick={() => (saveOpen = !saveOpen)}>
            <TIcon name={cur.saved ? "bookmark-fill" : "save"} size={22} /><span class="lbl">{$t("tube.watch.save")}</span>
          </button>
          <button bind:this={moreBtn} class="yt-btn icon" aria-haspopup="menu" aria-expanded={moreOpen} aria-label={$t("tube.card.actions") as string} onclick={() => (moreOpen = !moreOpen)}>
            <TIcon name="more-horiz" size={22} />
          </button>
        </div>
      </div>

      <Description info={cur} {onseek} />
      {#if cur.chapters.length > 1}
        <Chapters chapters={cur.chapters} {onseek} />
      {/if}
    {:else if error}
      <State icon="tube" {error} onretry={load} compact />
    {:else}
      <div class="info-sk" aria-hidden="true">
        <div class="yt-skel" style="height: 28px; width: 70%"></div>
        <div class="row-sk">
          <div class="yt-skel" style="width: 40px; height: 40px; border-radius: 50%"></div>
          <div class="yt-skel" style="height: 16px; width: 160px"></div>
          <div class="yt-skel" style="height: 36px; width: 110px; border-radius: 18px; margin-left: 8px"></div>
          <div class="yt-skel grow" style="height: 36px; border-radius: 18px"></div>
        </div>
        <div class="yt-skel" style="height: 96px; border-radius: 12px"></div>
      </div>
    {/if}
  </div>

  {#if hasSide}
    <aside class="side">
      {#if cur?.playlist}
        <PlaylistPanel playlist={cur.playlist} currentId={route.id} />
      {/if}
      {#if cur && !prefs.hide_related}
        <UpNext info={cur} />
      {:else if !cur && !error}
        <SkeletonGrid layout="compact" count={10} />
      {/if}
    </aside>
  {/if}

  {#if cur && !prefs.hide_comments}
    <div class="comm">
      <Comments token={cur.comments_token} countText={cur.comments_count_text} videoId={cur.id} ownerAvatar={ch?.avatar ?? null} ownerName={ch?.name ?? ""} {onseek} />
    </div>
  {/if}
</div>

{#if dlOpen && title}
  <DownloadMenu videoId={route.id} {title} anchor={dlBtn ?? null} onclose={() => (dlOpen = false)} />
{/if}
{#if saveOpen}
  <SaveDialog videoId={route.id} anchor={saveBtn ?? null} saved={cur?.saved ?? null} meta={cur ? { title: cur.title, channel_id: cur.channel?.id ?? null, channel_name: cur.channel?.name ?? null, avatar: cur.channel?.avatar ?? null } : null} onclose={() => (saveOpen = false)} />
{/if}
{#if moreOpen && cur}
  <Popup anchor={moreBtn ?? null} onclose={() => (moreOpen = false)} label={$t("tube.card.actions") as string}>
    <button class="yt-menu-item" role="menuitem" onclick={() => { moreOpen = false; void copyText(watchUrl(route.id, vplayer.position), $t("tube.menu.link_copied") as string); }}>
      <TIcon name="clock" />{$t("tube.watch.copy_at_time")}
    </button>
    <button class="yt-menu-item" role="menuitem" onclick={() => { moreOpen = false; void openExternal(watchUrl(route.id, vplayer.position)); }}>
      <TIcon name="external" />{$t("tube.menu.open_browser")}
    </button>
    {#if ch}
      <div class="yt-menu-sep"></div>
      <button class="yt-menu-item" role="menuitem" onclick={() => { moreOpen = false; void blockChannel({ id: ch!.id, name: ch!.name }, $t("tube.menu.blocked", { name: ch!.name }) as string); }}>
        <TIcon name="ban" />{$t("tube.menu.block_channel")}
      </button>
    {/if}
  </Popup>
{/if}
<UndoPill />

<style>
  .watch {
    display: grid;
    grid-template-columns: minmax(0, 1fr) var(--yt-related-w);
    grid-template-rows: auto auto 1fr;
    grid-template-areas:
      "player side"
      "info side"
      "comm side";
    column-gap: 24px;
    max-width: calc(1280px + 24px + var(--yt-related-w));
    margin: 0 auto;
    padding: 24px var(--yt-grid-gutter) 48px;
  }
  .watch.theater {
    grid-template-rows: auto auto 1fr;
    grid-template-areas:
      "player player"
      "info side"
      "comm side";
    max-width: none;
    padding-top: 0;
  }
  .watch.theater .player {
    margin: 0 calc(-1 * var(--yt-grid-gutter)) 12px;
    background: #000;
  }
  .watch.theater .info,
  .watch.theater .side,
  .watch.theater .comm {
    max-width: 1706px;
  }
  .watch.single {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto auto 1fr;
    grid-template-areas:
      "player"
      "info"
      "comm";
    max-width: 1280px;
  }
  .watch.single.theater {
    max-width: none;
  }
  .player {
    grid-area: player;
    min-width: 0;
  }
  .info {
    grid-area: info;
    min-width: 0;
  }
  .side {
    grid-area: side;
    min-width: 0;
  }
  .comm {
    grid-area: comm;
    min-width: 0;
  }
  .title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    margin: 12px 0 0;
    font: var(--yt-type-watch-title);
    color: var(--yt-text);
    word-break: break-word;
  }
  .live {
    display: inline-block;
    margin-right: 8px;
    padding: 0 6px;
    border-radius: 4px;
    vertical-align: 3px;
    font: 500 12px/20px var(--yt-font);
    color: #fff;
    background: var(--yt-live-red);
  }
  .owner-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 12px;
  }
  .owner {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .av {
    flex: none;
    display: inline-flex;
  }
  .ph {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    font: 500 16px/1 var(--yt-font);
    color: var(--yt-text-2);
    text-transform: uppercase;
  }
  .on {
    min-width: 0;
    margin-right: 12px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 4px;
    font: var(--yt-type-card-title);
    color: var(--yt-text);
    text-decoration: none;
  }
  .subs {
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .actions .icon {
    width: 36px;
    padding: 0;
  }
  .info-sk {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 12px;
  }
  .row-sk {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .row-sk .grow {
    flex: 1;
    max-width: 380px;
    margin-left: auto;
  }
  @container omnitube (max-width: 1000px) {
    .watch,
    .watch.theater,
    .watch.single {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto;
      grid-template-areas:
        "player"
        "info"
        "side"
        "comm";
      padding-top: 0;
    }
    .player {
      margin: 0 calc(-1 * var(--yt-grid-gutter));
    }
    .side {
      margin-top: 24px;
    }
  }
  @container omnitube (max-width: 720px) {
    .lbl {
      display: none;
    }
    .actions .yt-btn:not(.icon) {
      padding: 0 12px;
    }
  }
</style>
