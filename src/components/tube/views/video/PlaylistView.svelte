<script lang="ts">
  // Playlist (spec §2.11): a 360px panel with the cover blurred behind it,
  // title 28/700, owner, counts, Play all / Shuffle / Download all; the
  // numbered list on the right. Under ~1000px the panel sits on top.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { thumbUrl, videoThumb } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { PlaylistPage, VideoItem } from "$lib/tube/types";
  import type { Route } from "$lib/tube/ui.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../../TIcon.svelte";
  import Sentinel from "../../cards/Sentinel.svelte";
  import SkeletonGrid from "../../cards/SkeletonGrid.svelte";
  import State from "../../cards/State.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import VideoRow from "../../cards/VideoRow.svelte";
  import { isVisible } from "../../cards/feedback.svelte";
  import { channelHref, copyText, errText, openChannel, openWatch } from "../../cards/util";

  let { route }: { route: Extract<Route, { name: "playlist" }> } = $props();

  let data = $state<PlaylistPage | null>(null);
  let error = $state<string | null>(null);
  let loadingMore = $state(false);
  let moreError = $state<string | null>(null);
  let dlBusy = $state(false);
  let descOpen = $state(false);
  let gen = 0;

  async function load() {
    const g = ++gen;
    data = null;
    error = null;
    try {
      const p = await api.playlist(route.id);
      if (g === gen) data = p;
    } catch (e) {
      if (g === gen) error = errText(e);
    }
  }

  $effect(() => {
    void route.id;
    void load();
  });

  async function more() {
    const token = data?.continuation;
    if (!token || loadingMore || !data) return;
    loadingMore = true;
    moreError = null;
    const g = gen;
    try {
      const next = await api.continuation("playlist", token);
      if (g === gen && data) {
        data.items = [...data.items, ...next.shelves.flatMap((s) => s.items)];
        data.continuation = next.continuation ?? null;
      }
    } catch (e) {
      if (g === gen) moreError = errText(e);
    } finally {
      loadingMore = false;
    }
  }

  let videos = $derived((data?.items ?? []).filter((i): i is VideoItem => i.kind === "video"));
  let visible = $derived(videos.filter(isVisible));
  let cover = $derived(data ? thumbUrl(data.thumbs, 360) || (videos[0] ? videoThumb(videos[0].id, "hqdefault") : "") : "");
  let hideMetrics = $derived(tubePrefs().hide_metrics);

  function playAll(shuffle = false) {
    if (!videos.length) return;
    const index = shuffle ? Math.floor(Math.random() * videos.length) : 0;
    openWatch(videos[index].id, { list: route.id, index });
  }

  async function downloadAll() {
    if (dlBusy || !videos.length) return;
    dlBusy = true;
    let ok = 0;
    for (const v of videos) {
      try {
        await api.download({ video_id: v.id, title: v.title, audio_only: false, music: false });
        ok++;
      } catch {
        // keep going: one private/removed video shouldn't stop the rest
      }
    }
    dlBusy = false;
    showToast(ok ? "success" : "error", $t("tube.playlist.queued_all", { count: ok }) as string);
  }
</script>

<div class="pl" data-agent-readiness={!data && !error ? "loading" : error ? "error" : "ready"} data-agent-surface="omnitube-playlist">
  {#if error}
    <State icon="library" {error} onretry={load} />
  {:else}
    <div class="layout">
      <aside class="panel">
        {#if cover}<img class="bg" src={cover} alt="" aria-hidden="true" referrerpolicy="no-referrer" />{/if}
        <div class="panel-in">
          {#if data}
            <button class="cover yt-thumb" onclick={() => playAll()} aria-label={$t("tube.playlist.play_all") as string}>
              {#if cover}<img src={cover} alt="" referrerpolicy="no-referrer" />{/if}
              <span class="cover-play"><TIcon name="play" size={20} />{$t("tube.playlist.play_all")}</span>
            </button>
            <h1>{data.title}</h1>
            {#if data.owner}
              <a class="owner" href={channelHref(data.owner.id)} onclick={(e) => { e.preventDefault(); openChannel(data!.owner!.id); }}>
                {#if data.owner.avatar}<img class="yt-avatar" src={data.owner.avatar} width="24" height="24" alt="" referrerpolicy="no-referrer" />{/if}
                <b>{data.owner.name}</b>
              </a>
            {/if}
            <div class="facts">
              {#if data.count_text}<span>{data.count_text}</span>{/if}
              {#if data.views_text && !hideMetrics}<span>{data.views_text}</span>{/if}
              {#if data.duration_text}<span>{data.duration_text}</span>{/if}
              {#if data.updated_text}<span>{data.updated_text}</span>{/if}
            </div>
            <div class="acts">
              <button class="yt-btn tonal-light" onclick={() => copyText(`https://www.youtube.com/playlist?list=${route.id}`, $t("tube.menu.link_copied") as string)} aria-label={$t("tube.menu.share") as string}>
                <TIcon name="share-arrow" size={20} />
              </button>
              <button class="yt-btn tonal-light" disabled={dlBusy || !videos.length} onclick={downloadAll} title={$t("tube.playlist.download_all") as string} aria-label={$t("tube.playlist.download_all") as string}>
                <TIcon name={dlBusy ? "loader" : "download"} size={20} />
              </button>
            </div>
            <div class="play">
              <button class="yt-btn yt-btn--inverted big" onclick={() => playAll()} disabled={!videos.length}>
                <TIcon name="play" size={20} />{$t("tube.playlist.play_all")}
              </button>
              <button class="yt-btn tonal-light big" onclick={() => playAll(true)} disabled={!videos.length}>
                <TIcon name="shuffle" size={20} />{$t("tube.playlist.shuffle")}
              </button>
            </div>
            {#if data.description}
              <button class="desc" onclick={() => (descOpen = !descOpen)} aria-expanded={descOpen}>
                <span class:yt-clamp-2={!descOpen}>{data.description}</span>
              </button>
            {/if}
          {:else}
            <div class="yt-skel cover-sk"></div>
            <div class="yt-skel" style="height: 32px; width: 80%"></div>
            <div class="yt-skel" style="height: 16px; width: 50%"></div>
            <div class="yt-skel" style="height: 40px; width: 100%; border-radius: 20px"></div>
          {/if}
        </div>
      </aside>

      <section class="items">
        {#if !data}
          <SkeletonGrid layout="list" count={10} />
        {:else if !visible.length}
          <State icon="library" title={$t("tube.playlist.empty") as string} compact />
        {:else}
          <div class="rows">
            {#each visible as v, i (v.id + ":" + i)}
              <VideoRow item={v} size="list" number={i + 1} list={route.id} index={videos.indexOf(v)} feedbackActions={false} />
            {/each}
          </div>
          {#if data.continuation}
            <Sentinel onmore={more} loading={loadingMore} error={moreError} />
          {/if}
        {/if}
      </section>
    </div>
  {/if}
  <UndoPill />
</div>

<style>
  .pl {
    padding: 24px var(--yt-grid-gutter);
  }
  .layout {
    display: flex;
    align-items: flex-start;
    gap: 24px;
    max-width: 1500px;
    margin: 0 auto;
  }
  .panel {
    position: sticky;
    top: 0;
    flex: none;
    width: 360px;
    max-height: calc(100vh - var(--yt-masthead-h) - 48px);
    border-radius: 16px;
    overflow: hidden;
    isolation: isolate;
    background: var(--yt-bg-raised);
  }
  .bg {
    position: absolute;
    inset: -40px;
    width: calc(100% + 80px);
    height: calc(100% + 80px);
    object-fit: cover;
    filter: blur(60px) saturate(1.4);
    opacity: 0.8;
    z-index: -2;
  }
  .panel::after {
    content: "";
    position: absolute;
    inset: 0;
    z-index: -1;
    background: linear-gradient(to bottom, rgba(0, 0, 0, 0.15), color-mix(in srgb, var(--yt-bg) 80%, transparent) 70%, var(--yt-bg));
  }
  .panel-in {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 24px;
  }
  .cover {
    display: block;
    width: 100%;
    padding: 0;
    border: 0;
    cursor: pointer;
  }
  .cover:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  .cover-sk {
    aspect-ratio: 16 / 9;
    border-radius: 12px;
  }
  .cover-play {
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
    transition: opacity var(--yt-dur-base);
  }
  .cover:hover .cover-play {
    opacity: 0.85;
  }
  h1 {
    margin: 4px 0 0;
    font: 700 28px/36px var(--yt-font-display);
    color: var(--yt-text);
    word-break: break-word;
  }
  .owner {
    display: flex;
    align-items: center;
    gap: 8px;
    width: fit-content;
    font: var(--yt-type-meta);
    color: var(--yt-text);
    text-decoration: none;
  }
  .owner:hover b {
    text-decoration: underline;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 0 8px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .facts span + span::before {
    content: "• ";
  }
  .acts {
    display: flex;
    gap: 8px;
  }
  .acts .yt-btn {
    width: 40px;
    height: 40px;
    padding: 0;
  }
  .tonal-light {
    background: color-mix(in srgb, var(--yt-text) 12%, transparent);
  }
  .tonal-light:hover {
    background: color-mix(in srgb, var(--yt-text) 20%, transparent);
  }
  .play {
    display: flex;
    gap: 8px;
  }
  .big {
    flex: 1;
    height: 40px;
  }
  .desc {
    padding: 0;
    border: 0;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
    background: none;
    text-align: left;
    white-space: pre-line;
    cursor: pointer;
  }
  .items {
    flex: 1;
    min-width: 0;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  @container omnitube (max-width: 1000px) {
    .layout {
      flex-direction: column;
      align-items: stretch;
    }
    .panel {
      position: relative;
      width: 100%;
      max-height: none;
    }
    .panel-in {
      display: grid;
      grid-template-columns: minmax(0, 280px) minmax(0, 1fr);
      column-gap: 24px;
    }
    .panel-in > :global(*:not(.cover):not(.cover-sk)) {
      grid-column: 2;
    }
    .cover,
    .cover-sk {
      grid-row: 1 / span 6;
    }
  }
  @container omnitube (max-width: 600px) {
    .pl {
      padding: 16px 12px;
    }
    .panel-in {
      display: flex;
    }
  }
</style>
