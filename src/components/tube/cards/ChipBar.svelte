<script lang="ts">
  import Scroller from "./Scroller.svelte";

  export type ChipDef = { key: string; label: string; selected: boolean };

  let {
    chips,
    onselect,
    sticky = false,
    label = "",
  }: { chips: ChipDef[]; onselect: (key: string) => void; sticky?: boolean; label?: string } = $props();
</script>

<div class="chipbar" class:sticky>
  <Scroller gap={12} arrows class="chips" {label}>
    {#each chips as c (c.key)}
      <button class="yt-chip" aria-pressed={c.selected} onclick={() => onselect(c.key)}>{c.label}</button>
    {/each}
  </Scroller>
</div>

<style>
  .chipbar {
    display: flex;
    align-items: center;
    min-height: 56px;
    min-width: 0;
  }
  .chipbar.sticky {
    position: sticky;
    top: 0;
    z-index: 5;
    margin: 0 calc(-1 * var(--yt-grid-gutter));
    padding: 0 var(--yt-grid-gutter);
    background: var(--yt-frosted);
    backdrop-filter: blur(48px);
    -webkit-backdrop-filter: blur(48px);
  }
  .chipbar > :global(.scroller) {
    flex: 1;
  }
  .yt-chip {
    flex: none;
  }
  .yt-chip:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: 2px;
  }
</style>
