<script lang="ts">
  // Moods & genres buttons: tonal pill with the category color as a stripe on
  // the left, like YT Music. `rows` lays them out in N rows that scroll sideways
  // (home/explore shelf); without it they wrap in a responsive grid.
  import { go } from "$lib/tube/ui.svelte";
  import type { Item, MoodItem } from "$lib/tube/types";

  let { items, rows = 0 }: { items: Item[]; rows?: number } = $props();

  let moods = $derived(items.filter((i): i is MoodItem => i.kind === "mood"));

  function color(c: string | null | undefined): string {
    if (!c) return "var(--ytm-text-3, #909090)";
    return /^[0-9a-f]{6,8}$/i.test(c) ? `#${c}` : c;
  }
</script>

<div class="mg" class:rows={rows > 0} style:--mg-rows={rows || 1}>
  {#each moods as m, i (m.browse_id + (m.params ?? "") + i)}
    <button class="mg-btn" style:--mg-c={color(m.color)} onclick={() => go({ name: "m_browse", id: m.browse_id, params: m.params ?? null, title: m.title })}>
      <span class="yt-clamp-1">{m.title}</span>
    </button>
  {/each}
</div>

<style>
  .mg {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(176px, 1fr));
    gap: 12px 16px;
  }
  .mg.rows {
    grid-auto-flow: column;
    grid-template-columns: none;
    grid-template-rows: repeat(var(--mg-rows), 48px);
    grid-auto-columns: 176px;
  }
  .mg-btn {
    position: relative;
    display: flex; align-items: center;
    height: 48px; padding: 0 16px 0 22px;
    border: 0; border-radius: 8px; overflow: hidden;
    font: 500 14px/20px var(--yt-font); text-align: left;
    color: var(--ytm-text, var(--yt-text));
    background: var(--ytm-bg-a, var(--yt-tonal));
    cursor: pointer; scroll-snap-align: start;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .mg-btn::before {
    content: ""; position: absolute; left: 0; top: 0; bottom: 0; width: 6px;
    background: var(--mg-c);
  }
  .mg-btn:hover { background: var(--ytm-chip-hover, var(--yt-tonal-hover)); }
  .mg-btn:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .mg-btn span { min-width: 0; }
</style>
