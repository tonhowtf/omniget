<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { fmtLong, looksUpscaled, searchTracks, shuffled, sortTracks, type SortKey } from "$lib/music/library";
  import { library, trackById } from "$lib/music/library-store.svelte";
  import { enqueue, playTracks } from "$lib/music/player-store.svelte";
  import { analyze, showTrackMenu, ui } from "$lib/music/ui-store.svelte";
  import type { HistoryEntry, Track } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";
  import TrackList from "../TrackList.svelte";

  let {
    mode = "all",
    genre = "",
    query = "",
  }: { mode?: "all" | "liked" | "recent" | "genre" | "search"; genre?: string; query?: string } = $props();

  type Filter = "lossless" | "lyrics" | "unanalyzed" | "upscaled" | "unplayed";
  let filters = $state(new Set<Filter>());
  let local = $state("");
  let sort = $state<SortKey | null>(null);
  let desc = $state(false);
  let selected = $state(new Set<number>());
  let history = $state<HistoryEntry[]>([]);

  onMount(async () => {
    if (mode === "recent") history = await invoke<HistoryEntry[]>("music_history", { limit: 500 }).catch(() => []);
  });

  let lib = $derived(library());

  let base = $derived.by((): Track[] => {
    void lib.version;
    switch (mode) {
      case "liked":
        return lib.tracks.filter((x) => x.liked);
      case "genre": {
        const g = genre.toLowerCase();
        return lib.tracks.filter((x) => x.genre.split(/[;,/]/).some((p) => p.trim().toLowerCase() === g));
      }
      case "recent": {
        const seen = new Set<number>();
        const out: Track[] = [];
        for (const h of history) {
          if (seen.has(h.track_id)) continue;
          seen.add(h.track_id);
          const tr = trackById(h.track_id);
          if (tr) out.push(tr);
        }
        return out;
      }
      case "search":
        return searchTracks(lib.tracks, query);
      default:
        return lib.tracks;
    }
  });

  let shown = $derived.by(() => {
    let list = searchTracks(base, local);
    if (filters.has("lossless")) list = list.filter((x) => ["FLAC", "ALAC", "WAV", "AIFF", "WavPack", "APE"].includes(x.codec));
    if (filters.has("lyrics")) list = list.filter((x) => x.has_lyrics);
    if (filters.has("unanalyzed")) list = list.filter((x) => !x.analyzed);
    if (filters.has("upscaled")) list = list.filter((x) => looksUpscaled(x));
    if (filters.has("unplayed")) list = list.filter((x) => x.play_count === 0);
    if (sort) return sortTracks(list, sort, desc);
    if (mode === "recent" || mode === "search") return list;
    return sortTracks(list, mode === "liked" ? "added" : "artist", mode === "liked");
  });

  let totalMs = $derived(shown.reduce((a, x) => a + x.duration_ms, 0));

  function toggleFilter(f: Filter) {
    const n = new Set(filters);
    if (n.has(f)) n.delete(f);
    else n.add(f);
    filters = n;
  }

  function onsort(k: SortKey) {
    if (sort === k) {
      if (desc) {
        sort = null;
        desc = false;
      } else desc = true;
    } else {
      sort = k;
      desc = false;
    }
  }

  let title = $derived(
    mode === "liked"
      ? $t("music.nav.liked")
      : mode === "recent"
        ? $t("music.nav.recent")
        : mode === "genre"
          ? genre
          : mode === "search"
            ? $t("music.search.results", { query })
            : $t("music.nav.songs"),
  );
  let sel = $derived([...selected]);
  const FILTERS: Filter[] = ["lossless", "lyrics", "unplayed", "unanalyzed", "upscaled"];
</script>

<div class="sv">
  <header class="sv-head">
    <div>
      <h1>{title}</h1>
      <p class="sub">{$t("music.count_songs", { count: shown.length })} · {fmtLong(totalMs)}</p>
    </div>
    <div class="acts">
      <button class="btn btn-primary" onclick={() => void playTracks(shown.map((x) => x.id), undefined, title as string)} disabled={!shown.length}><MusicIcon name="play" size={14} /> {$t("music.play")}</button>
      <button class="btn" onclick={() => void playTracks(shuffled(shown.map((x) => x.id)), undefined, title as string)} disabled={!shown.length}><MusicIcon name="shuffle" size={14} /> {$t("music.shuffle")}</button>
    </div>
  </header>

  <div class="sv-tools">
    <label class="find">
      <MusicIcon name="search" size={14} />
      <input class="input input-search" bind:value={local} placeholder={$t("music.filter_placeholder") as string} />
    </label>
    <div class="chips">
      {#each FILTERS as f (f)}
        <button class="btn btn-pill btn-sm" class:active={filters.has(f)} aria-pressed={filters.has(f)} onclick={() => toggleFilter(f)}>{$t(`music.filter.${f}`)}</button>
      {/each}
    </div>
  </div>

  {#if sel.length > 1}
    <div class="selbar">
      <span>{$t("music.selected", { count: sel.length })}</span>
      <button class="btn btn-sm" onclick={() => void playTracks(shown.filter((x) => selected.has(x.id)).map((x) => x.id))}><MusicIcon name="play" size={13} /> {$t("music.play")}</button>
      <button class="btn btn-sm" onclick={() => enqueue(sel, false)}><MusicIcon name="queue" size={13} /> {$t("music.menu.add_queue")}</button>
      <button class="btn btn-sm" onclick={() => (ui.playlistPicker = sel)}><MusicIcon name="playlist" size={13} /> {$t("music.menu.add_playlist")}</button>
      <button class="btn btn-sm" onclick={() => (ui.tagEditor = sel)}><MusicIcon name="edit" size={13} /> {$t("music.menu.edit_tags")}</button>
      <button class="btn btn-sm" onclick={() => analyze(sel)}><MusicIcon name="activity" size={13} /> {$t("music.menu.analyze")}</button>
      <button class="btn btn-sm btn-ghost" onclick={() => (selected = new Set())}>{$t("music.clear_selection")}</button>
    </div>
  {/if}

  <div class="sv-list">
    <TrackList
      tracks={shown}
      bind:selected
      {sort}
      {desc}
      {onsort}
      columns={mode === "recent" ? ["album", "plays"] : ["album", "year", "plays", "bpm", "key", "quality"]}
      onplay={(tr, list) => void playTracks(list.map((x) => x.id), tr.id, title as string)}
      onmenu={(e, ids) => showTrackMenu(e, ids)}
      emptyText={local || filters.size ? ($t("music.no_match") as string) : ""}
    />
  </div>
</div>

<style>
  .sv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    gap: var(--space-3);
  }
  .sv-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
    letter-spacing: var(--track-tight);
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .acts {
    display: flex;
    gap: var(--space-2);
  }
  .sv-tools {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
    flex: 0 1 280px;
  }
  .find input {
    flex: 1;
  }
  .chips {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .selbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    padding: 6px var(--space-3);
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    font-size: var(--text-sm);
  }
  .selbar span {
    font-weight: 600;
    margin-right: var(--space-2);
  }
  .sv-list {
    flex: 1;
    min-height: 0;
    border-radius: var(--radius-md);
  }
</style>
