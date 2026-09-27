<script lang="ts">
  import { player, playTracks } from "$lib/music/player-store.svelte";
  import { go, showTrackMenu } from "$lib/music/ui-store.svelte";
  import type { Album } from "$lib/music/types";
  import AlbumCard from "./AlbumCard.svelte";

  let { albums, min = 168, limit = 0 }: { albums: Album[]; min?: number; limit?: number } = $props();

  // grids with thousands of albums render in pages as you scroll
  let shown = $state(120);
  let sentinel: HTMLDivElement | undefined = $state();
  let list = $derived(limit ? albums.slice(0, limit) : albums.slice(0, shown));

  $effect(() => {
    if (!sentinel || limit) return;
    const io = new IntersectionObserver((e) => {
      if (e[0].isIntersecting) shown += 120;
    });
    io.observe(sentinel);
    return () => io.disconnect();
  });

  let playingKey = $derived(
    player.current && player.playing ? albums.find((a) => a.tracks.some((t) => t.id === player.current?.id))?.key : null,
  );
</script>

<div class="ag" style:--min="{min}px">
  {#each list as a (a.key)}
    <AlbumCard
      album={a}
      playing={a.key === playingKey}
      onopen={() => go({ kind: "album", key: a.key })}
      onplay={() => void playTracks(a.tracks.map((t) => t.id), undefined, a.title)}
      onmenu={(e) => showTrackMenu(e, a.tracks.map((t) => t.id))}
    />
  {/each}
</div>
{#if !limit && shown < albums.length}<div bind:this={sentinel} class="more"></div>{/if}

<style>
  .ag {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--min), 1fr));
    gap: var(--space-5) var(--space-4);
  }
  .more {
    height: 40px;
  }
</style>
