<script lang="ts">
  // Squarified treemap. Layout and label tiers ported from the "Market
  // Heatmap" component on 21st.dev (arihantcodes); colours are one Apple
  // hue per folder, files lighter, the "other" tile grey.
  import { formatBytes } from "$lib/washmybelly/api";
  import { colorFor, labelTier, squarify, type UsageNode } from "$lib/washmybelly/disk";

  let {
    node,
    otherLabel,
    onOpen,
    onHover,
  }: { node: UsageNode; otherLabel: (n: UsageNode) => string; onOpen?: (n: UsageNode) => void; onHover?: (n: UsageNode | null) => void } = $props();

  let width = $state(0);
  const height = 380;
  const gap = 2;
  let hovered = $state<string | null>(null);

  let tiles = $derived(width > 0 ? squarify(node.children ?? [], 0, 0, width, height) : []);

  function key(n: UsageNode) {
    return n.path ?? `other:${n.name}`;
  }
  function label(n: UsageNode) {
    return n.kind === "other" ? otherLabel(n) : n.name;
  }
  function openable(n: UsageNode) {
    return n.kind === "dir" && !!n.expandable;
  }
  function enter(n: UsageNode) {
    hovered = key(n);
    onHover?.(n);
  }
</script>

<div class="tm" bind:clientWidth={width} style:height="{height}px">
  {#if width > 0}
    <svg {width} {height} viewBox="0 0 {width} {height}" role="group" onpointerleave={() => { hovered = null; onHover?.(null); }}>
      {#each tiles as tile, i (key(tile.node))}
        {@const tw = Math.max(0, tile.w - gap)}
        {@const th = Math.max(0, tile.h - gap)}
        {@const tier = labelTier(tw, th)}
        {@const active = hovered === key(tile.node)}
        <g
          class="tile"
          class:clickable={openable(tile.node)}
          style:animation-delay="{Math.min(i * 18, 360)}ms"
          role="button"
          tabindex={openable(tile.node) ? 0 : -1}
          aria-label="{label(tile.node)}, {formatBytes(tile.node.bytes)}"
          onpointerenter={() => enter(tile.node)}
          onfocus={() => enter(tile.node)}
          onclick={() => openable(tile.node) && onOpen?.(tile.node)}
          onkeydown={(e) => { if ((e.key === "Enter" || e.key === " ") && openable(tile.node)) { e.preventDefault(); onOpen?.(tile.node); } }}
        >
          <rect
            x={tile.x}
            y={tile.y}
            width={tw}
            height={th}
            rx="6"
            fill={colorFor(tile.index, tile.node)}
            fill-opacity={tile.node.kind === "file" ? 0.55 : 0.88}
            class:dim={hovered !== null && !active}
            class:active
          />
          {#if tier !== "none"}
            <text x={tile.x + 8} y={tile.y + 17} class="name">{label(tile.node).slice(0, Math.max(3, Math.floor(tw / 7.5)))}</text>
            {#if tier === "full"}
              <text x={tile.x + 8} y={tile.y + 33} class="size">{formatBytes(tile.node.bytes)}</text>
            {/if}
          {/if}
          <title>{label(tile.node)} · {formatBytes(tile.node.bytes)}</title>
        </g>
      {/each}
    </svg>
  {/if}
</div>

<style>
  .tm { width: 100%; border-radius: var(--radius-lg, 12px); overflow: hidden; background: var(--surface-mut, var(--surface)); }
  svg { display: block; user-select: none; }
  .tile { animation: tm-in 380ms ease-out both; }
  .tile.clickable { cursor: pointer; }
  .tile:focus-visible { outline: none; }
  .tile:focus-visible rect { stroke: var(--accent); stroke-width: 2; }
  rect { transition: opacity 160ms ease-out; stroke: transparent; stroke-width: 1.5; }
  rect.dim { opacity: 0.55; }
  rect.active { stroke: var(--text, #fff); }
  text { pointer-events: none; fill: #fff; font-family: var(--font-display, inherit); }
  .name { font-size: 12.5px; font-weight: 600; }
  .size { font-size: 11px; opacity: 0.85; font-variant-numeric: tabular-nums; }
  @keyframes tm-in { from { opacity: 0; } to { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .tile { animation: none; } rect { transition: none; } }
</style>
