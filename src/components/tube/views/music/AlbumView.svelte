<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { mplayer, toggle } from "$lib/tube/music.svelte";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import type { AlbumItem, AlbumPage } from "$lib/tube/types";
  import MusicShelf from "../../music/MusicShelf.svelte";
  import SongRow from "../../music/SongRow.svelte";
  import CollectionHeader from "./CollectionHeader.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";
  import { downloadSongs, playAlbumPage, saveToLibrary } from "./shared.svelte";

  let { route }: { route: Extract<Route, { name: "m_album" }> } = $props();

  let album = $state<AlbumPage | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let saved = $state(false);
  let menu = $state<{ x: number; y: number } | null>(null);
  let seq = 0;

  let ctx = $derived(album ? `album:${album.id}` : "");
  let isCtx = $derived(!!album && mplayer.context === ctx);
  let names = $derived(new Set((album?.artists ?? []).map((a) => a.name)));
  let showArtists = $derived(!!album && album.tracks.some((s) => s.artists.length && !s.artists.every((a) => names.has(a.name))));
  let asItem = $derived<AlbumItem | null>(
    album
      ? { kind: "album", id: album.id, playlist_id: album.playlist_id ?? null, title: album.title, artists: album.artists, year: album.year ?? null, thumbs: album.thumbs, album_type: album.album_type ?? null, explicit: album.explicit }
      : null,
  );

  async function load() {
    const my = ++seq;
    loading = true;
    error = null;
    try {
      const res = await api.musicAlbum(route.id);
      if (my === seq) album = res;
    } catch (e) {
      if (my === seq) error = String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  $effect(() => {
    void route.id;
    void load();
  });

  function play() {
    if (!album) return;
    if (isCtx) toggle();
    else playAlbumPage(album);
  }

  async function save() {
    if (!album?.playlist_id) return;
    if (await saveToLibrary(album.playlist_id, !saved)) saved = !saved;
  }

  function openMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: r.left, y: r.bottom + 6 };
  }
</script>

<MusicPage readiness={loading ? "loading" : error ? "error" : "ready"} label={album?.title ?? ($t("tube.music.kind_album") as string)}>
  {#if loading}
    <MusicSkeleton variant="collection" />
  {:else if error || !album}
    <MusicState icon="library" error title={$t("tube.music.error_title") as string} text={error ?? ""} onretry={load} />
  {:else}
    <CollectionHeader
      thumbs={album.thumbs}
      title={album.title}
      stats={[album.track_count_text, album.duration_text].filter(Boolean).join(" • ")}
      description={album.description}
      playing={isCtx && mplayer.playing}
      saved={album.playlist_id ? saved : null}
      onplay={play}
      onshuffle={() => album && playAlbumPage(album, 0, true)}
      onsave={save}
      ondownload={() => album && void downloadSongs(album.tracks)}
      onmore={openMenu}
    >
      {#snippet meta()}
        <span>{album!.album_type || $t("tube.music.kind_album")}</span>
        {#each album!.artists as a (a.id || a.name)}
          <span class="al-dot">•</span>
          {#if a.id}<button onclick={() => go({ name: "m_artist", id: a.id })}>{a.name}</button>{:else}<span>{a.name}</span>{/if}
        {/each}
        {#if album!.year}<span class="al-dot">•</span><span>{album!.year}</span>{/if}
        {#if album!.explicit}<span class="al-e" title={$t("tube.music.explicit") as string}>E</span>{/if}
      {/snippet}
    </CollectionHeader>

    {#if album.tracks.length}
      <div class="al-tracks" role="table" aria-label={$t("tube.music.tracks") as string}>
        {#each album.tracks as s, i (s.id + i)}
          <SongRow song={s} variant="album" index={i + 1} {showArtists} onplay={() => album && playAlbumPage(album, i)} />
        {/each}
      </div>
    {:else}
      <MusicState icon="library" title={$t("tube.music.album_empty") as string} />
    {/if}

    {#if album.other_versions.length}
      <div class="al-more">
        <MusicShelf shelf={{ id: "other_versions", title: $t("tube.music.other_versions") as string, kind: "carousel", items: album.other_versions }} />
      </div>
    {/if}
  {/if}
</MusicPage>

{#if menu && asItem}
  <ItemMenu item={asItem} x={menu.x} y={menu.y} onclose={() => (menu = null)} />
{/if}

<style>
  .al-dot { margin: 0 6px; color: var(--ytm-text-2, var(--yt-text-2)); }
  .al-e {
    display: inline-grid; place-items: center; width: 16px; height: 16px; margin-left: 8px; border-radius: 2px;
    font: 600 10px/1 var(--yt-font); vertical-align: 1px;
    color: var(--ytm-bg, var(--yt-bg)); background: var(--ytm-text-2, var(--yt-text-2));
  }
  .al-tracks { display: flex; flex-direction: column; margin-bottom: 48px; }
  .al-more { margin-top: 16px; }
</style>
