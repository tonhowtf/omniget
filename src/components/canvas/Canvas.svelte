<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { CanvasEngine, provideEngine } from "$lib/canvas/engine.svelte";
  import { attachGestures } from "$lib/canvas/camera/gestures";
  import { isEditableTarget, isMac, resolveKey, setShortcutOverrides, type Command } from "$lib/canvas/interaction/keyboard";
  import { goto } from "$app/navigation";
  import { nodeKind } from "$lib/canvas/nodes/registry";
  import { headerWorld, hitTest } from "$lib/canvas/spatial/hit";
  import { prefersDark } from "$lib/canvas/render/color";
  import type { Drawing, Op } from "$lib/canvas/model/types";
  import { registerBuiltinNodes } from "./nodes/builtin";
  import GlLayer from "./layers/GlLayer.svelte";
  import WorldLayer from "./layers/WorldLayer.svelte";
  import ScreenLayer from "./layers/ScreenLayer.svelte";
  import Toolbar from "./chrome/Toolbar.svelte";
  import ZoomControls from "./chrome/ZoomControls.svelte";
  import AgentUsage from "./chrome/AgentUsage.svelte";
  import Minimap from "./chrome/Minimap.svelte";
  import SpawnMenu from "./chrome/SpawnMenu.svelte";
  import ArtifactsPanel from "./artifacts/ArtifactsPanel.svelte";
  import { viewMenu } from "$lib/canvas/menu/model";
  import RemotePresence from "./chrome/RemotePresence.svelte";
  import AiActivityLayer from "./activity/AiActivityLayer.svelte";
  import { listen } from "@tauri-apps/api/event";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import "$lib/canvas/notes/register";
  import "$lib/canvas/portals/register";
  import { registerChat } from "$lib/canvas/chat/register";
  import { installCanvasDriver } from "$lib/canvas/testing/driver";
  import { canvasPrefs, defaultAgent, watchCanvasPrefs, zoomNotch } from "$lib/canvas/prefs.svelte";
  import { resetMetrics } from "$lib/canvas/term/metrics";
  import { termPass } from "$lib/canvas/term/pass";
  import { registerRoles } from "$lib/canvas/roles/register";
  import { roles } from "$lib/canvas/roles/store.svelte";
  import { attachFileDrop } from "$lib/canvas/files/drop";
  import { copySelection, pasteClipboard } from "$lib/canvas/clipboard";
  import { nodeForPath } from "$lib/canvas/files/open";
  import { canvasUi } from "$lib/canvas/palette/ui.svelte";
  import { switchTo } from "$lib/canvas/palette/navigate";
  import { workspaces } from "$lib/canvas/palette/workspaces.svelte";
  import { sidebarUi } from "$lib/canvas/sidebar.svelte";
  import { PARTITURA_DRAG_TYPE } from "$lib/canvas/partituras/api";
  import { isTerminalKind } from "$lib/canvas/model/types";
  import PromptComposer from "./composer/PromptComposer.svelte";
  import WorkspaceSidebar from "./sidebar/WorkspaceSidebar.svelte";
  import CommandPalette from "./palette/CommandPalette.svelte";
  import FloorBar from "./floors/FloorBar.svelte";
  import FloorOverview from "./floors/FloorOverview.svelte";
  import "$lib/canvas/companion/live-language";
  import PartituraLibrary from "./partituras/PartituraLibrary.svelte";
  import ImportPreview from "./partituras/ImportPreview.svelte";
  import RoutineEditor from "./routines/RoutineEditor.svelte";
  import RoleLibrary from "./roles/RoleLibrary.svelte";
  import RolePicker from "./roles/RolePicker.svelte";
  import CompanionPanel from "./companion/CompanionPanel.svelte";
  import VisitPanel from "./visit/VisitPanel.svelte";
  import VisitStage from "./visit/VisitStage.svelte";
  import NewPortalDialog from "./chrome/NewPortalDialog.svelte";
  import NewFileTreeDialog from "./chrome/NewFileTreeDialog.svelte";

  let { workspaceId }: { workspaceId?: string } = $props();

  registerBuiltinNodes();
  const engine = new CanvasEngine();
  provideEngine(engine);
  if (import.meta.env.DEV && typeof window !== "undefined") {
    (window as unknown as { __omnigetCanvas?: CanvasEngine }).__omnigetCanvas = engine;
  }

  let root = $state<HTMLDivElement | null>(null);
  let worldEl: HTMLDivElement | null = null;
  const mac = isMac();
  let badgeTimer: ReturnType<typeof setTimeout> | null = null;
  let closeArmed = 0;
  let spaceDownAt = 0;
  let spacePans = 0;
  let pinchZ0 = 1;

  function onWorldReady(el: HTMLDivElement) {
    worldEl = el;
    if (root) engine.attachElements(root, el);
  }

  onMount(() => {
    if (!root) return;
    engine.penColor = prefersDark() ? "#e8e6e3" : "#1d1d1f";
    engine.tools.bind(root);
    if (worldEl) engine.attachElements(root, worldEl);

    const ro = new ResizeObserver(() => {
      if (!root) return;
      const r = root.getBoundingClientRect();
      engine.resize(r.width, r.height, window.devicePixelRatio || 1);
    });
    ro.observe(root);
    const r = root.getBoundingClientRect();
    engine.resize(r.width, r.height, window.devicePixelRatio || 1);

    engine.chromeProbe = () => {
      if (!root) return [];
      const base = root.getBoundingClientRect();
      return Array.from(root.querySelectorAll<HTMLElement>("[data-canvas-occluder]")).map((el) => {
        const r = el.getBoundingClientRect();
        return { x: r.left - base.left, y: r.top - base.top, w: r.width, h: r.height };
      });
    };
    const stopPrefs = watchCanvasPrefs();
    const detachGestures = attachGestures(root, {
      zoomModifier: () => canvasPrefs().zoom_modifier,
      notch: (dir) => zoomNotch(dir),
      pan: (dx, dy) => engine.panBy(dx, dy),
      zoom: (f, sx, sy) => engine.zoomAt(f, sx, sy),
      pinch: (phase, scale, sx, sy) => {
        if (phase === "start") {
          pinchZ0 = engine.camera.zoom;
          engine.pinching = true;
        } else if (phase === "change") {
          engine.camera.zoomTo(pinchZ0 * scale, sx, sy, true);
          engine.invalidate();
        } else {
          engine.pinching = false;
          engine.invalidate();
        }
      },
      activity: () => engine.inertia.stop(),
      passThrough: (e) => {
        const target = e.target as HTMLElement | null;
        if (!target) return false;
        if (target.closest("[data-canvas-chrome]")) return true;
        const focused = engine.selection.focused;
        if (!focused || e.shiftKey || e.ctrlKey || e.metaKey) return false;
        const host = target.closest<HTMLElement>("[data-node-id]");
        return !!host && host.dataset.nodeId === focused && !!target.closest("[data-node-body]");
      },
    });

    const onBlur = () => {
      engine.sync.flush();
      engine.tools.spaceHeld = false;
      engine.showBadges = false;
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", onBlur);

    void engine.start(workspaceId);

    const stopDrop = attachFileDrop(engine, root);
    const stopDriver = installCanvasDriver(engine);
    const stopChat = registerChat({
      focused: () => engine.selection.focused,
      isTerminal: (id) => isTerminalKind(engine.store.node(id)?.kind ?? ""),
    });
    const stopRoles = registerRoles({
      workspaceId: () => engine.store.workspaceId,
      restart: (nodeId, ev) => {
        if (ev.restart) engine.requestRestart(nodeId);
      },
    });
    const unlisteners: Array<Promise<() => void>> = [
      listen<{ workspaceId?: string; name?: string; message: string; source?: string }>("canvas://notify", (e) => {
        const p = e.payload;
        if (!p?.message) return;
        showToast("info", p.name ? `${p.name}: ${p.message}` : p.message, 6000);
      }).catch(() => () => {}),
    ];

    return () => {
      stopDriver();
      engine.chromeProbe = null;
      stopPrefs();
      stopDrop();
      stopChat();
      stopRoles();
      for (const u of unlisteners) void u.then((f) => f());
      ro.disconnect();
      detachGestures();
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", onBlur);
      engine.destroy();
    };
  });

  function onPointerDownCapture(e: PointerEvent) {
    const target = e.target as HTMLElement;
    if (target.closest("[data-canvas-chrome]")) return;
    if (engine.spawnMenu) engine.spawnMenu = null;
    const host = target.closest<HTMLElement>("[data-node-id]");
    const id = host?.dataset.nodeId;
    if (id && engine.isRaised(id)) {
      if (e.button === 0 && engine.selection.focused !== id) {
        engine.selection.set([id]);
        engine.selection.focus(id);
        engine.invalidate();
      }
      return;
    }
    const inBody = !!target.closest("[data-node-body]");
    if (id && inBody && e.button === 0 && engine.tool === "select" && !engine.tools.spaceHeld) {
      const node = engine.store.node(id);
      const l = engine.tools.local(e);
      const w = engine.camera.screenToWorld(l.x, l.y);
      const onHeader = !!node && w.y - node.y <= headerWorld(node.kind, node.h, engine.camera.zoom);
      const passThrough = !onHeader || !!target.closest("button, input, textarea, a, [role='button']");
      if (engine.selection.focused === id && passThrough) return;
      const def = node ? nodeKind(node.kind) : undefined;
      if (!passThrough) {
        e.preventDefault();
        root?.focus({ preventScroll: true });
        engine.tools.pointerDown(e);
        return;
      }
      if (def?.focusOnClick && !e.shiftKey) {
        engine.selection.set([id]);
        engine.selection.focus(id);
        engine.invalidate();
        return;
      }
      if (def?.focusOnSecondClick && engine.selection.size === 1 && engine.selection.has(id)) {
        engine.selection.focus(id);
        engine.invalidate();
        return;
      }
    }
    if (e.button === 2) {
      if (canvasPrefs().right_drag_pan) {
        rightPan = { x: e.clientX, y: e.clientY, moved: false, id: e.pointerId };
        try {
          root?.setPointerCapture(e.pointerId);
        } catch {
        }
      }
      return;
    }
    e.preventDefault();
    root?.focus({ preventScroll: true });
    engine.tools.pointerDown(e);
  }

  let rightPan: { x: number; y: number; moved: boolean; id: number } | null = null;
  let rightPanned = false;
  let hoverFocusId: string | null = null;

  function onPointerMove(e: PointerEvent) {
    if (rightPan && (e.buttons & 2) !== 0) {
      const dx = e.clientX - rightPan.x;
      const dy = e.clientY - rightPan.y;
      if (rightPan.moved || Math.hypot(dx, dy) > 4) {
        rightPan.moved = true;
        engine.panBy(dx, dy);
        rightPan.x = e.clientX;
        rightPan.y = e.clientY;
      }
      return;
    }
    if (e.buttons === 0 && engine.tool === "select" && canvasPrefs().focus_on_hover) focusOnHover(e);
    engine.tools.pointerMove(e);
  }

  function focusOnHover(e: PointerEvent) {
    const host = (e.target as HTMLElement | null)?.closest?.<HTMLElement>("[data-node-id]");
    const id = host?.dataset.nodeId ?? null;
    if (!id || id === hoverFocusId || engine.selection.focused === id) {
      hoverFocusId = id;
      return;
    }
    hoverFocusId = id;
    const node = engine.store.node(id);
    if (!node || node.locked) return;
    const kind = node.kind;
    if (!(isTerminalKind(kind) || kind === "note" || kind === "portal" || kind === "filetree")) return;
    engine.selection.set([id]);
    engine.selection.focus(id);
    engine.invalidate();
  }

  function onPointerUp(e: PointerEvent) {
    if (rightPan && e.button === 2) {
      rightPanned = rightPan.moved;
      rightPan = null;
      return;
    }
    engine.tools.pointerUp(e);
  }

  function onContextMenu(e: MouseEvent) {
    if (rightPanned) {
      rightPanned = false;
      e.preventDefault();
      return;
    }
    const target = e.target as HTMLElement;
    if (target.closest("[data-canvas-chrome]")) return;
    const host = target.closest<HTMLElement>("[data-node-id]");
    if (host && host.dataset.nodeId === engine.selection.focused) return;
    e.preventDefault();
    const raisedId = host?.dataset.nodeId;
    if (raisedId && engine.isRaised(raisedId)) {
      const l = engine.tools.local(e);
      engine.selection.set([raisedId]);
      engine.spawnMenu = { sx: l.x, sy: l.y, wx: 0, wy: 0, nodeId: raisedId };
      return;
    }
    const l = engine.tools.local(e);
    const w = engine.camera.screenToWorld(l.x, l.y);
    const hit = hitTest(engine.store, w.x, w.y, { zoom: engine.camera.zoom });
    if (hit.type === "node" || hit.type === "drawing") {
      if (!engine.selection.has(hit.id)) engine.selection.set([hit.id]);
      engine.spawnMenu = { sx: l.x, sy: l.y, wx: w.x, wy: w.y, nodeId: hit.id };
    } else if (hit.type === "connection") {
      engine.selection.set([hit.id]);
      engine.spawnMenu = { sx: l.x, sy: l.y, wx: w.x, wy: w.y, connectionId: hit.id };
    } else {
      engine.spawnMenu = { sx: l.x, sy: l.y, wx: w.x, wy: w.y };
    }
  }

  function onDblClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target.closest("[data-canvas-chrome]")) return;
    const host = target.closest<HTMLElement>("[data-node-id]");
    if (host && host.dataset.nodeId === engine.selection.focused) return;
    if (host?.dataset.nodeId && engine.isRaised(host.dataset.nodeId)) return;
    const l = engine.tools.local(e);
    const w = engine.camera.screenToWorld(l.x, l.y);
    const hit = hitTest(engine.store, w.x, w.y, { zoom: engine.camera.zoom });
    if (hit.type === "node") {
      engine.selection.set([hit.id]);
      if (hit.zone === "header") engine.panels.toggleLift(hit.id);
      else engine.enterSelected();
      return;
    }
    if (hit.type === "none" && engine.tool === "select") {
      const node = engine.spawn("text", { x: w.x, y: w.y - 20 });
      engine.selection.focus(node.id);
    }
  }

  function mine(e: KeyboardEvent): boolean {
    if (!root) return false;
    const a = document.activeElement;
    return !a || a === document.body || root.contains(a) || root.contains(e.target as Node);
  }

  function onKeyDown(e: KeyboardEvent) {
    const primary = mac ? e.metaKey : e.ctrlKey;
    if (primary && !e.shiftKey && !e.altKey && (e.key === "," || e.code === "Comma")) {
      e.preventDefault();
      e.stopImmediatePropagation();
      void goto("/settings");
      return;
    }
    if (!mine(e)) {
      if (primary && !e.shiftKey && !e.altKey && /^[1-9]$/.test(e.key) && !isEditableTarget(e.target) && !document.querySelector("[role='dialog'][aria-modal='true']")) {
        e.preventDefault();
        e.stopImmediatePropagation();
        engine.jump(Number(e.key));
      }
      return;
    }
    if ((e.target as HTMLElement | null)?.closest?.("[role='menu']") && e.key !== "Escape") return;
    if (primary && e.shiftKey && !e.altKey && e.key.toLowerCase() === "l") {
      e.preventDefault();
      e.stopPropagation();
      engine.panel = engine.panel?.kind === "partituras" ? null : { kind: "partituras" };
      return;
    }
    if ((e.key === "Meta" && mac) || (e.key === "Control" && !mac)) {
      if (badgeTimer) clearTimeout(badgeTimer);
      badgeTimer = setTimeout(() => (engine.showBadges = true), 300);
    } else if (badgeTimer) {
      clearTimeout(badgeTimer);
      badgeTimer = null;
    }
    const editable = isEditableTarget(e.target);
    if (editable) {
      if (e.key === "Escape" && e.shiftKey) {
        e.preventDefault();
        engine.exitFocus();
      } else if (primary && /^[1-9]$/.test(e.key)) {
        e.preventDefault();
        engine.jump(Number(e.key));
      } else if (primary && e.shiftKey && e.key.toLowerCase() === "a") {
        e.preventDefault();
        engine.nextAttention();
      } else if (primary && e.altKey && !e.shiftKey && (e.key === "ArrowRight" || e.key === "ArrowLeft")) {
        e.preventDefault();
        e.stopPropagation();
        void engine.walkCord(e.key === "ArrowRight" ? 1 : -1);
      } else if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        cycleElements(e.shiftKey ? -1 : 1);
      }
      return;
    }
    if (e.key === " " && !e.repeat) {
      e.preventDefault();
      spaceDownAt = performance.now();
      spacePans = engine.tools.panStarts;
      engine.tools.spaceHeld = true;
      engine.tools.setCursor("grab");
      return;
    }
    if (primary && e.key.toLowerCase() === "c" && (window.getSelection()?.toString() ?? "") !== "") return;
    const selIds = engine.selection.list();
    const drawingsOnly = selIds.length > 0 && selIds.every((id) => !!engine.store.drawing(id));
    const cmd = resolveKey(e, { hasSelection: engine.selection.size > 0, drawingsOnly, mac });
    if (!cmd) return;
    e.preventDefault();
    e.stopPropagation();
    run(cmd);
  }

  function onKeyUp(e: KeyboardEvent) {
    if (e.key === "Meta" || e.key === "Control") {
      if (badgeTimer) clearTimeout(badgeTimer);
      badgeTimer = null;
      engine.showBadges = false;
    }
    if (e.key === " ") {
      engine.tools.spaceHeld = false;
      engine.tools.setCursor("");
      if (spaceDownAt && performance.now() - spaceDownAt < 350 && engine.tools.panStarts === spacePans && !isEditableTarget(e.target)) {
        const id = engine.selection.list().find((x) => engine.store.node(x));
        if (id && engine.selection.size === 1) engine.panels.toggleLift(id);
      }
      spaceDownAt = 0;
    }
  }

  function cycleElements(delta: number) {
    if (engine.panels.cycle(delta)) return;
    const order = engine.panels.order();
    if (!order.length) return;
    const cur = engine.selection.focused ?? engine.selection.list()[0];
    const i = cur ? order.indexOf(cur) : -1;
    const next = order[(((i < 0 ? (delta > 0 ? -1 : 0) : i) + delta) % order.length + order.length) % order.length];
    engine.selection.set([next]);
    engine.reveal(next);
    engine.selection.focus(next);
  }

  function styleSelectedDrawings(delta: number) {
    const ds = engine.selection
      .list()
      .map((id) => engine.store.drawing(id))
      .filter((d): d is Drawing => !!d);
    if (!ds.length) {
      engine.strokeWidth = Math.max(1, Math.min(16, engine.strokeWidth + delta));
      if (engine.tool === "select" || engine.tool === "hand") engine.setTool("pen");
      return;
    }
    const ops: Op[] = [];
    for (const d of ds) {
      const w = Math.max(1, Math.min(32, (d.strokeWidth ?? 2) + delta));
      ops.push({ op: "update_drawing", id: d.id, patch: { strokeWidth: w } });
    }
    engine.commit(ops);
  }

  function run(cmd: Command) {
    switch (cmd.cmd) {
      case "escape":
        if (engine.floorsView) engine.floorsView = false;
        else if (engine.panel) engine.panel = null;
        else if (engine.spawnMenu) engine.spawnMenu = null;
        else if (engine.panels.lifted) engine.panels.lift(null);
        else if (engine.tools.busy) engine.tools.cancel();
        else if (engine.tool !== "select") engine.setTool("select");
        else if (engine.selection.focused) engine.exitFocus();
        else engine.selection.clear();
        break;
      case "tool":
        engine.setTool(cmd.tool);
        break;
      case "create":
        if (cmd.kind === "terminal") {
          const d = defaultAgent();
          engine.armCreate(cmd.kind, d.agent, d.preset);
        } else engine.armCreate(cmd.kind);
        break;
      case "focus-dir":
        engine.focusDir(cmd.dir);
        break;
      case "nudge":
        engine.nudge(cmd.dx, cmd.dy);
        break;
      case "zoom-toggle":
        engine.zoomToggle();
        break;
      case "zoom-fit":
        engine.fitAll();
        break;
      case "zoom-in":
        engine.setZoom(engine.camera.zoom * zoomNotch(1));
        break;
      case "zoom-out":
        engine.setZoom(engine.camera.zoom * zoomNotch(-1));
        break;
      case "zoom-reset":
        engine.setZoom(1);
        break;
      case "split":
        engine.split(cmd.dir);
        break;
      case "close": {
        const now = performance.now();
        if (now - closeArmed < 1500) {
          closeArmed = 0;
          engine.deleteSelection();
        } else {
          closeArmed = now;
          engine.say("canvas.engine.pressAgainToClose");
        }
        break;
      }
      case "delete":
        engine.deleteSelection();
        break;
      case "jump":
        engine.jump(cmd.index);
        break;
      case "cycle":
        cycleElements(cmd.delta);
        break;
      case "enter":
        engine.enterSelected();
        break;
      case "undo":
        engine.undo();
        break;
      case "redo":
        engine.redo();
        break;
      case "group":
        engine.group();
        break;
      case "ungroup":
        engine.ungroup();
        break;
      case "raise":
      case "lower":
      case "front":
      case "back":
        engine.zOrder(cmd.cmd);
        break;
      case "select-all":
        engine.selection.set([...engine.store.nodes.map((n) => n.id), ...engine.store.drawings.map((d) => d.id)]);
        break;
      case "duplicate":
        engine.duplicateSelection();
        break;
      case "next-attention":
        engine.nextAttention();
        break;
      case "toggle-minimap":
        engine.minimapVisible = !engine.minimapVisible;
        break;
      case "floors-overview":
        engine.floorsView = !engine.floorsView;
        break;
      case "omni-window":
        void invoke("canvas_omni_window", { show: null }).catch((err) => showToast("error", String(err)));
        break;
      case "stroke-width":
        styleSelectedDrawings(cmd.delta);
        break;
      case "rotate":
        engine.tools.rotateSelection(cmd.deg);
        break;
      case "lock":
        engine.toggleLock();
        break;
      case "view": {
        const entry = viewMenu(engine).find((x) => x.id === cmd.id);
        void Promise.resolve(entry?.run?.()).catch(() => {});
        break;
      }
      case "cord-walk":
        void engine.walkCord(cmd.delta);
        break;
      case "new-note": {
        const n = engine.spawn("note", null);
        engine.reveal(n.id);
        engine.selection.focus(n.id);
        break;
      }
      case "lift": {
        const id = engine.selection.list().find((x) => engine.store.node(x));
        if (id) engine.panels.toggleLift(id);
        break;
      }
      case "align":
        engine.align(cmd.mode);
        break;
      case "new-terminal": {
        const c = engine.camera.center();
        engine.spawnMenu = { sx: engine.camera.vw / 2, sy: engine.camera.vh / 2, wx: c.x, wy: c.y, free: true };
        break;
      }
      case "copy":
        void copySelection(engine);
        break;
      case "cut":
        void copySelection(engine, true);
        break;
      case "paste":
        void pasteClipboard(engine);
        break;
      case "connect": {
        const nodes = engine.selection.list().filter((id) => engine.store.node(id));
        if (nodes.length === 2) engine.connect(nodes[0], nodes[1]);
        else engine.setTool("connect");
        break;
      }
    }
    engine.invalidate();
  }

  const FILE_DRAG = "application/x-omniget-file";

  function dropPoint(e: DragEvent): { x: number; y: number } {
    const r = root!.getBoundingClientRect();
    return engine.camera.screenToWorld(e.clientX - r.left, e.clientY - r.top);
  }

  function onDragOver(e: DragEvent) {
    const types = e.dataTransfer?.types ?? [];
    if (types.includes(FILE_DRAG) || types.includes(PARTITURA_DRAG_TYPE)) {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    }
  }

  function onDrop(e: DragEvent) {
    const dt = e.dataTransfer;
    if (!dt || !root) return;
    const partitura = dt.getData(PARTITURA_DRAG_TYPE);
    if (partitura) {
      e.preventDefault();
      engine.panel = { kind: "import", id: partitura, at: dropPoint(e) };
      return;
    }
    const raw = dt.getData(FILE_DRAG);
    if (raw) {
      e.preventDefault();
      try {
        const f = JSON.parse(raw) as { path: string; name?: string; isImage?: boolean };
        if (f?.path) {
          const node = nodeForPath(engine, f.path, false, dropPoint(e));
          engine.selection.set([node.id]);
        }
      } catch {
      }
    }
  }

  function onImported(r: { nodeIds?: string[] }) {
    engine.panel = null;
    if (r?.nodeIds?.length) engine.selection.set(r.nodeIds);
    engine.invalidate();
  }

  $effect(() => {
    void canvasPrefs();
    engine.invalidate();
  });
  $effect(() => {
    setShortcutOverrides(canvasPrefs().shortcuts);
  });
  let lastTermPrefs = "";
  $effect(() => {
    const tp = JSON.stringify(canvasPrefs().terminal);
    if (tp === lastTermPrefs) return;
    const first = !lastTermPrefs;
    lastTermPrefs = tp;
    if (first) return;
    resetMetrics();
    termPass().refreshTheme();
  });

  const sidebarW = $derived(canvasUi.sidebarOpen ? 232 : sidebarUi.mini ? 60 : 0);
  engine.camera.insetLeft = canvasUi.sidebarOpen ? 232 : sidebarUi.mini ? 60 : 0;
  $effect(() => {
    engine.panels.sidebarW = sidebarW;
    engine.camera.insetLeft = sidebarW + engine.panels.dockWidth("left");
    engine.camera.insetRight = engine.panels.dockWidth("right");
  });
  $effect(() => {
    void engine.syncMode;
    void engine.store.version;
    const ws = engine.store.workspaceId;
    const w = ws ? workspaces.byId(ws) : undefined;
    if (ws && w) engine.panels.syncFrom(ws, w.panels);
  });
  const terminalNodes = $derived.by(() => {
    void engine.store.structure;
    return engine.store.nodes
      .filter((n) => isTerminalKind(n.kind))
      .map((n) => ({ id: n.id, title: n.title ?? n.id }));
  });
  const libraryCenter = $derived.by(() => {
    const c = engine.camera.center();
    return { x: c.x - 240, y: c.y - 160 };
  });
  const pickerNode = $derived(engine.panel?.kind === "role-picker" ? engine.store.node(engine.panel.nodeId) : undefined);

  $effect(() => {
    const ws = engine.store.workspaceId;
    void engine.syncMode;
    if (ws) void roles.load(ws);
  });

  $effect(() => {
    const ws = workspaces.byId(engine.store.workspaceId);
    const title = ws?.name ? `${ws.name} — OmniGet` : "OmniGet";
    void import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) => getCurrentWindow().setTitle(title))
      .catch(() => {});
  });
  onMount(() => {
    const stop = workspaces.use();
    return () => {
      stop();
      void import("@tauri-apps/api/window")
        .then(({ getCurrentWindow }) => getCurrentWindow().setTitle("OmniGet"))
        .catch(() => {});
    };
  });

  const noticeText = $derived(
    engine.notice
      ? $t(engine.notice, {
          default:
            (
              {
                "canvas.engine.undoSkipped": "Some changes were kept because an agent edited them afterwards",
                "canvas.engine.terminalFailed": "Could not start the terminal",
                "canvas.engine.alreadyConnected": "These two are already connected",
                "canvas.engine.nobodyBlocked": "Nobody needs you right now",
                "canvas.engine.noCords": "Nothing is tied to this element by a cord",
                "canvas.engine.pressAgainToClose": "Press x again to close",
              } as Record<string, string>
            )[engine.notice] ?? engine.notice,
        })
      : "",
  );
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="canvas-root"
  class:tool-draw={engine.tool !== "select" && engine.tool !== "hand"}
  class:transparent={canvasPrefs().background === "transparent"}
  bind:this={root}
  tabindex="0"
  role="application"
  aria-label={$t("canvas.engine.canvas", { default: "Agent canvas" })}
  onpointerdowncapture={onPointerDownCapture}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={() => engine.tools.cancel()}
  oncontextmenu={onContextMenu}
  ondblclick={onDblClick}
  ondragover={onDragOver}
  ondrop={onDrop}
  style="--sidebar-w: {sidebarW}px; --dock-l: {engine.panels.dockWidth('left')}px; --dock-r: {engine.panels.dockWidth('right')}px"
>
  <div class="stage" class:tilted={engine.floorsView}>
    <GlLayer />
    <div class="world-clip">
      <WorldLayer onready={onWorldReady} />
    </div>
    <ScreenLayer />
  </div>
  {#if engine.floorsView}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="stage-catch" data-canvas-chrome onclick={() => (engine.floorsView = false)}></div>
  {/if}
  <RemotePresence />
  <AiActivityLayer />

  <div class="top" data-canvas-occluder>
    <Toolbar />
  </div>
  <div class="bottom-right">
    {#if !engine.panels.lifted}<div class="minimap-slot" data-canvas-occluder><Minimap /></div>{/if}
    <div class="bottom-row" data-canvas-occluder>
      {#if engine.syncMode === "rust" && engine.store.workspaceId}
        <FloorBar
          workspaceId={engine.store.workspaceId}
          floorId={engine.store.floorId}
          onSwitch={(id) => void switchTo(engine, engine.store.workspaceId, id)}
        />
      {/if}
      <AgentUsage />
      <button
        type="button"
        class="map-pill"
        use:agentActions={["click"]}
        class:on={engine.minimapVisible}
        data-canvas-chrome
        title={$t("canvas.engine.toggleMinimap", { default: "Minimap" })}
        aria-label={$t("canvas.engine.toggleMinimap", { default: "Minimap" })}
        aria-pressed={engine.minimapVisible}
        onclick={() => (engine.minimapVisible = !engine.minimapVisible)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6l6-3 6 3 6-3v15l-6 3-6-3-6 3z M9 3v15 M15 6v15" /></svg>
      </button>
      <ZoomControls />
    </div>
  </div>
  {#if engine.syncMode === "memory"}
    <div class="sync" data-canvas-chrome title={engine.sync.lastError ?? ""}>
      {$t("canvas.engine.memoryMode", { default: "Not saved: working in memory" })}
    </div>
  {/if}
  {#if noticeText}
    <div class="notice" role="status">{noticeText}</div>
  {/if}
  <SpawnMenu />
  <ArtifactsPanel />
  <PromptComposer />
  <WorkspaceSidebar />
  <CommandPalette />
  <VisitStage />

  {#if engine.panel?.kind === "companion" || engine.panel?.kind === "visit"}
    <div class="popover" data-canvas-chrome>
      {#if engine.panel.kind === "companion"}
        <CompanionPanel
          workspaceId={engine.store.workspaceId}
          terminals={terminalNodes}
          onFocusNode={(id) => {
            engine.selection.set([id]);
            engine.reveal(id);
          }}
          onClose={() => (engine.panel = null)}
        />
      {:else}
        <VisitPanel
          workspaceId={engine.store.workspaceId}
          floorId={engine.store.floorId}
          terminals={terminalNodes}
          onClose={() => (engine.panel = null)}
        />
      {/if}
    </div>
  {/if}
  {#if engine.panel && engine.panel.kind !== "import" && engine.panel.kind !== "companion" && engine.panel.kind !== "visit"}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="modal" data-canvas-chrome onclick={(e) => e.target === e.currentTarget && (engine.panel = null)}>
      {#if engine.panel.kind === "partituras"}
        <PartituraLibrary
          workspaceId={engine.store.workspaceId}
          floorId={engine.store.floorId}
          selection={{
            nodeIds: engine.selection.list().filter((id) => !!engine.store.node(id)),
            drawingIds: engine.selection.list().filter((id) => !!engine.store.drawing(id)),
          }}
          center={libraryCenter}
          onClose={() => (engine.panel = null)}
          {onImported}
        />
      {:else if engine.panel.kind === "routine"}
        <RoutineEditor
          workspaceId={engine.store.workspaceId}
          floorId={engine.store.floorId}
          nodes={terminalNodes}
          targetNodeId={engine.panel.targetNodeId}
          onClose={() => (engine.panel = null)}
        />
      {:else if engine.panel.kind === "floors"}
        <FloorOverview
          workspaceId={engine.store.workspaceId}
          floorId={engine.store.floorId}
          onSwitch={(id) => void switchTo(engine, engine.store.workspaceId, id)}
          onClose={() => (engine.panel = null)}
        />
      {:else if engine.panel.kind === "new-portal"}
        <NewPortalDialog onclose={() => (engine.panel = null)} />
      {:else if engine.panel.kind === "new-filetree"}
        <NewFileTreeDialog onclose={() => (engine.panel = null)} />
      {:else if engine.panel.kind === "roles"}
        <RoleLibrary workspaceId={engine.store.workspaceId} onclose={() => (engine.panel = null)} />
      {:else if engine.panel.kind === "role-picker" && pickerNode}
        <RolePicker
          workspaceId={engine.store.workspaceId}
          value={typeof pickerNode.data.roleId === "string" ? pickerNode.data.roleId : null}
          onpick={(roleId) => {
            const id = pickerNode.id;
            engine.panel = null;
            void roles.assign(id, roleId).catch((err) => showToast("error", String(err)));
          }}
          onmanage={() => (engine.panel = { kind: "roles" })}
          onclose={() => (engine.panel = null)}
        />
      {/if}
    </div>
  {/if}
  {#if engine.panel?.kind === "import"}
    <div data-canvas-chrome>
      <ImportPreview
        workspaceId={engine.store.workspaceId}
        floorId={engine.store.floorId}
        source={{ id: engine.panel.id }}
        at={engine.panel.at}
        onClose={() => (engine.panel = null)}
        {onImported}
      />
    </div>
  {/if}
</div>

<style>
  .canvas-root {
    position: absolute;
    inset: 0;
    overflow: hidden;
    background: var(--bg);
    outline: none;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
    --cx: 0;
    --cy: 0;
    --z: 1;
  }
  .canvas-root.transparent {
    background: transparent;
  }
  .canvas-root.tool-draw {
    cursor: crosshair;
  }
  .stage {
    position: absolute;
    inset: 0;
    transform-origin: 50% 70%;
    transition:
      transform 320ms cubic-bezier(0.2, 0.8, 0.2, 1),
      border-radius 320ms ease,
      box-shadow 320ms ease;
  }
  .stage.tilted {
    transform: perspective(1400px) translateY(8%) rotateX(46deg) scale(0.66);
    background: var(--bg);
    border-radius: 18px;
    overflow: hidden;
    box-shadow: 0 40px 80px rgba(0, 0, 0, 0.3);
    pointer-events: none;
  }
  .canvas-root:has(.stage.tilted) {
    background: color-mix(in srgb, var(--bg) 86%, var(--text) 14%);
  }
  .stage-catch {
    position: absolute;
    inset: 0;
    z-index: 9;
  }
  @media (prefers-reduced-motion: reduce) {
    .stage {
      transition: none;
    }
  }
  .world-clip {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    z-index: 1;
  }
  .world-clip :global(.node) {
    pointer-events: auto;
    user-select: text;
    -webkit-user-select: text;
  }
  .top {
    position: absolute;
    top: 12px;
    left: calc(var(--sidebar-w, 0px) + var(--dock-l, 0px) + 12px);
    right: calc(var(--dock-r, 0px) + 12px);
    display: flex;
    justify-content: center;
    pointer-events: none;
    z-index: 12;
  }
  .top > :global(*) {
    min-width: 0;
  }
  .bottom-right {
    position: absolute;
    right: calc(var(--dock-r, 0px) + 12px);
    bottom: 12px;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    z-index: 10;
  }
  .bottom-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }
  .map-pill {
    all: unset;
    display: inline-grid;
    place-items: center;
    width: 32px;
    height: 30px;
    border-radius: 999px;
    color: var(--text);
    background: var(--popup-bg, var(--surface));
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.06),
      0 4px 14px rgba(0, 0, 0, 0.1);
    cursor: pointer;
  }
  .map-pill:hover,
  .map-pill.on {
    background: color-mix(in srgb, var(--popup-bg, var(--surface)) 88%, var(--text) 12%);
  }
  .map-pill svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linejoin: round;
  }
  .popover {
    position: absolute;
    top: 64px;
    right: 12px;
    z-index: 25;
    max-height: calc(100% - 140px);
    overflow-x: hidden;
    overflow-y: auto;
  }
  .modal {
    position: absolute;
    inset: 0;
    z-index: 30;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px 12px 12px calc(var(--sidebar-w, 0px) + 12px);
    box-sizing: border-box;
    background: rgb(0 0 0 / 0.25);
  }
  .modal > :global(*) {
    max-height: 100%;
    overflow: auto;
  }
  .sync {
    position: absolute;
    left: calc(var(--sidebar-w, 0px) + 12px);
    bottom: 12px;
    z-index: 10;
    font-size: 11px;
    padding: 4px 8px;
    border-radius: 6px;
    color: var(--text-muted);
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
  }
  .notice {
    position: absolute;
    left: calc(50% + var(--sidebar-w, 0px) / 2);
    bottom: 24px;
    transform: translateX(-50%);
    z-index: 11;
    padding: 8px 14px;
    border-radius: 10px;
    font-size: 13px;
    color: var(--text);
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    pointer-events: none;
  }
</style>
