<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { DRAW_TOOLS, SHAPE_TOOLS, type ToolName } from "$lib/canvas/interaction/tools";
  import { TOOL_ICONS } from "$lib/canvas/icons";
  import Icon from "./Icon.svelte";

  const engine = useEngine();
  const COLORS = ["#e8e6e3", "#1d1d1f", "#e5484d", "#f5a524", "#46a758", "#3e8ef7", "#8e4ec6"];

  const PANEL_ICONS = {
    book: "M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2V5z M6 19h13 M8 7h8",
    clock: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M12 7v5l3 2",
    roles: "M12 4a4 4 0 1 0 0 8a4 4 0 1 0 0-8z M4 21a8 8 0 0 1 16 0",
    omni: "M4 5h16v11H9l-5 4V5z M8 10h.01 M12 10h.01 M16 10h.01",
    share: "M12 3v12 M7 8l5-5 5 5 M5 14v6h14v-6",
    help: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M9.5 9a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6V14 M12 17.5v.01",
    more: "M6 12h.01 M12 12h.01 M18 12h.01",
  };
  type PanelKind = "partituras" | "routine" | "roles" | "companion" | "visit";

  interface Item {
    id: string;
    group: number;
    prio: number;
    icon: string;
    key: string;
    fallback: string;
    shortcut?: string;
    tool?: ToolName;
    panel?: PanelKind;
    run?: () => void;
  }

  const mod = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl+";

  const tool = (id: ToolName, prio: number, fallback: string, shortcut = ""): Item => ({
    id,
    group: 1,
    prio,
    icon: TOOL_ICONS[id as keyof typeof TOOL_ICONS],
    key: `canvas.engine.tool.${id}`,
    fallback,
    shortcut,
    tool: id,
  });
  const panel = (id: PanelKind, prio: number, icon: string, key: string, fallback: string, shortcut = ""): Item => ({
    id: `panel-${id}`,
    group: 3,
    prio,
    icon,
    key,
    fallback,
    shortcut,
    panel: id,
  });

  const ITEMS: Item[] = [
    tool("select", 0, "Select", "V"),
    tool("hand", 2, "Hand", "H"),
    tool("pen", 3, "Pen", "D"),
    tool("highlighter", 11, "Highlighter"),
    tool("smart", 12, "Smart draw"),
    tool("eraser", 6, "Eraser", "E"),
    tool("rect", 5, "Rectangle", "B"),
    tool("ellipse", 10, "Ellipse", "O"),
    tool("diamond", 13, "Diamond"),
    tool("arrow", 7, "Arrow", "A"),
    tool("line", 14, "Line"),
    tool("connect", 1, "Connect", "C"),
    { id: "undo", group: 2, prio: 4, icon: TOOL_ICONS.undo, key: "canvas.engine.undo", fallback: "Undo", shortcut: `${mod}Z`, run: () => engine.undo() },
    { id: "redo", group: 2, prio: 8, icon: TOOL_ICONS.redo, key: "canvas.engine.redo", fallback: "Redo", shortcut: `${mod}⇧Z`, run: () => engine.redo() },
    panel("partituras", 9, PANEL_ICONS.book, "canvas.engine.partituras", "Partituras", `${mod}⇧L`),
    panel("routine", 15, PANEL_ICONS.clock, "canvas.engine.routines", "Routines"),
    panel("roles", 16, PANEL_ICONS.roles, "canvas.engine.roles", "Roles"),
    panel("companion", 15, PANEL_ICONS.omni, "canvas.engine.companion", "Omni"),
    panel("visit", 16, PANEL_ICONS.share, "canvas.engine.share", "Share and visit"),
  ];

  const BTN = 32;
  const SEP = 11;
  const PAD = 12;
  const FIXED = BTN + SEP + BTN;

  let avail = $state(0);
  let menuOpen = $state(false);
  let moreBtn = $state<HTMLButtonElement | null>(null);
  let menuPos = $state({ right: 0, top: 0 });

  const helpLines = $derived([
    $t("canvas.engine.help.escape", { default: "Shift+Esc: leave a focused terminal and return to the canvas (Esc goes to the program)" }),
    $t("canvas.engine.help.jump", { default: `${mod}1–9: jump to a terminal` }),
    $t("canvas.engine.help.palette", { default: `${mod}K / ${mod}P: command palette` }),
    $t("canvas.engine.help.chat", { default: `${mod}J: chat face of the focused agent` }),
    $t("canvas.engine.help.partituras", { default: `${mod}⇧L: partituras` }),
    $t("canvas.engine.help.attention", { default: `${mod}⇧A: next agent that needs you` }),
  ]);

  const visible = $derived.by(() => {
    const room = (avail || 10_000) - PAD - FIXED;
    const ranked = [...ITEMS].sort((a, b) => rank(a) - rank(b));
    const out = new Set<string>();
    const groups = new Set<number>();
    let used = 0;
    for (const it of ranked) {
      const cost = BTN + (groups.has(it.group) || it.group === 1 ? 0 : SEP);
      if (used + cost > room) continue;
      used += cost;
      out.add(it.id);
      groups.add(it.group);
    }
    return out;
  });

  function rank(it: Item): number {
    if (it.tool && it.tool === engine.tool) return -2;
    if (it.panel && engine.panel?.kind === it.panel) return -1;
    return it.prio;
  }

  const shown = $derived(ITEMS.filter((it) => visible.has(it.id)));
  const hidden = $derived(ITEMS.filter((it) => !visible.has(it.id)));

  function isOn(it: Item): boolean {
    if (it.tool) return engine.tool === it.tool;
    if (it.panel) return engine.panel?.kind === it.panel;
    return false;
  }

  function activate(it: Item) {
    if (it.tool) engine.setTool(it.tool);
    else if (it.panel) engine.panel = engine.panel?.kind === it.panel ? null : { kind: it.panel };
    else it.run?.();
  }

  function label(it: Item): string {
    return $t(it.key, { default: it.fallback });
  }

  function toggleMenu() {
    if (!menuOpen && moreBtn) {
      const r = moreBtn.getBoundingClientRect();
      menuPos = { right: Math.max(8, window.innerWidth - r.right), top: r.bottom + 6 };
    }
    menuOpen = !menuOpen;
  }

  $effect(() => {
    if (!menuOpen) return;
    const close = (e: PointerEvent) => {
      const el = e.target as HTMLElement | null;
      if (el?.closest(".tb-menu") || el?.closest(".more")) return;
      menuOpen = false;
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") menuOpen = false;
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", key, true);
    };
  });

  const styled = $derived(DRAW_TOOLS.includes(engine.tool) || SHAPE_TOOLS.includes(engine.tool));

  function openSpawn(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const root = (e.currentTarget as HTMLElement).closest(".canvas-root")?.getBoundingClientRect();
    const sx = r.left - (root?.left ?? 0);
    const sy = r.bottom - (root?.top ?? 0) + 6;
    const c = engine.camera.center();
    engine.spawnMenu = engine.spawnMenu ? null : { sx, sy, wx: c.x, wy: c.y, free: true };
  }
</script>

<div class="fit" bind:clientWidth={avail}>
  <div class="bar" data-canvas-chrome role="toolbar" tabindex="-1" aria-label={$t("canvas.engine.tools", { default: "Canvas tools" })}>
    <button type="button" class="add" title={$t("canvas.engine.add", { default: "Add" })} onclick={openSpawn}>
      <Icon d={TOOL_ICONS.plus} />
    </button>
    <span class="sep"></span>
    {#each shown as it, i (it.id)}
      {#if i > 0 && shown[i - 1].group !== it.group}
        <span class="sep"></span>
      {/if}
      <button
        type="button"
        class:on={isOn(it)}
        aria-pressed={it.tool || it.panel ? isOn(it) : undefined}
        title={`${label(it)}${it.shortcut ? ` (${it.shortcut})` : ""}`}
        onclick={() => activate(it)}
      >
        <Icon d={it.icon} />
      </button>
    {/each}
    <button
      type="button"
      class="more"
      class:on={menuOpen}
      bind:this={moreBtn}
      aria-haspopup="menu"
      aria-expanded={menuOpen}
      title={$t("canvas.engine.more", { default: "More tools and shortcuts" })}
      onclick={toggleMenu}
    >
      <Icon d={hidden.length ? PANEL_ICONS.more : PANEL_ICONS.help} />
    </button>
  </div>
  {#if styled}
    <div class="bar styles" data-canvas-chrome>
      {#each COLORS as c (c)}
        <button
          type="button"
          class="swatch"
          class:on={engine.penColor === c}
          style:--sw={c}
          aria-label={c}
          onclick={() => (engine.penColor = c)}
        ></button>
      {/each}
      <input
        class="width"
        type="range"
        min="1"
        max="16"
        step="1"
        bind:value={engine.strokeWidth}
        title={$t("canvas.engine.strokeWidth", { default: "Stroke width" })}
      />
      {#if SHAPE_TOOLS.includes(engine.tool)}
        <button type="button" class="chip" class:on={engine.sketch} onclick={() => (engine.sketch = !engine.sketch)}>
          {$t("canvas.engine.sketch", { default: "Sketch" })}
        </button>
      {/if}
    </div>
  {/if}
</div>

{#if menuOpen}
  <div class="tb-menu" data-canvas-chrome role="menu" style:right="{menuPos.right}px" style:top="{menuPos.top}px">
    {#each hidden as it, i (it.id)}
      {#if i > 0 && hidden[i - 1].group !== it.group}
        <div class="msep"></div>
      {/if}
      <button
        type="button"
        role="menuitem"
        class="mi"
        class:on={isOn(it)}
        onclick={() => {
          activate(it);
          menuOpen = false;
        }}
      >
        <Icon d={it.icon} />
        <span class="ml">{label(it)}</span>
        {#if it.shortcut}<kbd>{it.shortcut}</kbd>{/if}
      </button>
    {/each}
    {#if hidden.length}<div class="msep"></div>{/if}
    <div class="help">
      <div class="help-title">{$t("canvas.engine.shortcuts", { default: "Shortcuts" })}</div>
      {#each helpLines as line (line)}
        <div class="help-line">{line}</div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .fit {
    width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    pointer-events: none;
  }
  .bar {
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: 12px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.22);
    max-width: 100%;
    overflow: hidden;
    flex-wrap: nowrap;
  }
  .styles {
    flex-wrap: wrap;
    justify-content: center;
    padding: 4px 8px;
  }
  button {
    all: unset;
    min-width: 30px;
    height: 30px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 8px;
    color: var(--text);
    cursor: pointer;
    flex: none;
  }
  button:hover {
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  button.on {
    background: var(--accent);
    color: var(--on-accent, #fff);
  }
  button:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
  }
  .add {
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  .more.on {
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
    color: var(--text);
  }
  .swatch {
    min-width: 18px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--sw);
    border: 1px solid var(--border);
    margin: 0 1px;
  }
  .swatch.on {
    background: var(--sw);
    box-shadow: 0 0 0 2px var(--accent);
  }
  .chip {
    padding: 0 8px;
    font-size: 12px;
  }
  .width {
    width: 72px;
    margin: 0 6px;
    accent-color: var(--accent);
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--border);
    margin: 0 4px;
    flex: none;
  }
  .tb-menu {
    position: fixed;
    z-index: 26;
    min-width: 220px;
    max-width: min(320px, calc(100vw - 24px));
    max-height: calc(100vh - 120px);
    overflow-y: auto;
    padding: 4px;
    border-radius: 10px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28);
    pointer-events: auto;
  }
  .mi {
    display: flex;
    width: 100%;
    box-sizing: border-box;
    justify-content: flex-start;
    gap: 8px;
    padding: 0 8px;
    height: 28px;
    font-size: 13px;
  }
  .ml {
    flex: 1;
  }
  kbd {
    font: inherit;
    font-size: 11px;
    color: var(--text-muted);
  }
  .mi.on kbd {
    color: inherit;
  }
  .msep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
  .help {
    padding: 4px 8px 6px;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--text-muted);
    user-select: text;
    -webkit-user-select: text;
  }
  .help-title {
    font-weight: 600;
    color: var(--text);
    margin-bottom: 2px;
  }
</style>
