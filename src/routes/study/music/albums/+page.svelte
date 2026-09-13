<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import { colorFromString } from "$lib/study-music/format";
  import { spotifyStore } from "$lib/study-music/spotify-store.svelte";

  type Album = {
    name: string;
    artist: string | null;
    track_count: number;
    total_duration_ms: number;
    year: number | null;
    cover_path: string | null;
  };

  type SortKey = "added" | "name" | "year" | "track_count";

  let albums = $state<Album[]>([]);
  let loading = $state(true);
  let q = $state("");
  let sortKey = $state<SortKey>("added");

  async function load() {
    loading = true;
    try {
      const res = await pluginInvoke<{ albums: Album[] }>(
        "study",
        "study:music:albums:list",
        { limit: 500 },
      );
      albums = res.albums ?? [];
    } finally {
      loading = false;
    }
  }

  function coverUrl(path: string | null | undefined): string | null {
    if (!path) return null;
    try {
      return convertFileSrc(path);
    } catch {
      return path;
    }
  }

  function openAlbum(album: Album) {
    const params = new URLSearchParams({ name: album.name });
    if (album.artist) params.set("artist", album.artist);
    goto(`/study/music/album/by-name?${params.toString()}`);
  }

  onMount(async () => {
    void load();
    if (
      spotifyStore.status.logged_in &&
      spotifyStore.savedAlbums.length === 0
    ) {
      try {
        await spotifyStore.loadAll();
      } catch {
        /* ignore */
      }
    }
  });

  function spotifyCoverFor(images: { url: string; width?: number | null }[]): string | null {
    return spotifyStore.pickImage(images, 300);
  }

  function openSpotifyAlbum(id: string) {
    goto(`/study/music/spotify/album/${id}`);
  }

  const filteredSpotifyAlbums = $derived.by(() => {
    const term = q.trim().toLowerCase();
    if (!term) return spotifyStore.savedAlbums;
    return spotifyStore.savedAlbums.filter(
      (a) =>
        a.name.toLowerCase().includes(term) ||
        a.artists.some((ar) => ar.name.toLowerCase().includes(term)),
    );
  });

  const filtered = $derived.by(() => {
    const term = q.trim().toLowerCase();
    let list = albums;
    if (term) {
      list = list.filter(
        (a) =>
          a.name.toLowerCase().includes(term) ||
          (a.artist?.toLowerCase() ?? "").includes(term),
      );
    }
    const cmp = (a: Album, b: Album) => {
      switch (sortKey) {
        case "name":
          return a.name.localeCompare(b.name, undefined, { numeric: true });
        case "year":
          return (b.year ?? 0) - (a.year ?? 0);
        case "track_count":
          return b.track_count - a.track_count;
        default:
          return 0;
      }
    };
    return sortKey === "added" ? list : [...list].sort(cmp);
  });
</script>

<section class="albums-page">
  <header class="page-head">
    <h1>{$t("study.music.nav_albums")}</h1>
    <div class="head-actions">
      <input
        type="search"
        class="search"
        placeholder={$t("study.music.search_placeholder")}
        bind:value={q}
      />
      <select class="sort" bind:value={sortKey}>
        <option value="added">{$t("study.music.sort_added")}</option>
        <option value="name">{$t("study.music.sort_name")}</option>
        <option value="year">{$t("study.music.sort_year")}</option>
        <option value="track_count">{$t("study.music.sort_track_count")}</option>
      </select>
    </div>
  </header>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if filtered.length === 0}
    <div class="empty">
      <p>{$t("study.music.albums_empty")}</p>
    </div>
  {:else}
    <p class="result-count">{filtered.length} álbum(ns)</p>
    <div class="album-grid">
      {#each filtered as album (album.name + (album.artist ?? ""))}
        <div
          class="album-card"
          role="button"
          tabindex="0"
          onclick={() => openAlbum(album)}
          onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openAlbum(album); } }}
        >
          <div class="album-card-cover">
            {#if coverUrl(album.cover_path)}
              <img src={coverUrl(album.cover_path)} alt="" loading="lazy" />
            {:else}
              <div class="album-card-fallback" style:background={colorFromString(album.name + (album.artist ?? ""))}>
                <svg viewBox="0 0 24 24" width="36%" height="36%" fill="currentColor" aria-hidden="true"><circle cx="12" cy="12" r="10" opacity="0.2"/><circle cx="12" cy="12" r="3"/></svg>
              </div>
            {/if}
            <div class="album-card-overlay"></div>
            <button
              type="button"
              class="album-card-play"
              onclick={(e) => { e.stopPropagation(); openAlbum(album); }}
              aria-label={$t("study.music.album.open_aria")}
            >
              <svg viewBox="0 0 24 24" width="11" height="11" fill="currentColor" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>
            </button>
          </div>
          <h3 class="album-card-title">{album.name}</h3>
          {#if album.artist}
            <p class="album-card-sub">{album.artist}</p>
          {/if}
          <p class="album-card-meta">
            {album.track_count} faixa(s){album.year ? ` · ${album.year}` : ""}
          </p>
        </div>
      {/each}
    </div>
  {/if}

  {#if filteredSpotifyAlbums.length > 0}
    <section class="spotify-block">
      <header class="block-head">
        <h2>
          <span class="spotify-mark" aria-hidden="true">
            <svg viewBox="0 0 168 168" width="14" height="14"><circle cx="84" cy="84" r="84" fill="#1db954"/><path fill="#000" d="M119.6 110.6c-1.5 2.5-4.7 3.3-7.2 1.8-19.7-12-44.5-14.7-73.7-8-2.8.6-5.6-1.1-6.3-3.9-.6-2.8 1.1-5.6 3.9-6.3 31.9-7.3 59.4-4.2 81.5 9.2 2.5 1.5 3.3 4.7 1.8 7.2zm9.5-21.2c-1.9 3.1-5.9 4.1-9 2.2-22.6-13.9-57-17.9-83.8-9.8-3.5 1.1-7.1-.9-8.2-4.3-1.1-3.5.9-7.1 4.3-8.2 30.6-9.3 68.5-4.8 94.5 11.1 3.1 1.9 4.1 5.9 2.2 9zm.8-22c-27-16-71.6-17.5-97.4-9.7-4.1 1.2-8.4-1.1-9.6-5.2-1.2-4.1 1.1-8.4 5.2-9.6 29.6-9 78.7-7.2 109.8 11.3 3.7 2.2 4.9 7 2.7 10.7-2.2 3.7-7 4.9-10.7 2.5z"/></svg>
          </span>
          Seus álbuns Spotify
        </h2>
      </header>
      <div class="album-grid">
        {#each filteredSpotifyAlbums as album (album.id)}
          <button
            type="button"
            class="album-card"
            onclick={() => openSpotifyAlbum(album.id)}
          >
            <div class="album-card-cover">
              {#if spotifyCoverFor(album.images)}
                <img src={spotifyCoverFor(album.images)} alt="" loading="lazy" />
              {:else}
                <div class="album-card-fallback" style:background={colorFromString(album.name + (album.artists[0]?.name ?? ""))}>
                  <svg viewBox="0 0 24 24" width="36%" height="36%" fill="currentColor" aria-hidden="true"><circle cx="12" cy="12" r="10" opacity="0.2"/><circle cx="12" cy="12" r="3"/></svg>
                </div>
              {/if}
              <div class="album-card-overlay"></div>
            </div>
            <h3 class="album-card-title">{album.name}</h3>
            {#if album.artists.length > 0}
              <p class="album-card-sub">{album.artists.map((a) => a.name).join(", ")}</p>
            {/if}
            <p class="album-card-meta">
              {album.total_tracks} faixa(s){album.release_date ? ` · ${album.release_date.slice(0, 4)}` : ""}
            </p>
          </button>
        {/each}
      </div>
    </section>
  {/if}
</section>

<style>
  .albums-page {
    display: flex;
    flex-direction: column;
    gap: 20px;
    color: rgba(255, 255, 255, 0.95);
  }
  .spotify-block { display: flex; flex-direction: column; gap: 16px; margin-top: 24px; padding-top: 24px; border-top: none; }
  .block-head { display: flex; align-items: center; gap: 12px; }
  .block-head h2 { margin: 0; font-size: 18px; font-weight: 800; color: var(--secondary); display: inline-flex; align-items: center; gap: 10px; }
  .spotify-mark { display: inline-flex; align-items: center; }
  .spotify-block .album-card { background: transparent; border: 0; padding: 0; cursor: pointer; color: inherit; text-align: left; }
  .page-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .page-head h1 {
    margin: 0;
    font-size: clamp(28px, 3.5vw, 40px);
    font-weight: 900;
    letter-spacing: -0.02em;
    color: rgba(255, 255, 255, 0.95);
  }
  .head-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .search {
    padding: 8px 14px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.05);
    border: none;
    color: rgba(255, 255, 255, 0.95);
    font-family: inherit;
    font-size: 13px;
    min-width: 240px;
    outline: none;
  }
  .search:focus { border-color: var(--accent); }
  .sort {
    padding: 8px 14px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.05);
    border: none;
    color: rgba(255, 255, 255, 0.95);
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    outline: none;
  }
  .sort:hover { background: rgba(255, 255, 255, 0.1); }
  .result-count {
    margin: 0;
    font-size: 12px;
    color: rgba(255, 255, 255, 0.5);
  }
  .empty {
    padding: 64px 24px;
    text-align: center;
    color: rgba(255, 255, 255, 0.5);
  }
  .album-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 24px;
  }
  .album-card {
    cursor: pointer;
    background: transparent;
    border: 0;
    color: inherit;
    padding: 0;
    text-align: left;
  }
  .album-card-cover {
    position: relative;
    aspect-ratio: 1 / 1;
    border-radius: 16px;
    background: rgba(40, 40, 40, 0.8);
    overflow: hidden;
    margin-bottom: 14px;
  }
  .album-card-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: transform 500ms ease;
  }
  .album-card:hover .album-card-cover img {
    transform: scale(1.05);
  }
  .album-card-fallback {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    color: white;
  }
  .album-card-overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0);
    transition: background 300ms ease;
    border-radius: inherit;
  }
  .album-card:hover .album-card-overlay {
    background: rgba(0, 0, 0, 0.2);
  }
  .album-card-play {
    position: absolute;
    right: 12px;
    bottom: 12px;
    width: 40px;
    height: 40px;
    padding: 0;
    background: white;
    color: black;
    border: 0;
    border-radius: 50%;
    cursor: pointer;
    display: grid;
    place-items: center;
    opacity: 0;
    transform: translateY(8px);
    transition: opacity 300ms ease, transform 300ms ease;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.3);
  }
  .album-card:hover .album-card-play {
    opacity: 1;
    transform: translateY(0);
  }
  .album-card-title {
    margin: 0;
    color: white;
    font-weight: 700;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 4px;
  }
  .album-card-sub {
    margin: 4px 0 0;
    color: rgba(255, 255, 255, 0.5);
    font-size: 12px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 4px;
  }
  .album-card-meta {
    margin: 4px 0 0;
    color: rgba(255, 255, 255, 0.35);
    font-size: 11px;
    padding: 0 4px;
  }
  .muted { color: rgba(255, 255, 255, 0.5); font-size: 13px; }
  @media (prefers-reduced-motion: reduce) {
    .album-card-cover img, .album-card-overlay, .album-card-play { transition: none; }
    .album-card:hover .album-card-cover img { transform: none; }
  }
</style>
