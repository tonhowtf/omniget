<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { MusicSearchFilter } from "$lib/tube/api";
  import { bestAvatar, thumbUrl } from "$lib/tube/format";
  import { playRadio } from "$lib/tube/music.svelte";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import type { Item, Page, Shelf } from "$lib/tube/types";
  import MusicShelf from "../../music/MusicShelf.svelte";
  import SongRow from "../../music/SongRow.svelte";
  import SquareCard from "../../music/SquareCard.svelte";
  import TIcon from "../../TIcon.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";
  import { itemKey, nearEnd, openItem, playItem, songFromItem, subtitleOf, thumbsOf, titleOf } from "./shared.svelte";

  let { route }: { route: Extract<Route, { name: "m_search" }> } = $props();

  const FILTERS: { id: MusicSearchFilter; key: string }[] = [
    { id: "all", key: "tube.music.filter_all" },
    { id: "songs", key: "tube.music.filter_songs" },
    { id: "videos", key: "tube.music.filter_videos" },
    { id: "albums", key: "tube.music.filter_albums" },
    { id: "artists", key: "tube.music.filter_artists" },
    { id: "playlists", key: "tube.music.filter_playlists" },
    { id: "community_playlists", key: "tube.music.filter_community" },
    { id: "podcasts", key: "tube.music.filter_podcasts" },
  ];

  let filter = $derived<MusicSearchFilter>(route.filter ?? "all");
  let page = $state<Page | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let more = $state(false);
  let seq = 0;

  let topShelf = $derived.by<Shelf | null>(() => {
    if (!page || filter !== "all" || !page.shelves.length) return null;
    const s = page.shelves[0];
    return s.id.includes("top") || (s.items.length > 0 && s.items.length <= 5 && s.kind !== "list") ? s : null;
  });
  let top = $derived<Item | null>(topShelf?.items[0] ?? null);
  let beside = $derived.by(() => {
    if (!page || !topShelf) return [];
    const extra = topShelf.items.slice(1).filter((i) => songFromItem(i));
    if (extra.length) return extra.slice(0, 4);
    const songs = page.shelves.find((s) => s !== topShelf && s.items.some((i) => songFromItem(i)));
    return songs ? songs.items.slice(0, 4) : [];
  });
  let rest = $derived(page ? page.shelves.filter((s) => s !== topShelf) : []);

  async function load() {
    const my = ++seq;
    loading = true;
    error = null;
    try {
      const res = await api.musicSearch(route.q, route.filter ?? "all");
      if (my === seq) page = res;
    } catch (e) {
      if (my === seq) error = String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function loadMore() {
    if (!page?.continuation || more) return;
    more = true;
    const token = page.continuation;
    try {
      const res = await api.continuation("music_search", token);
      if (page && page.continuation === token) {
        if (page.shelves.length === 1 && res.shelves.length >= 1) {
          for (const s of res.shelves) page.shelves[0].items.push(...s.items);
        } else page.shelves.push(...res.shelves);
        page.continuation = res.continuation ?? null;
      }
    } catch {
      if (page) page.continuation = null;
    } finally {
      more = false;
    }
  }

  function setFilter(f: MusicSearchFilter) {
    go({ name: "m_search", q: route.q, filter: f }, { replace: true });
  }

  $effect(() => {
    void route.q;
    void route.filter;
    void load();
  });

  function layoutFor(s: Shelf): "grid" | "list" | "carousel" {
    if (s.items.length && s.items.every((i) => i.kind === "song" || (i.kind === "video" && filter === "songs"))) return "list";
    return filter === "all" ? "carousel" : "grid";
  }
</script>

<MusicPage readiness={loading ? "loading" : error ? "error" : page && page.shelves.length ? "ready" : "empty"} label={$t("tube.music.search_results_for", { query: route.q }) as string}>
  <div class="se-chips" role="toolbar" aria-label={$t("tube.music.search_filters") as string}>
    {#each FILTERS as f (f.id)}
      <button class="se-chip" class:is-on={filter === f.id} aria-pressed={filter === f.id} onclick={() => setFilter(f.id)}>{$t(f.key)}</button>
    {/each}
  </div>

  {#if page?.corrected_query}
    <p class="se-corrected">
      {$t("tube.music.showing_results_for")}
      <button class="se-link" onclick={() => go({ name: "m_search", q: page!.corrected_query!, filter }, { replace: true })}>{page.corrected_query}</button>
    </p>
  {/if}

  {#if loading}
    <MusicSkeleton variant={filter === "all" ? "shelves" : filter === "songs" || filter === "videos" ? "list" : "grid"} />
  {:else if error}
    <MusicState icon="explore" error title={$t("tube.music.error_title") as string} text={error} onretry={load} />
  {:else if !page || !page.shelves.length}
    <MusicState icon="explore" title={$t("tube.music.search_empty_title") as string} text={$t("tube.music.search_empty_text") as string} />
  {:else}
    {#if top}
      {@const round = top.kind === "artist" || top.kind === "channel"}
      <section class="se-top-wrap" aria-label={$t("tube.music.top_result") as string}>
        <div class="se-top">
          <h2 class="se-h">{topShelf?.title || $t("tube.music.top_result")}</h2>
          <div class="se-card">
            <button class="se-art" class:round onclick={() => openItem(top!)} aria-label={$t("tube.music.open_named", { title: titleOf(top) }) as string}>
              <img src={round ? bestAvatar(thumbsOf(top), 96) : thumbUrl(thumbsOf(top), 96)} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
            </button>
            <button class="se-title yt-clamp-2" onclick={() => openItem(top!)}>{titleOf(top)}</button>
            <p class="se-sub yt-clamp-1">{subtitleOf(top)}</p>
            <div class="se-actions">
              <button class="yt-btn se-play" onclick={() => void playItem(top!)}><TIcon name="play" size={20} />{$t("tube.music.play")}</button>
              {#if top.kind !== "song" && top.kind !== "video"}
                <button class="yt-btn se-alt" onclick={() => void playItem(top!, true)}><TIcon name="shuffle" size={20} />{$t("tube.music.shuffle")}</button>
              {:else}
                <button class="yt-btn se-alt" onclick={() => "id" in top! && void playRadio(top!.id)}><TIcon name="radio" size={20} />{$t("tube.music.start_radio")}</button>
              {/if}
            </div>
          </div>
        </div>
        {#if beside.length}
          <div class="se-beside" role="list">
            {#each beside as it, i (itemKey(it, i))}
              {@const s = songFromItem(it)}
              {#if s}<div role="listitem"><SongRow song={s} showAlbum={false} /></div>{/if}
            {/each}
          </div>
        {/if}
      </section>
    {/if}

    {#each rest as shelf, i (shelf.id + i)}
      {@const mode = layoutFor(shelf)}
      {#if filter === "all" || shelf.title}
        <MusicShelf {shelf} layout={mode} listLimit={filter === "all" && mode === "list" ? 4 : 0} />
      {:else if mode === "list"}
        <div class="se-list" role="list">
          {#each shelf.items as it, j (itemKey(it, j))}
            {@const s = songFromItem(it)}
            {#if s}<div role="listitem"><SongRow song={s} /></div>{/if}
          {/each}
        </div>
      {:else}
        <div class="se-grid" role="list">
          {#each shelf.items as it, j (itemKey(it, j))}
            <div role="listitem"><SquareCard item={it} fluid /></div>
          {/each}
        </div>
      {/if}
    {/each}
    {#if page.continuation && filter !== "all"}
      <div use:nearEnd={loadMore}><MusicSkeleton variant="list" /></div>
    {/if}
  {/if}
</MusicPage>

<style>
  .se-chips { display: flex; gap: 12px; flex-wrap: wrap; margin: 0 0 32px; }
  .se-chip {
    height: 32px; padding: 0 12px; border-radius: 8px; border: 0;
    font: 500 14px/20px var(--yt-font); color: var(--ytm-text, var(--yt-text));
    background: var(--ytm-chip, var(--yt-tonal)); cursor: pointer; white-space: nowrap;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .se-chip:hover { background: var(--ytm-chip-hover, var(--yt-tonal-hover)); }
  .se-chip.is-on { background: var(--ytm-text, var(--yt-text)); color: var(--ytm-bg, var(--yt-bg)); }
  .se-chip:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .se-corrected { margin: -16px 0 24px; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); }
  .se-link { border: 0; padding: 0; background: none; font: 500 14px/20px var(--yt-font); font-style: italic; color: var(--ytm-link, var(--yt-link)); cursor: pointer; }
  .se-link:hover { text-decoration: underline; }

  .se-top-wrap { display: grid; grid-template-columns: minmax(300px, 400px) minmax(0, 1fr); gap: 32px; margin-bottom: 48px; align-items: end; }
  .se-h { margin: 0 0 16px; font: var(--yt-type-headline-m); }
  .se-card {
    position: relative; padding: 20px; border-radius: 12px;
    background: var(--ytm-bg-a, var(--yt-bg-raised));
    border: 1px solid var(--ytm-divider, var(--yt-divider));
  }
  .se-art { display: block; width: 96px; height: 96px; padding: 0; border: 0; border-radius: 4px; overflow: hidden; background: var(--yt-skeleton); cursor: pointer; }
  .se-art.round { border-radius: 50%; }
  .se-art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .se-art:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .se-title { all: unset; cursor: pointer; margin-top: 16px; font: 700 28px/36px var(--yt-font-display); color: var(--ytm-text, var(--yt-text)); display: -webkit-box; }
  .se-title:hover { text-decoration: underline; }
  .se-title:focus-visible { outline: 2px solid var(--yt-link); }
  .se-sub { margin: 4px 0 0; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); }
  .se-actions { display: flex; gap: 8px; margin-top: 20px; flex-wrap: wrap; }
  .se-play { background: var(--ytm-text, #fff); color: var(--ytm-bg, #000); }
  .se-play:hover { background: color-mix(in srgb, var(--ytm-text, #fff) 85%, transparent); }
  .se-alt { background: var(--ytm-chip, var(--yt-tonal)); color: var(--ytm-text, var(--yt-text)); }
  .se-alt:hover { background: var(--ytm-chip-hover, var(--yt-tonal-hover)); }
  .se-beside { display: flex; flex-direction: column; }
  .se-list { display: flex; flex-direction: column; }
  .se-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 32px 16px; }

  @container omnitube (max-width: 900px) {
    .se-top-wrap { grid-template-columns: 1fr; }
  }
</style>
