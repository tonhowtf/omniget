<script lang="ts">
  import Icon from "./Icon.svelte";

  /// "Possible gain" card on the overview: what a module could free or fix,
  /// with the module's coloured tile. Clicking opens that module.
  let {
    title,
    value,
    detail = "",
    onClick,
    tile,
    busy = false,
  }: {
    title: string;
    value: string;
    detail?: string;
    onClick?: () => void;
    tile?: { glyph: string; from: string; to: string };
    busy?: boolean;
  } = $props();
</script>

<svelte:element
  this={onClick ? "button" : "div"}
  type={onClick ? "button" : undefined}
  class="wmb-gain"
  role={onClick ? undefined : "group"}
  class:interactive={!!onClick}
  onclick={onClick}
  aria-busy={busy || undefined}
>
  {#if tile}
    <span class="tile" style:--from={tile.from} style:--to={tile.to} aria-hidden="true">
      <span class="glyph" style:--glyph="url(/icons/{tile.glyph}.svg)"></span>
    </span>
  {/if}
  <span class="body">
    <span class="title">{title}</span>
    {#if busy}
      <span class="skeleton value-skel"></span>
    {:else}
      <span class="value">{value}</span>
    {/if}
    {#if detail}<span class="detail">{detail}</span>{/if}
  </span>
  {#if onClick}<span class="chev" aria-hidden="true"><Icon name="chevron" size={14} /></span>{/if}
</svelte:element>

<style>
  .wmb-gain {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-width: 0;
    padding: var(--space-3) var(--space-3) var(--space-3) var(--space-4);
    border: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font: inherit;
    text-align: left;
  }
  .wmb-gain.interactive {
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }
  @media (hover: hover) {
    .wmb-gain.interactive:hover {
      background: var(--surface-hi);
    }
  }
  .wmb-gain.interactive:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .tile {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 36px;
    height: 36px;
    border-radius: 9px;
    background: linear-gradient(180deg, var(--from), var(--to));
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.12), inset 0 1px 0 rgb(255 255 255 / 0.18);
  }
  .glyph {
    width: 21px;
    height: 21px;
    background: #fff;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .value {
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    font-variant-numeric: tabular-nums;
  }
  .value-skel {
    width: 72px;
    height: 20px;
    margin: 2px 0;
  }
  .detail {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chev {
    color: var(--text-faint);
    display: flex;
  }
</style>
