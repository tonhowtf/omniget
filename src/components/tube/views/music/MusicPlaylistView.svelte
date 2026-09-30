<script lang="ts">
  // Music playlist page (also "Liked music" = LM): header like the album's,
  // then the songs with cover, artists and album columns.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { mplayer, playPlaylist, playSongs, toggle } from "$lib/tube/music.svelte";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import type { PlaylistItem, PlaylistPage } from "$lib/tube/types";
  import SongRow from "../../music/SongRow.svelte";
  import SquareCard from "../../music/SquareCard.svelte";
  import CollectionHeader from "./CollectionHeader.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";
  import { downloadSongs, itemKey, nearEnd, saveToLibrary, shuffled, songFromItem, songsOf } from "./shared.svelte";

  let { route }: { route: Extract<Route, { name: "m_playlist" }> } = $props();

  let pl = $state<PlaylistPage | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let more = $state(false);
  let saved = $state(false);
  let menu = $state<{ x: number; y: number } | null>(null);
  let seq = 0;

  let liked = $derived(route.id === "LM" || route.id === "VLLM");
  let ctx = $derived(`playlist:${route.id}`);
  let isCtx = $derived(mplayer.context === ctx);
  let songs = $derived(pl ? songsOf(pl.items) : []);
  let others = $derived(pl ? pl.items.filter((i) => !songFromItem(i)) : []);
  let asItem = $derived<PlaylistItem | null>(
    pl ? { kind: "playlist", id: pl.id, title: pl.title, thumbs: pl.thumbs, channel: pl.owner ?? null, count_text: pl.count_text ?? null, is_mix: pl.id.startsWith("RD"), first_video_id: songs[0]?.id ?? null, is_music: true } : null,
  );

  async function load() {
    const my = ++seq;
    loading = true;
    error = null;
    try {
      const res = await api.musicPlaylist(route.id);
      if (my === seq) pl = res;
    } catch (e) {
      if (my === seq) error = String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function loadMore() {
    if (!pl?.continuation || more) return;
    more = true;
    const token = pl.continuation;
    try {
      const res = await api.continuation("music_playlist", token);
      if (pl && pl.continuation === token) {
        for (const s of res.shelves) pl.items.push(...s.items);
        pl.continuation = res.continuation ?? null;
      }
    } catch {
      if (pl) pl.continuation = null;
    } finally {
      more = false;
    }
  }

  $effect(() => {
    void route.id;
    void load();
  });

  // Local list first: it plays instantly and matches what is on screen.
  function playFrom(i: number) {
    if (!songs.length) return;
    void playSongs(songs, i, ctx);
  }

  function play() {
    if (isCtx) toggle();
    else if (songs.length) playFrom(0);
    else void playPlaylist(route.id);
  }

  function shuffle() {
    if (songs.length) {
      void playSongs(shuffled(songs), 0, ctx);
      mplayer.shuffle = true;
    } else void playPlaylist(route.id, true);
  }

  async function save() {
    if (await saveToLibrary(route.id, !saved)) saved = !saved;
  }

  function openMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: r.left, y: r.bottom + 6 };
  }
</script>

<MusicPage readiness={loading ? "loading" : error ? "error" : "ready"} label={pl?.title ?? ($t("tube.music.kind_playlist") as string)}>
  {#if loading}
    <MusicSkeleton variant="collection" />
  {:else if error || !pl}
    <MusicState icon={liked ? "liked" : "library"} error title={$t("tube.music.error_title") as string} text={error ?? ""} onretry={load} />
  {:else}
    <CollectionHeader
      thumbs={liked ? [] : pl.thumbs}
      title={liked ? ($t("tube.music.liked_music") as string) : pl.title}
      special={liked ? "liked" : null}
      stats={[pl.views_text, pl.count_text, pl.duration_text].filter(Boolean).join(" • ")}
      description={pl.description}
      playing={isCtx && mplayer.playing}
      saved={pl.editable || liked ? null : saved}
      onplay={play}
      onshuffle={shuffle}
      onsave={save}
      ondownload={songs.length ? () => void downloadSongs(songs) : undefined}
      onmore={liked ? undefined : openMenu}
    >
      {#snippet meta()}
        <span>{liked ? $t("tube.music.auto_playlist") : $t("tube.music.kind_playlist")}</span>
        {#if pl!.owner}
          <span class="pl-dot">•</span>
          {#if pl!.owner.id}<button onclick={() => go({ name: "m_artist", id: pl!.owner!.id })}>{pl!.owner.name}</button>{:else}<span>{pl!.owner.name}</span>{/if}
        {/if}
        {#if pl!.updated_text}<span class="pl-dot">•</span><span>{pl!.updated_text}</span>{/if}
      {/snippet}
    </CollectionHeader>

    {#if !pl.items.length}
      <MusicState icon={liked ? "liked" : "queue"} title={liked ? ($t("tube.music.liked_empty") as string) : ($t("tube.music.playlist_empty") as string)} />
    {:else}
      <div class="pl-list" role="table" aria-label={$t("tube.music.tracks") as string}>
        {#each songs as s, i (s.id + i)}
          <SongRow song={s} onplay={() => playFrom(i)} />
        {/each}
      </div>
      {#if others.length}
        <div class="pl-grid" role="list">
          {#each others as it, i (itemKey(it, i))}
            <div role="listitem"><SquareCard item={it} fluid /></div>
          {/each}
        </div>
      {/if}
      {#if pl.continuation}
        <div use:nearEnd={loadMore}><MusicSkeleton variant="list" /></div>
      {/if}
    {/if}
  {/if}
</MusicPage>

{#if menu && asItem}
  <ItemMenu item={asItem} x={menu.x} y={menu.y} onclose={() => (menu = null)} />
{/if}

<style>
  .pl-dot { margin: 0 6px; color: var(--ytm-text-2, var(--yt-text-2)); }
  .pl-list { display: flex; flex-direction: column; }
  .pl-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 32px 16px; margin-top: 32px; }
</style>
