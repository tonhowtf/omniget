<script lang="ts">
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { drawingBounds } from "$lib/canvas/model/store.svelte";
  import type { Rect } from "$lib/canvas/model/types";
  import { SIDES, portPoint } from "$lib/canvas/cords/route";
  import type { ResizeHandle } from "$lib/canvas/spatial/hit";
  import { atWorld, worldLen } from "$lib/canvas/screen";

  const engine = useEngine();

  const items = $derived.by(() => {
    void engine.store.version;
    const out: Array<{ id: string; r: Rect }> = [];
    for (const id of engine.selection.ids) {
      const n = engine.store.node(id);
      if (n) out.push({ id, r: n });
      const d = engine.store.drawing(id);
      if (d) out.push({ id, r: drawingBounds(d) });
    }
    return out;
  });

  const bounds = $derived.by(() => {
    void engine.store.version;
    return items.length ? engine.selectionBounds() : null;
  });

  const resizable = $derived(
    items.some((i) => {
      const n = engine.store.node(i.id);
      return (n && !n.locked) || !!engine.store.drawing(i.id);
    }),
  );

  const single = $derived.by(() => {
    if (items.length !== 1) return null;
    return engine.store.node(items[0].id) ?? null;
  });

  const HANDLES: ResizeHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

  function handlePos(h: ResizeHandle, r: Rect): string {
    const x = h.includes("w") ? r.x : h.includes("e") ? r.x + r.w : r.x + r.w / 2;
    const y = h.includes("n") ? r.y : h.includes("s") ? r.y + r.h : r.y + r.h / 2;
    return atWorld(x, y, -5, -5);
  }

  function onHandle(e: PointerEvent, h: ResizeHandle) {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    engine.tools.startResize(e, h);
  }

  function onPort(e: PointerEvent, id: string) {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    engine.tools.startCord(e, id);
  }
</script>

{#if !engine.selection.focused}
  {#each items as item (item.id)}
    {#if items.length > 1 || !single}
      <div
        class="outline"
        style:transform={atWorld(item.r.x, item.r.y, -2, -2)}
        style:width={worldLen(item.r.w, 4)}
        style:height={worldLen(item.r.h, 4)}
      ></div>
    {/if}
  {/each}

  {#if bounds && resizable && !engine.moving}
    <div
      class="bounds"
      style:transform={atWorld(bounds.x, bounds.y, -1, -1)}
      style:width={worldLen(bounds.w, 2)}
      style:height={worldLen(bounds.h, 2)}
    ></div>
    {#each HANDLES as h (h)}
      <div
        class="handle h-{h}"
        data-canvas-chrome
        role="presentation"
        style:transform={handlePos(h, bounds)}
        onpointerdown={(e) => onHandle(e, h)}
      ></div>
    {/each}
  {/if}

  {#if single && !engine.moving}
    {#each SIDES as side (side)}
      {@const p = portPoint(single, side)}
      <div
        class="port"
        data-canvas-chrome
        role="presentation"
        style:transform={atWorld(p.x, p.y, -6, -6)}
        onpointerdown={(e) => onPort(e, single.id)}
      ></div>
    {/each}
  {/if}
{/if}

<style>
  .outline,
  .bounds,
  .handle,
  .port {
    position: absolute;
    left: 0;
    top: 0;
    box-sizing: border-box;
  }
  .outline {
    border: 1.5px dashed var(--accent);
    border-radius: 8px;
    pointer-events: none;
  }
  .bounds {
    border: 1.5px dashed var(--accent);
    border-radius: 4px;
    pointer-events: none;
  }
  .handle {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    background: transparent;
    border: 1.5px solid transparent;
    pointer-events: auto;
  }
  .handle:hover {
    background: var(--popup-bg, #fff);
    border-color: var(--accent);
  }
  .h-nw,
  .h-se {
    cursor: nwse-resize;
  }
  .h-ne,
  .h-sw {
    cursor: nesw-resize;
  }
  .h-n,
  .h-s {
    cursor: ns-resize;
  }
  .h-e,
  .h-w {
    cursor: ew-resize;
  }
  .port {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent);
    border: 2px solid var(--popup-bg, #fff);
    pointer-events: auto;
    cursor: crosshair;
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .port:hover {
    opacity: 1;
  }
</style>
