<script lang="ts">
  import { t } from "$lib/i18n";
  import { fold, groupAlbums } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import AlbumGrid from "../AlbumGrid.svelte";
  import MusicIcon from "../MusicIcon.svelte";

  type Order = "artist" | "title" | "year" | "added";
  let order = $state<Order>("artist");
  let q = $state("");
  let size = $state(172);

  let lib = $derived(library());
  let albums = $derived.by(() => {
    void lib.version;
    const all = groupAlbums(lib.tracks);
    const f = fold(q.trim());
    const list = f ? all.filter((a) => fold(`${a.title} ${a.artist}`).includes(f)) : all;
    const s = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base" });
    return list.sort((a, b) =>
      order === "title"
        ? s(a.title, b.title)
        : order === "year"
          ? (b.year ?? 0) - (a.year ?? 0)
          : order === "added"
            ? b.added_at - a.added_at
            : s(a.artist, b.artist) || (a.year ?? 0) - (b.year ?? 0),
    );
  });
  const ORDERS: Order[] = ["artist", "title", "year", "added"];
</script>

<div class="av">
  <header>
    <h1>{$t("music.nav.albums")} <small>{albums.length}</small></h1>
    <div class="tools">
      <label class="find">
        <MusicIcon name="search" size={14} />
        <input class="input input-search" bind:value={q} placeholder={$t("music.filter_placeholder") as string} />
      </label>
      <div class="mac-segmented">
        {#each ORDERS as o (o)}
          <button class="mac-segmented-btn" class:active={order === o} onclick={() => (order = o)}>{$t(`music.order.${o}`)}</button>
        {/each}
      </div>
      <input type="range" min="120" max="260" step="10" bind:value={size} aria-label={$t("music.cover_size") as string} />
    </div>
  </header>
  <AlbumGrid {albums} min={size} />
</div>

<style>
  .av {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
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
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }
  input[type="range"] {
    width: 90px;
    accent-color: var(--accent);
  }
</style>
