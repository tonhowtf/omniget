<script lang="ts">
  // Shows found in the local library, as a grid; opening one goes to ShowView.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openPlayer } from "$lib/video/player.svelte";
  import { go } from "$lib/video/ui.svelte";
  import type { ShowSummary } from "$lib/video/types";
  import ContextMenu, { type MenuItem } from "$components/music/ContextMenu.svelte";
  import { addFolder, markWatched } from "./parts/actions";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import MediaCard from "./parts/MediaCard.svelte";
  import Skel from "./parts/Skel.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, onLibraryChanged, ready } from "./parts/util";

  type Filter = "all" | "unwatched" | "progress" | "watched";
  const FILTERS: Filter[] = ["all", "unwatched", "progress", "watched"];

  let list = $state<ShowSummary[] | null>(null);
  let error = $state<string | null>(null);
  let filter = $state<Filter>("all");
  let q = $state("");
  let cm = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  async function load() {
    try {
      await ready();
      list = await api.shows();
      error = null;
    } catch (e) {
      error = errText(e);
      list ??= [];
    }
  }

  onMount(() => {
    void load();
    return onLibraryChanged(() => void load());
  });

  let shown = $derived.by(() => {
    if (!list) return [];
    const needle = q.trim().toLowerCase();
    const coll = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
    return list
      .filter((s) => {
        if (needle && !s.name.toLowerCase().includes(needle)) return false;
        if (filter === "watched") return s.episodes > 0 && s.watched >= s.episodes;
        if (filter === "unwatched") return s.watched === 0;
        if (filter === "progress") return s.watched > 0 && s.watched < s.episodes;
        return true;
      })
      .sort((a, b) => coll.compare(a.name, b.name));
  });

  /** Plays the first unwatched episode of a show. */
  async function playNext(name: string) {
    try {
      const d = await api.show(name);
      const id = d.next ?? d.seasons[0]?.episodes[0]?.id;
      if (id != null) void openPlayer({ kind: "local", id });
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function markAll(name: string, watched: boolean) {
    try {
      const d = await api.show(name);
      const keys = d.seasons.flatMap((s) => s.episodes.map((e) => e.key));
      await markWatched(keys, watched, () => void load());
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function menu(e: MouseEvent, s: ShowSummary) {
    e.preventDefault();
    cm = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: $t("video.shows.play_next") as string, icon: "play", action: () => void playNext(s.name) },
        { label: $t("video.shows.open") as string, icon: "playlist", action: () => go({ name: "show", show: s.name }) },
        { divider: true },
        { label: $t("video.shows.mark_all_watched") as string, icon: "check", action: () => void markAll(s.name, true) },
        { label: $t("video.shows.mark_all_unwatched") as string, icon: "close", action: () => void markAll(s.name, false) },
      ],
    };
  }
</script>

<div class="page">
  <Head title={$t("video.shows.title") as string} subtitle={list?.length ? ($t("video.shows.count", { count: list.length }) as string) : ""} />

  {#if list && list.length}
    <div class="bar">
      <div class="segmented" role="tablist" aria-label={$t("video.common.filter") as string}>
        {#each FILTERS as f (f)}
          <button class="segmented-btn" role="tab" aria-selected={filter === f} onclick={() => (filter = f)}>{$t(`video.filter.${f}`)}</button>
        {/each}
      </div>
      <input class="input input-search" type="search" placeholder={$t("video.shows.search") as string} bind:value={q} />
    </div>
  {/if}

  {#if error}
    <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
  {/if}

  {#if !list}
    <Skel count={12} min={200} />
  {:else if !list.length}
    <Empty icon="series" title={$t("video.shows.empty_title") as string} body={$t("video.shows.empty_body") as string}>
      <button class="btn btn-primary" onclick={() => void addFolder("library").then((ok) => { if (ok) void load(); })}><VIcon name="folder" size={14} /> {$t("video.home.add_folder")}</button>
    </Empty>
  {:else if !shown.length}
    <Empty compact icon="series" title={$t("video.common.no_match") as string} />
  {:else}
    <div class="grid">
      {#each shown as s (s.name)}
        <MediaCard
          title={s.name}
          subtitle={$t("video.shows.meta", { seasons: s.seasons, episodes: s.episodes }) as string}
          src={s.thumb}
          icon="tv"
          progress={s.episodes && s.watched < s.episodes && s.watched > 0 ? s.watched / s.episodes : null}
          watched={s.episodes > 0 && s.watched >= s.episodes}
          badge={s.episodes > s.watched && s.watched > 0 ? ($t("video.shows.left", { count: s.episodes - s.watched }) as string) : ""}
          onopen={() => go({ name: "show", show: s.name })}
          onplay={() => void playNext(s.name)}
          onmenu={(e) => menu(e, s)}
        />
      {/each}
    </div>
  {/if}
</div>

{#if cm}<ContextMenu x={cm.x} y={cm.y} items={cm.items} onclose={() => (cm = null)} />{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .bar .input-search {
    flex: 1;
    min-width: 180px;
    max-width: 320px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-5) var(--space-4);
  }
</style>
