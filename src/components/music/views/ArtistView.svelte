<script lang="ts">
  import { t } from "$lib/i18n";
  import { fmtLong, groupAlbums, shuffled, splitArtists } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { playTracks } from "$lib/music/player-store.svelte";
  import { showTrackMenu } from "$lib/music/ui-store.svelte";
  import AlbumGrid from "../AlbumGrid.svelte";
  import CollectionHeader from "../CollectionHeader.svelte";
  import TrackList from "../TrackList.svelte";

  let { name }: { name: string } = $props();

  let lib = $derived(library());
  let tracks = $derived.by(() => {
    void lib.version;
    const n = name.toLowerCase();
    return lib.tracks.filter(
      (x) => splitArtists(x.artist || x.album_artist).some((a) => a.toLowerCase() === n) || x.album_artist.toLowerCase() === n,
    );
  });
  let albums = $derived(groupAlbums(tracks).sort((a, b) => (b.year ?? 0) - (a.year ?? 0)));
  let top = $derived([...tracks].sort((a, b) => b.play_count - a.play_count || b.rating - a.rating).slice(0, 10));
  let ids = $derived(tracks.map((x) => x.id));
  let cover = $derived(albums.find((a) => a.cover)?.cover ?? null);
  let genres = $derived([...new Set(tracks.map((x) => x.genre).filter(Boolean))].slice(0, 3).join(", "));
  let selected = $state(new Set<number>());
</script>

<div class="atv">
  <CollectionHeader
    kicker={$t("music.artist") as string}
    title={name}
    subtitle={[$t("music.count_albums", { count: albums.length }), $t("music.count_songs", { count: tracks.length }), fmtLong(tracks.reduce((a, x) => a + x.duration_ms, 0)), genres].filter(Boolean).join(" · ")}
    {cover}
    round
    icon="artist"
    onplay={() => void playTracks(top.length ? [...top, ...tracks.filter((x) => !top.includes(x))].map((x) => x.id) : ids, undefined, name)}
    onshuffle={() => void playTracks(shuffled(ids), undefined, name)}
  />

  {#if top.some((x) => x.play_count > 0)}
    <section>
      <h2>{$t("music.popular")}</h2>
      <div class="list" style:height="{Math.min(top.length, 10) * 46 + 34}px">
        <TrackList tracks={top} bind:selected columns={["album", "plays"]} onplay={(tr) => void playTracks(top.map((x) => x.id), tr.id, name)} onmenu={(e, sel) => showTrackMenu(e, sel)} />
      </div>
    </section>
  {/if}

  {#if albums.length}
    <section>
      <h2>{$t("music.nav.albums")}</h2>
      <AlbumGrid {albums} min={160} />
    </section>
  {/if}

  <section>
    <h2>{$t("music.all_songs")}</h2>
    <div class="list" style:height="{Math.min(tracks.length, 14) * 46 + 34}px">
      <TrackList tracks={tracks} bind:selected columns={["album", "year", "plays"]} onplay={(tr, list) => void playTracks(list.map((x) => x.id), tr.id, name)} onmenu={(e, sel) => showTrackMenu(e, sel)} />
    </div>
  </section>
</div>

<style>
  .atv {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
  }
  h2 {
    margin: 0 0 var(--space-3);
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .list {
    min-height: 120px;
  }
</style>
