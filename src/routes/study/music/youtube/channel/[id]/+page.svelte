<script lang="ts">
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    studyYoutubeChannel,
    studyYoutubeSubsAdd,
    studyYoutubeSubsRemove,
    studyYoutubeSubsIsSubscribed,
    type YoutubeChannel,
    type YoutubeChannelTab,
    type YoutubeSearchItem,
    type YoutubeSearchVideoItem,
  } from "$lib/study-bridge";
  import YoutubeItem from "$lib/study-music-youtube-components/YoutubeItem.svelte";
  import YoutubeSkeleton from "$lib/study-music-youtube-components/YoutubeSkeleton.svelte";
  import YoutubeError from "$lib/study-music-youtube-components/YoutubeError.svelte";
  import HeroGradient from "$lib/study-music-components/HeroGradient.svelte";
  import ChipsRow from "$lib/study-music-components/ChipsRow.svelte";
  import ExpandableText from "$lib/study-music-components/ExpandableText.svelte";
  import StickyHeader from "$lib/study-music-components/StickyHeader.svelte";
  import { playYoutubeVideoItem } from "$lib/study-music/youtube-play-helper";

  let channelId = $derived($page.params.id ?? "");
  let activeTab = $state<YoutubeChannelTab>("videos");
  let channel = $state<YoutubeChannel | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let subscribed = $state(false);
  let subBusy = $state(false);

  const tabChips = $derived([
    { id: "videos" as YoutubeChannelTab, label: $t("study.music.tab_songs") },
    { id: "playlists" as YoutubeChannelTab, label: $t("study.music.tab_albums") },
    { id: "shorts" as YoutubeChannelTab, label: $t("study.music.tab_videos") },
    { id: "about" as YoutubeChannelTab, label: $t("study.music.tab_about") },
  ]);

  async function load(tab: YoutubeChannelTab) {
    loading = true;
    error = null;
    try {
      channel = await studyYoutubeChannel({ channelId, tab });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function refreshSubscribed(id: string) {
    try {
      const res = await studyYoutubeSubsIsSubscribed({ channelId: id });
      subscribed = res.subscribed;
    } catch {
      /* ignore */
    }
  }

  async function toggleSub() {
    if (!channel || subBusy) return;
    subBusy = true;
    try {
      if (subscribed) {
        await studyYoutubeSubsRemove({ channelId: channel.header.channel_id });
        subscribed = false;
      } else {
        await studyYoutubeSubsAdd({
          channelId: channel.header.channel_id,
          title: channel.header.title,
          thumbnailUrl: channel.header.avatar_url ?? undefined,
        });
        subscribed = true;
      }
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      subBusy = false;
    }
  }

  function videoItems(items: YoutubeSearchItem[]): YoutubeSearchVideoItem[] {
    const out: YoutubeSearchVideoItem[] = [];
    for (const it of items) {
      if (it.kind === "video") out.push(it);
      else if (it.kind === "shelf") {
        for (const s of it.items) if (s.kind === "video") out.push(s);
      }
    }
    return out;
  }

  async function play(videoId: string) {
    if (!channel) return;
    const queue = videoItems(channel.items);
    const item = queue.find((v) => v.video_id === videoId);
    if (!item) return;
    try {
      await playYoutubeVideoItem(item, queue);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  function openPlaylist(playlistId: string) {
    const route = playlistId.startsWith("OLAK") ? "album" : "playlist";
    void goto(`/study/music/youtube/${route}/${encodeURIComponent(playlistId)}`);
  }

  async function shuffleRadio() {
    if (!channel) return;
    const queue = videoItems(channel.items);
    if (queue.length === 0) {
      showToast("info", $t("study.music.shelf_recent_empty"));
      return;
    }
    const shuffled = queue.slice();
    for (let i = shuffled.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
    }
    try {
      await playYoutubeVideoItem(shuffled[0], shuffled);
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  function switchTab(tab: string) {
    const target = tab as YoutubeChannelTab;
    if (activeTab === target) return;
    activeTab = target;
    void load(target);
  }

  $effect(() => {
    const id = channelId;
    if (id) {
      void refreshSubscribed(id);
      void load(activeTab);
    }
  });
</script>

<section class="channel-page">
  <StickyHeader>
    <div class="sticky-row">
      <button class="back" onclick={() => goto("/study/music/youtube")} type="button">
        ← {$t("study.music.back")}
      </button>
      {#if channel}
        <span class="sticky-title">{channel.header.title}</span>
      {/if}
    </div>
  </StickyHeader>

  {#if loading && !channel}
    <div class="grid-skel">
      <YoutubeSkeleton kind="card" count={6} />
    </div>
  {:else if error && !channel}
    <YoutubeError error={error} onRetry={() => { error = null; void load(activeTab); }} />
  {:else if channel}
    <HeroGradient coverUrl={channel.header.avatar_url ?? channel.header.banner_url} minHeight={260}>
      <div class="hero-inner">
        {#if channel.header.banner_url}
          <img class="banner" src={channel.header.banner_url} alt="" />
        {/if}
        <div class="hero-meta">
          <div class="avatar">
            {#if channel.header.avatar_url}
              <img src={channel.header.avatar_url} alt="" />
            {/if}
          </div>
          <div class="info">
            <h1 class="title">{channel.header.title}</h1>
            <p class="meta">
              {#if channel.header.handle}
                <span>{channel.header.handle}</span>
              {/if}
              {#if channel.header.subscribers_text}
                <span class="sep" aria-hidden="true"> · </span>
                <span>{channel.header.subscribers_text}</span>
              {/if}
              {#if channel.header.video_count_text}
                <span class="sep" aria-hidden="true"> · </span>
                <span>{channel.header.video_count_text}</span>
              {/if}
            </p>
            <div class="hero-actions">
              <button
                class="action primary"
                type="button"
                onclick={shuffleRadio}
                aria-label={$t("study.music.shuffle_radio")}
              >
                {$t("study.music.shuffle_radio")}
              </button>
              <button
                class="action sub-btn"
                class:is-subscribed={subscribed}
                disabled={subBusy}
                onclick={toggleSub}
                type="button"
                aria-label={subscribed ? $t("study.music.youtube.channel_unsubscribe_aria") : $t("study.music.youtube.channel_subscribe")}
              >
                <span class="default-label">{subscribed ? $t("study.music.youtube.channel_subscribed") : $t("study.music.youtube.channel_subscribe")}</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </HeroGradient>

    <ChipsRow
      chips={tabChips}
      activeId={activeTab}
      onSelect={switchTab}
      ariaLabel={$t("study.music.tab_about")}
    />

    {#if activeTab === "about"}
      <article class="about">
        {#if channel.header.description}
          <ExpandableText text={channel.header.description} maxLines={6} />
        {:else}
          <p class="muted">{$t("study.music.no_description")}</p>
        {/if}
      </article>
    {:else if loading}
      <div class="grid">
        <YoutubeSkeleton kind="card" count={6} />
      </div>
    {:else if error}
      <YoutubeError error={error} onRetry={() => { error = null; void load(activeTab); }} />
    {:else if channel.items.length === 0}
      <p class="muted">{$t("study.music.youtube.channel_no_items")}</p>
    {:else}
      <div class="grid">
        {#each channel.items as item, idx (idx)}
          <YoutubeItem item={item} onPlay={play} onPlaylistOpen={openPlaylist} />
        {/each}
      </div>
    {/if}
  {/if}
</section>

<style>
  .channel-page {
    display: flex;
    flex-direction: column;
    gap: 16px;
    margin: -16px;
    padding: 20px 24px 48px;
    background: #0f0f0f;
    color: #f1f1f1;
    min-height: 100%;
    border-radius: 14px;
    font-family:
      "Roboto", "YouTube Sans", system-ui, -apple-system, "Segoe UI", sans-serif;
  }
  .sticky-row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 32px;
  }
  .sticky-title {
    font-size: 14px;
    font-weight: 700;
    color: rgba(255, 255, 255, 0.9);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0;
    transition: opacity 200ms ease;
  }
  :global(.sticky-header.opaque) .sticky-title {
    opacity: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    .sticky-title {
      transition: none;
    }
  }
  .back {
    align-self: flex-start;
    padding: 6px 10px;
    background: transparent;
    border: 0;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 13px;
  }
  .back:hover { color: var(--secondary); }
  .hero-inner {
    position: relative;
    min-height: 260px;
  }
  .banner {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0.35;
    z-index: 0;
  }
  .hero-meta {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 24px;
  }
  .avatar {
    width: 96px;
    height: 96px;
    border-radius: 50%;
    overflow: hidden;
    background: rgba(255, 255, 255, 0.08);
    flex-shrink: 0;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  }
  .avatar img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .info { display: flex; flex-direction: column; gap: 6px; min-width: 0; color: #fff; }
  .title { margin: 0; font-size: 28px; font-weight: 800; }
  .meta { margin: 0; font-size: 13px; color: rgba(255, 255, 255, 0.7); }
  .sep { color: rgba(255, 255, 255, 0.35); }
  .hero-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 8px; }
  .action {
    padding: 10px 18px;
    border-radius: 999px;
    border: none;
    background: rgba(255, 255, 255, 0.08);
    color: #fff;
    font: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
  }
  .action:hover:not(:disabled) { background: rgba(255, 255, 255, 0.14); }
  .action.primary {
    background: var(--accent);
    color: var(--on-accent);
    border-color: transparent;
  }
  .action.primary:hover:not(:disabled) { filter: brightness(1.08); }
  .sub-btn { padding: 10px 18px; }
  .sub-btn:disabled { opacity: 0.6; cursor: not-allowed; }
  .about { padding: 8px 4px; color: rgba(255, 255, 255, 0.78); font-size: 14px; line-height: 1.5; }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 18px 14px;
  }
  .muted { color: var(--tertiary); font-size: 13px; }
  .grid-skel {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 18px 14px;
  }
</style>
