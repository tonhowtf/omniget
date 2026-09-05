<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import AlbumHero from "$lib/study-music-components/AlbumHero.svelte";
  import TrackRow from "$lib/study-music-components/TrackRow.svelte";
  import type { MusicTrack } from "$lib/study-music/player-store.svelte";

  type Album = {
    name: string;
    artist: string | null;
    track_count: number;
    total_duration_ms: number;
    cover_path: string | null;
    year: number | null;
  };

  const albumName = $derived($page.url.searchParams.get("name") ?? "");
  const artistName = $derived($page.url.searchParams.get("artist") ?? "");

  let album = $state<Album | null>(null);
  let tracks = $state<MusicTrack[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function load() {
    if (!albumName) {
      error = "Álbum não especificado.";
      loading = false;
      return;
    }
    loading = true;
    error = null;
    try {
      const args: { name: string; artist?: string } = { name: albumName };
      if (artistName) args.artist = artistName;
      const res = await pluginInvoke<{ album: Album; tracks: MusicTrack[] }>(
        "study",
        "study:music:albums:get",
        args,
      );
      album = res.album;
      tracks = res.tracks ?? [];
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  $effect(() => {
    void albumName;
    void artistName;
    void load();
  });
</script>

<section class="album-page">
  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if error || !album}
    <p class="error">{error ?? "Álbum não encontrado."}</p>
  {:else}
    <AlbumHero {album} {tracks} />
    {#if tracks.length === 0}
      <p class="muted">{$t("study.music.empty_album")}</p>
    {:else}
      <ul class="track-list">
        {#each tracks as track, i (track.id)}
          <TrackRow {track} queue={tracks} index={i} showNumber showCover />
        {/each}
      </ul>
    {/if}
  {/if}
</section>

<style>
  .album-page { display: flex; flex-direction: column; gap: 12px; }
  .track-list { list-style: none; margin: 8px 0 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
  .muted { color: var(--tertiary); font-size: 13px; }
  .error { color: var(--error); font-size: 13px; }
</style>
