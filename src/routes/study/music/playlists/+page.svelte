<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { playlistsStore, type Playlist } from "$lib/study-music/playlists-store.svelte";
  import { spotifyStore } from "$lib/study-music/spotify-store.svelte";
  import { soundcloudStore } from "$lib/study-music/soundcloud-store.svelte";
  import CoverImage from "$lib/study-music-components/CoverImage.svelte";

  let creatingNew = $state(false);
  let newName = $state("");
  let newRef = $state<HTMLInputElement | null>(null);
  let busy = $state(false);

  onMount(async () => {
    void playlistsStore.load();
    if (
      spotifyStore.status.logged_in &&
      spotifyStore.playlists.length === 0
    ) {
      try {
        await spotifyStore.loadAll();
      } catch {
        /* ignore */
      }
    }
    if (
      soundcloudStore.isLoggedIn &&
      soundcloudStore.playlists.length === 0
    ) {
      try {
        await soundcloudStore.loadAll();
      } catch {
        /* ignore */
      }
    }
  });

  const ownedSpotify = $derived(
    spotifyStore.playlists.filter((p) => spotifyStore.isPlaylistOwned(p)),
  );
  const followedSpotify = $derived(
    spotifyStore.playlists.filter((p) => !spotifyStore.isPlaylistOwned(p)),
  );

  function startNew() {
    creatingNew = true;
    newName = "";
    queueMicrotask(() => newRef?.focus());
  }

  async function confirmCreate() {
    const trimmed = newName.trim();
    if (!trimmed || busy) return;
    busy = true;
    try {
      const id = await playlistsStore.create(trimmed);
      creatingNew = false;
      newName = "";
      if (id) goto(`/study/music/playlists/${id}`);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    } finally {
      busy = false;
    }
  }

  function cancelNew() {
    creatingNew = false;
    newName = "";
  }
</script>

<section class="playlists-page">
  <header class="head">
    <h1>{$t("study.music.playlists_title")}</h1>
    <button type="button" class="cta" onclick={startNew} disabled={creatingNew}>
      + {$t("study.music.new_playlist")}
    </button>
  </header>

  {#if creatingNew}
    <div class="new-row">
      <input
        bind:this={newRef}
        bind:value={newName}
        type="text"
        placeholder={$t("study.music.playlist_name_placeholder")}
        onkeydown={(e) => {
          if (e.key === "Enter") confirmCreate();
          else if (e.key === "Escape") cancelNew();
        }}
      />
      <button type="button" class="confirm" onclick={confirmCreate} disabled={!newName.trim() || busy}>
        {$t("study.common.create")}
      </button>
      <button type="button" class="ghost" onclick={cancelNew}>
        {$t("study.common.cancel")}
      </button>
    </div>
  {/if}

  {#if playlistsStore.loading && playlistsStore.list.length === 0}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if playlistsStore.list.length === 0}
    <div class="empty">
      <p>{$t("study.music.playlists_empty_title")}</p>
      <p class="sub">{$t("study.music.playlists_empty_body")}</p>
      <button type="button" class="cta" onclick={startNew}>
        {$t("study.music.new_playlist")}
      </button>
    </div>
  {:else}
    <div class="playlist-grid">
      {#each playlistsStore.list as p (p.id)}
        <a class="playlist-card" href={`/study/music/playlists/${p.id}`}>
          <div class="cover-wrap">
            <CoverImage
              src={p.resolved_cover}
              alt={p.name}
              fallbackSeed={p.name}
              rounded="lg"
            />
          </div>
          <span class="title">{p.name}</span>
          <span class="sub">{p.track_count} faixa(s)</span>
        </a>
      {/each}
    </div>
  {/if}

  {#if ownedSpotify.length > 0}
    <section class="spotify-section">
      <header class="section-head">
        <h2>Suas playlists Spotify</h2>
        <span class="badge-spotify">
          <svg viewBox="0 0 168 168" width="14" height="14" aria-hidden="true">
            <circle cx="84" cy="84" r="84" fill="#1db954" />
            <path fill="#000" d="M119.6 110.6c-1.5 2.5-4.7 3.3-7.2 1.8-19.7-12-44.5-14.7-73.7-8-2.8.6-5.6-1.1-6.3-3.9-.6-2.8 1.1-5.6 3.9-6.3 31.9-7.3 59.4-4.2 81.5 9.2 2.5 1.5 3.3 4.7 1.8 7.2zm9.5-21.2c-1.9 3.1-5.9 4.1-9 2.2-22.6-13.9-57-17.9-83.8-9.8-3.5 1.1-7.1-.9-8.2-4.3-1.1-3.5.9-7.1 4.3-8.2 30.6-9.3 68.5-4.8 94.5 11.1 3.1 1.9 4.1 5.9 2.2 9zm.8-22c-27-16-71.6-17.5-97.4-9.7-4.1 1.2-8.4-1.1-9.6-5.2-1.2-4.1 1.1-8.4 5.2-9.6 29.6-9 78.7-7.2 109.8 11.3 3.7 2.2 4.9 7 2.7 10.7-2.2 3.7-7 4.9-10.7 2.5z"/>
          </svg>
          Suas
        </span>
      </header>
      <div class="playlist-grid">
        {#each ownedSpotify as p (p.id)}
          <a class="playlist-card" href={`/study/music/spotify/playlist/${p.id}`}>
            <div class="cover-wrap">
              <CoverImage
                src={spotifyStore.pickImage(p.images, 300)}
                alt={p.name}
                fallbackSeed={p.name}
                rounded="lg"
              />
            </div>
            <span class="title">{p.name}</span>
            <span class="sub">{p.tracks_total} faixa(s){p.owner_name ? ` · ${p.owner_name}` : ""}</span>
          </a>
        {/each}
      </div>
    </section>
  {/if}

  {#if followedSpotify.length > 0}
    <section class="spotify-section">
      <header class="section-head">
        <h2>Playlists seguidas</h2>
        <span class="badge-warn" title={$t("study.music.spotify.dev_mode_warn")}>
          algumas podem dar erro
        </span>
      </header>
      <div class="playlist-grid">
        {#each followedSpotify as p (p.id)}
          <a class="playlist-card" href={`/study/music/spotify/playlist/${p.id}`}>
            <div class="cover-wrap">
              <CoverImage
                src={spotifyStore.pickImage(p.images, 300)}
                alt={p.name}
                fallbackSeed={p.name}
                rounded="lg"
              />
            </div>
            <span class="title">{p.name}</span>
            <span class="sub">{p.tracks_total} faixa(s){p.owner_name ? ` · ${p.owner_name}` : ""}</span>
          </a>
        {/each}
      </div>
    </section>
  {/if}

  {#if soundcloudStore.playlists.length > 0}
    <section class="spotify-section">
      <header class="section-head">
        <h2>Suas playlists SoundCloud</h2>
        <span class="badge-soundcloud">SoundCloud</span>
      </header>
      <div class="playlist-grid">
        {#each soundcloudStore.playlists as p (p.id)}
          <a class="playlist-card" href={`/study/music/soundcloud/playlist/${p.id}`}>
            <div class="cover-wrap">
              <CoverImage
                src={soundcloudStore.pickArtwork(p.artwork_url)}
                alt={p.title}
                fallbackSeed={p.title}
                rounded="lg"
              />
            </div>
            <span class="title">{p.title}</span>
            <span class="sub">{p.track_count} faixa(s){p.user.username ? ` · ${p.user.username}` : ""}</span>
          </a>
        {/each}
      </div>
    </section>
  {/if}
</section>

<style>
  .playlists-page { display: flex; flex-direction: column; gap: 20px; }
  .spotify-section { display: flex; flex-direction: column; gap: 16px; margin-top: 16px; padding-top: 24px; border-top: none; }
  .section-head { display: flex; align-items: center; gap: 12px; }
  .section-head h2 { margin: 0; font-size: 18px; font-weight: 800; color: var(--secondary); }
  .badge-spotify { display: inline-flex; align-items: center; gap: 6px; font-size: 11px; font-weight: 700; color: #1db954; }
  .badge-warn { font-size: 10px; font-weight: 700; color: #ffa726; padding: 2px 8px; background: rgba(255, 167, 38, 0.12); border-radius: 999px; cursor: help; }
  .badge-soundcloud { font-size: 10px; font-weight: 700; color: #ff5500; padding: 2px 8px; background: rgba(255, 85, 0, 0.12); border-radius: 999px; }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .head h1 {
    margin: 0;
    font-size: 28px;
    font-weight: 800;
    letter-spacing: -0.01em;
    color: var(--secondary);
  }
  .cta {
    padding: 8px 18px;
    background: var(--accent);
    color: var(--on-accent, white);
    border: 0;
    border-radius: 999px;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: transform 120ms ease;
  }
  .cta:hover { transform: scale(1.03); }
  .cta:disabled { opacity: 0.5; cursor: default; transform: none; }
  .new-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .new-row input {
    flex: 1;
    max-width: 340px;
    padding: 8px 14px;
    border: 1px solid var(--accent);
    border-radius: 8px;
    background: var(--input-bg);
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    outline: none;
  }
  .confirm {
    padding: 8px 14px;
    background: var(--accent);
    color: var(--on-accent, white);
    border: 0;
    border-radius: 8px;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .confirm:disabled { opacity: 0.5; cursor: default; }
  .ghost {
    padding: 8px 12px;
    background: transparent;
    border: none;
    border-radius: 8px;
    color: var(--secondary);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .ghost:hover { color: var(--accent); border-color: var(--accent); }
  .empty {
    padding: 48px 24px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .empty p { margin: 0; color: var(--secondary); font-weight: 600; }
  .empty .sub { color: var(--tertiary); font-weight: 400; max-width: 50ch; line-height: 1.5; }
  .playlist-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 16px;
  }
  .playlist-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    background: color-mix(in oklab, var(--button) 50%, transparent);
    border-radius: 10px;
    color: inherit;
    text-decoration: none;
    transition: background 120ms ease;
  }
  .playlist-card:hover {
    background: color-mix(in oklab, var(--accent) 8%, var(--button));
  }
  .cover-wrap {
    aspect-ratio: 1 / 1;
    width: 100%;
    border-radius: 10px;
    overflow: hidden;
    box-shadow: 0 4px 14px color-mix(in oklab, black 28%, transparent);
  }
  .title {
    font-size: 14px;
    font-weight: 600;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 12px;
    color: var(--tertiary);
  }
  .muted { color: var(--tertiary); font-size: 13px; }
</style>
