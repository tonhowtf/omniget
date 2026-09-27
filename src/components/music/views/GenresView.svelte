<script lang="ts">
  import { t } from "$lib/i18n";
  import { genres } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { go } from "$lib/music/ui-store.svelte";
  import Cover from "../Cover.svelte";

  let lib = $derived(library());
  let list = $derived.by(() => {
    void lib.version;
    return genres(lib.tracks);
  });

  function hue(s: string) {
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (h * 33 + s.charCodeAt(i)) >>> 0;
    return h % 360;
  }
</script>

<div class="gv">
  <h1>{$t("music.nav.genres")} <small>{list.length}</small></h1>
  {#if !list.length}
    <p class="dim">{$t("music.genres_empty")}</p>
  {/if}
  <div class="grid">
    {#each list as g (g.name)}
      {@const covers = [...new Set(g.tracks.map((x) => x.cover).filter(Boolean))].slice(0, 2) as string[]}
      <button class="g" style:--h={hue(g.name)} onclick={() => go({ kind: "genre", name: g.name })}>
        <span class="name">{g.name}</span>
        <span class="count">{$t("music.count_songs", { count: g.tracks.length })}</span>
        {#if covers[0]}<span class="art"><Cover hash={covers[0]} seed={g.name} size={72} radius={6} /></span>{/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .gv {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
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
  .dim {
    color: var(--text-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: var(--space-3);
  }
  .g {
    all: unset;
    position: relative;
    height: 104px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    overflow: hidden;
    cursor: pointer;
    background: linear-gradient(135deg, hsl(var(--h) 60% 42%), hsl(calc(var(--h) + 35) 55% 28%));
    color: white;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }
  .g:focus-visible {
    outline: var(--focus-ring);
  }
  .name {
    font-weight: 750;
    font-size: var(--text-lg);
    max-width: 70%;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
  }
  .count {
    font-size: var(--text-xs);
    opacity: 0.85;
  }
  .art {
    position: absolute;
    right: -10px;
    bottom: -8px;
    transform: rotate(18deg);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.35);
    border-radius: 6px;
    transition: transform var(--duration-fast);
  }
  .g:hover .art {
    transform: rotate(10deg) scale(1.05);
  }
</style>
