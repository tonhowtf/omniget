<script lang="ts">
  import type { Snippet } from "svelte";
  import { hueOf, initial } from "$lib/washmybelly/startup";

  let {
    name,
    sub = "",
    live = false,
    trailing,
    extra,
  }: { name: string; sub?: string; live?: boolean; trailing?: Snippet; extra?: Snippet } = $props();
</script>

<div class="group-row gr-row" role="listitem">
  <span class="gr-tile" class:live style:--h={hueOf(name)} aria-hidden="true">{initial(name)}</span>
  <div class="group-row-content">
    <span class="group-row-title gr-name" title={name}>{name}</span>
    {#if sub}<span class="group-row-sub">{sub}</span>{/if}
    {#if extra}{@render extra()}{/if}
  </div>
  {#if trailing}
    <div class="group-row-trailing">{@render trailing()}</div>
  {/if}
</div>

<style>
  .gr-row {
    align-items: center;
  }
  .gr-tile {
    display: inline-grid;
    place-items: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: 8px;
    background: linear-gradient(160deg, hsl(var(--h) 70% 62%), hsl(var(--h) 62% 46%));
    color: #fff;
    font-weight: 700;
    font-size: 15px;
  }
  .gr-tile.live {
    box-shadow: 0 0 0 2px var(--surface, transparent), 0 0 0 4px color-mix(in srgb, var(--success) 70%, transparent);
  }
  .gr-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
