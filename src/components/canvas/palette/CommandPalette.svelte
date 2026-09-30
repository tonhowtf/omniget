<script lang="ts">
  // Batuta Search (study 95 §J, 03 §10): ⌘P / ⌘K opens and closes; Esc goes
  // back one level; closing without running anything gives the focus back.
  // Fuzzy over every node, note body, role, workspace, floor and action, in
  // every workspace and floor; jumping flies the camera there. Modes: Ask…
  // (message any terminal, live preview, ⌘Enter jumps), Check… (read-only
  // live output), Connect to…, Disconnect…, Search Files (a file tree
  // selected), Notifications (⌥⌘A), elements (⌥⌘E / ⇧⌥⌘E), Rename.
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { isEditableTarget, isMac } from "$lib/canvas/interaction/keyboard";
  import { isTerminalKind, terminalIdOf, type CanvasNode } from "$lib/canvas/model/types";
  import { ICONS } from "$lib/canvas/nodes/registry";
  import { TOOL_ICONS } from "$lib/canvas/icons";
  import { stateCss } from "$lib/canvas/state/agents.svelte";
  import { highlightRuns } from "$lib/canvas/palette/fuzzy";
  import {
    PaletteIndex,
    actionItems,
    attentionItems,
    contextItems,
    nodeItems,
    rank,
    roleItems,
    workspaceItems,
    type PaletteItem,
    type Ranked,
    type Section,
  } from "$lib/canvas/palette/items";
  import { goTo, switchTo } from "$lib/canvas/palette/navigate";
  import { workspaces } from "$lib/canvas/palette/workspaces.svelte";
  import { canvasUi } from "$lib/canvas/palette/ui.svelte";
  import { filesSearch, type SearchHit } from "$lib/canvas/files/api";
  import { openEditorBeside } from "$lib/canvas/files/open";
  import { listPartituras, savePartitura, type PartituraSummary } from "$lib/canvas/partituras/api";
  import { searchChats, type ChatHitItem } from "$lib/canvas/palette/chats";
  import Icon from "../chrome/Icon.svelte";

  type Mode =
    | "search"
    | "attention"
    | "elements"
    | "pick"
    | "ask"
    | "check"
    | "files"
    | "rename"
    | "edit"
    | "workspaces"
    | "partitura-name"
    | "partitura-pick";
  type PickFor = "ask" | "check" | "connect" | "disconnect";
  interface Target {
    node: CanvasNode;
    workspaceId: string;
    floorId: string;
  }

  const engine = useEngine();
  const mac = isMac();
  const index = new PaletteIndex();

  let open = $state(false);
  let mode = $state<Mode>("search");
  let pickFor = $state<PickFor>("ask");
  let elementsAll = $state(false);
  let query = $state("");
  let active = $state(0);
  let target = $state<Target | null>(null);
  let message = $state("");
  let preview = $state("");
  let sent = $state(false);
  let fileHits = $state<SearchHit[]>([]);
  let fileTree = $state<CanvasNode | null>(null);
  let renameNode = $state<CanvasNode | null>(null);
  // Edit… (terminal): name, command and working directory.
  let editNode = $state<CanvasNode | null>(null);
  let editCommand = $state("");
  let editCwd = $state("");
  // Search mode with a file tree selected also searches its files (⌘P).
  let treeHits = $state<SearchHit[]>([]);
  let treeTimer: ReturnType<typeof setTimeout> | null = null;
  // Chat transcripts matching the query (every thread of every terminal).
  let chatHits = $state<ChatHitItem[]>([]);
  let chatTimer: ReturnType<typeof setTimeout> | null = null;
  // Update Partitura from Selection: the library to pick from.
  let partituras = $state<PartituraSummary[]>([]);
  let loadTick = $state(0);
  let host = $state<HTMLSpanElement | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let area = $state<HTMLTextAreaElement | null>(null);
  let list = $state<HTMLDivElement | null>(null);
  let returnFocus: HTMLElement | null = null;
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let filesTimer: ReturnType<typeof setTimeout> | null = null;

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) });

  // ---- open / close ----------------------------------------------------

  function visible(): boolean {
    return !!host && host.isConnected && (host.parentElement?.offsetParent ?? null) !== null;
  }

  function openAs(m: Mode, opts: { keepQuery?: boolean } = {}) {
    if (!open) {
      const a = document.activeElement;
      returnFocus = a instanceof HTMLElement ? a : null;
    }
    open = true;
    mode = m;
    if (!opts.keepQuery) query = "";
    active = 0;
    target = null;
    message = "";
    preview = "";
    sent = false;
    stopPoll();
    void index.load(engine).then(() => loadTick++);
    void tick().then(() => (m === "ask" || m === "check" ? area : input)?.focus());
  }

  function close(ran = false) {
    open = false;
    stopPoll();
    target = null;
    renameNode = null;
    editNode = null;
    treeHits = [];
    chatHits = [];
    if (!ran && returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
    returnFocus = null;
  }

  /** Esc goes back one level. */
  function back() {
    if (mode === "ask" || mode === "check") {
      if (target) {
        stopPoll();
        target = null;
        mode = "pick";
        void tick().then(() => input?.focus());
        return;
      }
    }
    if (mode !== "search") {
      mode = "search";
      query = "";
      active = 0;
      stopPoll();
      void tick().then(() => input?.focus());
      return;
    }
    close();
  }

  function selectedNode(kind?: string): CanvasNode | null {
    const ids = engine.selection.list();
    for (let i = ids.length - 1; i >= 0; i--) {
      const n = engine.store.node(ids[i]);
      if (n && (!kind || n.kind === kind)) return n;
    }
    return null;
  }

  function onGlobalKey(e: KeyboardEvent) {
    if (!visible()) return;
    if (open && e.key === "Escape" && !isEditableTarget(e.target)) {
      stop(e);
      back();
      return;
    }
    const primary = mac ? e.metaKey : e.ctrlKey;
    if (!primary) return;
    const code = e.code;
    const key = e.key.toLowerCase();
    if (e.altKey && code === "KeyA") {
      stop(e);
      openAs("attention");
      return;
    }
    if (e.altKey && code === "KeyE") {
      stop(e);
      elementsAll = e.shiftKey;
      openAs("elements");
      return;
    }
    if (e.altKey || e.shiftKey) return;
    if (key !== "p" && key !== "k") return;
    // Inside a focused terminal ⌘K is the terminal's Clear (Maestri; the
    // terminal input stops it, so the app's palette never sees it either).
    if (key === "k" && (e.target as HTMLElement | null)?.closest?.(".term-node")) return;
    // ⌘K and ⌘P always open the canvas palette, even from a focused
    // terminal: letting ⌘K through opened the app's palette over the canvas,
    // and keys typed "into the terminal" then ran its navigation entries
    // (the window jumped to /superpowers).
    stop(e);
    if (open) {
      close();
      return;
    }
    const tree = selectedNode("filetree");
    if (key === "p" && tree && engine.selection.size === 1) startFiles(tree);
    else openAs("search");
  }

  function stop(e: KeyboardEvent) {
    e.preventDefault();
    e.stopImmediatePropagation();
  }

  onMount(() => {
    window.addEventListener("keydown", onGlobalKey, true);
    const offWs = workspaces.use();
    return () => {
      window.removeEventListener("keydown", onGlobalKey, true);
      offWs();
      stopPoll();
      if (filesTimer) clearTimeout(filesTimer);
      if (treeTimer) clearTimeout(treeTimer);
      if (chatTimer) clearTimeout(chatTimer);
    };
  });

  // ---- items -----------------------------------------------------------

  const SECTION_LABEL: Record<Section, [string, string]> = {
    context: ["canvas.palette.section.context", "Selection"],
    actions: ["canvas.palette.section.actions", "Actions"],
    attention: ["canvas.palette.section.attention", "Needs you"],
    nodes: ["canvas.palette.section.nodes", "Elements"],
    notes: ["canvas.palette.section.notes", "Notes"],
    chats: ["canvas.palette.section.chats", "Chats"],
    partituras: ["canvas.palette.section.partituras", "Partituras"],
    workspaces: ["canvas.palette.section.workspaces", "Workspaces"],
    floors: ["canvas.palette.section.floors", "Floors"],
    roles: ["canvas.palette.section.roles", "Roles"],
    files: ["canvas.palette.section.files", "Files"],
  };

  function onSwitch(wsId: string, floorId?: string) {
    const ws = workspaces.byId(wsId);
    void switchTo(engine, wsId, floorId ?? ws?.activeFloorId);
  }

  const results = $derived.by((): Ranked[] => {
    if (!open) return [];
    void loadTick;
    void engine.store.version;
    void workspaces.list;
    const ws = engine.store.workspaceId;
    switch (mode) {
      case "search": {
        const ctx = contextItems(engine, tr);
        const acts = actionItems(engine, tr, {
          toggleSidebar: () => canvasUi.toggleSidebar(),
          newWorkspace: () => canvasUi.askNewWorkspace(),
          newFloor: () => canvasUi.askNewFloor(),
        });
        const att = attentionItems(engine, index, tr);
        if (!query.trim()) return rank([...ctx, ...att, ...acts], "", 200);
        const all: PaletteItem[] = [
          ...ctx,
          ...acts,
          ...att,
          ...nodeItems(engine, index, tr),
          ...workspaceItems(engine, tr, (w, f) => onSwitch(w.id, f)),
          ...roleItems(engine, index, tr),
        ];
        const ranked = rank(all, query);
        // Files of the selected tree and chat hits were matched by Rust:
        // they come after the best named matches, in their own order.
        const files = treeHits.map((h, i) => ({ item: fileItem(h), score: 40 - i * 0.01, nameIdx: [] }));
        const chats = chatHits.map((c, i) => ({ item: c, score: 30 - i * 0.01, nameIdx: [] }));
        return [...ranked, ...files, ...chats].sort((a, b) => b.score - a.score).slice(0, 120);
      }
      case "workspaces":
        return rank(
          workspaceItems(engine, tr, (w, f) => onSwitch(w.id, f)).filter((i) => i.section === "workspaces"),
          query,
          200,
        );
      case "partitura-pick":
        return rank(
          partituras.map((p) => ({
            id: `partitura:${p.id}`,
            section: "partituras" as const,
            label: p.name,
            detail: p.workspaceName ?? p.summary ?? "",
            icon: "action",
            body: p.summary,
            payload: p.id,
          })),
          query,
          200,
        );
      case "attention":
        return rank(attentionItems(engine, index, tr), query, 200);
      case "elements": {
        const nodes = nodeItems(engine, index, tr).filter((i) => elementsAll || i.workspaceId === ws);
        return rank(nodes, query, 300);
      }
      case "pick":
        return rank(pickItems(), query, 200);
      case "files":
        return fileHits.map((h) => ({ item: fileItem(h), score: 0, nameIdx: [] }));
      default:
        return [];
    }
  });

  function fileItem(h: SearchHit): PaletteItem {
    return {
      id: `file:${h.path}:${h.line ?? ""}`,
      section: "files",
      label: h.line ? `${h.path}:${h.line}` : h.path,
      detail: h.preview ?? (h.isDir ? tr("canvas.palette.folder", "Folder") : ""),
      icon: h.isDir ? "filetree" : "file",
      payload: h,
    };
  }

  // ⌘P with a file tree selected: its files join the results (Maestri).
  $effect(() => {
    if (mode !== "search" || !open) return;
    const q = query.trim();
    const tree = engine.selection.size === 1 ? selectedNode("filetree") : null;
    if (treeTimer) clearTimeout(treeTimer);
    if (!q || !tree) {
      treeHits = [];
      return;
    }
    const root = String(tree.data.rootPath ?? "");
    const ws = engine.store.workspaceId;
    const hidden = !!tree.data.showsHiddenFiles;
    fileTree = tree;
    treeTimer = setTimeout(async () => {
      treeHits = root ? await filesSearch(ws, root, q, false, hidden).catch(() => []) : [];
    }, 120);
  });

  // Chat transcripts of every terminal, workspace and floor.
  $effect(() => {
    if (mode !== "search" || !open) return;
    const q = query.trim();
    if (chatTimer) clearTimeout(chatTimer);
    if (q.length < 2) {
      chatHits = [];
      return;
    }
    chatTimer = setTimeout(async () => {
      chatHits = await searchChats(engine, index, q, tr);
    }, 150);
  });

  function pickItems(): PaletteItem[] {
    const sel = engine.selection.list();
    if (pickFor === "ask" || pickFor === "check") {
      return nodeItems(engine, index, tr).filter((i) => i.icon === "terminal" || i.icon === "agent");
    }
    const here = nodeItems(engine, index, tr).filter(
      (i) => i.workspaceId === engine.store.workspaceId && i.floorId === engine.store.floorId && !sel.includes(i.nodeId ?? ""),
    );
    if (pickFor === "connect") {
      // Terminals of other floors and workspaces too (a connector pair), when
      // every selected node is a terminal.
      const terminalSel =
        sel.length > 0 && sel.every((id) => isTerminalKind(engine.store.node(id)?.kind ?? ""));
      if (!terminalSel) return here;
      const away = nodeItems(engine, index, tr).filter(
        (i) =>
          (i.icon === "terminal" || i.icon === "agent") &&
          !(i.workspaceId === engine.store.workspaceId && i.floorId === engine.store.floorId),
      );
      return [...here, ...away];
    }
    const linked = new Set<string>();
    for (const c of engine.store.connections) {
      if (sel.includes(c.from)) linked.add(c.to);
      if (sel.includes(c.to)) linked.add(c.from);
    }
    return here.filter((i) => linked.has(i.nodeId ?? ""));
  }

  $effect(() => {
    void results.length;
    if (active >= results.length) active = Math.max(0, results.length - 1);
  });

  $effect(() => {
    void active;
    if (!list) return;
    const el = list.querySelector<HTMLElement>(`[data-row="${active}"]`);
    el?.scrollIntoView({ block: "nearest" });
  });

  // ---- running ---------------------------------------------------------

  function nodeOf(item: PaletteItem): Target | null {
    if (!item.nodeId || !item.workspaceId || !item.floorId) return null;
    const node =
      item.workspaceId === engine.store.workspaceId && item.floorId === engine.store.floorId
        ? engine.store.node(item.nodeId)
        : index.remote.find((r) => r.node.id === item.nodeId)?.node;
    return node ? { node, workspaceId: item.workspaceId, floorId: item.floorId } : null;
  }

  async function runItem(r: Ranked | undefined, jump = false) {
    if (!r) return;
    const item = r.item;
    if (item.section === "files" && item.payload) {
      if (fileTree) runFile(item.payload as SearchHit);
      return;
    }
    if (item.section === "chats") {
      close(true);
      await (item as ChatHitItem).open();
      return;
    }
    if (mode === "partitura-pick") {
      await savePartituraFromSelection({ updateExisting: String(item.payload ?? ""), name: item.label });
      return;
    }
    if (mode === "pick") {
      const tg = nodeOf(item);
      if (!tg) return;
      if (pickFor === "connect") {
        const from = engine.selection.list().filter((id) => engine.store.node(id));
        const away = tg.workspaceId !== engine.store.workspaceId || tg.floorId !== engine.store.floorId;
        if (away) {
          for (const id of from) {
            await invoke("canvas_cord_link", { fromNodeId: id, toNodeId: tg.node.id }).catch((e) =>
              showToast("error", String(e)),
            );
          }
        } else {
          for (const id of from) engine.connect(id, tg.node.id);
        }
        close(true);
        return;
      }
      if (pickFor === "disconnect") {
        const sel = engine.selection.list();
        const ops = engine.store.connections
          .filter((c) => (sel.includes(c.from) && c.to === tg.node.id) || (sel.includes(c.to) && c.from === tg.node.id))
          .map((c) => ({ op: "remove_connection" as const, id: c.id }));
        engine.commit(ops);
        close(true);
        return;
      }
      startTerminalMode(pickFor, tg);
      return;
    }
    switch (item.mode) {
      case "ask":
      case "check": {
        const sel = item.nodeId ? engine.store.node(item.nodeId) : null;
        if (sel && isTerminalKind(sel.kind)) {
          startTerminalMode(item.mode, { node: sel, workspaceId: engine.store.workspaceId, floorId: engine.store.floorId });
        } else {
          pickFor = item.mode;
          mode = "pick";
          query = "";
          active = 0;
          await tick();
          input?.focus();
        }
        return;
      }
      case "connect":
      case "disconnect":
        pickFor = item.mode;
        mode = "pick";
        query = "";
        active = 0;
        await tick();
        input?.focus();
        return;
      case "files": {
        const tree = item.nodeId ? engine.store.node(item.nodeId) : null;
        if (tree) startFiles(tree);
        return;
      }
      case "attention":
        openAs("attention");
        return;
      case "edit-terminal": {
        const n = item.nodeId ? engine.store.node(item.nodeId) : null;
        if (!n) return;
        editNode = n;
        editCommand = typeof n.data.command === "string" ? n.data.command : "";
        editCwd = typeof n.data.cwd === "string" ? n.data.cwd : "";
        mode = "edit";
        query = n.title ?? "";
        await tick();
        input?.select();
        return;
      }
      case "workspaces":
      case "partitura-new":
        mode = item.mode === "workspaces" ? "workspaces" : "partitura-name";
        query = "";
        active = 0;
        await tick();
        input?.focus();
        return;
      case "partitura-update":
        mode = "partitura-pick";
        query = "";
        active = 0;
        partituras = await listPartituras().catch(() => []);
        await tick();
        input?.focus();
        return;
      case "rename-note": {
        const n = item.nodeId ? engine.store.node(item.nodeId) : null;
        if (!n) return;
        renameNode = n;
        mode = "rename";
        query = n.title ?? "";
        await tick();
        input?.select();
        return;
      }
    }
    if (item.run) {
      close(true);
      await item.run();
      return;
    }
    if (item.nodeId && item.workspaceId && item.floorId) {
      close(true);
      await goTo(engine, { workspaceId: item.workspaceId, floorId: item.floorId, nodeId: item.nodeId }, { focus: jump || undefined });
    }
  }

  function startFiles(tree: CanvasNode) {
    fileTree = tree;
    fileHits = [];
    openAs("files");
  }

  $effect(() => {
    if (mode !== "files" || !open || !fileTree) return;
    const q = query;
    const root = String(fileTree.data.rootPath ?? "");
    const ws = engine.store.workspaceId;
    const hidden = !!fileTree.data.showsHiddenFiles;
    if (filesTimer) clearTimeout(filesTimer);
    filesTimer = setTimeout(async () => {
      const content = q.startsWith(">");
      const text = content ? q.slice(1).trim() : q.trim();
      if (!text || !root) {
        fileHits = [];
        return;
      }
      fileHits = await filesSearch(ws, root, text, content, hidden).catch(() => []);
      active = 0;
    }, 150);
  });

  function runFile(hit: SearchHit) {
    const tree = fileTree;
    if (!tree) return;
    const root = String(tree.data.rootPath ?? "");
    close(true);
    if (hit.isDir) {
      const parts = hit.path.split("/");
      const expanded = new Set<string>(Array.isArray(tree.data.expanded) ? (tree.data.expanded as string[]) : []);
      for (let i = 1; i <= parts.length; i++) expanded.add(parts.slice(0, i).join("/"));
      engine.updateNode(tree.id, { data: { ...tree.data, expanded: Array.from(expanded) } });
      engine.reveal(tree.id);
      return;
    }
    openEditorBeside(engine, tree, root, hit.path, hit.line);
  }

  function submitRename() {
    const n = renameNode;
    if (!n) return;
    const name = query.trim();
    if (name) engine.updateNode(n.id, { title: name, data: { ...n.data, hasCustomName: true } });
    else engine.updateNode(n.id, { data: { ...n.data, hasCustomName: null } });
    close(true);
  }

  function submitEdit() {
    const n = editNode;
    if (!n) return;
    const name = query.trim();
    const command = editCommand.trim();
    const cwd = editCwd.trim();
    const oldCommand = typeof n.data.command === "string" ? n.data.command : "";
    const oldCwd = typeof n.data.cwd === "string" ? n.data.cwd : "";
    engine.updateNode(n.id, {
      ...(name ? { title: name } : {}),
      data: { ...n.data, command: command || null, cwd: cwd || null, ...(name ? { hasCustomName: true } : {}) },
    });
    // A new program or folder only takes effect in a new process.
    if (command !== oldCommand || cwd !== oldCwd) engine.requestRestart(n.id);
    close(true);
  }

  async function savePartituraFromSelection(opts: { name: string; updateExisting?: string }) {
    const nodeIds = engine.selection.list().filter((id) => engine.store.node(id));
    if (!nodeIds.length || !opts.name.trim()) return;
    try {
      await savePartitura(engine.store.workspaceId, engine.store.floorId, {
        selection: { nodeIds },
        options: { name: opts.name.trim(), includeStartupCommands: true },
        scope: engine.store.workspaceId,
        updateExisting: opts.updateExisting || undefined,
      });
      showToast("success", tr("canvas.palette.partituraSaved", "Saved partitura '{{name}}'", { name: opts.name.trim() }));
    } catch (e) {
      showToast("error", String(e));
    }
    close(true);
  }

  // ---- Ask / Check -----------------------------------------------------

  function startTerminalMode(m: "ask" | "check", tg: Target) {
    target = tg;
    mode = m;
    message = "";
    sent = false;
    preview = "";
    startPoll();
    void tick().then(() => area?.focus());
  }

  function startPoll() {
    stopPoll();
    const id = target ? terminalIdOf(target.node) : null;
    if (!id) return;
    const read = async () => {
      try {
        const snap = await invoke<{ text: string }>("term_screen", { id, lastLines: 40 });
        preview = (snap?.text ?? "").replace(/\s+$/, "");
      } catch {
        preview = "";
      }
    };
    void read();
    pollTimer = setInterval(read, 500);
  }

  function stopPoll() {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = null;
  }

  async function send(jump: boolean) {
    if (!target) return;
    const id = terminalIdOf(target.node);
    const text = message;
    if (text.trim()) {
      try {
        await invoke("term_paste", { id, text });
        await invoke("term_text", { id, text: "\r" });
        message = "";
        sent = true;
      } catch {
        engine.say("canvas.palette.askFailed");
      }
    }
    if (jump) {
      const tg = target;
      close(true);
      await goTo(engine, { workspaceId: tg.workspaceId, floorId: tg.floorId, nodeId: tg.node.id }, { focus: true });
    }
  }

  // ---- keys inside the palette -----------------------------------------

  function onInputKey(e: KeyboardEvent) {
    const primary = mac ? e.metaKey : e.ctrlKey;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      back();
      return;
    }
    if (mode === "rename" || mode === "edit" || mode === "partitura-name") {
      if (e.key === "Enter") {
        e.preventDefault();
        if (mode === "rename") submitRename();
        else if (mode === "edit") submitEdit();
        else void savePartituraFromSelection({ name: query });
      }
      return;
    }
    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
      e.preventDefault();
      active = Math.min(results.length - 1, active + 1);
    } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
      e.preventDefault();
      active = Math.max(0, active - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      void runItem(results[active], primary);
    } else if (e.key === "Backspace" && !query && mode !== "search") {
      e.preventDefault();
      back();
    }
  }

  function onAreaKey(e: KeyboardEvent) {
    const primary = mac ? e.metaKey : e.ctrlKey;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      back();
      return;
    }
    if (mode !== "ask") return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send(primary);
    }
  }

  // ---- view helpers ----------------------------------------------------

  const ICON_FOR: Record<string, string> = {
    ...ICONS,
    agent: ICONS.agent,
    filetree: "M3 6h6l2 2h10v11H3z",
    stack: "M6 7h12v13H6z M8 4h10v2 M10 1.5h8",
    workspace: "M3 5h18v14H3z M3 9h18 M8 9v10",
    floor: "M12 3l9 5-9 5-9-5z M3 13l9 5 9-5",
    role: "M12 3l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 16.4 6.8 19.1l1-5.8L3.5 9.2l5.9-.9z",
    action: TOOL_ICONS.smart,
  };

  function iconOf(item: PaletteItem): string {
    return ICON_FOR[item.icon] ?? ICONS.unknown;
  }

  const placeholder = $derived.by(() => {
    switch (mode) {
      case "attention":
        return tr("canvas.palette.placeholder.attention", "Terminals waiting for you");
      case "elements":
        return elementsAll
          ? tr("canvas.palette.placeholder.elementsAll", "Elements in every workspace")
          : tr("canvas.palette.placeholder.elements", "Elements in this workspace");
      case "pick":
        return pickFor === "connect"
          ? tr("canvas.palette.placeholder.connect", "Connect to…")
          : pickFor === "disconnect"
            ? tr("canvas.palette.placeholder.disconnect", "Disconnect from…")
            : tr("canvas.palette.placeholder.pickTerminal", "Pick a terminal");
      case "files":
        return tr("canvas.palette.placeholder.files", "Search files (start with > to search inside)");
      case "rename":
        return tr("canvas.palette.placeholder.rename", "Name (empty = follow the first line)");
      case "edit":
        return tr("canvas.palette.placeholder.editName", "Terminal name");
      case "workspaces":
        return tr("canvas.palette.placeholder.workspaces", "Switch to workspace…");
      case "partitura-name":
        return tr("canvas.palette.placeholder.partituraName", "Name of the new partitura, Enter saves");
      case "partitura-pick":
        return tr("canvas.palette.placeholder.partituraPick", "Partitura to update with the selection");
      default:
        return tr("canvas.palette.placeholder.search", "Search everything or run an action");
    }
  });

  const showHeaders = $derived(!query.trim() && (mode === "search" || mode === "elements"));
</script>

<span class="anchor" bind:this={host} aria-hidden="true"></span>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div class="scrim" data-canvas-chrome onpointerdown={() => close()}></div>
  <div
    class="palette"
    data-canvas-chrome
    role="dialog"
    aria-modal="true"
    aria-label={tr("canvas.palette.title", "Batuta Search")}
  >
    {#if (mode === "ask" || mode === "check") && target}
      <div class="head">
        <span class="dot" style:background={stateCss(undefined)}></span>
        <span class="who">
          {mode === "ask" ? tr("canvas.palette.askTitle", "Ask {{name}}", { name: target.node.title ?? "" }) : tr("canvas.palette.checkTitle", "Check {{name}}", { name: target.node.title ?? "" })}
        </span>
        {#if target.workspaceId !== engine.store.workspaceId}
          <span class="where">{workspaces.byId(target.workspaceId)?.name ?? ""}</span>
        {/if}
      </div>
      <pre class="preview" aria-live="polite">{preview || tr("canvas.palette.noOutput", "No output yet")}</pre>
      {#if mode === "ask"}
        <textarea
          bind:this={area}
          bind:value={message}
          rows="3"
          onkeydown={onAreaKey}
          placeholder={tr("canvas.palette.askPlaceholder", "Message. Enter sends, ⇧Enter new line, ⌘Enter sends and jumps")}
          spellcheck="false"
        ></textarea>
        <div class="foot">
          {#if sent}<span class="ok">{tr("canvas.palette.sent", "Sent")}</span>{/if}
          <span class="hint">{tr("canvas.palette.askHint", "Esc to go back")}</span>
          <button type="button" onclick={() => void send(true)}>{tr("canvas.palette.sendJump", "Send and jump")}</button>
          <button type="button" class="primary" onclick={() => void send(false)}>{tr("canvas.palette.send", "Send")}</button>
        </div>
      {:else}
        <textarea class="hidden-area" bind:this={area} onkeydown={onAreaKey} aria-label={tr("canvas.palette.checkTitle", "Check {{name}}", { name: target.node.title ?? "" })} readonly></textarea>
        <div class="foot">
          <span class="hint">{tr("canvas.palette.checkHint", "Read-only, live. Esc to go back")}</span>
          <button
            type="button"
            class="primary"
            onclick={() => {
              const tg = target;
              if (!tg) return;
              close(true);
              void goTo(engine, { workspaceId: tg.workspaceId, floorId: tg.floorId, nodeId: tg.node.id }, { focus: true });
            }}>{tr("canvas.palette.jump", "Jump there")}</button
          >
        </div>
      {/if}
    {:else}
      <div class="search">
        <Icon d={TOOL_ICONS.zoomTo} size={16} />
        <input
          bind:this={input}
          bind:value={query}
          onkeydown={onInputKey}
          oninput={() => (active = 0)}
          {placeholder}
          spellcheck="false"
          autocomplete="off"
          role="combobox"
          aria-expanded="true"
          aria-controls="batuta-list"
          aria-activedescendant={results[active] ? `batuta-row-${active}` : undefined}
        />
        {#if mode !== "search"}
          <button type="button" class="chip" onclick={back}>{tr("canvas.palette.back", "Back")}</button>
        {/if}
      </div>
      {#if mode === "edit"}
        <div class="edit-form">
          <label>
            <span>{tr("canvas.palette.edit.command", "Command")}</span>
            <input
              bind:value={editCommand}
              onkeydown={onInputKey}
              placeholder={tr("canvas.palette.edit.commandHint", "Program to run (empty = the agent's default)")}
              spellcheck="false"
              autocomplete="off"
            />
          </label>
          <label>
            <span>{tr("canvas.palette.edit.cwd", "Folder")}</span>
            <input
              bind:value={editCwd}
              onkeydown={onInputKey}
              placeholder={tr("canvas.palette.edit.cwdHint", "Working directory (empty = the workspace folder)")}
              spellcheck="false"
              autocomplete="off"
            />
          </label>
          <div class="edit-actions">
            <span class="hint">{tr("canvas.palette.edit.hint", "Enter saves; a new command or folder restarts the terminal")}</span>
            <button type="button" class="primary" onclick={submitEdit}>{tr("canvas.palette.edit.save", "Save")}</button>
          </div>
        </div>
      {/if}
      {#if mode !== "rename" && mode !== "edit" && mode !== "partitura-name"}
        <div class="list" id="batuta-list" role="listbox" bind:this={list}>
          {#each results as r, i (r.item.id)}
            {#if showHeaders && (i === 0 || results[i - 1].item.section !== r.item.section)}
              <div class="section">{tr(SECTION_LABEL[r.item.section][0], SECTION_LABEL[r.item.section][1])}</div>
            {/if}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="row"
              class:active={i === active}
              id="batuta-row-{i}"
              data-row={i}
              role="option"
              tabindex="-1"
              aria-selected={i === active}
              onpointermove={() => (active = i)}
              onclick={(e) => void runItem(r, mac ? e.metaKey : e.ctrlKey)}
            >
              <span class="icon">
                {#if r.item.state}
                  <span class="state" style:background={stateCss(r.item.state)}></span>
                {/if}
                <Icon d={iconOf(r.item)} size={15} />
              </span>
              <span class="label">
                {#each highlightRuns(r.item.label, r.nameIdx) as run}
                  {#if run.hit}<b>{run.s}</b>{:else}{run.s}{/if}
                {/each}
              </span>
              {#if r.item.detail}<span class="detail">{r.item.detail}</span>{/if}
              {#if r.item.keys}<kbd>{r.item.keys}</kbd>{/if}
            </div>
          {:else}
            <div class="empty">
              {#if mode === "attention"}
                {tr("canvas.palette.nobodyWaiting", "Nobody needs you right now")}
              {:else if mode === "files" && !query.trim()}
                {tr("canvas.palette.typeToSearchFiles", "Type a file name")}
              {:else}
                {tr("canvas.palette.noResults", "Nothing found")}
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      <div class="foot">
        <span class="hint">
          {tr("canvas.palette.footHint", "↑↓ to move · Enter to open · ⌘Enter to open and focus · Esc to go back")}
        </span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .anchor {
    position: absolute;
    width: 0;
    height: 0;
    pointer-events: none;
  }
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 40;
    background: color-mix(in srgb, #000 18%, transparent);
  }
  .palette {
    position: absolute;
    top: 72px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 41;
    width: min(640px, calc(100% - 32px));
    max-height: min(560px, calc(100% - 120px));
    display: flex;
    flex-direction: column;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.35);
    overflow: hidden;
    color: var(--text);
    user-select: none;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 15px;
  }
  .edit-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 14px;
  }
  .edit-form label {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .edit-form label span {
    width: 72px;
    flex-shrink: 0;
  }
  .edit-form input {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    user-select: text;
  }
  .edit-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .edit-actions .primary {
    padding: 5px 12px;
    border-radius: 8px;
    border: none;
    background: var(--accent);
    color: var(--on-accent, #fff);
    cursor: pointer;
  }
  .chip {
    font-size: 11px;
    padding: 3px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .list {
    overflow-y: auto;
    padding: 6px;
    min-height: 60px;
  }
  .section {
    padding: 8px 10px 4px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
  }
  .row.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .icon {
    position: relative;
    display: flex;
    color: var(--text-muted);
  }
  .state {
    position: absolute;
    right: -3px;
    top: -3px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }
  .label {
    flex: none;
    max-width: 55%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .label b {
    font-weight: 700;
    color: var(--accent);
  }
  .detail {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
    font-size: 12px;
  }
  kbd {
    flex: none;
    font: 11px var(--font-mono, monospace);
    color: var(--text-muted);
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
  }
  .empty {
    padding: 18px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-muted);
  }
  .foot .hint {
    flex: 1;
  }
  .foot button {
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .foot button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent, #fff);
  }
  .ok {
    color: var(--success, #5ca97a);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 14px 8px;
    font-size: 13px;
    font-weight: 600;
  }
  .head .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .head .where {
    font-weight: 400;
    color: var(--text-muted);
  }
  .preview {
    margin: 0 12px;
    padding: 10px;
    height: 220px;
    overflow: auto;
    border-radius: 8px;
    background: var(--terminal-bg, #111);
    color: var(--terminal-fg, #ddd);
    font: 12px/1.35 var(--font-mono, monospace);
    white-space: pre-wrap;
    user-select: text;
  }
  textarea {
    margin: 10px 12px 0;
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    font: 13px/1.45 var(--font-sans, inherit);
    resize: vertical;
    outline: none;
  }
  textarea:focus {
    border-color: var(--accent);
  }
  .hidden-area {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
    margin: 0;
    padding: 0;
    border: 0;
  }
</style>
