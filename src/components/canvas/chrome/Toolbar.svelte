<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { DRAW_TOOLS, SHAPE_TOOLS, type ToolName } from "$lib/canvas/interaction/tools";
  import { TOOL_ICONS } from "$lib/canvas/icons";
  import Icon from "./Icon.svelte";
  import { aiActivity } from "$lib/canvas/activity/store.svelte";
  import { ICONS } from "$lib/canvas/nodes/registry";
  import { defaultAgent } from "$lib/canvas/prefs.svelte";
  import { openInIde, IDE_LABELS } from "$lib/canvas/sidebar.svelte";
  import { workspaces } from "$lib/canvas/palette/workspaces.svelte";
  import { dropPaths } from "$lib/canvas/files/drop";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { invoke } from "@tauri-apps/api/core";

  const engine = useEngine();
  const COLORS = ["#e8e6e3", "#1d1d1f", "#e5484d", "#f5a524", "#46a758", "#3e8ef7", "#8e4ec6"];

  const ICON = {
    book: "M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2V5z M6 19h13 M8 7h8",
    clock: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M12 7v5l3 2",
    roles: "M12 4a4 4 0 1 0 0 8a4 4 0 1 0 0-8z M4 21a8 8 0 0 1 16 0",
    omni: "M8 4c-2 0-3 1-3 3v2c0 1.5-1 2.5-2 3 1 .5 2 1.5 2 3v2c0 2 1 3 3 3 M16 4c2 0 3 1 3 3v2c0 1.5 1 2.5 2 3-1 .5-2 1.5-2 3v2c0 2-1 3-3 3",
    share: "M12 3v12 M7 8l5-5 5 5 M5 14v6h14v-6",
    visit: "M4 12a8 8 0 0 1 16 0 M8 12a4 4 0 0 1 8 0 M12 12v8",
    more: "M6 12h.01 M12 12h.01 M18 12h.01",
    activity: "M3 12h4l3-8 4 16 3-8h4",
    attach: "M20 11l-8.5 8.5a5 5 0 0 1-7-7L13 4a3.5 3.5 0 0 1 5 5l-8.5 8.5a2 2 0 0 1-3-3L14 7",
    folder: "M3 6h6l2 2h10v11H3z",
    globe: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M3 12h18 M12 3c3 3 3 15 0 18 M12 3c-3 3-3 15 0 18",
    code: "M8 7l-5 5 5 5 M16 7l5 5-5 5 M14 4l-4 16",
    draw: "M4 20l4-1 11-11-3-3L5 16z M14 7l3 3 M3 3l18 18",
  };

  const mod = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl+";
  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  let menuOpen = $state(false);
  let moreBtn = $state<HTMLButtonElement | null>(null);
  let menuPos = $state({ right: 0, top: 0 });
  let avail = $state(0);
  const roomy = $derived(avail === 0 || avail > 900);

  const helpLines = $derived([
    $t("canvas.engine.help.escape", { default: "Shift+Esc: leave a focused terminal and return to the canvas (Esc goes to the program)" }),
    $t("canvas.engine.help.jump", { default: `${mod}1–9: jump to a terminal` }),
    $t("canvas.engine.help.palette", { default: `${mod}K / ${mod}P: command palette` }),
    $t("canvas.engine.help.chat", { default: `${mod}J: chat face of the focused agent` }),
    $t("canvas.engine.help.partituras", { default: `${mod}⇧L: partituras` }),
    $t("canvas.engine.help.attention", { default: `${mod}⇧A: next agent that needs you` }),
  ]);

  const drawing = $derived(DRAW_TOOLS.includes(engine.tool) || SHAPE_TOOLS.includes(engine.tool) || engine.tool === "eraser");
  const DRAW_SET: Array<[ToolName, string]> = [
    ["pen", "Pen"],
    ["highlighter", "Highlighter"],
    ["smart", "Smart draw"],
    ["eraser", "Eraser"],
    ["rect", "Rectangle"],
    ["ellipse", "Ellipse"],
    ["diamond", "Diamond"],
    ["arrow", "Arrow"],
    ["line", "Line"],
  ];

  function creating(kind: string): boolean {
    return engine.tool === "create" && engine.createKind === kind;
  }

  function arm(kind: string) {
    if (creating(kind)) return engine.setTool("select");
    if (kind === "terminal") {
      const d = defaultAgent();
      engine.armCreate("terminal", d.agent, d.preset);
    } else engine.armCreate(kind);
  }

  function togglePanel(kind: "partituras" | "routine" | "roles" | "companion" | "visit") {
    engine.panel = engine.panel?.kind === kind ? null : { kind };
  }

  async function attach() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: true });
      const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
      if (paths.length) await dropPaths(engine, paths, engine.camera.center());
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function ide() {
    const ws = engine.store.workspaceId;
    if (!ws) return;
    try {
      const r = await openInIde(ws);
      showToast("success", tr("canvas.sidebar.openedIn", "Opened in {{ide}}", { ide: IDE_LABELS[r.ide] ?? r.ide }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function exportWs() {
    const ws = workspaces.byId(engine.store.workspaceId);
    if (!ws) return;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        defaultPath: `${ws.name.replace(/[\\/:*?"<>|]/g, "-")}.omniget`,
        filters: [{ name: "OmniGet workspace", extensions: ["omniget"] }],
      });
      if (typeof path !== "string") return;
      const r = await invoke<{ nodes: number; omitted: unknown[] }>("canvas_workspace_export", { workspaceId: ws.id, path });
      showToast("success", tr("canvas.workspaceIo.exported", "Exported {{nodes}} items ({{left}} left out)", { nodes: r.nodes, left: r.omitted.length }));
    } catch (e) {
      showToast("error", String(e));
    }
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

  function openSpawn(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const root = (e.currentTarget as HTMLElement).closest(".canvas-root")?.getBoundingClientRect();
    const sx = r.left - (root?.left ?? 0);
    const sy = r.bottom - (root?.top ?? 0) + 6;
    const c = engine.camera.center();
    engine.spawnMenu = engine.spawnMenu ? null : { sx, sy, wx: c.x, wy: c.y, free: true };
  }

  function run(fn: () => void) {
    fn();
    menuOpen = false;
  }
</script>

<div class="fit" bind:clientWidth={avail}>
  <div class="bar main" data-canvas-chrome role="toolbar" tabindex="-1" aria-label={tr("canvas.engine.tools", "Canvas tools")}>
    <button type="button" class:on={engine.tool === "select"} title="{tr('canvas.engine.tool.select', 'Select')} (V)" aria-label={tr("canvas.engine.tool.select", "Select")} onclick={() => engine.setTool("select")}>
      <Icon d={TOOL_ICONS.select} />
    </button>
    <button type="button" class:on={creating("terminal")} title="{tr('canvas.engine.spawn.terminal', 'Terminal')} (T)" aria-label={tr("canvas.engine.spawn.terminal", "Terminal")} onclick={() => arm("terminal")}>
      <Icon d={ICONS.terminal} />
    </button>
    <button type="button" class:on={creating("note")} title="{tr('canvas.engine.spawn.note', 'Note')} (N)" aria-label={tr("canvas.engine.spawn.note", "Note")} onclick={() => arm("note")}>
      <Icon d={ICONS.note} />
    </button>
    <button type="button" title={tr("canvas.engine.attach", "Attach a file")} aria-label={tr("canvas.engine.attach", "Attach a file")} onclick={() => void attach()}>
      <Icon d={ICON.attach} />
    </button>
    <button type="button" class:on={engine.panel?.kind === "new-filetree"} title="{tr('canvas.menu.add.filetree', 'File Tree')} (F)" aria-label={tr("canvas.menu.add.filetree", "File Tree")} onclick={() => (engine.panel = { kind: "new-filetree" })}>
      <Icon d={ICON.folder} />
    </button>
    <button type="button" class:on={engine.panel?.kind === "new-portal"} title="{tr('canvas.engine.spawn.portal', 'Portal')} (P)" aria-label={tr("canvas.engine.spawn.portal", "Portal")} onclick={() => (engine.panel = { kind: "new-portal" })}>
      <Icon d={ICON.globe} />
    </button>
    <button type="button" class="aa" class:on={creating("text")} title="{tr('canvas.engine.spawn.text', 'Text')} (L)" aria-label={tr("canvas.engine.spawn.text", "Text")} onclick={() => arm("text")}>Aa</button>
    <button type="button" class:on={drawing} title="{tr('canvas.engine.tool.pen', 'Pen')} (D)" aria-label={tr("canvas.engine.tool.pen", "Pen")} onclick={() => engine.setTool(drawing ? "select" : "pen")}>
      <Icon d={ICON.draw} />
    </button>
  </div>
  {#if drawing}
    <div class="bar styles" data-canvas-chrome>
      {#each DRAW_SET as [id, fallback] (id)}
        <button type="button" class:on={engine.tool === id} title={tr(`canvas.engine.tool.${id}`, fallback)} aria-label={tr(`canvas.engine.tool.${id}`, fallback)} onclick={() => engine.setTool(id)}>
          <Icon d={TOOL_ICONS[id as keyof typeof TOOL_ICONS]} />
        </button>
      {/each}
      <span class="sep"></span>
      {#each COLORS as c (c)}
        <button type="button" class="swatch" class:on={engine.penColor === c} style:--sw={c} aria-label={c} onclick={() => (engine.penColor = c)}></button>
      {/each}
      <input class="width" type="range" min="1" max="16" step="1" bind:value={engine.strokeWidth} title={tr("canvas.engine.strokeWidth", "Stroke width")} />
      {#if SHAPE_TOOLS.includes(engine.tool)}
        <button type="button" class="chip" class:on={engine.sketch} onclick={() => (engine.sketch = !engine.sketch)}>{tr("canvas.engine.sketch", "Sketch")}</button>
      {/if}
    </div>
  {/if}
  <div class="right" data-canvas-chrome>
    {#if roomy}
      <div class="bar">
        <button type="button" class:on={engine.panel?.kind === "companion"} title={tr("canvas.engine.companion", "Omni")} aria-label={tr("canvas.engine.companion", "Omni")} onclick={() => togglePanel("companion")}>
          <Icon d={ICON.omni} />
        </button>
        <button type="button" class:on={engine.panel?.kind === "partituras"} title="{tr('canvas.engine.partituras', 'Partituras')} ({mod}⇧L)" aria-label={tr("canvas.engine.partituras", "Partituras")} onclick={() => togglePanel("partituras")}>
          <Icon d={ICON.book} />
        </button>
        <button type="button" class:on={aiActivity.open} title={tr("canvas.activity.title", "Activity")} aria-label={tr("canvas.activity.title", "Activity")} onclick={() => aiActivity.toggle("activity")}>
          <Icon d={ICON.activity} />
        </button>
      </div>
      <div class="bar">
        <button type="button" title={tr("canvas.menu.openInIde", "Open in IDE")} aria-label={tr("canvas.menu.openInIde", "Open in IDE")} onclick={() => void ide()}>
          <Icon d={ICON.code} />
        </button>
      </div>
      <div class="bar">
        <button type="button" title={tr("canvas.workspaceIo.export", "Export…")} aria-label={tr("canvas.workspaceIo.export", "Export…")} onclick={() => void exportWs()}>
          <Icon d={ICON.share} />
        </button>
      </div>
    {/if}
    <div class="bar">
      <button
        type="button"
        class="more"
        class:on={menuOpen}
        bind:this={moreBtn}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        title={tr("canvas.engine.more", "More tools and shortcuts")}
        aria-label={tr("canvas.engine.more", "More tools and shortcuts")}
        onclick={toggleMenu}
      >
        <Icon d={ICON.more} />
      </button>
    </div>
  </div>
</div>

{#if menuOpen}
  <div class="tb-menu" data-canvas-chrome role="menu" style:right="{menuPos.right}px" style:top="{menuPos.top}px">
    <button type="button" role="menuitem" class="mi" onclick={(e) => { openSpawn(e); menuOpen = false; }}>
      <Icon d={TOOL_ICONS.plus} /><span class="ml">{tr("canvas.engine.add", "Add")}</span>
    </button>
    <button type="button" role="menuitem" class="mi" class:on={engine.tool === "hand"} onclick={() => run(() => engine.setTool("hand"))}>
      <Icon d={TOOL_ICONS.hand} /><span class="ml">{tr("canvas.engine.tool.hand", "Hand")}</span><kbd>H</kbd>
    </button>
    <button type="button" role="menuitem" class="mi" class:on={engine.tool === "connect"} onclick={() => run(() => engine.setTool("connect"))}>
      <Icon d={TOOL_ICONS.connect} /><span class="ml">{tr("canvas.engine.tool.connect", "Connect")}</span><kbd>C</kbd>
    </button>
    <div class="msep"></div>
    <button type="button" role="menuitem" class="mi" onclick={() => run(() => engine.undo())}>
      <Icon d={TOOL_ICONS.undo} /><span class="ml">{tr("canvas.engine.undo", "Undo")}</span><kbd>{mod}Z</kbd>
    </button>
    <button type="button" role="menuitem" class="mi" onclick={() => run(() => engine.redo())}>
      <Icon d={TOOL_ICONS.redo} /><span class="ml">{tr("canvas.engine.redo", "Redo")}</span><kbd>{mod}⇧Z</kbd>
    </button>
    <div class="msep"></div>
    {#if !roomy}
      <button type="button" role="menuitem" class="mi" onclick={() => run(() => togglePanel("companion"))}>
        <Icon d={ICON.omni} /><span class="ml">{tr("canvas.engine.companion", "Omni")}</span>
      </button>
      <button type="button" role="menuitem" class="mi" onclick={() => run(() => togglePanel("partituras"))}>
        <Icon d={ICON.book} /><span class="ml">{tr("canvas.engine.partituras", "Partituras")}</span><kbd>{mod}⇧L</kbd>
      </button>
      <button type="button" role="menuitem" class="mi" onclick={() => run(() => aiActivity.toggle("activity"))}>
        <Icon d={ICON.activity} /><span class="ml">{tr("canvas.activity.title", "Activity")}</span>
      </button>
      <button type="button" role="menuitem" class="mi" onclick={() => run(() => void ide())}>
        <Icon d={ICON.code} /><span class="ml">{tr("canvas.menu.openInIde", "Open in IDE")}</span>
      </button>
      <button type="button" role="menuitem" class="mi" onclick={() => run(() => void exportWs())}>
        <Icon d={ICON.share} /><span class="ml">{tr("canvas.workspaceIo.export", "Export…")}</span>
      </button>
    {/if}
    <button type="button" role="menuitem" class="mi" class:on={engine.panel?.kind === "routine"} onclick={() => run(() => togglePanel("routine"))}>
      <Icon d={ICON.clock} /><span class="ml">{tr("canvas.engine.routines", "Routines")}</span>
    </button>
    <button type="button" role="menuitem" class="mi" class:on={engine.panel?.kind === "roles"} onclick={() => run(() => togglePanel("roles"))}>
      <Icon d={ICON.roles} /><span class="ml">{tr("canvas.engine.roles", "Roles")}</span>
    </button>
    <button type="button" role="menuitem" class="mi" class:on={engine.panel?.kind === "visit"} onclick={() => run(() => togglePanel("visit"))}>
      <Icon d={ICON.visit} /><span class="ml">{tr("canvas.engine.share", "Share and visit")}</span>
    </button>
    <div class="msep"></div>
    <div class="help">
      <div class="help-title">{tr("canvas.engine.shortcuts", "Shortcuts")}</div>
      {#each helpLines as line (line)}
        <div class="help-line">{line}</div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .fit {
    position: relative;
    width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    pointer-events: none;
  }
  .right {
    position: absolute;
    top: 0;
    right: 0;
    display: flex;
    gap: 6px;
    pointer-events: none;
  }
  .aa {
    font-size: 14px;
    font-weight: 500;
    padding: 0 4px;
  }
  .bar {
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    gap: 1px;
    padding: 3px 5px;
    border-radius: 999px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.06),
      0 6px 18px rgba(0, 0, 0, 0.12);
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
    min-width: 32px;
    height: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    color: var(--text);
    cursor: pointer;
    flex: none;
  }
  button:hover {
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  button.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  button:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
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
    height: 16px;
    background: var(--border);
    margin: 0 3px;
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
    border-radius: 6px;
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
