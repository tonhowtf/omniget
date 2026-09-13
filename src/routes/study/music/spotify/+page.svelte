<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { spotifyStore } from "$lib/study-music/spotify-store.svelte";
  import type {
    SpotifyArtist,
    SpotifyPlaylist,
    SpotifyTrack,
  } from "$lib/study-music/spotify-store.svelte";
  import { colorFromString } from "$lib/study-music/format";

  let unlistenSuccess: UnlistenFn | null = null;
  let unlistenError: UnlistenFn | null = null;
  let booting = $state(true);

  onMount(async () => {
    try {
      unlistenSuccess = await listen("study-spotify-auth-success", async () => {
        spotifyStore.authInProgress = false;
        await spotifyStore.refreshStatus();
        await spotifyStore.loadAll();
        void spotifyStore.prewarmSdk();
        showToast("success", $t("study.music.spotify_connected") as string);
      });
      unlistenError = await listen<{ error: string }>(
        "study-spotify-auth-error",
        (event) => {
          spotifyStore.authInProgress = false;
          spotifyStore.error = event.payload.error ?? ($t("study.music.spotify_auth_failed") as string);
          showToast("error", spotifyStore.error);
        },
      );
    } catch {
      /* ignore — listen unavailable in non-tauri context */
    }

    await spotifyStore.detectCapabilities();
    await spotifyStore.refreshStatus();
    if (spotifyStore.status.logged_in) {
      await spotifyStore.loadAll();
      void spotifyStore.prewarmSdk();
    }
    booting = false;
  });

  async function refreshLibrary() {
    await spotifyStore.loadAll();
    showToast("info", $t("study.music.spotify_library_updated") as string);
  }

  onDestroy(() => {
    unlistenSuccess?.();
    unlistenError?.();
  });

  async function doLogin() {
    try {
      await spotifyStore.login();
      showToast(
        "info",
        $t("study.music.spotify_auth_waiting") as string,
      );
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  async function doCancel() {
    await spotifyStore.cancelAuth();
  }

  async function doLogout() {
    await spotifyStore.logout();
    showToast("info", $t("study.music.spotify_disconnected") as string);
  }

  async function playTrack(track: SpotifyTrack, queue?: SpotifyTrack[]) {
    try {
      const mode = await spotifyStore.playTrack(track, queue);
      if (mode === "youtube") {
        showToast("info", $t("study.music.spotify_play_via_yt") as string);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  function openPlaylist(p: SpotifyPlaylist) {
    goto(`/study/music/spotify/playlist/${p.id}`);
  }

  function openArtist(a: SpotifyArtist) {
    goto(`/study/music/spotify/artist/${a.id}`);
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

  function scrollContainer(id: string, dir: 1 | -1) {
    const el = document.getElementById(id);
    if (!el) return;
    el.scrollBy({ left: dir * 360, behavior: "smooth" });
  }
</script>

<section class="spotify-home">
  <header class="page-head">
    <h1>Spotify</h1>
    {#if spotifyStore.status.logged_in && spotifyStore.profile}
      <div class="head-actions">
        {#if spotifyStore.isPremium}
          <button
            type="button"
            class="ghost-btn"
            onclick={() => goto("/study/music/spotify/search")}
            aria-label={$t("study.music.spotify_search_aria") as string}
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <circle cx="11" cy="11" r="8"/>
              <line x1="21" y1="21" x2="16.65" y2="16.65"/>
            </svg>
            {$t("study.music.spotify_search_label")}
          </button>
        {/if}
        <button type="button" class="ghost-btn" onclick={refreshLibrary}>
          {$t("study.music.spotify_refresh_label")}
        </button>
        <span class="profile-pill">
          {#if spotifyStore.profile.images?.[0]?.url}
            <img src={spotifyStore.profile.images[0].url} alt="" />
          {/if}
          <span>{spotifyStore.profile.display_name ?? spotifyStore.profile.id}</span>
          {#if spotifyStore.profile.product === "premium"}
            <span class="badge premium">Premium</span>
          {:else if spotifyStore.profile.product}
            <span class="badge">{spotifyStore.profile.product}</span>
          {/if}
        </span>
        <button type="button" class="ghost-btn" onclick={doLogout}>Sair</button>
      </div>
    {/if}
  </header>

  {#if booting}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if !spotifyStore.status.logged_in}
    <div class="login-card">
      <div class="login-art">
        <svg viewBox="0 0 168 168" width="120" height="120" aria-hidden="true">
          <circle cx="84" cy="84" r="84" fill="#1db954" />
          <path
            fill="#000"
            d="M119.6 110.6c-1.5 2.5-4.7 3.3-7.2 1.8-19.7-12-44.5-14.7-73.7-8-2.8.6-5.6-1.1-6.3-3.9-.6-2.8 1.1-5.6 3.9-6.3 31.9-7.3 59.4-4.2 81.5 9.2 2.5 1.5 3.3 4.7 1.8 7.2zm9.5-21.2c-1.9 3.1-5.9 4.1-9 2.2-22.6-13.9-57-17.9-83.8-9.8-3.5 1.1-7.1-.9-8.2-4.3-1.1-3.5.9-7.1 4.3-8.2 30.6-9.3 68.5-4.8 94.5 11.1 3.1 1.9 4.1 5.9 2.2 9zm.8-22c-27-16-71.6-17.5-97.4-9.7-4.1 1.2-8.4-1.1-9.6-5.2-1.2-4.1 1.1-8.4 5.2-9.6 29.6-9 78.7-7.2 109.8 11.3 3.7 2.2 4.9 7 2.7 10.7-2.2 3.7-7 4.9-10.7 2.5z"
          />
        </svg>
      </div>
      <h2>Conectar sua conta Spotify</h2>
      <p class="login-body">
        Veja sua biblioteca, playlists e histórico do Spotify dentro do OmniGet.
        Você pode tocar em qualquer dispositivo Spotify ativo (celular, app
        oficial). Em breve, tocar direto aqui também.
      </p>
      {#if !spotifyStore.status.has_client_id}
        <p class="warn">
          ⚠️ Client ID do Spotify não configurado. Reinstale ou rebuilde o plugin
          study.
        </p>
      {/if}
      {#if spotifyStore.error}
        <p class="error">{spotifyStore.error}</p>
      {/if}
      {#if spotifyStore.authInProgress}
        <div class="waiting">
          <span class="spinner"></span>
          <span>Aguardando autorização no navegador…</span>
          <button type="button" class="ghost-btn" onclick={doCancel}>Cancelar</button>
        </div>
      {:else}
        <button
          type="button"
          class="cta-spotify"
          onclick={doLogin}
          disabled={!spotifyStore.status.has_client_id}
        >
          Conectar com Spotify
        </button>
      {/if}
    </div>
  {:else if spotifyStore.loadingLibrary && spotifyStore.savedTracks.length === 0}
    <p class="muted">Carregando biblioteca…</p>
  {:else}
    {#if spotifyStore.isPremium && spotifyStore.widevineSupported}
      <div class="status-banner sdk">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>
        Tocando dentro do OmniGet (Web Playback SDK)
      </div>
    {:else if !spotifyStore.isPremium}
      <div class="status-banner info">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
        Modo Free: faixas tocam via YouTube em tempo real (match automático por título/artista).
      </div>
    {:else if spotifyStore.widevineSupported === false}
      <div class="status-banner warn">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
        Widevine DRM indisponível. Vai usar fallback YouTube.
      </div>
    {/if}
    {#if spotifyStore.recentlyPlayed.length > 0}
      <section class="block">
        <header class="block-head">
          <h2>Tocadas recentemente</h2>
        </header>
        <div class="album-pills-grid">
          {#each spotifyStore.recentlyPlayed.slice(0, 8) as track (track.id + (track.played_at ?? ""))}
            <button
              type="button"
              class="track-pill"
              onclick={() => playTrack(track)}
            >
              <div class="track-pill-cover">
                {#if spotifyStore.pickImage(track.album.images, 80)}
                  <img src={spotifyStore.pickImage(track.album.images, 80)} alt="" loading="lazy" />
                {:else}
                  <div
                    class="track-pill-fallback"
                    style:background={colorFromString(track.album.name)}
                  ></div>
                {/if}
              </div>
              <div class="track-pill-info">
                <span class="track-pill-title">{track.name}</span>
                <span class="track-pill-sub">{fmtArtists(track)}</span>
              </div>
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if spotifyStore.playlists.length > 0}
      <section class="block">
        <header class="block-head">
          <h2>Suas playlists</h2>
          <div class="scroll-arrows">
            <button type="button" class="arrow" onclick={() => scrollContainer("sp-playlists", -1)} aria-label={$t("study.music.spotify_prev_aria") as string}>
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="15 18 9 12 15 6"/></svg>
            </button>
            <button type="button" class="arrow" onclick={() => scrollContainer("sp-playlists", 1)} aria-label={$t("study.music.spotify_next_aria") as string}>
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/></svg>
            </button>
          </div>
        </header>
        <div id="sp-playlists" class="h-scroll">
          {#each spotifyStore.playlists as p (p.id)}
            <div
              class="album-card"
              role="button"
              tabindex="0"
              onclick={() => openPlaylist(p)}
              onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openPlaylist(p); } }}
            >
              <div class="album-card-cover">
                {#if spotifyStore.pickImage(p.images, 300)}
                  <img src={spotifyStore.pickImage(p.images, 300)} alt="" loading="lazy" />
                {:else}
                  <div class="album-card-fallback" style:background={colorFromString(p.name)}>
                    <svg viewBox="0 0 24 24" width="40%" height="40%" fill="currentColor" aria-hidden="true"><path d="M9 18V5l12-2v13" opacity="0.4"/><circle cx="6" cy="18" r="3" opacity="0.4"/><circle cx="18" cy="16" r="3" opacity="0.4"/></svg>
                  </div>
                {/if}
              </div>
              <h3 class="album-card-title">{p.name}</h3>
              <p class="album-card-sub">{p.tracks_total} faixa(s){p.owner_name ? ` · ${p.owner_name}` : ""}</p>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if spotifyStore.topArtists.length > 0}
      <section class="block">
        <header class="block-head">
          <h2>Seus artistas</h2>
          <div class="scroll-arrows">
            <button type="button" class="arrow" onclick={() => scrollContainer("sp-artists", -1)} aria-label={$t("study.music.spotify_prev_aria") as string}>
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="15 18 9 12 15 6"/></svg>
            </button>
            <button type="button" class="arrow" onclick={() => scrollContainer("sp-artists", 1)} aria-label={$t("study.music.spotify_next_aria") as string}>
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/></svg>
            </button>
          </div>
        </header>
        <div id="sp-artists" class="h-scroll">
          {#each spotifyStore.topArtists as a (a.id)}
            <div
              class="artist-card"
              role="button"
              tabindex="0"
              onclick={() => openArtist(a)}
              onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openArtist(a); } }}
            >
              <div class="artist-circle">
                {#if spotifyStore.pickImage(a.images, 200)}
                  <img src={spotifyStore.pickImage(a.images, 200)} alt="" loading="lazy" />
                {:else}
                  <div class="artist-fallback" style:background={colorFromString(a.name)}>
                    {a.name.slice(0, 1).toUpperCase()}
                  </div>
                {/if}
              </div>
              <h3 class="artist-name">{a.name}</h3>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if spotifyStore.savedTracks.length > 0}
      <section class="block">
        <header class="block-head">
          <h2>Curtidas</h2>
        </header>
        <div class="track-list">
          {#each spotifyStore.savedTracks.slice(0, 30) as track, i (track.id)}
            <button
              type="button"
              class="track-row"
              onclick={() =>
                playTrack(track, spotifyStore.savedTracks.slice(0, 30))}
            >
              <span class="track-num">{i + 1}</span>
              <div class="track-cover">
                {#if spotifyStore.pickImage(track.album.images, 80)}
                  <img src={spotifyStore.pickImage(track.album.images, 80)} alt="" loading="lazy" />
                {/if}
              </div>
              <div class="track-meta">
                <span class="track-title-row">
                  <span class="track-title">{track.name}</span>
                  {#if spotifyStore.localMatches.has(track.id)}
                    <span class="local-badge" title={$t("study.music.spotify.in_local_library")}>
                      ●
                    </span>
                  {/if}
                </span>
                <span class="track-artists">{fmtArtists(track)}</span>
              </div>
              <span class="track-album">{track.album.name}</span>
              <span class="track-dur">{fmtDuration(track.duration_ms)}</span>
            </button>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</section>

<style>
  .spotify-home {
    display: flex;
    flex-direction: column;
    gap: 32px;
    color: rgba(255, 255, 255, 0.95);
  }
  .page-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .page-head h1 {
    margin: 0;
    font-size: clamp(28px, 3.5vw, 40px);
    font-weight: 900;
    letter-spacing: -0.02em;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .profile-pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 4px 12px 4px 4px;
    background: rgba(255, 255, 255, 0.05);
    border-radius: 999px;
    font-size: 13px;
    font-weight: 600;
  }
  .profile-pill img {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    object-fit: cover;
  }
  .badge {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.08);
    letter-spacing: 0.05em;
  }
  .badge.premium {
    background: #1db954;
    color: #000;
  }
  .ghost-btn {
    padding: 7px 16px;
    background: rgba(255, 255, 255, 0.05);
    border: none;
    border-radius: 999px;
    color: rgba(255, 255, 255, 0.85);
    font-family: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: background 200ms ease;
  }
  .ghost-btn:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .login-card {
    margin: 0 auto;
    max-width: 540px;
    padding: 56px 32px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
  .login-art {
    margin-bottom: 8px;
  }
  .login-card h2 {
    margin: 0;
    font-size: 28px;
    font-weight: 800;
    letter-spacing: -0.01em;
  }
  .login-body {
    margin: 0;
    color: rgba(255, 255, 255, 0.6);
    font-size: 14px;
    line-height: 1.6;
    max-width: 44ch;
  }
  .warn {
    color: #ffa726;
    font-size: 13px;
    margin: 0;
  }
  .error {
    color: #e22134;
    font-size: 13px;
    margin: 0;
  }
  .cta-spotify {
    margin-top: 8px;
    padding: 14px 40px;
    background: #1db954;
    color: #000;
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 15px;
    font-weight: 700;
    cursor: pointer;
    transition: transform 200ms ease, background 200ms ease;
  }
  .cta-spotify:hover:not(:disabled) {
    background: #1ed760;
    transform: scale(1.04);
  }
  .cta-spotify:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .waiting {
    display: flex;
    align-items: center;
    gap: 12px;
    color: rgba(255, 255, 255, 0.7);
    font-size: 14px;
  }
  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid rgba(255, 255, 255, 0.2);
    border-top-color: #1db954;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .block {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .block-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .block-head h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 800;
    letter-spacing: -0.01em;
  }
  .scroll-arrows {
    display: flex;
    gap: 8px;
  }
  .arrow {
    width: 32px;
    height: 32px;
    background: rgba(255, 255, 255, 0.05);
    border: 0;
    border-radius: 50%;
    color: white;
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: background 200ms ease;
  }
  .arrow:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .album-pills-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 8px;
  }
  .track-pill {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px;
    background: rgba(255, 255, 255, 0.04);
    border: 0;
    border-radius: 8px;
    color: inherit;
    cursor: pointer;
    text-align: left;
    transition: background 200ms ease;
  }
  .track-pill:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .track-pill-cover {
    width: 56px;
    height: 56px;
    border-radius: 4px;
    overflow: hidden;
    background: rgba(40, 40, 40, 0.8);
    flex-shrink: 0;
  }
  .track-pill-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .track-pill-fallback {
    width: 100%;
    height: 100%;
  }
  .track-pill-info {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .track-pill-title {
    font-size: 14px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .track-pill-sub {
    font-size: 12px;
    color: rgba(255, 255, 255, 0.55);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .h-scroll {
    display: flex;
    gap: 24px;
    overflow-x: auto;
    overflow-y: visible;
    padding: 8px 4px 24px;
    scroll-behavior: smooth;
    scrollbar-width: none;
  }
  .h-scroll::-webkit-scrollbar { display: none; }

  .album-card {
    flex: 0 0 176px;
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
    margin-bottom: 16px;
    transition: all 300ms ease;
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

  .artist-card {
    flex: 0 0 144px;
    cursor: pointer;
    background: transparent;
    border: 0;
    color: inherit;
    padding: 0;
    text-align: center;
  }
  .artist-circle {
    aspect-ratio: 1 / 1;
    border-radius: 50%;
    background: rgba(40, 40, 40, 0.8);
    overflow: hidden;
    margin: 0 auto 16px;
    transition: transform 700ms ease;
    position: relative;
  }
  .artist-card:hover .artist-circle {
    transform: scale(1.04);
  }
  .artist-circle img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .artist-fallback {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    color: white;
    font-size: 48px;
    font-weight: 800;
  }
  .artist-name {
    margin: 0;
    color: white;
    font-weight: 700;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 4px;
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

  .status-banner {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-radius: 8px;
    font-size: 12px;
    font-weight: 600;
    align-self: flex-start;
  }
  .status-banner.sdk {
    background: rgba(29, 185, 84, 0.15);
    color: #1db954;
  }
  .status-banner.warn {
    background: rgba(255, 167, 38, 0.15);
    color: #ffa726;
  }
  .status-banner.info {
    background: rgba(100, 181, 246, 0.15);
    color: #64b5f6;
  }
  .track-title-row {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
  }
  .local-badge {
    font-size: 10px;
    color: #1db954;
    line-height: 1;
    flex-shrink: 0;
  }

  .muted {
    color: rgba(255, 255, 255, 0.5);
    font-size: 13px;
  }

  @media (prefers-reduced-motion: reduce) {
    .cta-spotify, .arrow, .album-card-cover img, .artist-circle, .spinner {
      transition: none;
      animation: none;
    }
    .cta-spotify:hover, .album-card:hover .album-card-cover img,
    .artist-card:hover .artist-circle {
      transform: none;
    }
  }
</style>
