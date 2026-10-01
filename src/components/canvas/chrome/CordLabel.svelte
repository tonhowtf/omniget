<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { cordPath, sampleCord } from "$lib/canvas/cords/route";
  import type { Connection } from "$lib/canvas/model/types";
  import { pointAtLength } from "$lib/canvas/spatial/geometry";
  import { atWorld } from "$lib/canvas/screen";

  let { connection }: { connection: Connection } = $props();
  const engine = useEngine();

  const mid = $derived.by(() => {
    void engine.store.version;
    const a = engine.store.node(connection.from);
    const b = engine.store.node(connection.to);
    if (!a || !b) return null;
    const s = sampleCord(cordPath(a, b), 24);
    return pointAtLength(s.pts, s.lens, 0.5);
  });
  const selected = $derived(engine.selection.has(connection.id));
  const text = $derived(connection.label || $t(`canvas.engine.cord.${connection.kind}`, { default: connection.kind }));

  function select(e: PointerEvent) {
    if (e.button !== 0) return;
    e.stopPropagation();
    if (e.shiftKey) engine.selection.toggle(connection.id);
    else engine.selection.set([connection.id]);
    engine.invalidate();
  }
</script>

{#if mid}
  <div class="label-pos" style:transform={atWorld(mid.x, mid.y)}>
    <button type="button" class="chip" class:selected data-canvas-chrome onpointerdown={select}>
      {text}
      {#if connection.condition && connection.condition !== "always"}
        <span class="cond">{$t(`canvas.engine.cond.${connection.condition}`, { default: connection.condition })}</span>
      {/if}
    </button>
  </div>
{/if}

<style>
  .label-pos {
    position: absolute;
    left: 0;
    top: 0;
    pointer-events: none;
  }
  .chip {
    all: unset;
    transform: translate(-50%, -50%);
    display: inline-flex;
    gap: 4px;
    align-items: center;
    padding: 1px 7px;
    border-radius: 999px;
    font-size: 10px;
    line-height: 16px;
    color: var(--text-muted);
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    pointer-events: auto;
    cursor: pointer;
    white-space: nowrap;
  }
  .chip.selected {
    color: var(--accent-text, var(--accent));
    border-color: var(--accent);
  }
  .cond {
    opacity: 0.8;
  }
</style>
