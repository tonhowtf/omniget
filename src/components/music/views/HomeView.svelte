<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { fmtLong, groupAlbums, groupArtists, shuffled } from "$lib/music/library";
  import { addLibraryFolder, library, trackById } from "$lib/music/library-store.svelte";
  import { playTracks } from "$lib/music/player-store.svelte";
  import { go, showTrackMenu } from "$lib/music/ui-store.svelte";
  import type { HistoryEntry, Track } from "$lib/music/types";
  import AlbumGrid from "../AlbumGrid.svelte";
  import Cover from "../Cover.svelte";
  import EmptyState from "../EmptyState.svelte";
  import MusicIcon from "../MusicIcon.svelte";

  let history = $state<HistoryEntry[]>([]);

  onMount(async () => {
    history = await invoke<HistoryEntry[]>("music_history", { limit: 200 }).catch(() => []);
  });

  let lib = $derived(library());
  let tracks = $derived(lib.tracks);
  let albums = $derived.by(() => {
    void lib.version;
    return groupAlbums(tracks);
  });
  let totalMs = $derived(tracks.reduce((a, x) => a + x.duration_ms, 0));

  let recentAlbums = $derived([...albums].sort((a, b) => b.added_at - a.added_at).slice(0, 12));
  let jumpBack = $derived.by(() => {
    const seen = new Set<number>();
    const out: Track[] = [];
    for (const h of history) {
      if (seen.has(h.track_id)) continue;
      seen.add(h.track_id);
      const tr = trackById(h.track_id);
      if (tr) out.push(tr);
      if (out.length >= 8) break;
    }
    return out;
  });
  let topArtists = $derived(
    groupArtists(tracks, albums)
      .filter((a) => a.plays > 0)
      .sort((a, b) => b.plays - a.plays)
      .slice(0, 8),
  );
  let rediscover = $derived.by(() => {
    const cutoff = Date.now() - 60 * 86_400_000;
    const stale = albums.filter((a) => a.tracks.every((x) => !x.last_played || x.last_played < cutoff));
    return shuffled(stale).slice(0, 6);
  });
  let liked = $derived(tracks.filter((x) => x.liked));
  let mostPlayed = $derived([...tracks].filter((x) => x.play_count > 0).sort((a, b) => b.play_count - a.play_count).slice(0, 50));

  let greeting = $derived.by(() => {
    const h = new Date().getHours();
    return h < 5 ? "night" : h < 12 ? "morning" : h < 18 ? "afternoon" : "evening";
  });

  function shuffleAll() {
    void playTracks(shuffled(tracks.map((x) => x.id)), undefined, $t("music.home.shuffle_all") as string);
  }

  /** A mix from the most played genre, harmonically sorted by key then tempo. */
  function dailyMix() {
    const genreCount = new Map<string, number>();
    for (const x of tracks) if (x.genre) genreCount.set(x.genre, (genreCount.get(x.genre) ?? 0) + x.play_count + 1);
    const top = [...genreCount.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
    const pool = top ? tracks.filter((x) => x.genre === top) : tracks;
    const picked = shuffled(pool).slice(0, 40);
    picked.sort((a, b) => (a.camelot || "~").localeCompare(b.camelot || "~", undefined, { numeric: true }) || (a.bpm ?? 0) - (b.bpm ?? 0));
    void playTracks(picked.map((x) => x.id), undefined, `${$t("music.home.daily_mix")}${top ? ` · ${top}` : ""}`);
  }
</script>

{#if lib.loaded && !tracks.length}
  <EmptyState art="empty-library" title={$t("music.empty.title") as string} body={$t("music.empty.body") as string}>
    <button class="btn btn-primary btn-lg" onclick={() => void addLibraryFolder()}><MusicIcon name="folder" size={15} /> {$t("music.empty.add_folder")}</button>
    <button class="btn btn-lg" onclick={() => go({ kind: "download" })}><MusicIcon name="download" size={15} /> {$t("music.empty.download")}</button>
    <button class="btn btn-lg" onclick={() => go({ kind: "radio" })}><MusicIcon name="radio" size={15} /> {$t("music.empty.radio")}</button>
  </EmptyState>
  {#if lib.scan}
    <p class="scanning">{$t("music.scan.progress", { done: lib.scan.done, total: lib.scan.total })}</p>
  {/if}
{:else}
  <div class="home">
    <header class="hero">
      <div>
        <h1>{$t(`music.home.greeting_${greeting}`)}</h1>
        <p>{$t("music.home.summary", { tracks: tracks.length, albums: albums.length, time: fmtLong(totalMs) })}</p>
      </div>
    </header>

    <div class="tiles">
      <button class="tile t-liked" onclick={() => go({ kind: "liked" })}>
        <span class="ti"><MusicIcon name="heart-fill" size={20} /></span>
        <span class="tt"><b>{$t("music.nav.liked")}</b><small>{liked.length}</small></span>
      </button>
      <button class="tile t-shuffle" onclick={shuffleAll} disabled={!tracks.length}>
        <span class="ti"><MusicIcon name="shuffle" size={20} /></span>
        <span class="tt"><b>{$t("music.home.shuffle_all")}</b><small>{tracks.length}</small></span>
      </button>
      <button class="tile t-mix" onclick={dailyMix} disabled={!tracks.length}>
        <span class="ti"><MusicIcon name="mix" size={20} /></span>
        <span class="tt"><b>{$t("music.home.daily_mix")}</b><small>{$t("music.home.daily_mix_hint")}</small></span>
      </button>
      <button class="tile t-top" onclick={() => mostPlayed.length && void playTracks(mostPlayed.map((x) => x.id), undefined, $t("music.home.most_played") as string)} disabled={!mostPlayed.length}>
        <span class="ti"><MusicIcon name="fire" size={20} /></span>
        <span class="tt"><b>{$t("music.home.most_played")}</b><small>{mostPlayed.length}</small></span>
      </button>
      <button class="tile t-radio" onclick={() => go({ kind: "radio" })}>
        <span class="ti"><MusicIcon name="radio" size={20} /></span>
        <span class="tt"><b>{$t("music.nav.radio")}</b><small>{$t("music.home.radio_hint")}</small></span>
      </button>
      <button class="tile t-dl" onclick={() => go({ kind: "download" })}>
        <span class="ti"><MusicIcon name="download" size={20} /></span>
        <span class="tt"><b>{$t("music.nav.download")}</b><small>{$t("music.home.download_hint")}</small></span>
      </button>
    </div>

    {#if jumpBack.length}
      <section>
        <h2>{$t("music.home.jump_back")}</h2>
        <div class="strip">
          {#each jumpBack as tr (tr.id)}
            <button class="mini" onclick={() => void playTracks(jumpBack.map((x) => x.id), tr.id)} oncontextmenu={(e) => showTrackMenu(e, [tr.id])}>
              <Cover hash={tr.cover} seed={tr.album || tr.title} size={48} radius={6} />
              <span class="mt"><b>{tr.title}</b><small>{tr.artist}</small></span>
              <span class="mp"><MusicIcon name="play" size={14} /></span>
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if recentAlbums.length}
      <section>
        <div class="sh">
          <h2>{$t("music.home.recently_added")}</h2>
          <button class="btn btn-sm btn-ghost" onclick={() => go({ kind: "albums" })}>{$t("music.see_all")}</button>
        </div>
        <AlbumGrid albums={recentAlbums} limit={12} min={150} />
      </section>
    {/if}

    {#if topArtists.length}
      <section>
        <h2>{$t("music.home.top_artists")}</h2>
        <div class="artists">
          {#each topArtists as a (a.name)}
            <button class="artist" onclick={() => go({ kind: "artist", name: a.name })}>
              <Cover hash={a.cover} seed={a.name} size={96} round icon="artist" />
              <span>{a.name}</span>
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if rediscover.length}
      <section>
        <h2>{$t("music.home.rediscover")}</h2>
        <AlbumGrid albums={rediscover} limit={6} min={150} />
      </section>
    {/if}
  </div>
{/if}

<style>
  .home {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    padding-bottom: var(--space-7);
  }
  .hero h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-3xl);
    font-weight: 750;
    letter-spacing: var(--track-tight);
  }
  .hero p {
    margin: var(--space-1) 0 0;
    color: var(--text-dim);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
    gap: var(--space-2);
  }
  .tile {
    all: unset;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 60px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
    transition:
      background var(--duration-fast),
      transform var(--duration-fast);
  }
  .tile:hover {
    background: var(--surface-hi);
  }
  .tile:active {
    transform: scale(0.99);
  }
  .tile:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .tile:focus-visible {
    outline: var(--focus-ring);
  }
  .ti {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 10px;
    color: white;
    background: linear-gradient(135deg, var(--c1), var(--c2));
  }
  .t-liked {
    --c1: #ff6b8b;
    --c2: #d81b60;
  }
  .t-shuffle {
    --c1: #5aa9ff;
    --c2: #1e6fe8;
  }
  .t-mix {
    --c1: #c77dff;
    --c2: #7b3fe4;
  }
  .t-top {
    --c1: #ffb340;
    --c2: #f25c05;
  }
  .t-radio {
    --c1: #67d27e;
    --c2: #2f9e52;
  }
  .t-dl {
    --c1: #ff9f0a;
    --c2: #d97706;
  }
  .tt {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .tt b {
    font-weight: 650;
    font-size: var(--text-base);
  }
  .tt small {
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .sh {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }
  .strip {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: var(--space-2);
  }
  .mini {
    all: unset;
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 6px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    cursor: pointer;
  }
  .mini:hover {
    background: var(--fill-2);
  }
  .mini:focus-visible {
    outline: var(--focus-ring);
  }
  .mt {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .mt b,
  .mt small {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mt b {
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .mt small {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .mp {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  .mini:hover .mp {
    opacity: 1;
  }
  .artists {
    display: flex;
    gap: var(--space-4);
    overflow-x: auto;
    padding-bottom: var(--space-2);
  }
  .artist {
    all: unset;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    width: 104px;
    cursor: pointer;
    text-align: center;
  }
  .artist span {
    font-size: var(--text-sm);
    font-weight: 600;
    max-width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .artist:focus-visible {
    outline: var(--focus-ring);
    border-radius: 8px;
  }
  .scanning {
    text-align: center;
    color: var(--text-dim);
  }
</style>
