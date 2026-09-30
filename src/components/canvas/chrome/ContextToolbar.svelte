<script lang="ts">
  // Floating toolbar above the selection: rename, arrange, group, z-order,
  // lock, duplicate, delete; drawing style when drawings are selected.
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { AlignMode } from "$lib/canvas/interaction/snap";
  import type { Drawing, FillStyle, Op } from "$lib/canvas/model/types";
  import { TOOL_ICONS } from "$lib/canvas/icons";
  import { atWorld } from "$lib/canvas/screen";
  import Icon from "./Icon.svelte";

  const engine = useEngine();
  const COLORS = ["#e8e6e3", "#1d1d1f", "#e5484d", "#f5a524", "#46a758", "#3e8ef7", "#8e4ec6"];

  const sel = $derived.by(() => {
    void engine.store.version;
    const ids = engine.selection.list();
    const nodes = ids.map((id) => engine.store.node(id)).filter((n) => !!n);
    const drawings = ids.map((id) => engine.store.drawing(id)).filter((d): d is Drawing => !!d);
    const connections = ids.map((id) => engine.store.connection(id)).filter((c) => !!c);
    return { ids, nodes, drawings, connections };
  });
  const bounds = $derived.by(() => {
    void engine.store.version;
    return sel.ids.length ? engine.selectionBounds() : null;
  });
  const grouped = $derived(sel.nodes.some((n) => n.groupId) || sel.drawings.some((d) => d.groupId));
  const ungroupedMembers = $derived(sel.nodes.some((n) => !n.groupId) || sel.drawings.some((d) => !d.groupId));
  const canUngroup = $derived(grouped && !ungroupedMembers);
  const allLocked = $derived(sel.nodes.length > 0 && sel.nodes.every((n) => n.locked));
  let renaming = $state(false);
  let nameDraft = $state("");

  function startRename() {
    const n = sel.nodes[0];
    if (!n) return;
    nameDraft = n.title ?? "";
    renaming = true;
  }

  function commitRename() {
    const n = sel.nodes[0];
    renaming = false;
    const name = nameDraft.trim();
    if (!n || !name || name === n.title) return;
    engine.updateNode(n.id, { title: engine.uniqueTitle(name) });
  }

  function styleDrawings(patch: Partial<Drawing>) {
    const ops: Op[] = [];
    for (const d of sel.drawings) ops.push({ op: "update_drawing", id: d.id, patch });
    engine.commit(ops);
  }

  function setColor(c: string) {
    engine.penColor = c;
    if (sel.drawings.length) {
      styleDrawings({ color: c, ...(sel.drawings.some((d) => d.fill) ? { fill: c } : {}) });
    }
  }

  function setFill(f: FillStyle) {
    engine.fillStyle = f;
    if (sel.drawings.length) styleDrawings({ fillStyle: f === "none" ? undefined : f, fill: f === "none" ? undefined : engine.penColor });
  }

  // Text block style (M-B29): size 8 to 200, weight, family, color.
  const texts = $derived(sel.nodes.length > 0 && sel.nodes.every((n) => n.kind === "text") ? sel.nodes : []);
  const TEXT_SIZES = [8, 10, 12, 14, 16, 20, 24, 32, 40, 48, 64, 80, 96, 128, 160, 200];
  const WEIGHT_CYCLE = ["regular", "medium", "bold"];
  const FAMILY_CYCLE = ["sans", "serif", "mono"];

  function styleTexts(data: Record<string, unknown>) {
    engine.commit(texts.map((n) => ({ op: "update_node" as const, id: n.id, patch: { data } as never })));
  }

  function stepSize(dir: 1 | -1) {
    const cur = Number(texts[0]?.data.fontSize ?? 20) || 20;
    const next = dir > 0 ? (TEXT_SIZES.find((v) => v > cur) ?? 200) : ([...TEXT_SIZES].reverse().find((v) => v < cur) ?? 8);
    styleTexts({ fontSize: next });
  }

  function cycle(key: "fontWeight" | "fontFamily", list: string[]) {
    const cur = String(texts[0]?.data[key] ?? list[0]);
    styleTexts({ [key]: list[(list.indexOf(cur) + 1) % list.length] });
  }

  function setKind(kind: string) {
    const c = sel.connections[0];
    if (!c) return;
    engine.commit([{ op: "update_connection", id: c.id, patch: { kind } }]);
  }

  const aligns: Array<[AlignMode, keyof typeof TOOL_ICONS, string, string]> = [
    ["left", "alignLeft", "canvas.engine.alignLeft", "Align left"],
    ["hcenter", "alignHCenter", "canvas.engine.alignCenter", "Align centre"],
    ["right", "alignRight", "canvas.engine.alignRight", "Align right"],
    ["top", "alignTop", "canvas.engine.alignTop", "Align top"],
    ["vcenter", "alignVCenter", "canvas.engine.alignMiddle", "Align middle"],
    ["bottom", "alignBottom", "canvas.engine.alignBottom", "Align bottom"],
    ["hdistribute", "distH", "canvas.engine.distributeH", "Distribute horizontally"],
    ["vdistribute", "distV", "canvas.engine.distributeV", "Distribute vertically"],
  ];
</script>

{#if bounds && !engine.moving && !engine.selection.focused && !engine.tools.busy}
  <div
    class="ctx"
    data-canvas-chrome
    role="toolbar"
    tabindex="-1"
    aria-label={$t("canvas.engine.selectionTools", { default: "Selection tools" })}
    style:transform={atWorld(bounds.x + bounds.w / 2, bounds.y, 0, -46)}
  >
    <div class="inner">
      {#if sel.nodes.length === 1}
        {#if renaming}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="name"
            bind:value={nameDraft}
            autofocus
            onblur={commitRename}
            onkeydown={(e) => {
              if (e.key === "Enter") commitRename();
              if (e.key === "Escape") renaming = false;
              e.stopPropagation();
            }}
          />
        {:else}
          <button type="button" class="label" onclick={startRename} title={$t("canvas.engine.rename", { default: "Rename" })}>
            {sel.nodes[0].title || sel.nodes[0].kind}
          </button>
        {/if}
        <span class="sep"></span>
      {/if}

      {#if sel.connections.length === 1 && sel.ids.length === 1}
        {#each ["ask", "message", "handoff", "spawn"] as k (k)}
          <button type="button" class="chip" class:on={sel.connections[0].kind === k} onclick={() => setKind(k)}>
            {$t(`canvas.engine.cord.${k}`, { default: k })}
          </button>
        {/each}
        <span class="sep"></span>
      {/if}

      {#if texts.length}
        <button type="button" title={$t("canvas.engine.textSmaller", { default: "Smaller text" })} onclick={() => stepSize(-1)}>A−</button>
        <span class="chip">{Number(texts[0].data.fontSize ?? 20)}</span>
        <button type="button" title={$t("canvas.engine.textBigger", { default: "Bigger text" })} onclick={() => stepSize(1)}>A+</button>
        <button type="button" class="chip" title={$t("canvas.engine.textWeight", { default: "Weight" })} onclick={() => cycle("fontWeight", WEIGHT_CYCLE)}>
          {String(texts[0].data.fontWeight ?? "regular")}
        </button>
        <button type="button" class="chip" title={$t("canvas.engine.textFamily", { default: "Font" })} onclick={() => cycle("fontFamily", FAMILY_CYCLE)}>
          {String(texts[0].data.fontFamily ?? "sans")}
        </button>
        {#each COLORS as c (c)}
          <button type="button" class="swatch" style:background={c} aria-label={c} onclick={() => styleTexts({ color: c.toUpperCase() })}></button>
        {/each}
        <button type="button" class="chip" title={$t("canvas.engine.textThemeColor", { default: "Theme color" })} onclick={() => styleTexts({ color: null })}>—</button>
        <span class="sep"></span>
      {/if}

      {#if sel.nodes.length > 1}
        {#each aligns as [mode, icon, key, fallback] (mode)}
          <button type="button" title={$t(key, { default: fallback })} onclick={() => engine.align(mode)}>
            <Icon d={TOOL_ICONS[icon]} />
          </button>
        {/each}
        <span class="sep"></span>
      {/if}

      {#if sel.nodes.length === 2}
        <button
          type="button"
          title={$t("canvas.engine.connect", { default: "Connect" })}
          onclick={() => engine.connect(sel.nodes[0].id, sel.nodes[1].id)}
        >
          <Icon d={TOOL_ICONS.connect} />
        </button>
      {/if}

      {#if sel.drawings.length}
        {#each COLORS as c (c)}
          <button type="button" class="swatch" style:background={c} title={c} aria-label={c} onclick={() => setColor(c)}></button>
        {/each}
        <span class="sep"></span>
        {#each ["none", "solid", "hachure", "cross"] as f (f)}
          <button type="button" class="chip" class:on={(sel.drawings[0].fillStyle ?? "none") === f} onclick={() => setFill(f as FillStyle)}>
            {$t(`canvas.engine.fill.${f}`, { default: f })}
          </button>
        {/each}
        <button
          type="button"
          class="chip"
          class:on={!!sel.drawings[0].sketch}
          onclick={() => styleDrawings({ sketch: sel.drawings[0].sketch ? undefined : true })}
        >
          {$t("canvas.engine.sketch", { default: "Sketch" })}
        </button>
        <span class="sep"></span>
      {/if}

      {#if sel.nodes.length + sel.drawings.length >= 2 || grouped}
        <button
          type="button"
          title={canUngroup ? $t("canvas.engine.ungroup", { default: "Ungroup" }) : $t("canvas.engine.group", { default: "Group" })}
          onclick={() => (canUngroup ? engine.ungroup() : engine.group())}
        >
          <Icon d={canUngroup ? TOOL_ICONS.ungroup : TOOL_ICONS.group} />
        </button>
      {/if}
      {#if sel.nodes.length}
        <button type="button" title={$t("canvas.engine.front", { default: "Bring to front" })} onclick={() => engine.zOrder("front")}>
          <Icon d={TOOL_ICONS.front} />
        </button>
        <button type="button" title={$t("canvas.engine.back", { default: "Send to back" })} onclick={() => engine.zOrder("back")}>
          <Icon d={TOOL_ICONS.back} />
        </button>
        <button
          type="button"
          class:on={allLocked}
          title={allLocked ? $t("canvas.engine.unlock", { default: "Unlock" }) : $t("canvas.engine.lock", { default: "Lock" })}
          onclick={() => engine.toggleLock()}
        >
          <Icon d={TOOL_ICONS.lock} />
        </button>
      {/if}
      {#if sel.nodes.length || sel.drawings.length}
        <button type="button" title={$t("canvas.engine.duplicate", { default: "Duplicate" })} onclick={() => engine.duplicateSelection()}>
          <Icon d={TOOL_ICONS.copy} />
        </button>
        <button type="button" title={$t("canvas.engine.zoomTo", { default: "Zoom to selection" })} onclick={() => engine.zoomToggle()}>
          <Icon d={TOOL_ICONS.zoomTo} />
        </button>
      {/if}
      <button type="button" class="danger" title={$t("canvas.engine.delete", { default: "Delete" })} onclick={() => engine.deleteSelection()}>
        <Icon d={TOOL_ICONS.trash} />
      </button>
    </div>
  </div>
{/if}

<style>
  .ctx {
    position: absolute;
    left: 0;
    top: 0;
    pointer-events: none;
  }
  .inner {
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: 10px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.22);
    pointer-events: auto;
    white-space: nowrap;
  }
  button {
    all: unset;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    height: 28px;
    border-radius: 6px;
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
  }
  button:hover,
  button.on {
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  button.danger:hover {
    color: var(--danger, #e5484d);
  }
  button:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
  }
  .label {
    padding: 0 8px;
    font-weight: 600;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name {
    height: 26px;
    width: 160px;
    border-radius: 6px;
    border: 1px solid var(--accent);
    background: var(--input-bg, transparent);
    color: var(--text);
    padding: 0 6px;
    font: inherit;
    font-size: 12px;
  }
  .chip {
    padding: 0 8px;
    text-transform: capitalize;
  }
  .swatch {
    min-width: 18px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 1px solid var(--border);
    margin: 0 2px;
  }
  .sep {
    width: 1px;
    height: 18px;
    background: var(--border);
    margin: 0 4px;
  }
</style>
