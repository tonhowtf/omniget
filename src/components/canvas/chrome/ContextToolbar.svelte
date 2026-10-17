<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { AlignMode } from "$lib/canvas/interaction/snap";
  import type { Drawing, FillStyle, Op } from "$lib/canvas/model/types";
  import { TOOL_ICONS } from "$lib/canvas/icons";
  import Icon from "./Icon.svelte";
  import { isTerminalKind, terminalIdOf } from "$lib/canvas/model/types";
  import { NOTE_COLORS } from "$lib/canvas/notes/api";
  import { chats } from "$lib/canvas/chat/store.svelte";
  import { actDevice } from "$lib/canvas/devices/client";
  import { termClose, errorText } from "$lib/canvas/term/client";
  import { termHub } from "$lib/canvas/term/hub";
  import { openTerminal } from "$lib/canvas/spawn";
  import { showToast } from "$lib/stores/toast-store.svelte";

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

  const cordEnds = $derived.by(() => {
    const c = sel.connections.length === 1 && sel.ids.length === 1 ? sel.connections[0] : null;
    if (!c) return [];
    return [c.from, c.to].flatMap((id) => {
      const n = engine.store.node(id);
      if (!n) return [];
      return [{ id, label: $t("canvas.engine.cord.goTo", { default: "Go to {{name}}", name: n.title || n.kind }) as string }];
    });
  });

  function goTo(id: string) {
    engine.selection.set([id]);
    engine.reveal(id);
  }

  function cutCord(id: string) {
    engine.commit([{ op: "remove_connection", id }], { selectionAfter: [] });
  }

  function setKind(kind: string) {
    const c = sel.connections[0];
    if (!c) return;
    engine.commit([{ op: "update_connection", id: c.id, patch: { kind } }]);
  }

  const one = $derived(sel.nodes.length === 1 && sel.ids.length === 1 ? sel.nodes[0] : null);
  const oneKind = $derived.by(() => {
    if (!one) return "";
    if (isTerminalKind(one.kind)) return "terminal";
    if (one.kind === "portal") {
      const pk = String(one.data.portalKind ?? "web");
      return pk === "ios" || pk === "android" ? "device" : "portal";
    }
    return one.kind;
  });
  const ICON_CHAT = "M4 5h16v11H9l-5 4z";
  const ICON_LIFT = "M4 9V4h5 M15 4h5v5 M20 15v5h-5 M9 20H4v-5";
  const ICON_RESTART = "M20 11a8 8 0 1 0-2.3 5.7 M20 4v7h-7";
  const ICON_HOME = "M4 11l8-7 8 7 M6 10v10h12V10";
  const ICON_LOCK_SCREEN = "M12 3v9 M6.3 6.3a8 8 0 1 0 11.4 0";

  function noteColor(id: string | null) {
    if (!one) return;
    engine.commit([{ op: "update_node", id: one.id, patch: { data: { ...one.data, customColor: id ?? undefined } } as never }]);
  }

  async function restartTerminal() {
    const n = one;
    if (!n) return;
    const id = terminalIdOf(n);
    try {
      await termClose(id, false).catch(() => {});
      termHub.forget(id);
      await openTerminal(n, engine.store.workspaceId, { floorId: engine.store.floorId, restore: false });
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  function device(action: string, args: Record<string, unknown> = {}) {
    if (!one) return;
    actDevice(one.id, action, args).catch((e) => showToast("error", String(e)));
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

{#if bounds && !engine.selection.focused && !engine.tools.busy}
  <div
    class="ctx"
    data-canvas-chrome
    role="toolbar"
    tabindex="-1"
    aria-label={$t("canvas.engine.selectionTools", { default: "Selection tools" })}
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

      {#if oneKind === "note"}
        {#each NOTE_COLORS as c (c.id)}
          <button type="button" class="swatch" class:on={one?.data.customColor === c.id} style:background={c.css} aria-label={c.id} title={c.id} onclick={() => noteColor(c.id)}></button>
        {/each}
        <button type="button" class="swatch none" aria-label={$t("canvas.notes.noColor", { default: "No color" })} title={$t("canvas.notes.noColor", { default: "No color" })} onclick={() => noteColor(null)}></button>
        <span class="sep"></span>
      {:else if oneKind === "terminal" && one}
        <button type="button" title={$t("canvas.chat.open", { default: "Chat (⌘J)" })} aria-label={$t("canvas.chat.open", { default: "Chat (⌘J)" })} onclick={() => one && chats.toggle(one.id)}>
          <Icon d={ICON_CHAT} />
        </button>
        <button type="button" title={$t("canvas.panels.lift", { default: "Lift" })} aria-label={$t("canvas.panels.lift", { default: "Lift" })} onclick={() => one && engine.panels.lift(one.id)}>
          <Icon d={ICON_LIFT} />
        </button>
        <button type="button" title={$t("canvas.term.action.restart")} aria-label={$t("canvas.term.action.restart")} onclick={() => void restartTerminal()}>
          <Icon d={ICON_RESTART} />
        </button>
        <span class="sep"></span>
      {:else if oneKind === "device"}
        <button type="button" title={$t("canvas.portal.deviceNode.home", { default: "Home" })} aria-label={$t("canvas.portal.deviceNode.home", { default: "Home" })} onclick={() => device("home")}>
          <Icon d={ICON_HOME} />
        </button>
        <button type="button" title={$t("canvas.engine.deviceLock", { default: "Lock screen" })} aria-label={$t("canvas.engine.deviceLock", { default: "Lock screen" })} onclick={() => device("key", { key: "lock" })}>
          <Icon d={ICON_LOCK_SCREEN} />
        </button>
        <button type="button" title={$t("canvas.panels.lift", { default: "Lift" })} aria-label={$t("canvas.panels.lift", { default: "Lift" })} onclick={() => one && engine.panels.lift(one.id)}>
          <Icon d={ICON_LIFT} />
        </button>
        <span class="sep"></span>
      {/if}

      {#if sel.nodes.length === 1}
        <button type="button" title={$t("canvas.engine.tool.connect", { default: "Connect" })} aria-label={$t("canvas.engine.tool.connect", { default: "Connect" })} onclick={() => engine.setTool("connect")}>
          <Icon d={TOOL_ICONS.connect} />
        </button>
      {/if}

      {#if sel.connections.length === 1 && sel.ids.length === 1}
        {#each ["ask", "message", "handoff", "spawn"] as k (k)}
          <button type="button" class="chip" class:on={sel.connections[0].kind === k} onclick={() => setKind(k)}>
            {$t(`canvas.engine.cord.${k}`, { default: k })}
          </button>
        {/each}
        <span class="sep"></span>
        {#each cordEnds as end (end.id)}
          <button type="button" class="chip end" title={end.label} onclick={() => goTo(end.id)}>{end.label}</button>
        {/each}
        <button type="button" class="chip danger" onclick={() => cutCord(sel.connections[0].id)}>
          {$t("canvas.engine.cord.cut", { default: "Cut connection" })}
        </button>
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
    left: calc(var(--sidebar-w, 0px) + var(--dock-l, 0px) + (100% - var(--sidebar-w, 0px) - var(--dock-l, 0px) - var(--dock-r, 0px)) / 2);
    top: 60px;
    z-index: 11;
    pointer-events: none;
  }
  .inner {
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 3px 6px;
    border-radius: 999px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.06),
      0 6px 18px rgba(0, 0, 0, 0.12);
    pointer-events: auto;
    white-space: nowrap;
  }
  button {
    all: unset;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 30px;
    height: 30px;
    border-radius: 999px;
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
  .chip.end {
    display: inline-block;
    line-height: 30px;
    text-transform: none;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip.danger {
    text-transform: none;
  }
  .swatch.on {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .swatch.none {
    background: linear-gradient(135deg, transparent 45%, var(--danger, #e5484d) 47%, var(--danger, #e5484d) 53%, transparent 55%);
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
