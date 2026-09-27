<script lang="ts">
  import { t } from "$lib/i18n";
  import { fold, groupAlbums, groupArtists } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { go } from "$lib/music/ui-store.svelte";
  import Cover from "../Cover.svelte";
  import MusicIcon from "../MusicIcon.svelte";

  let q = $state("");
  let byPlays = $state(false);
  let shown = $state(150);
  let sentinel: HTMLDivElement | undefined = $state();

  let lib = $derived(library());
  let artists = $derived.by(() => {
    void lib.version;
    const all = groupArtists(lib.tracks, groupAlbums(lib.tracks));
    const f = fold(q.trim());
    const list = f ? all.filter((a) => fold(a.name).includes(f)) : all;
    return byPlays ? [...list].sort((a, b) => b.plays - a.plays) : list;
  });

  $effect(() => {
    if (!sentinel) return;
    const io = new IntersectionObserver((e) => {
      if (e[0].isIntersecting) shown += 150;
    });
    io.observe(sentinel);
    return () => io.disconnect();
  });
</script>

<div class="arv">
  <header>
    <h1>{$t("music.nav.artists")} <small>{artists.length}</small></h1>
    <div class="tools">
      <label class="find">
        <MusicIcon name="search" size={14} />
        <input class="input input-search" bind:value={q} placeholder={$t("music.filter_placeholder") as string} />
      </label>
      <div class="mac-segmented">
        <button class="mac-segmented-btn" class:active={!byPlays} onclick={() => (byPlays = false)}>{$t("music.order.name")}</button>
        <button class="mac-segmented-btn" class:active={byPlays} onclick={() => (byPlays = true)}>{$t("music.order.plays")}</button>
      </div>
    </div>
  </header>
  <div class="grid">
    {#each artists.slice(0, shown) as a (a.name)}
      <button class="ar" onclick={() => go({ kind: "artist", name: a.name })}>
        <Cover hash={a.cover} seed={a.name} size="100%" round icon="artist" />
        <span class="n">{a.name}</span>
        <span class="c">{$t("music.count_songs", { count: a.tracks.length })}</span>
      </button>
    {/each}
  </div>
  {#if shown < artists.length}<div bind:this={sentinel} style="height:40px"></div>{/if}
</div>

<style>
  .arv {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  h1 small {
    font-size: var(--text-md);
    color: var(--text-muted);
    font-weight: 500;
  }
  .tools {
    display: flex;
    gap: var(--space-3);
    align-items: center;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: var(--space-5) var(--space-4);
  }
  .ar {
    all: unset;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    text-align: center;
    min-width: 0;
  }
  .ar :global(.cover) {
    width: 100% !important;
    height: auto !important;
    aspect-ratio: 1;
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.25);
    transition: transform var(--duration-fast);
  }
  .ar:hover :global(.cover) {
    transform: scale(1.03);
  }
  .ar:focus-visible {
    outline: var(--focus-ring);
    border-radius: 8px;
  }
  .n {
    font-weight: 600;
    max-width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .c {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
