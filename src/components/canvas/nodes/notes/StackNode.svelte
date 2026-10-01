<script lang="ts">
  import { tick } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { newId, type CanvasNode, type Op } from "$lib/canvas/model/types";
  import { NOTE_COLORS, noteColorCss, noteRead } from "$lib/canvas/notes/api";
  import {
    bringToFrontOps,
    dataWith,
    ensureStackController,
    frontPage,
    removeFromStackOps,
    reorderOps,
    stackData,
    stackPages,
  } from "$lib/canvas/notes/stack";
  import NoteNodeFull from "./NoteNodeFull.svelte";

  let {
    node,
    selected = false,
    focused = false,
    onpatch,
    onexit,
    onfocusrequest,
  }: {
    node: CanvasNode;
    selected?: boolean;
    focused?: boolean;
    onpatch?: (patch: Partial<CanvasNode>) => void;
    onexit?: () => void;
    onfocusrequest?: () => void;
  } = $props();

  const engine = useEngine();
  ensureStackController(engine);

  let fanned = $state(false);
  let naming = $state(false);
  let nameText = $state("");
  let colorsOpen = $state(false);
  let previews = $state<Record<string, string>>({});
  let strip = $state<HTMLDivElement | null>(null);
  let tabsEl = $state<HTMLDivElement | null>(null);
  let drag = $state<{ id: string; x0: number; y0: number; moved: boolean; overIndex: number } | null>(null);
  let nameInput = $state<HTMLInputElement | null>(null);

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) });

  const data = $derived(stackData(node));
  const pages = $derived.by(() => {
    void engine.store.version;
    return stackPages(engine.store, node);
  });
  const front = $derived.by(() => {
    void engine.store.version;
    return frontPage(engine.store, node);
  });
  const uniform = $derived(noteColorCss(data.uniformColor));

  function commit(ops: Op[]) {
    if (ops.length) engine.commit(ops);
  }

  function pageColor(p: CanvasNode): string | undefined {
    return uniform ?? noteColorCss(p.data.customColor as string | undefined);
  }

  function select(p: CanvasNode) {
    commit(bringToFrontOps(node, p.id));
    fanned = false;
  }

  function addPage() {
    const page: CanvasNode = {
      id: newId(),
      kind: "note",
      x: node.x,
      y: node.y,
      w: node.w,
      h: Math.max(120, node.h - 36),
      z: node.z,
      title: tr("canvas.notes.untitled", "Untitled"),
      data: { text: "" },
    };
    (page as CanvasNode & { stackId?: string }).stackId = node.id;
    const tabs = [page.id, ...data.tabs];
    engine.commit([
      { op: "add_node", node: page },
      { op: "update_node", id: node.id, patch: { data: dataWith(node, { tabs, frontNoteId: page.id }) } },
    ]);
    fanned = false;
    onfocusrequest?.();
  }

  async function toggleFan() {
    fanned = !fanned;
    if (!fanned) return;
    const out: Record<string, string> = {};
    await Promise.all(
      pages.map(async (p) => {
        try {
          const c = await noteRead(engine.store.workspaceId, engine.store.floorId, p.id);
          out[p.id] = c.preview || c.text.slice(0, 240);
        } catch {
          out[p.id] = String(p.data.text ?? "").slice(0, 240);
        }
      }),
    );
    previews = out;
  }

  async function startName() {
    naming = true;
    nameText = data.name ?? node.title ?? "";
    await tick();
    nameInput?.select();
  }

  function commitName() {
    naming = false;
    const name = nameText.trim();
    if (name) onpatch?.({ title: name, data: dataWith(node, { name, hasCustomName: true }) });
    else onpatch?.({ data: dataWith(node, { name: null, hasCustomName: null }) });
  }

  function nameKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitName();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      naming = false;
    }
  }

  function setUniform(id: string | null) {
    colorsOpen = false;
    onpatch?.({ data: dataWith(node, { uniformColor: id }) });
  }

  function pageDown(e: PointerEvent, p: CanvasNode) {
    if (e.button !== 0) return;
    e.stopPropagation();
    drag = { id: p.id, x0: e.clientX, y0: e.clientY, moved: false, overIndex: data.tabs.indexOf(p.id) };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function pageMove(e: PointerEvent) {
    if (!drag) return;
    if (!drag.moved && Math.hypot(e.clientX - drag.x0, e.clientY - drag.y0) < 5) return;
    drag.moved = true;
    if (!tabsEl || fanned) return;
    const tabs = Array.from(tabsEl.querySelectorAll<HTMLElement>("[data-tab]"));
    let idx = tabs.length;
    for (let i = 0; i < tabs.length; i++) {
      const r = tabs[i].getBoundingClientRect();
      if (e.clientY < r.top + r.height / 2) {
        idx = i;
        break;
      }
    }
    drag.overIndex = idx;
  }

  function pageUp(e: PointerEvent, p: CanvasNode) {
    const d = drag;
    drag = null;
    if (!d) return;
    if (!d.moved) {
      select(p);
      return;
    }
    const l = engine.tools.local(e);
    const w = engine.camera.screenToWorld(l.x, l.y);
    const inside = w.x >= node.x && w.x <= node.x + node.w && w.y >= node.y && w.y <= node.y + node.h;
    if (!inside) {
      const ops = removeFromStackOps(engine.store, node, p.id, { x: w.x - 40, y: w.y - 14 });
      engine.commit(ops, { selectionAfter: [p.id] });
      return;
    }
    const from = data.tabs.indexOf(p.id);
    const to = d.overIndex > from ? d.overIndex - 1 : d.overIndex;
    commit(reorderOps(node, p.id, to));
  }

  function takeOut(p: CanvasNode) {
    engine.commit(removeFromStackOps(engine.store, node, p.id), { selectionAfter: [p.id] });
  }
</script>

<div class="stack" class:focused class:selected>
  <div class="strip" data-canvas-chrome bind:this={strip}>
    {#if naming}
      <input class="name-input" bind:this={nameInput} bind:value={nameText} onkeydown={nameKey} onblur={commitName} placeholder={tr("canvas.notes.stackName", "Fichário name")} />
    {:else}
      <button type="button" class="name" title={tr("canvas.notes.renameStack", "Rename fichário")} ondblclick={() => void startName()} onclick={() => void startName()}>
        {data.hasCustomName ? (data.name ?? node.title) : "📁"}
      </button>
    {/if}
    <span class="grow"></span>
    <button type="button" class="tool" title={tr("canvas.notes.addPage", "New page")} onclick={addPage}>+</button>
    <button type="button" class="tool" class:on={fanned} title={tr("canvas.notes.fanOut", "Show all pages")} aria-pressed={fanned} onclick={() => void toggleFan()}>▦</button>
    <div class="colors-wrap">
      <button type="button" class="tool" title={tr("canvas.notes.uniformColor", "Uniform color")} onclick={() => (colorsOpen = !colorsOpen)}>
        <span class="dot" style:background={uniform ?? "var(--border)"}></span>
      </button>
      {#if colorsOpen}
        <div class="colors">
          <button type="button" class="swatch none" aria-label={tr("canvas.notes.noColor", "No color")} onclick={() => setUniform(null)}></button>
          {#each NOTE_COLORS as c}
            <button type="button" class="swatch" class:on={data.uniformColor === c.id} style:background={c.css} aria-label={c.id} onclick={() => setUniform(c.id)}></button>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <div class="body">
  <div class="page">
    {#if fanned}
      <div class="fan" data-canvas-chrome>
        {#each pages as p, i (p.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="card"
            class:on={front?.id === p.id}
            style:--tab={pageColor(p) ?? "var(--border)"}
            style:--i={i}
            onpointerdown={(e) => pageDown(e, p)}
            onpointermove={pageMove}
            onpointerup={(e) => pageUp(e, p)}
          >
            <strong>{p.title || tr("canvas.notes.untitled", "Untitled")}</strong>
            <span>{previews[p.id] ?? ""}</span>
          </div>
        {/each}
        {#if !pages.length}
          <p class="empty">{tr("canvas.notes.stackEmpty", "Drop notes here")}</p>
        {/if}
      </div>
    {:else if front}
      {#key front.id}
        <NoteNodeFull
          node={front}
          embedded
          {selected}
          {focused}
          onpatch={(p: Partial<CanvasNode>) => engine.updateNode(front.id, p)}
          {onexit}
          {onfocusrequest}
        />
      {/key}
    {:else}
      <p class="empty">{tr("canvas.notes.stackEmpty", "Drop notes here")}</p>
    {/if}
  </div>
      <div class="tabs" role="tablist" aria-orientation="vertical" data-canvas-chrome bind:this={tabsEl}>
        {#each pages as p, i (p.id)}
          {#if drag?.moved && !fanned && drag.overIndex === i && drag.id !== p.id}<span class="drop"></span>{/if}
          <button
            type="button"
            class="tab"
            class:on={front?.id === p.id}
            class:dragging={drag?.id === p.id && drag.moved}
            data-tab
            role="tab"
            aria-selected={front?.id === p.id}
            title={p.title ?? ""}
            style:--tab={pageColor(p) ?? "var(--border)"}
            onpointerdown={(e) => pageDown(e, p)}
            onpointermove={pageMove}
            onpointerup={(e) => pageUp(e, p)}
            oncontextmenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              takeOut(p);
            }}
          >
            {p.title || tr("canvas.notes.untitled", "Untitled")}
          </button>
        {/each}
      </div>
  </div>
</div>

<style>
  .stack {
    height: 100%;
    display: flex;
    flex-direction: column;
    color: var(--text);
  }
  .strip {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px 0;
    border-bottom: 1px solid var(--border);
    user-select: none;
    min-width: 0;
  }
  .name {
    flex: none;
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: text;
    padding: 2px 4px;
  }
  .name-input {
    width: 100px;
    font: inherit;
    font-size: 11px;
    padding: 1px 4px;
    border: 1px solid var(--accent);
    border-radius: 5px;
    background: var(--input-bg, transparent);
    color: var(--text);
    outline: none;
  }
  .grow {
    flex: 1;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .tabs {
    flex: none;
    width: 88px;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    padding-top: 6px;
    overflow-y: auto;
    scrollbar-width: none;
    border-left: 1px solid var(--border);
    user-select: none;
  }
  .tab {
    flex: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-left: none;
    border-right: 3px solid var(--tab);
    border-radius: 0 6px 6px 0;
    background: color-mix(in srgb, var(--text) 4%, transparent);
    color: var(--text-muted);
    font: inherit;
    font-size: 11px;
    cursor: grab;
  }
  .tab.on {
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    font-weight: 600;
  }
  .tab.dragging {
    opacity: 0.4;
  }
  .drop {
    flex: none;
    height: 2px;
    align-self: stretch;
    background: var(--accent);
  }
  .tool {
    flex: none;
    min-width: 22px;
    height: 20px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    display: grid;
    place-items: center;
  }
  .tool:hover,
  .tool.on {
    border-color: var(--border);
    color: var(--text);
  }
  .colors-wrap {
    position: relative;
  }
  .dot {
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 1px solid var(--border);
  }
  .colors {
    position: absolute;
    right: 0;
    top: 24px;
    z-index: 5;
    display: grid;
    grid-template-columns: repeat(5, 18px);
    gap: 5px;
    padding: 7px;
    border-radius: 9px;
    border: 1px solid var(--border);
    background: var(--popup-bg, var(--surface));
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
  }
  .swatch {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
  }
  .swatch.on {
    border-color: var(--text);
  }
  .swatch.none {
    background: linear-gradient(135deg, transparent 45%, var(--danger, #d96a62) 45% 55%, transparent 55%);
    border-color: var(--border);
  }
  .page {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .fan {
    height: 100%;
    overflow: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(110px, 1fr));
    gap: 8px;
    padding: 10px;
    box-sizing: border-box;
    align-content: start;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    height: 96px;
    padding: 6px 8px;
    border-radius: 8px;
    border: 1px solid var(--border);
    border-top: 4px solid var(--tab);
    background: var(--popup-bg, var(--surface));
    box-shadow: 0 3px 10px rgba(0, 0, 0, 0.15);
    font-size: 11px;
    overflow: hidden;
    cursor: grab;
    transform: rotate(calc((var(--i) - 1) * 0.8deg));
    transition: transform 120ms ease;
  }
  .card:hover {
    transform: translateY(-2px);
  }
  .card.on {
    border-color: var(--accent);
  }
  .card strong {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card span {
    color: var(--text-muted);
    white-space: pre-wrap;
    overflow: hidden;
  }
  .empty {
    margin: 0;
    padding: 16px;
    color: var(--text-muted);
    font-size: 12px;
    text-align: center;
  }
  @media (prefers-reduced-motion: reduce) {
    .card {
      transform: none;
      transition: none;
    }
  }
</style>
