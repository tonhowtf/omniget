<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/video/api";
  import { openPlayer } from "$lib/video/player.svelte";
  import type { MediaItem } from "$lib/video/types";
  import ContextMenu, { type MenuItem } from "$components/music/ContextMenu.svelte";
  import { addFolder, itemMenu } from "./parts/actions";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import MediaCard from "./parts/MediaCard.svelte";
  import Skel from "./parts/Skel.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, fmtDur, lsGet, lsSet, onLibraryChanged, ratio, ready } from "./parts/util";

  type Filter = "all" | "unwatched" | "progress" | "watched";
  type Sort = "added" | "title" | "year" | "played";
  const FILTERS: Filter[] = ["all", "unwatched", "progress", "watched"];

  let list = $state<MediaItem[] | null>(null);
  let error = $state<string | null>(null);
  let filter = $state<Filter>(lsGet<Filter>("omniget.video.movies.filter", "all"));
  let sort = $state<Sort>(lsGet<Sort>("omniget.video.movies.sort", "added"));
  let q = $state("");
  let cm = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  async function load() {
    try {
      await ready();
      list = await api.items({ kind: "movie", limit: 10000 });
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

  $effect(() => lsSet("omniget.video.movies.filter", filter));
  $effect(() => lsSet("omniget.video.movies.sort", sort));

  let shown = $derived.by(() => {
    if (!list) return [];
    const needle = q.trim().toLowerCase();
    const out = list.filter((m) => {
      if (needle && !m.title.toLowerCase().includes(needle)) return false;
      const p = m.progress;
      if (filter === "watched") return !!p?.watched;
      if (filter === "unwatched") return !p?.watched;
      if (filter === "progress") return !!p && !p.watched && p.position_ms > 0;
      return true;
    });
    const coll = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
    out.sort((a, b) => {
      if (sort === "title") return coll.compare(a.title, b.title);
      if (sort === "year") return (b.year ?? 0) - (a.year ?? 0) || coll.compare(a.title, b.title);
      if (sort === "played") return (b.progress?.last_played ?? 0) - (a.progress?.last_played ?? 0);
      return b.added_at - a.added_at;
    });
    return out;
  });

  function menu(e: MouseEvent, it: MediaItem) {
    e.preventDefault();
    cm = { x: e.clientX, y: e.clientY, items: itemMenu(it, () => void load()) };
  }
</script>

<div class="page">
  <Head title={$t("video.movies.title") as string} subtitle={list?.length ? ($t("video.movies.count", { count: list.length }) as string) : ""} />

  {#if list && list.length}
    <div class="bar">
      <div class="segmented" role="tablist" aria-label={$t("video.common.filter") as string}>
        {#each FILTERS as f (f)}
          <button class="segmented-btn" role="tab" aria-selected={filter === f} onclick={() => (filter = f)}>{$t(`video.filter.${f}`)}</button>
        {/each}
      </div>
      <input class="input input-search" type="search" placeholder={$t("video.movies.search") as string} bind:value={q} />
      <label class="sort">
        <span>{$t("video.common.sort_by")}</span>
        <select class="input" bind:value={sort}>
          <option value="added">{$t("video.sort.added")}</option>
          <option value="title">{$t("video.sort.title")}</option>
          <option value="year">{$t("video.sort.year")}</option>
          <option value="played">{$t("video.sort.played")}</option>
        </select>
      </label>
    </div>
  {/if}

  {#if error}
    <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
  {/if}

  {#if !list}
    <Skel count={12} min={200} />
  {:else if !list.length}
    <Empty icon="movies" title={$t("video.movies.empty_title") as string} body={$t("video.movies.empty_body") as string}>
      <button class="btn btn-primary" onclick={() => void addFolder("library").then((ok) => { if (ok) void load(); })}><VIcon name="folder" size={14} /> {$t("video.home.add_folder")}</button>
    </Empty>
  {:else if !shown.length}
    <Empty compact icon="movies" title={$t("video.common.no_match") as string} />
  {:else}
    <div class="grid">
      {#each shown as m (m.id)}
        <MediaCard
          title={m.title}
          subtitle={[m.year, fmtDur(m.duration_ms)].filter(Boolean).join(" · ")}
          src={m.thumb}
          progress={ratio(m.progress)}
          watched={!!m.progress?.watched}
          offline={m.offline}
          badge={m.height && m.height >= 2000 ? "4K" : ""}
          onopen={() => void openPlayer({ kind: "local", id: m.id })}
          onplay={() => void openPlayer({ kind: "local", id: m.id })}
          onmenu={(e) => menu(e, m)}
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
  .sort {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-5) var(--space-4);
  }
</style>
