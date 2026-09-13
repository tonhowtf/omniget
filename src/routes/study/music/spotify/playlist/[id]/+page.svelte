<script lang="ts">
  import { onMount } from "svelte";
import { t } from "$lib/i18n";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    spotifyStore,
    type SpotifyPlaylist,
    type SpotifyTrack,
  } from "$lib/study-music/spotify-store.svelte";
  import { colorFromString } from "$lib/study-music/format";

  let tracks = $state<SpotifyTrack[]>([]);
  let playlist = $state<SpotifyPlaylist | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  const playlistId = $derived($page.params.id ?? "");

  onMount(async () => {
    if (!playlistId) {
      goto("/study/music/spotify");
      return;
    }
    if (!spotifyStore.status.logged_in) {
      goto("/study/music/spotify");
      return;
    }
    if (spotifyStore.playlists.length === 0) {
      try {
        await spotifyStore.loadAll();
      } catch {
        /* ignore */
      }
    }
    playlist = spotifyStore.playlists.find((p) => p.id === playlistId) ?? null;

    try {
      tracks = await spotifyStore.loadPlaylistTracks(playlistId);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg.includes("403") || msg.toLowerCase().includes("forbidden")) {
        error =
          $t("study.music.spotify.restricted_playlist");
      } else {
        error = msg;
      }
    } finally {
      loading = false;
    }
  });

  async function playFromIndex(idx: number) {
    if (tracks.length === 0) return;
    const startTrack = tracks[idx] ?? tracks[0];
    const reordered = [
      ...tracks.slice(idx),
      ...tracks.slice(0, idx),
    ];
    try {
      const mode = await spotifyStore.playPlaylistContext(
        playlist?.uri ?? `spotify:playlist:${playlistId}`,
        startTrack,
        reordered,
      );
      if (mode === "youtube") {
        showToast("info", "Tocando via YouTube (modo Free)");
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  function fmtDuration(ms: number): string {
    const total = Math.floor(ms / 1000);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function fmtArtists(track: SpotifyTrack): string {
    return track.artists.map((a) => a.name).join(", ");
  }
</script>

<section class="playlist-page">
  <button type="button" class="back-btn" onclick={() => goto("/study/music/spotify")}>
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="15 18 9 12 15 6"/>
    </svg>
    Voltar
  </button>

  {#if playlist}
    <header class="hero">
      <div class="hero-cover">
        {#if spotifyStore.pickImage(playlist.images, 300)}
          <img src={spotifyStore.pickImage(playlist.images, 300)} alt="" />
        {:else}
          <div class="hero-fallback" style:background={colorFromString(playlist.name)}>
            <svg viewBox="0 0 24 24" width="40%" height="40%" fill="currentColor" aria-hidden="true">
              <path d="M9 18V5l12-2v13" opacity="0.4"/>
              <circle cx="6" cy="18" r="3" opacity="0.4"/>
              <circle cx="18" cy="16" r="3" opacity="0.4"/>
            </svg>
          </div>
        {/if}
      </div>
      <div class="hero-info">
        <span class="eyebrow">Playlist</span>
        <h1>{playlist.name}</h1>
        {#if playlist.description}
          <p class="desc">{playlist.description}</p>
        {/if}
        <p class="meta">
          {playlist.owner_name ?? ""} · {playlist.tracks_total} faixas
        </p>
        <button type="button" class="play-btn" onclick={() => playFromIndex(0)}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
            <path d="M8 5v14l11-7z"/>
          </svg>
          Tocar
        </button>
      </div>
    </header>
  {/if}

  {#if loading}
    <p class="muted">Carregando faixas…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else}
    <div class="track-list">
      {#each tracks as track, i (track.id + i)}
        <button type="button" class="track-row" onclick={() => playFromIndex(i)}>
          <span class="track-num">{i + 1}</span>
          <div class="track-cover">
            {#if spotifyStore.pickImage(track.album.images, 80)}
              <img src={spotifyStore.pickImage(track.album.images, 80)} alt="" loading="lazy" />
            {/if}
          </div>
          <div class="track-meta">
            <span class="track-title">{track.name}</span>
            <span class="track-artists">{fmtArtists(track)}</span>
          </div>
          <span class="track-album">{track.album.name}</span>
          <span class="track-dur">{fmtDuration(track.duration_ms)}</span>
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .playlist-page {
    display: flex;
    flex-direction: column;
    gap: 24px;
    color: rgba(255, 255, 255, 0.95);
  }
  .back-btn {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px 6px 8px;
    background: rgba(255, 255, 255, 0.05);
    border: 0;
    border-radius: 999px;
    color: rgba(255, 255, 255, 0.85);
    font-family: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }
  .back-btn:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .hero {
    display: flex;
    gap: 32px;
    align-items: flex-end;
    padding: 24px 0;
  }
  .hero-cover {
    width: 220px;
    height: 220px;
    border-radius: 8px;
    overflow: hidden;
    flex-shrink: 0;
    box-shadow: 0 4px 60px rgba(0, 0, 0, 0.5);
  }
  .hero-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .hero-fallback {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    color: white;
  }
  .hero-info {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .eyebrow {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: rgba(255, 255, 255, 0.6);
  }
  .hero-info h1 {
    margin: 0;
    font-size: clamp(28px, 4vw, 56px);
    font-weight: 900;
    letter-spacing: -0.02em;
    line-height: 1.05;
  }
  .desc {
    margin: 0;
    color: rgba(255, 255, 255, 0.6);
    font-size: 14px;
  }
  .meta {
    margin: 0;
    color: rgba(255, 255, 255, 0.5);
    font-size: 13px;
  }
  .play-btn {
    align-self: flex-start;
    margin-top: 8px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 12px 32px;
    background: #1db954;
    color: #000;
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 14px;
    font-weight: 700;
    cursor: pointer;
    transition: transform 200ms ease, background 200ms ease;
  }
  .play-btn:hover {
    background: #1ed760;
    transform: scale(1.04);
  }

  .track-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .track-row {
    display: grid;
    grid-template-columns: 40px 56px 1fr 1fr 60px;
    gap: 12px;
    align-items: center;
    padding: 8px 12px;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: inherit;
    cursor: pointer;
    text-align: left;
    transition: background 200ms ease;
  }
  .track-row:hover {
    background: rgba(255, 255, 255, 0.06);
  }
  .track-num {
    color: rgba(255, 255, 255, 0.5);
    font-size: 13px;
    text-align: center;
  }
  .track-cover {
    width: 44px;
    height: 44px;
    border-radius: 4px;
    overflow: hidden;
    background: rgba(40, 40, 40, 0.8);
  }
  .track-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .track-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .track-title {
    font-size: 14px;
    font-weight: 600;
    color: white;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .track-artists {
    font-size: 12px;
    color: rgba(255, 255, 255, 0.55);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .track-album {
    font-size: 13px;
    color: rgba(255, 255, 255, 0.55);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .track-dur {
    font-size: 13px;
    color: rgba(255, 255, 255, 0.55);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .muted {
    color: rgba(255, 255, 255, 0.5);
    font-size: 13px;
  }
  .error {
    color: #e22134;
    font-size: 13px;
  }

  @media (prefers-reduced-motion: reduce) {
    .play-btn { transition: none; }
    .play-btn:hover { transform: none; }
  }
</style>
