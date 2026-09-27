<script lang="ts">
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { albumKey, fmtLong, groupAlbums, qualityLabel, shuffled } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { enqueue, playTracks } from "$lib/music/player-store.svelte";
  import { analyze, go, showTrackMenu, ui } from "$lib/music/ui-store.svelte";
  import CollectionHeader from "../CollectionHeader.svelte";
  import MusicIcon from "../MusicIcon.svelte";
  import TrackList from "../TrackList.svelte";

  let { key }: { key: string } = $props();

  let lib = $derived(library());
  let album = $derived.by(() => {
    void lib.version;
    return groupAlbums(lib.tracks.filter((x) => albumKey(x) === key))[0] ?? null;
  });
  let ids = $derived(album?.tracks.map((x) => x.id) ?? []);
  let quality = $derived(album ? (album.codec ? qualityLabel(album.tracks[0]) : $t("music.mixed_formats")) : "");
  let selected = $state(new Set<number>());
  let moreByArtist = $derived.by(() => {
    if (!album) return [];
    return groupAlbums(lib.tracks.filter((x) => (x.album_artist || x.artist) === album!.artist && albumKey(x) !== key)).slice(0, 6);
  });
</script>

{#if !album}
  <p class="missing">{$t("music.not_found")}</p>
{:else}
  <div class="alv">
    <CollectionHeader
      kicker={$t("music.album") as string}
      title={album.title}
      subtitle={[album.artist, album.year, $t("music.count_songs", { count: album.tracks.length }), fmtLong(album.duration_ms), quality].filter(Boolean).join(" · ")}
      cover={album.cover}
      icon="disc"
      onplay={() => void playTracks(ids, undefined, album!.title)}
      onshuffle={() => void playTracks(shuffled(ids), undefined, album!.title)}
    >
      {#snippet actions()}
        <button class="btn btn-lg" onclick={() => enqueue(ids)} title={$t("music.menu.add_queue") as string} aria-label={$t("music.menu.add_queue") as string}><MusicIcon name="queue" size={15} /></button>
        <button class="btn btn-lg" onclick={() => (ui.tagEditor = ids)} title={$t("music.menu.edit_tags") as string} aria-label={$t("music.menu.edit_tags") as string}><MusicIcon name="edit" size={15} /></button>
        <button class="btn btn-lg" onclick={() => analyze(ids)} title={$t("music.menu.analyze") as string} aria-label={$t("music.menu.analyze") as string}><MusicIcon name="activity" size={15} /></button>
        <button class="btn btn-lg" onclick={(e) => showTrackMenu(e, ids)} aria-label={$t("music.more") as string}><MusicIcon name="more" size={15} /></button>
        {#if album && album.artist}
          <button class="btn btn-lg btn-ghost" onclick={() => album && go({ kind: "artist", name: album.artist })}><MusicIcon name="artist" size={15} /> {album.artist}</button>
        {/if}
      {/snippet}
    </CollectionHeader>

    <div class="list" style:height="{Math.min(album.tracks.length, 30) * 46 + 34}px">
      <TrackList
        tracks={album.tracks}
        bind:selected
        numbered="track"
        showCover={false}
        columns={["plays", "bpm", "key"]}
        onplay={(tr) => void playTracks(ids, tr.id, album!.title)}
        onmenu={(e, sel) => showTrackMenu(e, sel)}
      />
    </div>

    {#if moreByArtist.length}
      <section>
        <h2>{$t("music.more_by", { name: album.artist })}</h2>
        <div class="more">
          {#each moreByArtist as a (a.key)}
            <button class="mb" onclick={() => go({ kind: "album", key: a.key })}>
              <span class="t1">{a.title}</span><span class="t2">{a.year ?? ""}</span>
            </button>
          {/each}
        </div>
      </section>
    {/if}
    {#if album.tracks[0]?.source_url}
      <p class="src"><MusicIcon name="link" size={12} /> <button class="linkbtn" onclick={() => album && void openExternal(album.tracks[0].source_url).catch(() => {})}>{album.tracks[0].source_url}</button></p>
    {/if}
  </div>
{/if}

<style>
  .alv {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  .list {
    min-height: 120px;
  }
  .missing {
    color: var(--text-dim);
    text-align: center;
    margin-top: var(--space-7);
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-lg);
    font-weight: 700;
  }
  .more {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .mb {
    all: unset;
    display: flex;
    flex-direction: column;
    padding: 8px 12px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    cursor: pointer;
  }
  .mb:hover {
    background: var(--fill-2);
  }
  .mb:focus-visible {
    outline: var(--focus-ring);
  }
  .t1 {
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .t2 {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .src {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .linkbtn {
    all: unset;
    cursor: pointer;
    text-decoration: underline;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
