<script lang="ts">
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import { nodeKind } from "$lib/canvas/nodes/registry";
  import { atWorld, worldLen } from "$lib/canvas/screen";
  import NodeCard from "../cards/NodeCard.svelte";
  import CordLabel from "../chrome/CordLabel.svelte";
  import ContextToolbar from "../chrome/ContextToolbar.svelte";
  import GroupHeader from "../chrome/GroupHeader.svelte";
  import NumberBadges from "../chrome/NumberBadges.svelte";
  import SelectionOverlay from "../chrome/SelectionOverlay.svelte";

  const engine = useEngine();

  const cards = $derived.by(() => {
    void engine.store.version;
    void engine.registryRev;
    return engine.cards
      .map((id) => engine.store.node(id))
      .filter((n): n is CanvasNode => !!n)
      .sort((a, b) => a.z - b.z);
  });
  const groups = $derived.by(() => {
    void engine.store.version;
    return engine.store.groups;
  });
  const connections = $derived.by(() => {
    void engine.store.version;
    return engine.store.connections;
  });
  const showLabels = $derived(engine.zoomUi >= 0.4 && !engine.moving);
</script>

<div class="screen">
  {#each cards as node (node.id)}
    {@const Card = nodeKind(node.kind)?.card ?? NodeCard}
    <div
      class="card-pos"
      style:transform={atWorld(node.x, node.y)}
      style:width={worldLen(node.w)}
      style:height={worldLen(node.h)}
    >
      <Card {node} selected={engine.selection.has(node.id)} />
    </div>
  {/each}

  {#each groups as group (group.id)}
    <GroupHeader {group} />
  {/each}

  {#if showLabels}
    {#each connections as connection (connection.id)}
      <CordLabel {connection} />
    {/each}
  {/if}

  <SelectionOverlay />
  <NumberBadges />
  <ContextToolbar />
</div>

<style>
  .screen {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
  }
  .card-pos {
    position: absolute;
    left: 0;
    top: 0;
    pointer-events: none;
  }
</style>
