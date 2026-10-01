<script lang="ts">
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { atWorld } from "$lib/canvas/screen";

  const engine = useEngine();
  const numbered = $derived.by(() => {
    void engine.store.version;
    return engine.numbered().slice(0, 9);
  });
</script>

{#if engine.showBadges}
  {#each numbered as n, i (n.id)}
    <div class="badge" style:transform={atWorld(n.x, n.y, -10, -10)}>{i + 1}</div>
  {/each}
{/if}

<style>
  .badge {
    position: absolute;
    left: 0;
    top: 0;
    min-width: 22px;
    height: 22px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    font: 700 12px/1 var(--font-system, system-ui);
    color: var(--on-accent, #fff);
    background: var(--accent);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
    pointer-events: none;
  }
</style>
