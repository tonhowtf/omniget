<script lang="ts">
  import { useEngine } from "$lib/canvas/engine.svelte";

  const engine = useEngine();

  const items = $derived.by(() => {
    void engine.store.version;
    return engine.store.drawings.filter((d) => (d.text ?? "").trim() !== "" && d.kind !== "arrow" && d.kind !== "line");
  });
</script>

{#each items as d (d.id)}
  <div
    class="dtext"
    class:free={d.kind === "text"}
    data-drawing-id={d.id}
    style:left="{d.x}px"
    style:top="{d.y}px"
    style:width="{Math.max(1, d.w)}px"
    style:height="{Math.max(1, d.h)}px"
    style:color={d.color || "var(--text)"}
    style:opacity={d.opacity ?? 1}
    style:font-size="{d.textFontSize ?? (d.kind === 'text' ? 20 : 16)}px"
    style:transform={d.rotation ? `rotate(${d.rotation}deg)` : undefined}
  >
    <span>{d.text}</span>
  </div>
{/each}

<style>
  .dtext {
    position: absolute;
    z-index: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 6px;
    box-sizing: border-box;
    overflow: hidden;
    pointer-events: none;
    transform-origin: 50% 50%;
    line-height: 1.25;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .dtext.free {
    align-items: flex-start;
    justify-content: flex-start;
    text-align: left;
    padding: 0;
    overflow: visible;
  }
</style>
