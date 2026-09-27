<script lang="ts">
  // Sunburst: the current folder in the centre, one ring per level. Own
  // SVG (no chart library); hover and focus behave like the treemap.
  import { formatBytes } from "$lib/washmybelly/api";
  import { arcPath, sunburst, type UsageNode } from "$lib/washmybelly/disk";

  let {
    node,
    depth = 3,
    otherLabel,
    onOpen,
    onHover,
  }: { node: UsageNode; depth?: number; otherLabel: (n: UsageNode) => string; onOpen?: (n: UsageNode) => void; onHover?: (n: UsageNode | null) => void } = $props();

  const R = 180;
  const HOLE = 56;
  let hovered = $state<UsageNode | null>(null);
  let arcs = $derived(sunburst(node, depth, R, HOLE).filter((a) => a.a1 - a.a0 > 0.004));

  function label(n: UsageNode) {
    return n.kind === "other" ? otherLabel(n) : n.name;
  }
  function openable(n: UsageNode) {
    return n.kind === "dir" && !!n.expandable;
  }
  function enter(n: UsageNode | null) {
    hovered = n;
    onHover?.(n);
  }
  let centre = $derived(hovered ?? node);
</script>

<div class="sb">
  <svg viewBox="{-R - 4} {-R - 4} {2 * R + 8} {2 * R + 8}" role="group" onpointerleave={() => enter(null)}>
    {#each arcs as a (a.node.path ?? `${a.depth}:${a.a0}`)}
      <path
        d={arcPath(a.a0, a.a1, a.r0 + 0.75, a.r1 - 0.75)}
        fill={a.color}
        fill-opacity={a.node.kind === "file" ? 0.5 : Math.max(0.45, 0.92 - (a.depth - 1) * 0.16)}
        class:dim={hovered !== null && hovered !== a.node}
        class:clickable={openable(a.node)}
        role="button"
        tabindex={openable(a.node) ? 0 : -1}
        aria-label="{label(a.node)}, {formatBytes(a.node.bytes)}"
        onpointerenter={() => enter(a.node)}
        onfocus={() => enter(a.node)}
        onclick={() => openable(a.node) && onOpen?.(a.node)}
        onkeydown={(e) => { if ((e.key === "Enter" || e.key === " ") && openable(a.node)) { e.preventDefault(); onOpen?.(a.node); } }}
      ><title>{label(a.node)} · {formatBytes(a.node.bytes)}</title></path>
    {/each}
    <text y="-4" class="c-name">{label(centre).slice(0, 18)}</text>
    <text y="16" class="c-size">{formatBytes(centre.bytes)}</text>
  </svg>
</div>

<style>
  .sb { display: grid; place-items: center; padding: 8px 0; }
  svg { width: min(100%, 400px); height: auto; display: block; user-select: none; }
  path { stroke: var(--surface, #fff); stroke-width: 0; transition: opacity 160ms ease-out; }
  path.dim { opacity: 0.5; }
  path.clickable { cursor: pointer; }
  path:focus-visible { outline: none; stroke: var(--accent); stroke-width: 2; }
  text { text-anchor: middle; fill: var(--text); pointer-events: none; }
  .c-name { font-size: 13px; font-weight: 600; }
  .c-size { font-size: 12px; fill: var(--text-dim, var(--text)); font-variant-numeric: tabular-nums; }
  @media (prefers-reduced-motion: reduce) { path { transition: none; } }
</style>
