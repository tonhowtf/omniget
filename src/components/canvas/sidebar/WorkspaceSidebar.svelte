<script lang="ts">
  // Workspaces sidebar (study 95 §A, study 100 §A): list with groups
  // (dividers) and folders (collapsible), create (⌘⇧N, the new one opens),
  // rename, edit (icon, IDE, jump number, CLAUDE.md/AGENTS.md sync), pin,
  // copy path, open in IDE, delete, reorder by dragging or Alt+↑/↓, floors
  // under each one. Switching: ⌃1–⌃9 (Alt+1–9 off macOS) honouring fixed
  // jump numbers, ⌘⌘ then a digit, ⌘↑/⌘↓ and ⌘[/⌘], ⌘ + wheel; ⌘S shows or
  // hides. Closed, it becomes the mini rail (hover label, hold to list the
  // terminals). Hibernated workspaces are dimmed. Switching keeps the old
  // workspace's terminals running (Maestri).
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { isEditableTarget, isMac } from "$lib/canvas/interaction/keyboard";
  import { goTo, switchTo } from "$lib/canvas/palette/navigate";
  import { workspaces, type WorkspaceSummary } from "$lib/canvas/palette/workspaces.svelte";
  import { canvasUi } from "$lib/canvas/palette/ui.svelte";
  import { WORKSPACE_STEP_EVENT } from "$lib/canvas/camera/gestures";
  import {
    IDE_LABELS,
    agentFiles,
    iconOf,
    installedIdes,
    jumpTargets,
    layoutWorkspaces,
    numberOf,
    openInIde,
    sidebarUi,
    wsIcons,
    type Ws,
  } from "$lib/canvas/sidebar.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import WorkspaceIoPanel from "./WorkspaceIoPanel.svelte";

  const engine = useEngine();
  const mac = isMac();

  const COLORS = ["#6cb6ff", "#5ccfb9", "#8fd16a", "#f6d860", "#f4a261", "#ef6f6c", "#f28ab2", "#b392f0", "#a0a4ab"];
  const FOLDERS_KEY = "omniget.canvas.sidebarFolders";

  interface Form {
    id: string | null;
    name: string;
    workingDirectory: string;
    icon: string;
    color: string;
    groupName: string;
    folderName: string;
    ide: string;
    jumpNumber: string;
    instructions: string;
    instructionsLoaded: string;
    syncAgentFiles: boolean;
  }

  let form = $state<Form | null>(null);
  let renaming = $state<string | null>(null);
  let renameText = $state("");
  let menu = $state<{ ws: Ws; x: number; y: number } | null>(null);
  let confirmDelete = $state<string | null>(null);
  let expanded = $state<Record<string, boolean>>({});
  let collapsed = $state<Record<string, boolean>>(readFolders());
  let drag = $state<{ id: string; y0: number; over: number; moved: boolean } | null>(null);
  let newFloor = $state<string | null>(null);
  let floorName = $state("");
  let nameInput = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  let host = $state<HTMLElement | null>(null);
  let error = $state<string | null>(null);
  let ides = $state<string[]>([]);
  let railHover = $state<{ ws: Ws; y: number } | null>(null);
  let railHold = $state<{ ws: Ws; y: number } | null>(null);
  let sync = $state<{
    ws: Ws;
    winner: string;
    changes: Array<{ file: string; linesLost: number; linesAdded: number; lost: string[] }> | null;
  } | null>(null);
  let ioPanel = $state<
    { kind: "import"; path: string } | { kind: "backups"; workspaceId: string; name: string } | null
  >(null);

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) });

  const list = $derived(workspaces.list);
  const layout = $derived(layoutWorkspaces(list));
  /** Visual order: what ⌘↑/⌘↓, ⌘[/⌘] and the drag indices follow. */
  const flat = $derived(layout.flat);
  const targets = $derived(jumpTargets(flat));
  const currentWs = $derived.by(() => {
    void engine.store.version;
    return engine.store.workspaceId;
  });
  const currentFloor = $derived.by(() => {
    void engine.store.version;
    return engine.store.floorId;
  });

  function readFolders(): Record<string, boolean> {
    try {
      return JSON.parse(localStorage.getItem(FOLDERS_KEY) ?? "{}") ?? {};
    } catch {
      return {};
    }
  }

  function toggleFolder(key: string) {
    collapsed = { ...collapsed, [key]: !collapsed[key] };
    try {
      localStorage.setItem(FOLDERS_KEY, JSON.stringify(collapsed));
    } catch {
      // Private mode.
    }
  }

  onMount(() => {
    const off = workspaces.use();
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", disarm);
    const onStep = (e: Event) => {
      if (visible()) step(Number((e as CustomEvent).detail) > 0 ? 1 : -1);
    };
    window.addEventListener(WORKSPACE_STEP_EVENT, onStep);
    // Terminal starts and exits change `loaded` (dimmed icons, M-A13).
    let timer: ReturnType<typeof setTimeout> | null = null;
    const soon = () => {
      if (timer) return;
      timer = setTimeout(() => {
        timer = null;
        void workspaces.refresh();
      }, 1200);
    };
    const unlisten: Promise<UnlistenFn | null>[] = [
      listen("term://exit", soon).catch(() => null),
      listen("term://state", soon).catch(() => null),
    ];
    void installedIdes().then((l) => (ides = l));
    return () => {
      off();
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", disarm);
      window.removeEventListener(WORKSPACE_STEP_EVENT, onStep);
      if (timer) clearTimeout(timer);
      for (const u of unlisten) void u.then((f) => f?.());
    };
  });

  // Palette asks: "New Workspace", "New Floor".
  let seenWsAsk = canvasUi.newWorkspaceAsk;
  let seenFloorAsk = canvasUi.newFloorAsk;
  $effect(() => {
    if (canvasUi.newWorkspaceAsk !== seenWsAsk) {
      seenWsAsk = canvasUi.newWorkspaceAsk;
      startCreate();
    }
  });
  $effect(() => {
    if (canvasUi.newFloorAsk !== seenFloorAsk) {
      seenFloorAsk = canvasUi.newFloorAsk;
      startFloor(currentWs);
    }
  });

  function visible(): boolean {
    return !!host && host.isConnected && host.offsetParent !== null;
  }

  // ---- keyboard --------------------------------------------------------

  // ⌘ tapped twice (nothing else pressed) arms a digit jump (Maestri).
  let metaDownAt = 0;
  let metaClean = false;
  let lastTap = 0;
  let armTimer: ReturnType<typeof setTimeout> | null = null;

  function disarm() {
    sidebarUi.jumpArmed = false;
    if (armTimer) clearTimeout(armTimer);
    armTimer = null;
  }

  function onKeyUp(e: KeyboardEvent) {
    if (e.key !== "Meta" || !mac) return;
    const now = performance.now();
    if (!metaClean || now - metaDownAt > 350) {
      lastTap = 0;
      return;
    }
    if (now - lastTap < 450) {
      lastTap = 0;
      if (!visible()) return;
      sidebarUi.jumpArmed = true;
      if (armTimer) clearTimeout(armTimer);
      armTimer = setTimeout(disarm, 2500);
    } else {
      lastTap = now;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Meta") {
      metaDownAt = performance.now();
      metaClean = !e.shiftKey && !e.altKey && !e.ctrlKey;
      return;
    }
    metaClean = false;
    if (!visible()) return;
    const digitCode = /^Digit([1-9])$/.exec(e.code)?.[1] ?? (/^[1-9]$/.test(e.key) ? e.key : undefined);
    if (sidebarUi.jumpArmed) {
      disarm();
      if (digitCode && !e.shiftKey && !e.altKey && !e.ctrlKey) {
        stop(e);
        const ws = targets.get(Number(digitCode));
        if (ws) go(ws);
        return;
      }
    }
    const primary = mac ? e.metaKey : e.ctrlKey;
    // ⌃1–⌃9 on macOS; Alt+1–9 elsewhere (Ctrl+digit is ⌘1–9 there).
    const switchMod = mac ? e.ctrlKey && !e.metaKey && !e.altKey : e.altKey && !e.ctrlKey && !e.metaKey;
    const digit = /^Digit([1-9])$/.exec(e.code)?.[1];
    if (switchMod && digit && !e.shiftKey) {
      const ws = targets.get(Number(digit));
      if (ws) {
        stop(e);
        go(ws);
      }
      return;
    }
    if (primary && !e.altKey && !e.shiftKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      if (isEditableTarget(e.target)) return;
      stop(e);
      step(e.key === "ArrowDown" ? 1 : -1);
      return;
    }
    // ⌘] / ⌘[ : next / previous workspace (View menu in the Maestri).
    const bracket =
      e.key === "]" || e.code === "BracketRight" ? 1 : e.key === "[" || e.code === "BracketLeft" ? -1 : 0;
    if (primary && bracket && !e.altKey && !e.shiftKey && !isEditableTarget(e.target)) {
      stop(e);
      step(bracket);
      return;
    }
    if (primary && e.shiftKey && !e.altKey && e.key.toLowerCase() === "n") {
      stop(e);
      startCreate();
      return;
    }
    if (primary && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s" && !isEditableTarget(e.target)) {
      stop(e);
      canvasUi.toggleSidebar();
    }
  }

  function stop(e: KeyboardEvent) {
    e.preventDefault();
    e.stopImmediatePropagation();
  }

  function step(dir: number) {
    if (!flat.length) return;
    const i = flat.findIndex((w) => w.id === currentWs);
    const next = flat[(i + dir + flat.length) % flat.length];
    if (next) go(next);
  }

  function go(ws: WorkspaceSummary, floorId?: string) {
    menu = null;
    railHold = null;
    void switchTo(engine, ws.id, floorId ?? ws.activeFloorId ?? ws.floors[0]?.id);
  }

  // ---- create / edit ---------------------------------------------------

  async function focusName() {
    await tick();
    nameInput?.focus();
    // The canvas may take focus back in the same frame (terminal input).
    requestAnimationFrame(() => {
      if (nameInput && document.activeElement !== nameInput) nameInput.focus();
    });
  }

  async function startCreate() {
    if (!canvasUi.sidebarOpen) canvasUi.toggleSidebar();
    form = {
      id: null,
      name: "",
      workingDirectory: "",
      icon: "",
      color: COLORS[list.length % COLORS.length],
      groupName: "",
      folderName: "",
      ide: "",
      jumpNumber: "",
      instructions: "",
      instructionsLoaded: "",
      syncAgentFiles: false,
    };
    await focusName();
  }

  async function startEdit(ws: Ws) {
    menu = null;
    if (!canvasUi.sidebarOpen) canvasUi.toggleSidebar();
    form = {
      id: ws.id,
      name: ws.name,
      workingDirectory: ws.workingDirectory ?? "",
      icon: ws.icon ?? "",
      color: ws.color ?? COLORS[0],
      groupName: ws.groupName ?? "",
      folderName: ws.folderName ?? "",
      ide: ws.ide ?? "",
      jumpNumber: ws.jumpNumber ? String(ws.jumpNumber) : "",
      instructions: "",
      instructionsLoaded: "",
      syncAgentFiles: !!ws.syncAgentFiles,
    };
    await focusName();
    if (ws.workingDirectory) {
      const r = await agentFiles(ws.id).catch(() => null);
      if (form && form.id === ws.id && r?.text != null) {
        form.instructions = r.text;
        form.instructionsLoaded = r.text;
      }
    }
  }

  async function chooseFolder() {
    if (!form) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, defaultPath: form.workingDirectory || undefined });
      if (typeof picked === "string" && form) {
        form.workingDirectory = picked;
        if (!form.name.trim()) form.name = picked.split(/[\\/]/).filter(Boolean).pop() ?? "";
      }
    } catch {
      // Dialog unavailable: the text field still works.
    }
  }

  async function chooseIconImage() {
    if (!form) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg", "ico"] }],
      });
      if (typeof picked === "string" && form) form.icon = `file:${picked}`;
    } catch {
      // Dialog unavailable.
    }
  }

  /** Fields beyond the create input; null clears. */
  function extras(f: Form): Record<string, unknown> {
    const patch: Record<string, unknown> = {
      groupName: f.groupName.trim() || null,
      folderName: f.folderName.trim() || null,
      ide: f.ide || null,
      jumpNumber: f.jumpNumber ? Number(f.jumpNumber) : null,
      syncAgentFiles: f.syncAgentFiles,
    };
    if (f.workingDirectory.trim() && f.instructions !== f.instructionsLoaded) patch.instructions = f.instructions;
    return patch;
  }

  async function submitForm() {
    const f = form;
    if (!f) return;
    const name = f.name.trim() || f.workingDirectory.split(/[\\/]/).filter(Boolean).pop() || tr("canvas.sidebar.untitled", "Workspace");
    error = null;
    if (f.id) {
      await workspaces.update(f.id, {
        name,
        icon: f.icon.trim() || undefined,
        color: f.color,
        workingDirectory: f.workingDirectory.trim() || undefined,
        ...extras(f),
      } as Partial<WorkspaceSummary>);
      form = null;
      await workspaces.refresh();
      return;
    }
    const ws = await workspaces.create({
      name,
      icon: f.icon.trim() || undefined,
      color: f.color,
      workingDirectory: f.workingDirectory.trim() || undefined,
    });
    if (!ws) {
      error = workspaces.error;
      return;
    }
    const more = extras(f);
    if (more.groupName || more.folderName || more.ide || more.jumpNumber || more.syncAgentFiles || more.instructions) {
      await workspaces.update(ws.id, more as Partial<WorkspaceSummary>);
    }
    form = null;
    go(ws);
  }

  function formKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      form = null;
    } else if (e.key === "Enter" && !(e.target instanceof HTMLTextAreaElement)) {
      e.preventDefault();
      void submitForm();
    }
  }

  // ---- rename / menu / delete -----------------------------------------

  async function startRename(ws: WorkspaceSummary) {
    menu = null;
    renaming = ws.id;
    renameText = ws.name;
    await tick();
    listEl?.querySelector<HTMLInputElement>(".rename")?.select();
  }

  function commitRename() {
    const id = renaming;
    renaming = null;
    const name = renameText.trim();
    if (id && name && name !== workspaces.byId(id)?.name) void workspaces.update(id, { name });
  }

  function renameKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitRename();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      renaming = null;
    }
  }

  function openMenu(e: MouseEvent, ws: Ws) {
    e.preventDefault();
    e.stopPropagation();
    railHold = null;
    const r = host?.getBoundingClientRect();
    menu = { ws, x: e.clientX - (r?.left ?? 0), y: e.clientY - (r?.top ?? 0) };
  }

  async function copyPath(ws: WorkspaceSummary) {
    menu = null;
    if (ws.workingDirectory) await navigator.clipboard?.writeText(ws.workingDirectory).catch(() => {});
  }

  async function ide(ws: Ws) {
    menu = null;
    try {
      const r = await openInIde(ws.id);
      showToast("success", tr("canvas.sidebar.openedIn", "Opened in {{ide}}", { ide: IDE_LABELS[r.ide] ?? r.ide }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  // ---- agent files ------------------------------------------------------

  async function startSync(ws: Ws, winner?: string) {
    menu = null;
    const files = ws.agentFiles?.files?.filter((f) => f.exists).map((f) => f.name) ?? [];
    const w = winner ?? files[0] ?? "CLAUDE.md";
    sync = { ws, winner: w, changes: null };
    try {
      const r = await agentFiles(ws.id, w, false);
      if (sync && sync.ws.id === ws.id && sync.winner === w) sync.changes = r.resolve?.changes ?? [];
    } catch (e) {
      sync = null;
      showToast("error", String(e));
    }
  }

  async function applySync() {
    const s = sync;
    if (!s) return;
    sync = null;
    try {
      await agentFiles(s.ws.id, s.winner, true);
      await workspaces.refresh();
      showToast("success", tr("canvas.sidebar.syncDone", "Agent files now match {{file}}", { file: s.winner }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  // ---- export / import / backups ---------------------------------------

  async function exportWs(ws: WorkspaceSummary) {
    menu = null;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        defaultPath: `${ws.name.replace(/[\\/:*?"<>|]/g, "-")}.omniget`,
        filters: [{ name: "OmniGet workspace", extensions: ["omniget"] }],
      });
      if (typeof path !== "string") return;
      const r = await invoke<{ nodes: number; omitted: unknown[] }>("canvas_workspace_export", {
        workspaceId: ws.id,
        path,
      });
      showToast(
        "success",
        tr("canvas.workspaceIo.exported", "Exported {{nodes}} items ({{left}} left out)", {
          nodes: r.nodes,
          left: r.omitted.length,
        }),
      );
    } catch (e) {
      error = String(e);
    }
  }

  async function importWs() {
    menu = null;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({
        multiple: false,
        filters: [{ name: "OmniGet workspace", extensions: ["omniget", "json"] }],
      });
      if (typeof path === "string") ioPanel = { kind: "import", path };
    } catch (e) {
      error = String(e);
    }
  }

  async function afterImport(id: string) {
    await workspaces.refresh();
    const ws = workspaces.list.find((w) => w.id === id);
    if (ws) go(ws);
  }

  async function doDelete(ws: WorkspaceSummary) {
    confirmDelete = null;
    menu = null;
    const wasCurrent = ws.id === currentWs;
    const ok = await workspaces.remove(ws.id);
    if (!ok) {
      error = workspaces.error;
      return;
    }
    if (wasCurrent) {
      await workspaces.refresh();
      const next = layoutWorkspaces(workspaces.list).flat[0];
      if (next) go(next);
    }
  }

  // ---- floors ----------------------------------------------------------

  async function startFloor(wsId: string) {
    if (!wsId) return;
    expanded = { ...expanded, [wsId]: true };
    newFloor = wsId;
    floorName = "";
    await tick();
    listEl?.querySelector<HTMLInputElement>(".floor-input")?.focus();
  }

  async function commitFloor() {
    const wsId = newFloor;
    newFloor = null;
    const name = floorName.trim();
    if (!wsId || !name) return;
    try {
      const res = await invoke<{ floor: { id: string } }>("canvas_floor_create", { workspaceId: wsId, input: { name } });
      const floor = res?.floor;
      await workspaces.refresh();
      const ws = workspaces.byId(wsId);
      if (ws && floor?.id) go(ws, floor.id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function floorKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commitFloor();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      newFloor = null;
    }
  }

  // ---- reorder (pointer drag; HTML5 DnD is unreliable in the webview) ----

  function rowDown(e: PointerEvent, ws: WorkspaceSummary) {
    if (e.button !== 0 || renaming === ws.id) return;
    drag = { id: ws.id, y0: e.clientY, over: flat.findIndex((w) => w.id === ws.id), moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function rowMove(e: PointerEvent) {
    if (!drag || !listEl) return;
    if (!drag.moved && Math.abs(e.clientY - drag.y0) < 5) return;
    drag.moved = true;
    const rows = Array.from(listEl.querySelectorAll<HTMLElement>("[data-ws-row]"));
    let over = rows.length;
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i].getBoundingClientRect();
      if (e.clientY < r.top + r.height / 2) {
        over = i;
        break;
      }
    }
    // Rows of collapsed folders are not in the DOM: map DOM index to flat index.
    const id = rows[over]?.dataset.wsId;
    drag.over = id ? flat.findIndex((w) => w.id === id) : flat.length;
  }

  /** Moves `id` to flat position `to` (before the row now there) and adopts the group/folder around it. */
  async function placeAt(id: string, to: number) {
    const moving = flat.find((w) => w.id === id);
    if (!moving) return;
    const rest = flat.filter((w) => w.id !== id);
    const at = Math.max(0, Math.min(rest.length, to));
    const neighbour = rest[at - 1] ?? rest[at];
    const ids = rest.map((w) => w.id);
    ids.splice(at, 0, id);
    if (neighbour && ((neighbour.groupName ?? "") !== (moving.groupName ?? "") || (neighbour.folderName ?? "") !== (moving.folderName ?? ""))) {
      await workspaces.update(id, {
        groupName: neighbour.groupName ?? null,
        folderName: neighbour.folderName ?? null,
      } as unknown as Partial<WorkspaceSummary>);
    }
    if (ids.some((x, i) => x !== flat[i]?.id)) await workspaces.reorder(ids);
  }

  function rowUp(ws: WorkspaceSummary) {
    const d = drag;
    drag = null;
    if (!d) return;
    if (!d.moved) {
      go(ws);
      return;
    }
    const from = flat.findIndex((w) => w.id === d.id);
    const to = d.over > from ? d.over - 1 : d.over;
    if (to !== from) void placeAt(d.id, to);
  }

  /** Alt+↑/↓: move the workspace one row, keep focus on it and announce it. */
  async function moveBy(ws: Ws, dir: number) {
    const from = flat.findIndex((w) => w.id === ws.id);
    const to = from + dir;
    if (from < 0 || to < 0 || to >= flat.length) return;
    const rest = flat.filter((w) => w.id !== ws.id);
    const neighbour = rest[dir > 0 ? to - 1 : to];
    // Crossing into another group/folder adopts it first (one step per press).
    if (neighbour && ((neighbour.groupName ?? "") !== (ws.groupName ?? "") || (neighbour.folderName ?? "") !== (ws.folderName ?? ""))) {
      await workspaces.update(ws.id, {
        groupName: neighbour.groupName ?? null,
        folderName: neighbour.folderName ?? null,
      } as unknown as Partial<WorkspaceSummary>);
    }
    const ids = rest.map((w) => w.id);
    ids.splice(to, 0, ws.id);
    await workspaces.reorder(ids);
    sidebarUi.announce = tr("canvas.sidebar.moved", "{{name}} moved to position {{pos}} of {{total}}", {
      name: ws.name,
      pos: to + 1,
      total: flat.length,
    });
    await tick();
    listEl?.querySelector<HTMLElement>(`[data-ws-id="${CSS.escape(ws.id)}"]`)?.focus();
  }

  function rowKey(e: KeyboardEvent, ws: Ws) {
    if (e.altKey && !e.metaKey && !e.ctrlKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      e.stopPropagation();
      void moveBy(ws, e.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (e.key === "Enter") go(ws);
    if (e.key === "F2") void startRename(ws);
  }

  function numberLabel(n: number): string {
    return mac ? `⌃${n}` : `Alt+${n}`;
  }

  const dimmed = (ws: Ws) => ws.loaded === false && (ws.terminals?.length ?? ws.terminalCount ?? 0) > 0;

  // ---- mini rail: hover label and hold-to-list ---------------------------

  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  let held = false;

  function railDown(e: PointerEvent, ws: Ws) {
    if (e.button !== 0) return;
    held = false;
    const y = (e.currentTarget as HTMLElement).offsetTop;
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = setTimeout(() => {
      held = true;
      railHover = null;
      railHold = { ws, y };
    }, 450);
  }

  function railUp(ws: Ws) {
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = null;
    if (!held) go(ws);
    held = false;
  }

  function railCancel() {
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = null;
  }

  function openTerminal(ws: Ws, term: NonNullable<Ws["terminals"]>[number]) {
    railHold = null;
    void goTo(engine, { workspaceId: ws.id, floorId: term.floorId, nodeId: term.nodeId }, { focus: true });
  }
</script>

<svelte:window onpointerdown={(e) => {
  const el = e.target as HTMLElement;
  if (menu && !el.closest(".menu")) menu = null;
  if (railHold && !el.closest(".hold-list") && !el.closest(".rail-ws")) railHold = null;
}} />

{#snippet badge(ws: Ws, big: boolean)}
  {@const ic = iconOf(ws)}
  {@const n = numberOf(targets, ws.id)}
  {#if ic.kind === "image" && wsIcons.url(ws)}
    <img class="icon-img" src={wsIcons.url(ws)} alt="" draggable="false" />
  {:else}
    <span class="icon-text" class:big>{ic.kind === "text" ? ic.text : (ws.name.trim().charAt(0) || "?").toUpperCase()}</span>
  {/if}
  {#if sidebarUi.jumpArmed && n}<span class="jump-num">{n}</span>{/if}
{/snippet}

{#snippet row(ws: Ws)}
  {@const active = ws.id === currentWs}
  {@const open = active || expanded[ws.id]}
  {@const i = flat.findIndex((w) => w.id === ws.id)}
  {@const n = numberOf(targets, ws.id)}
  {#if drag?.moved && drag.over === i && drag.id !== ws.id}<div class="drop-line"></div>{/if}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div
    class="ws"
    class:active
    class:unloaded={dimmed(ws)}
    class:dragging={drag?.id === ws.id && drag.moved}
    data-ws-row
    data-ws-id={ws.id}
    role="button"
    tabindex="0"
    aria-current={active ? "true" : undefined}
    aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
    onpointerdown={(e) => rowDown(e, ws)}
    onpointermove={rowMove}
    onpointerup={() => rowUp(ws)}
    oncontextmenu={(e) => openMenu(e, ws)}
    ondblclick={() => void startRename(ws)}
    onkeydown={(e) => rowKey(e, ws)}
  >
    <span class="badge" style:background={iconOf(ws).kind === "image" ? "transparent" : (ws.color ?? "var(--accent)")}>{@render badge(ws, false)}</span>
    {#if renaming === ws.id}
      <input
        class="rename"
        bind:value={renameText}
        onkeydown={renameKey}
        onblur={commitRename}
        onpointerdown={(e) => e.stopPropagation()}
      />
    {:else}
      <span
        class="name"
        title={dimmed(ws) ? `${ws.workingDirectory ?? ws.name} · ${tr("canvas.sidebar.unloaded", "Unloaded")}` : (ws.workingDirectory ?? ws.name)}
        >{ws.isPinned ? "• " : ""}{ws.name}</span
      >
    {/if}
    {#if ws.agentFiles && !ws.agentFiles.synced}
      <button
        type="button"
        class="warn"
        title={tr("canvas.sidebar.agentFilesDiverge", "CLAUDE.md, AGENTS.md and GEMINI.md differ: {{files}}", { files: ws.agentFiles.divergent.join(", ") })}
        onpointerdown={(e) => e.stopPropagation()}
        onclick={(e) => {
          e.stopPropagation();
          void startSync(ws);
        }}>≠</button
      >
    {/if}
    {#if ws.terminalCount}<span class="count" title={tr("canvas.sidebar.terminals", "Terminals")}>{ws.terminalCount}</span>{/if}
    {#if n}<kbd>{numberLabel(n)}</kbd>{/if}
    <button
      type="button"
      class="chev"
      aria-label={tr("canvas.sidebar.floors", "Floors")}
      aria-expanded={!!open}
      onpointerdown={(e) => e.stopPropagation()}
      onclick={(e) => {
        e.stopPropagation();
        expanded = { ...expanded, [ws.id]: !expanded[ws.id] };
      }}>{open ? "▾" : "▸"}</button
    >
  </div>
  {#if open}
    <div class="floors">
      {#each ws.floors as f (f.id)}
        <button type="button" class="floor" class:on={active && f.id === currentFloor} onclick={() => go(ws, f.id)}>
          <span class="fdot" style:background={f.color ?? "var(--text-muted)"}></span>
          <span class="fname">{f.name || f.id}</span>
          {#if f.branch}<span class="branch">{f.branch}</span>{/if}
        </button>
      {/each}
      {#if newFloor === ws.id}
        <input
          class="floor-input"
          bind:value={floorName}
          onkeydown={floorKey}
          onblur={() => void commitFloor()}
          placeholder={tr("canvas.sidebar.floorName", "Floor name")}
        />
      {:else}
        <button type="button" class="floor add" onclick={() => void startFloor(ws.id)}>+ {tr("canvas.sidebar.newFloor", "New floor")}</button>
      {/if}
    </div>
  {/if}
{/snippet}

<aside
  class="sidebar"
  class:closed={!canvasUi.sidebarOpen}
  class:hidden={!canvasUi.sidebarOpen && !sidebarUi.mini}
  data-canvas-chrome
  bind:this={host}
  aria-label={tr("canvas.sidebar.title", "Workspaces")}
>
  <div class="sr-only" aria-live="polite">{sidebarUi.announce}</div>
  {#if canvasUi.sidebarOpen}
    <div class="top">
      <span class="heading">{tr("canvas.sidebar.title", "Workspaces")}</span>
      <button type="button" class="icon-btn" title={tr("canvas.sidebar.new", "New workspace (⌘⇧N)")} onclick={() => void startCreate()}>+</button>
      <button type="button" class="icon-btn" title={tr("canvas.sidebar.hide", "Hide sidebar (⌘S)")} onclick={() => canvasUi.toggleSidebar()}>‹</button>
    </div>

    {#if form}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="form" onkeydown={formKey}>
        <label>
          <span>{tr("canvas.sidebar.name", "Name")}</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:this={nameInput} bind:value={form.name} autofocus placeholder={tr("canvas.sidebar.namePlaceholder", "My project")} />
        </label>
        <label>
          <span>{tr("canvas.sidebar.folder", "Working directory")}</span>
          <div class="row-input">
            <input bind:value={form.workingDirectory} placeholder="/path/to/project" spellcheck="false" />
            <button type="button" onclick={() => void chooseFolder()}>{tr("canvas.sidebar.choose", "Choose…")}</button>
          </div>
        </label>
        <label>
          <span>{tr("canvas.sidebar.iconAny", "Icon (letters, emoji or image)")}</span>
          <div class="row-input">
            <input bind:value={form.icon} placeholder="🎼" spellcheck="false" />
            <button type="button" onclick={() => void chooseIconImage()}>{tr("canvas.sidebar.image", "Image…")}</button>
          </div>
        </label>
        <div class="swatches" role="radiogroup" aria-label={tr("canvas.sidebar.color", "Color")}>
          {#each COLORS as c}
            <button
              type="button"
              class="swatch"
              class:on={form.color === c}
              style:background={c}
              role="radio"
              aria-checked={form.color === c}
              aria-label={c}
              onclick={() => form && (form.color = c)}
            ></button>
          {/each}
        </div>
        <div class="pair">
          <label>
            <span>{tr("canvas.sidebar.group", "Group")}</span>
            <input bind:value={form.groupName} placeholder={tr("canvas.sidebar.groupPlaceholder", "Work")} />
          </label>
          <label>
            <span>{tr("canvas.sidebar.folderGroup", "Folder")}</span>
            <input bind:value={form.folderName} placeholder="API" />
          </label>
        </div>
        <div class="pair">
          <label>
            <span>{tr("canvas.sidebar.ide", "Editor")}</span>
            <select bind:value={form.ide}>
              <option value="">{tr("canvas.sidebar.ideAuto", "First found")}</option>
              {#each Object.keys(IDE_LABELS) as id}
                {#if ides.includes(id) || form.ide === id}<option value={id}>{IDE_LABELS[id]}</option>{/if}
              {/each}
            </select>
          </label>
          <label>
            <span>{tr("canvas.sidebar.jumpNumber", "Number")}</span>
            <select bind:value={form.jumpNumber}>
              <option value="">{tr("canvas.sidebar.jumpAuto", "By position")}</option>
              {#each [1, 2, 3, 4, 5, 6, 7, 8, 9] as n}<option value={String(n)}>{numberLabel(n)}</option>{/each}
            </select>
          </label>
        </div>
        {#if form.workingDirectory.trim()}
          <label>
            <span>{tr("canvas.sidebar.instructions", "Agent instructions (CLAUDE.md)")}</span>
            <textarea bind:value={form.instructions} rows="4" spellcheck="false"></textarea>
          </label>
          <label class="check">
            <input type="checkbox" bind:checked={form.syncAgentFiles} />
            <span>{tr("canvas.sidebar.syncAgentFiles", "Keep AGENTS.md and GEMINI.md the same")}</span>
          </label>
        {/if}
        {#if error}<p class="error">{error}</p>{/if}
        <div class="form-actions">
          <button type="button" onclick={() => (form = null)}>{tr("canvas.sidebar.cancel", "Cancel")}</button>
          <button type="button" class="primary" onclick={() => void submitForm()}>
            {form.id ? tr("canvas.sidebar.save", "Save") : tr("canvas.sidebar.create", "Create")}
          </button>
        </div>
      </div>
    {/if}

    <div class="list" bind:this={listEl}>
      {#each layout.sections as sec (sec.group)}
        {#if sec.group}<div class="ws-group" role="separator">{sec.group}</div>{/if}
        {#each sec.items as it (it.kind === "ws" ? it.ws.id : it.key)}
          {#if it.kind === "ws"}
            {@render row(it.ws)}
          {:else}
            {@const shut = collapsed[it.key] && !it.list.some((w) => w.id === currentWs)}
            <button type="button" class="folder" aria-expanded={!shut} onclick={() => toggleFolder(it.key)}>
              <span class="fchev">{shut ? "▸" : "▾"}</span>
              <span class="fname">{it.name}</span>
              <span class="count">{it.list.length}</span>
            </button>
            {#if !shut}
              <div class="in-folder">
                {#each it.list as ws (ws.id)}{@render row(ws)}{/each}
              </div>
            {/if}
          {/if}
        {/each}
      {/each}
      {#if drag?.moved && drag.over >= flat.length && drag.id !== flat[flat.length - 1]?.id}<div class="drop-line"></div>{/if}
      {#if !list.length && workspaces.loaded}
        <p class="empty">{tr("canvas.sidebar.empty", "No workspaces yet")}</p>
      {/if}
    </div>
    {#if error && !form}<p class="error">{error}</p>{/if}
  {:else if sidebarUi.mini}
    <div class="rail">
      <button type="button" class="icon-btn" title={tr("canvas.sidebar.show", "Show sidebar (⌘S)")} onclick={() => canvasUi.toggleSidebar()}>›</button>
      {#each flat as ws (ws.id)}
        <button
          type="button"
          class="rail-ws"
          class:active={ws.id === currentWs}
          class:unloaded={dimmed(ws)}
          aria-label={ws.name}
          style:background={iconOf(ws).kind === "image" ? "transparent" : (ws.color ?? "var(--accent)")}
          onpointerenter={(e) => {
            if (!railHold) railHover = { ws, y: (e.currentTarget as HTMLElement).offsetTop };
          }}
          onpointerleave={() => {
            railHover = null;
            railCancel();
          }}
          onpointerdown={(e) => railDown(e, ws)}
          onpointerup={() => railUp(ws)}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              go(ws);
            }
          }}
          oncontextmenu={(e) => openMenu(e, ws)}>{@render badge(ws, true)}</button
        >
      {/each}
      <button type="button" class="icon-btn" title={tr("canvas.sidebar.new", "New workspace (⌘⇧N)")} onclick={() => void startCreate()}>+</button>
    </div>
    {#if railHover}
      {@const n = numberOf(targets, railHover.ws.id)}
      <div class="rail-label" style:top="{railHover.y + 4}px" role="tooltip">
        <strong>{railHover.ws.name}</strong>
        {#if n}<kbd>{numberLabel(n)}</kbd>{/if}
        {#if dimmed(railHover.ws)}<span class="muted">{tr("canvas.sidebar.unloaded", "Unloaded")}</span>{/if}
      </div>
    {/if}
    {#if railHold}
      {@const hw = railHold.ws}
      <div class="hold-list" style:top="{railHold.y}px" role="menu">
        <div class="hold-title">{hw.name}</div>
        {#each hw.terminals ?? [] as term (term.nodeId)}
          <button type="button" role="menuitem" onclick={() => openTerminal(hw, term)}>
            <span class="dot" class:on={term.running}></span>
            <span class="fname">{term.title || term.agent || term.nodeId.slice(0, 6)}</span>
            {#if hw.floors.length > 1}<span class="branch">{hw.floors.find((f) => f.id === term.floorId)?.name ?? ""}</span>{/if}
          </button>
        {:else}
          <p class="empty">{tr("canvas.sidebar.noTerminals", "No terminals")}</p>
        {/each}
      </div>
    {/if}
  {:else}
    <button type="button" class="icon-btn reopen" title={tr("canvas.sidebar.show", "Show sidebar (⌘S)")} onclick={() => canvasUi.toggleSidebar()}>›</button>
  {/if}

  {#if menu}
    {@const ws = menu.ws}
    <div class="menu" role="menu" style:left="{menu.x}px" style:top="{menu.y}px">
      <button type="button" role="menuitem" onclick={() => go(ws)}>{tr("canvas.sidebar.open", "Open")}</button>
      <button type="button" role="menuitem" onclick={() => void startRename(ws)}>{tr("canvas.sidebar.rename", "Rename")}</button>
      <button type="button" role="menuitem" onclick={() => void startEdit(ws)}>{tr("canvas.sidebar.edit", "Edit…")}</button>
      <button
        type="button"
        role="menuitem"
        onclick={() => {
          menu = null;
          void workspaces.update(ws.id, { isPinned: !ws.isPinned });
        }}>{ws.isPinned ? tr("canvas.sidebar.unpin", "Unpin") : tr("canvas.sidebar.pin", "Pin")}</button
      >
      <button type="button" role="menuitem" onclick={() => void startFloor(ws.id)}>{tr("canvas.sidebar.newFloor", "New floor")}</button>
      {#if ws.workingDirectory}
        <button type="button" role="menuitem" onclick={() => void copyPath(ws)}>{tr("canvas.sidebar.copyPath", "Copy path")}</button>
        <button type="button" role="menuitem" onclick={() => void ide(ws)}>
          {ws.ide
            ? tr("canvas.sidebar.openInIdeNamed", "Open in {{ide}}", { ide: IDE_LABELS[ws.ide] ?? ws.ide })
            : tr("canvas.sidebar.openInIde", "Open in IDE")}
        </button>
      {/if}
      {#if ws.agentFiles && !ws.agentFiles.synced}
        <button type="button" role="menuitem" onclick={() => void startSync(ws)}>{tr("canvas.sidebar.syncAgentFilesMenu", "Sync agent files…")}</button>
      {/if}
      <div class="sep"></div>
      <button type="button" role="menuitem" onclick={() => void exportWs(ws)}>{tr("canvas.workspaceIo.export", "Export…")}</button>
      <button type="button" role="menuitem" onclick={() => void importWs()}>{tr("canvas.workspaceIo.importMenu", "Import…")}</button>
      <button
        type="button"
        role="menuitem"
        onclick={() => {
          menu = null;
          ioPanel = { kind: "backups", workspaceId: ws.id, name: ws.name };
        }}>{tr("canvas.workspaceIo.backups", "Backups…")}</button
      >
      <div class="sep"></div>
      {#if confirmDelete === ws.id}
        <button type="button" role="menuitem" class="danger" onclick={() => void doDelete(ws)}>
          {tr("canvas.sidebar.confirmDelete", "Delete “{{name}}”? Click again", { name: ws.name })}
        </button>
      {:else}
        <button type="button" role="menuitem" class="danger" disabled={list.length <= 1} onclick={() => (confirmDelete = ws.id)}>
          {tr("canvas.sidebar.delete", "Delete…")}
        </button>
      {/if}
    </div>
  {/if}
  {#if sync}
    {@const sy = sync}
    <div class="sync" role="dialog" aria-label={tr("canvas.sidebar.syncTitle", "Sync agent files")}>
      <div class="hold-title">{tr("canvas.sidebar.syncTitle", "Sync agent files")} · {sy.ws.name}</div>
      <label class="check">
        <span>{tr("canvas.sidebar.syncWinner", "Keep this version")}</span>
        <select value={sy.winner} onchange={(e) => void startSync(sy.ws, (e.currentTarget as HTMLSelectElement).value)}>
          {#each sy.ws.agentFiles?.files?.filter((f) => f.exists) ?? [] as f (f.name)}<option value={f.name}>{f.name}</option>{/each}
        </select>
      </label>
      {#if sy.changes === null}
        <p class="muted">…</p>
      {:else if !sy.changes.length}
        <p class="muted">{tr("canvas.sidebar.syncNothing", "Nothing changes.")}</p>
      {:else}
        {#each sy.changes as c (c.file)}
          <p class="change">
            {tr("canvas.sidebar.syncChange", "{{file}}: loses {{lost}} lines, gains {{added}}", { file: c.file, lost: c.linesLost, added: c.linesAdded })}
          </p>
          {#each c.lost.slice(0, 4) as l}<pre class="lost">− {l}</pre>{/each}
        {/each}
      {/if}
      <div class="form-actions">
        <button type="button" onclick={() => (sync = null)}>{tr("canvas.sidebar.cancel", "Cancel")}</button>
        <button type="button" class="primary" disabled={sy.changes === null} onclick={() => void applySync()}>{tr("canvas.sidebar.syncApply", "Sync")}</button>
      </div>
    </div>
  {/if}
  {#if ioPanel}
    <WorkspaceIoPanel
      mode={ioPanel}
      onclose={() => (ioPanel = null)}
      onimported={(id) => void afterImport(id)}
      onrestored={() => void workspaces.refresh()}
    />
  {/if}
</aside>

<style>
  .sidebar {
    position: absolute;
    top: 0;
    left: 0;
    bottom: 0;
    z-index: 12;
    width: 232px;
    display: flex;
    flex-direction: column;
    background: var(--popup-bg, var(--surface));
    border-right: 1px solid var(--border);
    color: var(--text);
    font-size: 13px;
    user-select: none;
  }
  .sidebar.closed {
    width: 48px;
  }
  .sidebar.hidden {
    width: 0;
    border-right: none;
    overflow: visible;
  }
  .reopen {
    position: absolute;
    top: 8px;
    left: 6px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .ws.unloaded .badge,
  .ws.unloaded .name,
  .rail-ws.unloaded {
    opacity: 0.45;
    filter: grayscale(0.6);
  }
  .rail-ws.unloaded.active {
    opacity: 0.7;
  }
  .badge,
  .rail-ws {
    position: relative;
    overflow: visible;
  }
  /* Laid over the whole badge: as a grid item a 64×64 image got stretched
     into a tall pill in the full sidebar (M-A4). */
  .icon-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
    object-fit: cover;
    border-radius: inherit;
  }
  .icon-text.big {
    font-size: 14px;
  }
  .jump-num {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    border-radius: inherit;
    background: color-mix(in srgb, #000 55%, transparent);
    color: #fff;
    font-size: 13px;
    font-weight: 800;
  }
  /* Not `.group`: that is a global card primitive (lib/style/primitives.css)
     and drew a pill around the label (M-A6). */
  .ws-group {
    margin: 10px 6px 2px;
    padding-top: 6px;
    border-top: 1px solid var(--border);
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 5px 6px;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }
  .folder:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .fchev {
    width: 12px;
    font-size: 10px;
    color: var(--text-muted);
  }
  .in-folder {
    padding-left: 10px;
    border-left: 1px solid var(--border);
    margin-left: 11px;
  }
  .warn {
    border: none;
    background: color-mix(in srgb, var(--warning, #e0a040) 22%, transparent);
    color: var(--warning, #e0a040);
    border-radius: 5px;
    font-size: 11px;
    font-weight: 700;
    padding: 0 5px;
    cursor: pointer;
  }
  .pair {
    display: flex;
    gap: 6px;
  }
  .pair label {
    flex: 1;
    min-width: 0;
  }
  .form select,
  .form textarea,
  .sync select {
    font: inherit;
    font-size: 12px;
    padding: 4px 6px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    min-width: 0;
  }
  .form textarea {
    font-family: var(--font-mono, monospace);
    resize: vertical;
  }
  .form label.check,
  .sync label.check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .rail-label,
  .hold-list,
  .sync {
    position: absolute;
    left: 52px;
    z-index: 21;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
  }
  .rail-label {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 9px;
    white-space: nowrap;
    pointer-events: none;
  }
  .hold-list {
    min-width: 200px;
    max-height: 60vh;
    overflow-y: auto;
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .hold-list button {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 5px 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .hold-list button:hover {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .hold-title {
    padding: 4px 8px 6px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--text-muted);
    opacity: 0.5;
  }
  .dot.on {
    background: var(--success, #5ccf7a);
    opacity: 1;
  }
  .muted {
    color: var(--text-muted);
    font-size: 11px;
  }
  .sync {
    top: 60px;
    left: 12px;
    width: 300px;
    padding: 8px 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .sync .form-actions button {
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .sync .form-actions button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent, #fff);
  }
  .change {
    margin: 4px 0 0;
    font-size: 12px;
  }
  .lost {
    margin: 0;
    font: 11px var(--font-mono, monospace);
    color: var(--danger, #d96a62);
    white-space: pre-wrap;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 10px 8px 6px 12px;
  }
  .heading {
    flex: 1;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
  }
  .icon-btn {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 16px;
    cursor: pointer;
  }
  .icon-btn:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text);
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 2px 6px 10px;
  }
  .ws {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 6px;
    border-radius: 8px;
    cursor: pointer;
    outline: none;
  }
  .ws:hover,
  .ws:focus-visible {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .ws.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .ws.dragging {
    opacity: 0.5;
  }
  .badge {
    flex: none;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    font-size: 12px;
    font-weight: 700;
    color: #fff;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rename {
    flex: 1;
    min-width: 0;
    font: inherit;
    padding: 1px 4px;
    border: 1px solid var(--accent);
    border-radius: 5px;
    background: var(--input-bg, transparent);
    color: var(--text);
    outline: none;
  }
  .count {
    font-size: 11px;
    color: var(--text-muted);
  }
  kbd {
    font: 10px var(--font-mono, monospace);
    color: var(--text-muted);
    opacity: 0.8;
  }
  .chev {
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0 2px;
    font-size: 11px;
  }
  .floors {
    display: flex;
    flex-direction: column;
    padding: 2px 0 6px 28px;
  }
  .floor {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .floor:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .floor.on {
    color: var(--text);
    font-weight: 600;
  }
  .floor.add {
    opacity: 0.75;
  }
  .fdot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: none;
  }
  .fname {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .branch {
    font: 10px var(--font-mono, monospace);
    opacity: 0.8;
  }
  .floor-input {
    margin: 2px 6px;
    font: inherit;
    font-size: 12px;
    padding: 3px 6px;
    border-radius: 6px;
    border: 1px solid var(--accent);
    background: var(--input-bg, transparent);
    color: var(--text);
    outline: none;
  }
  .drop-line {
    height: 2px;
    margin: 0 6px;
    background: var(--accent);
    border-radius: 1px;
  }
  .form {
    margin: 4px 8px 8px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .form label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .form input {
    font: inherit;
    font-size: 12px;
    padding: 5px 7px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    outline: none;
    min-width: 0;
  }
  .form input:focus {
    border-color: var(--accent);
  }
  .row-input {
    display: flex;
    gap: 4px;
  }
  .row-input input {
    flex: 1;
  }
  .form button,
  .row-input button {
    font-size: 12px;
    padding: 4px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .form button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent, #fff);
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .swatches {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .swatch {
    width: 18px !important;
    height: 18px;
    padding: 0 !important;
    border-radius: 50% !important;
    border: 2px solid transparent !important;
  }
  .swatch.on {
    border-color: var(--text) !important;
  }
  .error {
    margin: 0 10px;
    font-size: 11px;
    color: var(--danger, #d96a62);
  }
  .empty {
    color: var(--text-muted);
    font-size: 12px;
    padding: 8px;
  }
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 10px 0;
    overflow-y: auto;
  }
  .rail-ws {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    border: 2px solid transparent;
    color: #fff;
    font-weight: 700;
    font-size: 13px;
    cursor: pointer;
    opacity: 0.8;
  }
  .rail-ws.active {
    border-color: var(--text);
    opacity: 1;
  }
  .menu {
    position: absolute;
    z-index: 20;
    min-width: 180px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
  }
  .menu button {
    text-align: left;
    padding: 6px 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .menu button:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .menu button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .menu .danger {
    color: var(--danger, #d96a62);
  }
  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
