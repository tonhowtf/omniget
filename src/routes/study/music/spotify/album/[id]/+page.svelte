<script lang="ts">
  import { onMount } from "svelte";
import { t } from "$lib/i18n";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    spotifyStore,
    type SpotifyTrack,
  } from "$lib/study-music/spotify-store.svelte";
  import { colorFromString } from "$lib/study-music/format";

  type AlbumDetail = {
    id: string;
    name: string;
    uri: string;
    images: { url: string; width?: number | null; height?: number | null }[];
    artists: { id: string; name: string; uri: string }[];
    release_date: string | null;
    total_tracks: number;
    label?: string | null;
    popularity?: number | null;
    genres?: string[] | null;
  };

  let album = $state<AlbumDetail | null>(null);
  let tracks = $state<SpotifyTrack[]>([]);
  let isSaved = $state(false);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let savingToggle = $state(false);

  const albumId = $derived($page.params.id ?? "");

  onMount(async () => {
    if (!albumId) {
      goto("/study/music/spotify");
      return;
    }
    if (!spotifyStore.status.logged_in) {
      goto("/study/music/spotify");
      return;
    }

    try {
      const [info, savedRes] = await Promise.allSettled([
        spotifyStore.getAlbum(albumId),
        import("$lib/plugin-invoke").then(({ pluginInvoke }) =>
          pluginInvoke<boolean[]>("study", "study:spotify:albums:contains", {
            ids: [albumId],
          }),
        ),
      ]);

      if (info.status === "fulfilled" && info.value) {
        const a: any = info.value;
        album = {
          id: a.id,
          name: a.name ?? "",
          uri: a.uri ?? "",
          images: a.images ?? [],
          artists: (a.artists ?? []).map((x: any) => ({
            id: x.id,
            name: x.name,
            uri: x.uri,
          })),
          release_date: a.release_date ?? null,
          total_tracks: a.total_tracks ?? 0,
          label: a.label ?? null,
          popularity: a.popularity ?? null,
          genres: a.genres ?? [],
        };
      }
      if (savedRes.status === "fulfilled") {
        isSaved = savedRes.value?.[0] === true;
      }

      tracks = await spotifyStore.getAlbumTracks(albumId);
      tracks = tracks.map((t) => ({
        ...t,
        album: album
          ? {
              id: album.id,
              name: album.name,
              uri: album.uri,
              images: album.images,
              release_date: album.release_date ?? undefined,
            }
          : t.album,
      }));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  });

  async function playFromIndex(idx: number) {
    if (tracks.length === 0) return;
    const reordered = [...tracks.slice(idx), ...tracks.slice(0, idx)];
    try {
      const mode = await spotifyStore.playTrack(reordered[0], reordered);
      if (mode === "youtube") {
        showToast("info", "Tocando via YouTube (modo Free)");
      }
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  async function toggleSave() {
    if (savingToggle) return;
    savingToggle = true;
    try {
      isSaved = await spotifyStore.toggleSaveAlbum(albumId);
      showToast("success", isSaved ? "Álbum salvo" : "Álbum removido");
    } catch (e) {
      showToast("error", e instanceof Error ? e.message : String(e));
    } finally {
      savingToggle = false;
    }
  }

  function fmtDuration(ms: number): string {
    const total = Math.floor(ms / 1000);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }
</script>

<section class="album-page">
  <button type="button" class="back-btn" onclick={() => history.back()}>
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="15 18 9 12 15 6"/>
    </svg>
    Voltar
  </button>

  {#if album}
    <header class="hero">
      <div class="hero-cover">
        {#if spotifyStore.pickImage(album.images, 300)}
          <img src={spotifyStore.pickImage(album.images, 300)} alt="" />
        {:else}
          <div class="hero-fallback" style:background={colorFromString(album.name)}></div>
        {/if}
      </div>
      <div class="hero-info">
        <span class="eyebrow">Álbum</span>
        <h1>{album.name}</h1>
        <p class="meta">
          {album.artists.map((a) => a.name).join(", ")}{album.release_date ? ` · ${album.release_date.slice(0, 4)}` : ""}
          · {album.total_tracks} faixas
        </p>
        {#if album.genres && album.genres.length > 0}
          <p class="genres">{album.genres.slice(0, 3).join(" · ")}</p>
        {/if}
        <div class="actions">
          <button type="button" class="play-btn" onclick={() => playFromIndex(0)}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>
            Tocar
          </button>
          <button
            type="button"
            class="fav-btn"
            class:on={isSaved}
            onclick={toggleSave}
            disabled={savingToggle}
            aria-label={isSaved ? $t("study.music.spotify.remove_from_albums") : $t("study.music.spotify.save_album")}
          >
            <svg viewBox="0 0 24 24" width="18" height="18" fill={isSaved ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
            </svg>
          </button>
        </div>
      </div>
    </header>
  {/if}

  {#if loading}
    <p class="muted">Carregando…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else}
    <div class="track-list">
      {#each tracks as track, i (track.id + i)}
        <button type="button" class="track-row" onclick={() => playFromIndex(i)}>
          <span class="track-num">{i + 1}</span>
          <div class="track-meta">
            <span class="track-title">{track.name}</span>
            <span class="track-artists">{track.artists.map((a) => a.name).join(", ")}</span>
          </div>
          <span class="track-dur">{fmtDuration(track.duration_ms)}</span>
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .album-page { display: flex; flex-direction: column; gap: 24px; color: rgba(255, 255, 255, 0.95); }
  .back-btn {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 8px;
    padding: 6px 12px 6px 8px; background: rgba(255, 255, 255, 0.05); border: 0;
    border-radius: 999px; color: rgba(255, 255, 255, 0.85);
    font-family: inherit; font-size: 12px; font-weight: 600; cursor: pointer;
  }
  .back-btn:hover { background: rgba(255, 255, 255, 0.1); }
  .hero { display: flex; gap: 32px; align-items: flex-end; padding: 24px 0; }
  .hero-cover { width: 220px; height: 220px; border-radius: 8px; overflow: hidden; flex-shrink: 0; box-shadow: 0 4px 60px rgba(0,0,0,0.5); }
  .hero-cover img { width: 100%; height: 100%; object-fit: cover; }
  .hero-fallback { width: 100%; height: 100%; }
  .hero-info { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  .eyebrow { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.1em; color: rgba(255,255,255,0.6); }
  .hero-info h1 { margin: 0; font-size: clamp(28px, 4vw, 56px); font-weight: 900; letter-spacing: -0.02em; line-height: 1.05; }
  .meta, .genres { margin: 0; color: rgba(255,255,255,0.5); font-size: 13px; }
  .genres { color: rgba(255,255,255,0.4); }
  .actions { display: flex; align-items: center; gap: 12px; margin-top: 8px; }
  .play-btn { display: inline-flex; align-items: center; gap: 8px; padding: 12px 32px; background: #1db954; color: #000; border: 0; border-radius: 999px; font-family: inherit; font-size: 14px; font-weight: 700; cursor: pointer; transition: transform 200ms ease, background 200ms ease; }
  .play-btn:hover { background: #1ed760; transform: scale(1.04); }
  .fav-btn { width: 44px; height: 44px; padding: 0; background: rgba(255,255,255,0.08); border: none; border-radius: 50%; color: white; cursor: pointer; display: grid; place-items: center; transition: background 200ms ease; }
  .fav-btn:hover { background: rgba(255,255,255,0.16); }
  .fav-btn.on { color: #1db954; }
  .fav-btn:disabled { opacity: 0.5; cursor: default; }

  .track-list { display: flex; flex-direction: column; gap: 2px; }
  .track-row { display: grid; grid-template-columns: 40px 1fr 60px; gap: 12px; align-items: center; padding: 8px 12px; background: transparent; border: 0; border-radius: 6px; color: inherit; cursor: pointer; text-align: left; transition: background 200ms ease; }
  .track-row:hover { background: rgba(255,255,255,0.06); }
  .track-num { color: rgba(255,255,255,0.5); font-size: 13px; text-align: center; }
  .track-meta { display: flex; flex-direction: column; min-width: 0; }
  .track-title { font-size: 14px; font-weight: 600; color: white; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .track-artists { font-size: 12px; color: rgba(255,255,255,0.55); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .track-dur { font-size: 13px; color: rgba(255,255,255,0.55); text-align: right; font-variant-numeric: tabular-nums; }
  .muted { color: rgba(255,255,255,0.5); font-size: 13px; }
  .error { color: #e22134; font-size: 13px; }
  @media (prefers-reduced-motion: reduce) { .play-btn { transition: none; } .play-btn:hover { transform: none; } }
</style>
