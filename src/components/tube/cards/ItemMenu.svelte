<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { vplayer } from "$lib/tube/video.svelte";
  import type { Item, VideoItem } from "$lib/tube/types";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../TIcon.svelte";
  import DownloadMenu from "../watch/DownloadMenu.svelte";
  import Popup from "./Popup.svelte";
  import SaveDialog from "./SaveDialog.svelte";
  import { blockChannel, channelOf, dontRecommendChannel, notInterested } from "./feedback.svelte";
  import { copyText, errText, itemUrl, openExternal, openPlaylist, openWatch } from "./util";

  let {
    item,
    anchor = null,
    point = null,
    onclose,
    feedbackActions = true,
  }: {
    item: Item;
    anchor?: HTMLElement | null;
    point?: { x: number; y: number } | null;
    onclose: () => void;
    feedbackActions?: boolean;
  } = $props();

  let mode = $state<"menu" | "download" | "save">("menu");
  let isVideo = $derived(item.kind === "video" || item.kind === "short");
  let title = $derived("title" in item ? item.title : "name" in item ? item.name : "");
  let channel = $derived(channelOf(item));

  function asVideo(): VideoItem | null {
    if (item.kind === "video") return item;
    if (item.kind === "short") return { ...item, kind: "video", badges: [], views: null };
    return null;
  }

  function addToQueue() {
    const v = asVideo();
    if (v && !vplayer.upNext.some((x) => x.id === v.id)) vplayer.upNext.push(v);
    showToast("info", $t("tube.menu.queued") as string);
    onclose();
  }

  async function watchLater() {
    onclose();
    if (!("id" in item)) return;
    try {
      const ch = "channel" in item ? item.channel : null;
      await api.action("watch_later_add", {
        video_id: item.id,
        playlist_id: "WL",
        title: "title" in item ? item.title : null,
        channel_id: ch?.id ?? null,
        channel_name: ch?.name ?? null,
        avatar: ch?.avatar ?? null,
      });
      showToast("success", $t("tube.save.added_wl") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function share() {
    void copyText(itemUrl(item), $t("tube.menu.link_copied") as string);
    onclose();
  }

  function openBrowser() {
    void openExternal(itemUrl(item));
    onclose();
  }

  function playPlaylist() {
    if (item.kind !== "playlist") return;
    if (item.first_video_id) openWatch(item.first_video_id, { list: item.id, index: 0 });
    else openPlaylist(item.id);
    onclose();
  }

  function notInt() {
    notInterested(item, $t("tube.menu.hidden_video") as string);
    onclose();
  }

  function dontRecommend() {
    dontRecommendChannel(item, $t("tube.menu.hidden_channel") as string);
    onclose();
  }

  function block() {
    if (channel) void blockChannel(channel, $t("tube.menu.blocked", { name: channel.name }) as string);
    onclose();
  }
</script>

{#if mode === "download" && "id" in item}
  <DownloadMenu videoId={item.id} {title} {anchor} {onclose} />
{:else if mode === "save" && "id" in item}
  <SaveDialog
    videoId={item.id}
    {anchor}
    {onclose}
    meta={{
      title: "title" in item ? item.title : null,
      channel_id: "channel" in item ? (item.channel?.id ?? null) : null,
      channel_name: "channel" in item ? (item.channel?.name ?? null) : null,
      avatar: "channel" in item ? (item.channel?.avatar ?? null) : null,
    }}
  />
{:else}
  <Popup {anchor} {point} {onclose} label={$t("tube.menu.label") as string}>
    {#if isVideo}
      <button class="yt-menu-item" role="menuitem" onclick={addToQueue}><TIcon name="queue" />{$t("tube.menu.add_queue")}</button>
      <button class="yt-menu-item" role="menuitem" onclick={watchLater}><TIcon name="clock" />{$t("tube.menu.watch_later")}</button>
      <button class="yt-menu-item" role="menuitem" onclick={() => (mode = "save")}><TIcon name="save" />{$t("tube.menu.save")}</button>
      <button class="yt-menu-item" role="menuitem" onclick={() => (mode = "download")}><TIcon name="download" />{$t("tube.menu.download")}</button>
    {:else if item.kind === "playlist"}
      <button class="yt-menu-item" role="menuitem" onclick={playPlaylist}><TIcon name="play" />{$t("tube.menu.play_all")}</button>
    {/if}
    <button class="yt-menu-item" role="menuitem" onclick={share}><TIcon name="share-arrow" />{$t("tube.menu.share")}</button>
    <button class="yt-menu-item" role="menuitem" onclick={openBrowser}><TIcon name="external" />{$t("tube.menu.open_browser")}</button>
    {#if isVideo || channel}
      <div class="yt-menu-sep"></div>
    {/if}
    {#if isVideo && feedbackActions}
      <button class="yt-menu-item" role="menuitem" onclick={notInt}><TIcon name="eye-off" />{$t("tube.menu.not_interested")}</button>
      {#if channel}
        <button class="yt-menu-item" role="menuitem" onclick={dontRecommend}><TIcon name="minus" />{$t("tube.menu.dont_recommend")}</button>
      {/if}
    {/if}
    {#if channel}
      <button class="yt-menu-item" role="menuitem" onclick={block}><TIcon name="ban" />{$t("tube.menu.block_channel")}</button>
    {/if}
  </Popup>
{/if}
