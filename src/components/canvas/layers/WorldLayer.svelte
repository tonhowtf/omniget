<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { isTerminalKind, type CanvasNode } from "$lib/canvas/model/types";
  import { nodeKind } from "$lib/canvas/nodes/registry";
  import { agentStates, stateCss } from "$lib/canvas/state/agents.svelte";
  import Icon from "../chrome/Icon.svelte";
  import BlurCover from "../chrome/BlurCover.svelte";
  import DrawingTexts from "./DrawingTexts.svelte";
  import { canvasPrefs } from "$lib/canvas/prefs.svelte";
  import type { Rect } from "$lib/canvas/model/types";
  import type { DockEdge, DockedPanel } from "$lib/canvas/panels/panels.svelte";

  let { onready }: { onready?: (el: HTMLDivElement) => void } = $props();

  const engine = useEngine();
  let el = $state<HTMLDivElement | null>(null);

  const items = $derived.by(() => {
    void engine.store.version;
    void engine.registryRev;
    const out: Array<{ node: CanvasNode; hidden: boolean }> = [];
    for (const m of engine.mounted) {
      const node = engine.store.node(m.id);
      if (node) out.push({ node, hidden: m.hidden });
    }
    return out;
  });

  onMount(() => {
    if (el) onready?.(el);
  });

  function patch(id: string, p: Partial<CanvasNode>) {
    engine.updateNode(id, p);
  }

  const panels = engine.panels;
  const raisedItems = $derived.by(() => {
    void engine.store.version;
    void panels.rev;
    void panels.lifted;
    void panels.docked;
    const out: Array<{ node: CanvasNode; view: CanvasNode; rect: Rect; bar: Rect; dock: DockedPanel | undefined; lifted: boolean }> = [];
    for (const id of panels.raisedIds()) {
      const node = engine.store.node(id);
      const rect = panels.rect(id);
      const bar = panels.barRect(id);
      if (!node || !rect || !bar) continue;
      out.push({ node, view: { ...node, w: rect.w, h: rect.h }, rect, bar, dock: panels.dockOf(id), lifted: panels.lifted === id });
    }
    return out;
  });
  const liftedItem = $derived(raisedItems.find((r) => r.lifted));

  function polygonWithHoles(holes: Rect[]): string {
    void panels.rev;
    const W = engine.camera.vw;
    const H = engine.camera.vh;
    const pts: string[] = ["0px 0px", `${W}px 0px`, `${W}px ${H}px`, `0px ${H}px`, "0px 0px"];
    for (const h of holes) {
      pts.push(`${h.x}px ${h.y}px`, `${h.x + h.w}px ${h.y}px`, `${h.x + h.w}px ${h.y + h.h}px`, `${h.x}px ${h.y + h.h}px`, `${h.x}px ${h.y}px`, "0px 0px");
    }
    return `polygon(evenodd, ${pts.join(", ")})`;
  }
  const outer = (r: { rect: Rect; bar: Rect }): Rect => ({ x: r.bar.x, y: r.bar.y, w: r.rect.w, h: r.rect.h + r.bar.h });
  const worldClip = $derived(raisedItems.length ? polygonWithHoles(raisedItems.map(outer)) : "none");
  const backdropClip = $derived(liftedItem ? polygonWithHoles([outer(liftedItem)]) : "none");

  let drag: { id: string; x0: number; moved: boolean; pointerId: number } | null = null;
  let dragX = $state<number | null>(null);
  const EDGE_ZONE = 64;

  function barDown(e: PointerEvent, id: string) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    e.preventDefault();
    e.stopPropagation();
    drag = { id, x0: e.clientX, moved: false, pointerId: e.pointerId };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  }
  function barMove(e: PointerEvent) {
    if (!drag) return;
    if (Math.abs(e.clientX - drag.x0) > 4) drag.moved = true;
    if (drag.moved) dragX = e.clientX;
  }
  function barUp(e: PointerEvent, root: HTMLElement | null) {
    const d = drag;
    drag = null;
    dragX = null;
    if (!d || !d.moved) return;
    const box = root?.getBoundingClientRect();
    const x = e.clientX - (box?.left ?? 0);
    const vw = engine.camera.vw;
    if (x <= panels.sidebarW + EDGE_ZONE) panels.dock(d.id, "left");
    else if (x >= vw - EDGE_ZONE) panels.dock(d.id, "right");
    else if (panels.dockOf(d.id)) {
      panels.dock(d.id, "none");
      panels.lift(d.id);
    }
  }

  let resizing: { id: string; edge: DockEdge; x0: number; w0: number } | null = null;
  function gripDown(e: PointerEvent, id: string, dock: DockedPanel) {
    e.preventDefault();
    e.stopPropagation();
    resizing = { id, edge: dock.edge, x0: e.clientX, w0: dock.width };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  }
  function gripMove(e: PointerEvent) {
    if (!resizing) return;
    const dx = e.clientX - resizing.x0;
    panels.setDockWidth(resizing.id, resizing.w0 + (resizing.edge === "left" ? dx : -dx), false);
  }
  function gripUp() {
    if (!resizing) return;
    const id = resizing.id;
    resizing = null;
    const d = panels.dockOf(id);
    if (d) panels.setDockWidth(id, d.width, true);
  }

  let raisedEl = $state<HTMLDivElement | null>(null);
</script>

<div class="world-area" style:clip-path={worldClip}>
<div
  class="world"
  class:moving={engine.moving}
  data-selection-style={canvasPrefs().selection_style}
  data-material={canvasPrefs().node_material}
  bind:this={el}
>
  <DrawingTexts />
  {#each items as { node, hidden } (node.id)}
    {@const def = nodeKind(node.kind)}
    {@const Comp = def?.component}
    {@const selected = engine.selection.has(node.id)}
    {@const focused = engine.selection.focused === node.id}
    {@const own = isTerminalKind(node.kind)}
    {@const bare = own || node.kind === "text" || node.kind === "connector"}
    <div
      class="node"
      class:bare
      class:own
      class:hidden
      class:selected
      class:focused
      class:locked={node.locked}
      data-node-id={node.id}
      data-kind={node.kind}
      style:left="{node.x}px"
      style:top="{node.y}px"
      style:width="{node.w}px"
      style:height="{node.h}px"
      style:z-index={node.z}
    >
      {#if !bare}
        <div class="header" data-node-header>
          {#if isTerminalKind(node.kind)}
            <span class="dot" style:background={stateCss(agentStates.stateOf(node))}></span>
          {:else if def}
            <span class="icon"><Icon d={def.icon} size={14} /></span>
          {/if}
          <span class="title">{node.title || node.kind}</span>
          {#if node.locked}<span class="lock">{$t("canvas.engine.locked", { default: "Locked" })}</span>{/if}
        </div>
      {/if}
      <div class="body" data-node-body>
        {#if Comp}
          <Comp
            {node}
            {selected}
            {focused}
            zoom={engine.zoomUi}
            lod={engine.lod[node.id] ?? "near"}
            workspaceId={engine.store.workspaceId}
            onpatch={(p: Partial<CanvasNode>) => patch(node.id, p)}
            onfocusrequest={() => engine.selection.focus(node.id)}
            onexit={() => engine.exitFocus()}
          />
        {:else}
          <div class="pending">{node.kind}</div>
        {/if}
        {#if node.data.blur}<BlurCover {node} />{/if}
      </div>
    </div>
  {/each}
</div>
</div>

{#if raisedItems.length}
  <div class="raised" bind:this={raisedEl}>
    {#if liftedItem}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div
        class="backdrop"
        data-canvas-chrome
        style:clip-path={backdropClip}
        onclick={() => panels.lift(null)}
      ></div>
    {/if}
    {#each raisedItems as item (item.node.id)}
      {@const node = item.node}
      {@const def = nodeKind(node.kind)}
      {@const Comp = def?.component}
      {@const own = isTerminalKind(node.kind)}
      {@const bare = own || node.kind === "text"}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="panel-bar"
        class:docked={!!item.dock && !item.lifted}
        data-canvas-chrome
        data-panel-bar={node.id}
        style:left="{item.bar.x}px"
        style:top="{item.bar.y}px"
        style:width="{item.bar.w}px"
        style:height="{item.bar.h}px"
        onpointerdown={(e) => barDown(e, node.id)}
        onpointermove={barMove}
        onpointerup={(e) => barUp(e, raisedEl)}
        ondblclick={() => (item.lifted ? panels.lift(null) : panels.dock(node.id, "none"))}
      >
        {#if own}
          <span class="dot" style:background={stateCss(agentStates.stateOf(node))}></span>
        {:else if def}
          <span class="icon"><Icon d={def.icon} size={14} /></span>
        {/if}
        <span class="title">{node.title || node.kind}</span>
        <span class="bar-actions">
          {#if item.lifted}
            <button type="button" title={$t("canvas.panels.dockLeft", { default: "Dock left" })} onclick={() => panels.dock(node.id, "left")}>⇤</button>
            <button type="button" title={$t("canvas.panels.dockRight", { default: "Dock right" })} onclick={() => panels.dock(node.id, "right")}>⇥</button>
          {:else}
            <button type="button" title={$t("canvas.panels.lift", { default: "Lift" })} onclick={() => panels.lift(node.id)}>⤢</button>
          {/if}
          <button
            type="button"
            title={$t("canvas.panels.putBack", { default: "Put back on the canvas" })}
            onclick={() => (item.lifted ? panels.lift(null) : panels.dock(node.id, "none"))}>✕</button
          >
        </span>
      </div>
      <div
        class="node raised-node"
        class:bare
        class:own
        class:focused={engine.selection.focused === node.id}
        data-node-id={node.id}
        data-kind={node.kind}
        data-raised={item.lifted ? "lifted" : "docked"}
        style:left="{item.rect.x}px"
        style:top="{item.rect.y}px"
        style:width="{item.rect.w}px"
        style:height="{item.rect.h}px"
      >
        {#if !bare}
          <div class="header" data-node-header>
            {#if def}<span class="icon"><Icon d={def.icon} size={14} /></span>{/if}
            <span class="title">{node.title || node.kind}</span>
          </div>
        {/if}
        <div class="body" data-node-body>
          {#if Comp}
            <Comp
              node={item.view}
              selected={engine.selection.has(node.id)}
              focused={engine.selection.focused === node.id}
              zoom={1}
              lod="near"
              workspaceId={engine.store.workspaceId}
              onpatch={(p: Partial<CanvasNode>) => patch(node.id, p)}
              onfocusrequest={() => engine.selection.focus(node.id)}
              onexit={() => engine.exitFocus()}
            />
          {/if}
          {#if node.data.blur}<BlurCover {node} />{/if}
        </div>
      </div>
      {#if item.dock && !item.lifted}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="grip"
          data-canvas-chrome
          style:left="{item.dock.edge === 'left' ? item.rect.x + item.rect.w - 3 : item.rect.x - 3}px"
          style:top="{item.bar.y}px"
          style:height="{item.rect.h + item.bar.h}px"
          onpointerdown={(e) => item.dock && gripDown(e, node.id, item.dock)}
          onpointermove={gripMove}
          onpointerup={gripUp}
        ></div>
      {/if}
    {/each}
    {#if dragX !== null}
      <div class="dock-hint" class:left={dragX < engine.camera.vw / 2}></div>
    {/if}
  </div>
{/if}

<style>
  .world-area {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .raised {
    position: absolute;
    inset: 0;
    z-index: 8;
    pointer-events: none;
    --z: 1;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    background: rgb(0 0 0 / 0.38);
    pointer-events: auto;
  }
  .raised-node {
    position: absolute;
    pointer-events: auto;
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.35);
    user-select: text;
    -webkit-user-select: text;
  }
  .raised-node.own {
    border-radius: 0 0 10px 10px;
  }
  .panel-bar {
    position: absolute;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 10px;
    font-size: 12px;
    color: var(--text-muted);
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-bottom: none;
    border-radius: 10px 10px 0 0;
    cursor: grab;
    user-select: none;
    pointer-events: auto;
  }
  .panel-bar.docked {
    border-radius: 0;
  }
  .bar-actions {
    margin-left: auto;
    display: flex;
    gap: 2px;
  }
  .bar-actions button {
    all: unset;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    cursor: pointer;
    color: var(--text-muted);
  }
  .bar-actions button:hover {
    background: var(--hover, rgba(127, 127, 127, 0.16));
    color: var(--text);
  }
  .grip {
    position: absolute;
    width: 6px;
    cursor: col-resize;
    pointer-events: auto;
  }
  .grip:hover {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .dock-hint {
    position: absolute;
    top: 0;
    bottom: 0;
    right: 0;
    width: 64px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    pointer-events: none;
  }
  .dock-hint.left {
    left: var(--sidebar-w, 0px);
    right: auto;
  }
  .world {
    position: absolute;
    left: 0;
    top: 0;
    width: 0;
    height: 0;
    transform-origin: 0 0;
  }
  .world.moving {
    will-change: transform;
  }
  .node {
    position: absolute;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    border-radius: 10px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.18);
    overflow: hidden;
  }
  .node.bare {
    background: transparent;
    border-color: transparent;
    box-shadow: none;
    overflow: visible;
  }
  .node.own.selected,
  .node.own.focused {
    border-color: transparent;
    box-shadow: none;
  }
  .world[data-selection-style="dashed"] .node.selected {
    outline: 2px dashed var(--accent);
    outline-offset: 3px;
  }
  .world[data-selection-style="corners"] .node.selected::after,
  .world[data-selection-style="corner_dots"] .node.selected::after {
    content: "";
    position: absolute;
    inset: -6px;
    pointer-events: none;
    z-index: 5;
  }
  .world[data-selection-style="corners"] .node.selected::after {
    --c: var(--accent);
    background:
      linear-gradient(var(--c), var(--c)) top left / 14px 2px,
      linear-gradient(var(--c), var(--c)) top left / 2px 14px,
      linear-gradient(var(--c), var(--c)) top right / 14px 2px,
      linear-gradient(var(--c), var(--c)) top right / 2px 14px,
      linear-gradient(var(--c), var(--c)) bottom left / 14px 2px,
      linear-gradient(var(--c), var(--c)) bottom left / 2px 14px,
      linear-gradient(var(--c), var(--c)) bottom right / 14px 2px,
      linear-gradient(var(--c), var(--c)) bottom right / 2px 14px;
    background-repeat: no-repeat;
  }
  .world[data-selection-style="corner_dots"] .node.selected::after {
    --d: radial-gradient(circle, var(--accent) 3.5px, transparent 4px);
    background:
      var(--d) top left / 10px 10px,
      var(--d) top right / 10px 10px,
      var(--d) bottom left / 10px 10px,
      var(--d) bottom right / 10px 10px;
    background-repeat: no-repeat;
  }
  .world[data-selection-style="elevation"] .node.selected {
    transform: translateY(-3px);
    box-shadow:
      0 18px 44px rgba(0, 0, 0, 0.35),
      0 4px 12px rgba(0, 0, 0, 0.2);
  }
  .world[data-selection-style="elevation"] .node.selected:not(.own) {
    border-color: var(--border);
  }
  .world:not([data-selection-style="solid"]) .node.selected:not(.focused) {
    border-color: var(--border);
  }
  .world:not([data-selection-style="solid"]) .node.own.selected:not(.focused) {
    border-color: transparent;
  }
  .world[data-material="glass"] .node:not(.bare) {
    background: color-mix(in srgb, var(--popup-bg, var(--surface)) 62%, transparent);
    backdrop-filter: blur(22px) saturate(1.6);
    -webkit-backdrop-filter: blur(22px) saturate(1.6);
  }
  .node.hidden {
    display: none;
  }
  .node.selected {
    border-color: var(--accent);
  }
  .node.focused {
    border-color: var(--accent);
    box-shadow:
      0 0 0 2px color-mix(in srgb, var(--accent) 45%, transparent),
      0 8px 28px rgba(0, 0, 0, 0.25);
  }
  .header {
    flex: none;
    height: calc(28px / var(--z, 1));
    min-height: 16px;
    max-height: 56px;
    display: flex;
    align-items: center;
    gap: calc(6px / var(--z, 1));
    padding: 0 calc(10px / var(--z, 1));
    font-size: calc(12px / var(--z, 1));
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    cursor: grab;
    user-select: none;
    white-space: nowrap;
  }
  .dot {
    width: calc(8px / var(--z, 1));
    height: calc(8px / var(--z, 1));
    border-radius: 50%;
    flex: none;
  }
  .icon {
    display: inline-flex;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
    font-weight: 600;
  }
  .lock {
    margin-left: auto;
    font-size: 0.85em;
  }
  .body {
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .pending {
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--text-muted);
    font-size: 12px;
  }
</style>
