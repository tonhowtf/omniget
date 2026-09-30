<script lang="ts">
  // File Tree node (study 95 §K 115–122): a lazy tree rooted at the
  // workspace folder (or a dropped folder), with Pinned paths, git status
  // badges, name search (`>` searches inside files), file operations (new,
  // rename, delete to trash, pin, copy path, reload, sort), and three views:
  // Files, Changes (working-tree diff with stage/unstage/discard per file and
  // Current/Incoming of conflicts) and History. The git button adds pull,
  // fetch, sync, branches, merge and stash through `canvas_files_git_op`. Opening a file puts an editor
  // node beside the tree; dragging a row onto the canvas makes a file node.
  // Several trees per canvas, each with its own state in `data`.
  import { onMount, tick, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import {
    baseName,
    dirName,
    fileSource,
    filesGitDiff,
    filesGitOp,
    filesGitStatus,
    isRemoteEnv,
    joinPath,
    remoteRoot,
    relativeTo,
    statusLetter,
    type FileEntry,
    type ConflictView,
    type FileTreeData,
    type SearchHit,
  } from "$lib/canvas/files/api";
  import { nodeForPath, openEditorBeside } from "$lib/canvas/files/open";
  import { workspaces } from "$lib/canvas/palette/workspaces.svelte";
  import { vcsLog, type FileDiff, type LogEntry, type RepoStatus } from "$lib/central/vcs";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";
  import GitActionsButton from "$components/central/vcs/GitActionsButton.svelte";

  let {
    node,
    selected = false,
    focused = false,
  }: {
    node: CanvasNode;
    selected?: boolean;
    focused?: boolean;
  } = $props();

  const engine = useEngine();

  type View = "list" | "diff" | "log";
  interface Row {
    entry: FileEntry;
    depth: number;
  }

  let children = $state<Record<string, FileEntry[]>>({});
  let loadingDirs = $state<Record<string, boolean>>({});
  let error = $state<string | null>(null);
  let status = $state<RepoStatus | null>(null);
  let diffFiles = $state<FileDiff[]>([]);
  let diffLoading = $state(false);
  let log = $state<LogEntry[]>([]);
  let query = $state("");
  let hits = $state<SearchHit[]>([]);
  let searching = $state(false);
  let active = $state<string | null>(null);
  let menu = $state<{ entry: FileEntry | null; x: number; y: number } | null>(null);
  let conflict = $state<ConflictView | null>(null);
  let gitBusy = $state<string | null>(null);
  let editing = $state<{ mode: "new-file" | "new-dir" | "rename"; dir: string; entry?: FileEntry; text: string } | null>(null);
  let drag = $state<{ entry: FileEntry; x0: number; y0: number; x: number; y: number; moved: boolean } | null>(null);
  let rootEl = $state<HTMLDivElement | null>(null);
  let editInput = $state<HTMLInputElement | null>(null);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let statusTimer: ReturnType<typeof setInterval> | null = null;

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) });

  const data = $derived((node.data ?? {}) as Partial<FileTreeData>);
  const wsDir = $derived(workspaces.byId(engine.store.workspaceId)?.workingDirectory ?? "");
  /** Maestri §K9: a tree on the SSH host / container of `data.environment`. */
  const envData = $derived(data.environment);
  const remote = $derived(isRemoteEnv(envData));
  const root = $derived(remote ? String(data.rootPath ?? "") : String(data.rootPath ?? "") || wsDir);
  const view = $derived<View>(!isRemoteEnv(data.environment) && (data.viewMode === "diff" || data.viewMode === "log") ? data.viewMode : "list");
  const expanded = $derived(new Set<string>(Array.isArray(data.expanded) ? data.expanded : []));
  const pinned = $derived<string[]>(Array.isArray(data.pinnedPaths) ? data.pinnedPaths : []);
  const hidden = $derived(!!data.showsHiddenFiles);
  type SortBy = "name" | "type" | "modified" | "size";
  const sortBy = $derived<SortBy>(
    data.sortBy === "type" || data.sortBy === "modified" || data.sortBy === "size" ? data.sortBy : "name",
  );
  const sortReverse = $derived(!!data.sortReverse);
  const foldersFirst = $derived(data.foldersFirst !== false);
  const ws = $derived(engine.store.workspaceId);
  const src = $derived(fileSource(ws, root, envData));

  /** UI state lives in the node but is not an undo step. */
  function setData(changes: Record<string, unknown>) {
    const n = engine.store.node(node.id);
    if (!n) return;
    engine.commit([{ op: "update_node", id: n.id, patch: { data: { ...n.data, ...changes } } }], { history: false });
  }

  // A tree spawned without a root adopts the workspace folder.
  $effect(() => {
    if (remote || data.environment !== undefined) return;
    // A workspace on SSH/Docker gets remote trees (its folder over there).
    const wenv = workspaces.byId(engine.store.workspaceId)?.environment;
    if (!data.rootPath && wenv && isRemoteEnv(wenv.kind)) {
      untrack(() => setData({ environment: "workspace" }));
      return;
    }
    if (!data.rootPath && wsDir) untrack(() => setData({ rootPath: wsDir }));
  });

  // A remote tree without a folder shows the environment's (or its home).
  $effect(() => {
    if (!remote || data.rootPath) return;
    const env = envData;
    untrack(() => {
      void remoteRoot(ws, env, "")
        .then((r) => {
          if (r) setData({ rootPath: r });
        })
        .catch((e) => (error = e instanceof Error ? e.message : String(e)));
    });
  });

  // ---- listing ---------------------------------------------------------

  async function loadDir(dir: string) {
    if (!root) return;
    loadingDirs = { ...loadingDirs, [dir]: true };
    try {
      const list = await src.list(dir, hidden);
      children = { ...children, [dir]: list };
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      const next = { ...loadingDirs };
      delete next[dir];
      loadingDirs = next;
    }
  }

  async function refresh() {
    children = {};
    const dirs = ["", ...Array.from(expanded)];
    await Promise.all(dirs.map((d) => loadDir(d)));
    void refreshStatus();
    if (view === "diff") void loadDiff();
    if (view === "log") void loadLog();
  }

  async function refreshStatus() {
    if (!root || remote) return;
    status = await filesGitStatus(ws, root).catch(() => null);
    // Maestri §K8: an unselected tree in Changes follows the agent's edits.
    const sig = status?.files.map((f) => `${f.path}:${f.kind}:${f.worktree}`).join("|") ?? "";
    // The signature misses a second edit of an already-modified file, so an
    // unselected Changes view re-reads the diff each poll (kept when equal).
    if (view === "diff" && !selected && lastSig !== null) void loadDiff(sig !== lastSig);
    lastSig = sig;
  }

  let lastSig: string | null = null;
  let followPath = $state<string | null>(null);
  let prevDiff = new Map<string, string>();

  // Root, workspace or hidden-files change → reload everything.
  $effect(() => {
    void root;
    void ws;
    void hidden;
    untrack(() => void refresh());
  });

  // Expanding a folder loads it once.
  $effect(() => {
    for (const d of expanded) {
      if (!untrack(() => children[d]) && !untrack(() => loadingDirs[d])) untrack(() => void loadDir(d));
    }
  });

  onMount(() => {
    const offWs = workspaces.use();
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    statusTimer = setInterval(() => void refreshStatus(), 5000);
    return () => {
      offWs();
      window.removeEventListener("focus", onFocus);
      if (statusTimer) clearInterval(statusTimer);
      if (searchTimer) clearTimeout(searchTimer);
    };
  });

  function sorted(list: FileEntry[]): FileEntry[] {
    const ext = (n: string) => (n.includes(".") ? n.slice(n.lastIndexOf(".") + 1).toLowerCase() : "");
    const byName = (a: FileEntry, b: FileEntry) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
    const cmp = (a: FileEntry, b: FileEntry): number => {
      let r = 0;
      if (sortBy === "type") r = ext(a.name).localeCompare(ext(b.name));
      else if (sortBy === "modified") r = (b.modifiedMs ?? 0) - (a.modifiedMs ?? 0);
      else if (sortBy === "size") r = b.size - a.size;
      r = r || byName(a, b);
      return sortReverse ? -r : r;
    };
    return [...list].sort((a, b) => (foldersFirst ? Number(b.isDir) - Number(a.isDir) : 0) || cmp(a, b));
  }

  const rows = $derived.by((): Row[] => {
    const out: Row[] = [];
    const walk = (dir: string, depth: number) => {
      for (const entry of sorted(children[dir] ?? [])) {
        out.push({ entry, depth });
        if (entry.isDir && expanded.has(entry.path)) walk(entry.path, depth + 1);
      }
    };
    walk("", 0);
    return out;
  });

  // ---- git badges ------------------------------------------------------

  const badges = $derived.by(() => {
    const map = new Map<string, string>();
    const dirs = new Set<string>();
    if (!status?.is_repo || !status.root || !root) return { map, dirs };
    const prefix = relativeTo(status.root, root);
    if (prefix === null) return { map, dirs };
    for (const f of status.files) {
      let rel = f.path;
      if (prefix) {
        if (!rel.startsWith(`${prefix}/`)) continue;
        rel = rel.slice(prefix.length + 1);
      }
      const letter = statusLetter(f.kind);
      if (!letter) continue;
      map.set(rel, letter);
      let d = dirName(rel);
      while (d) {
        dirs.add(d);
        d = dirName(d);
      }
    }
    return { map, dirs };
  });

  // ---- actions ---------------------------------------------------------

  function toggle(entry: FileEntry) {
    const next = new Set(expanded);
    if (next.has(entry.path)) next.delete(entry.path);
    else next.add(entry.path);
    setData({ expanded: Array.from(next) });
  }

  function openEntry(entry: FileEntry, line?: number) {
    active = entry.path;
    if (entry.isDir) toggle(entry);
    else openEditorBeside(engine, engine.store.node(node.id) ?? node, root, entry.path, line);
  }

  function setView(v: View) {
    setData({ viewMode: v === "list" ? null : v });
    if (v === "diff") void loadDiff();
    if (v === "log") void loadLog();
  }

  async function loadDiff(follow = false) {
    if (!root) return;
    diffLoading = true;
    try {
      const d = await filesGitDiff(ws, root);
      if ((follow || prevDiff.size > 0) && !selected) {
        // The file whose patch changed since the last load gets revealed
        // (also a file that was already modified: the status signature
        // does not change then, only the patch does).
        const changed = d.files.filter((f) => prevDiff.get(f.path) !== `${f.additions}:${f.deletions}:${f.patch.length}:${f.patch}`);
        followPath = changed.length ? changed[changed.length - 1].path : null;
      }
      const next = new Map(d.files.map((f) => [f.path, `${f.additions}:${f.deletions}:${f.patch.length}:${f.patch}`]));
      const same = next.size === prevDiff.size && [...next].every(([k, v]) => prevDiff.get(k) === v);
      prevDiff = next;
      // Same patches: keep the list (and the viewer's scroll and folds).
      if (!same || !diffFiles.length) diffFiles = d.files;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      diffFiles = [];
    } finally {
      diffLoading = false;
    }
  }

  async function loadLog() {
    if (!root) return;
    log = await vcsLog(root, 60).catch(() => []);
  }

  function togglePin(path: string) {
    const cur = engine.store.node(node.id)?.data?.pinnedPaths;
    const list: string[] = Array.isArray(cur) ? (cur as string[]) : pinned;
    const next = list.includes(path) ? list.filter((p) => p !== path) : [...list, path];
    setData({ pinnedPaths: next.length ? next : null });
  }

  // ---- git per file (Changes view) ---------------------------------------

  /** Changed files of the repo that fall under this tree, relative to its root. */
  const changes = $derived.by(() => {
    if (!status?.is_repo || !status.root || !root) return [];
    const prefix = relativeTo(status.root, root);
    if (prefix === null) return [];
    const out: { rel: string; kind: string; staged: boolean; unstaged: boolean }[] = [];
    for (const f of status.files) {
      let rel = f.path;
      if (prefix) {
        if (!rel.startsWith(`${prefix}/`)) continue;
        rel = rel.slice(prefix.length + 1);
      }
      const staged = f.index !== "." && f.index !== " " && f.index !== "?" && f.kind !== "untracked";
      const unstaged = f.kind === "untracked" || (f.worktree !== "." && f.worktree !== " ");
      out.push({ rel, kind: f.kind, staged, unstaged });
    }
    return out;
  });

  const gitOps = {
    run: (action: string, args: Record<string, unknown> = {}) =>
      filesGitOp(ws, root, action as Parameters<typeof filesGitOp>[2], args),
  };

  async function fileGit(action: "stage" | "unstage" | "discard", rel: string) {
    gitBusy = rel;
    try {
      await filesGitOp(ws, root, action, { paths: [rel] });
      await refreshStatus();
      if (view === "diff") void loadDiff();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      gitBusy = null;
    }
  }

  let confirmDiscard = $state<string | null>(null);

  async function showConflict(rel: string) {
    try {
      const r = await filesGitOp(ws, root, "conflict", { paths: [rel] });
      conflict = r.conflict ?? null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function setSort(by: SortBy) {
    menu = null;
    setData({ sortBy: by === "name" ? null : by });
  }

  async function chooseRoot() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, defaultPath: root || undefined });
      if (typeof picked === "string") {
        engine.updateNode(node.id, { title: baseName(picked), data: { ...node.data, rootPath: picked, expanded: [] } });
      }
    } catch {
      // No dialog.
    }
  }

  // ---- search ----------------------------------------------------------

  $effect(() => {
    const q = query;
    if (searchTimer) clearTimeout(searchTimer);
    if (!q.trim()) {
      hits = [];
      searching = false;
      return;
    }
    searching = true;
    searchTimer = setTimeout(async () => {
      const content = q.startsWith(">");
      const text = content ? q.slice(1).trim() : q.trim();
      hits = text ? await src.search(text, content, hidden).catch(() => []) : [];
      searching = false;
    }, 180);
  });

  function searchKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      query = "";
    } else if (e.key === "Enter" && hits[0]) {
      e.preventDefault();
      const h = hits[0];
      openEntry({ name: baseName(h.path), path: h.path, isDir: h.isDir, isSymlink: false, hidden: false, size: 0 }, h.line);
    }
  }

  // ---- file operations -------------------------------------------------

  function openMenu(e: MouseEvent, entry: FileEntry | null) {
    e.preventDefault();
    e.stopPropagation();
    // Screen coordinates: the menu is portaled to <body> (the node is
    // transformed and clips its content).
    const x = Math.min(e.clientX, window.innerWidth - 200);
    const y = Math.min(e.clientY, window.innerHeight - 380);
    menu = { entry, x: Math.max(4, x), y: Math.max(4, y) };
  }

  // Keeps the portaled menu inside the window once its real size is known
  // (the row menu is taller than the guess above near the bottom edge).
  let menuEl = $state<HTMLDivElement | undefined>();
  $effect(() => {
    const el = menuEl;
    const m = menu;
    if (!el || !m) return;
    const r = el.getBoundingClientRect();
    const x = Math.max(4, Math.min(m.x, window.innerWidth - r.width - 4));
    const y = Math.max(4, Math.min(m.y, window.innerHeight - r.height - 4));
    if (x !== m.x || y !== m.y) menu = { ...m, x, y };
  });

  async function startEdit(mode: "new-file" | "new-dir" | "rename", entry: FileEntry | null) {
    menu = null;
    const dir = entry ? (entry.isDir && mode !== "rename" ? entry.path : dirName(entry.path)) : "";
    if (mode !== "rename" && entry?.isDir && !expanded.has(entry.path)) toggle(entry);
    editing = { mode, dir, entry: entry ?? undefined, text: mode === "rename" ? (entry?.name ?? "") : "" };
    await tick();
    editInput?.focus();
    if (mode === "rename" && entry) {
      const dot = entry.isDir ? -1 : entry.name.lastIndexOf(".");
      editInput?.setSelectionRange(0, dot > 0 ? dot : entry.name.length);
    }
  }

  async function commitEdit() {
    const ed = editing;
    editing = null;
    if (!ed) return;
    const name = ed.text.trim();
    if (!name || name.includes("/") || name.includes("\\") || name === "." || name === "..") return;
    const path = ed.dir ? `${ed.dir}/${name}` : name;
    try {
      if (ed.mode === "rename" && ed.entry) {
        if (name === ed.entry.name) return;
        await src.rename(ed.entry.path, path);
        if (pinned.includes(ed.entry.path)) setData({ pinnedPaths: pinned.map((p) => (p === ed.entry!.path ? path : p)) });
      } else {
        await src.create(path, ed.mode === "new-dir");
        if (ed.mode !== "new-dir") openEntry({ name, path, isDir: false, isSymlink: false, hidden: name.startsWith("."), size: 0 });
      }
      await loadDir(ed.dir);
      void refreshStatus();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function editKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commitEdit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      editing = null;
    }
  }

  let confirmDelete = $state<string | null>(null);

  async function doDelete(entry: FileEntry) {
    menu = null;
    confirmDelete = null;
    try {
      await src.remove(entry.path);
      if (pinned.includes(entry.path)) setData({ pinnedPaths: pinned.filter((p) => p !== entry.path) });
      await loadDir(dirName(entry.path));
      void refreshStatus();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function copyPath(entry: FileEntry) {
    menu = null;
    await navigator.clipboard?.writeText(joinPath(root, entry.path)).catch(() => {});
  }

  // ---- drag a row onto the canvas ---------------------------------------

  function rowDown(e: PointerEvent, entry: FileEntry) {
    if (e.button !== 0) return;
    e.stopPropagation();
    drag = { entry, x0: e.clientX, y0: e.clientY, x: e.clientX, y: e.clientY, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function rowMove(e: PointerEvent) {
    if (!drag) return;
    drag.x = e.clientX;
    drag.y = e.clientY;
    if (!drag.moved && Math.hypot(e.clientX - drag.x0, e.clientY - drag.y0) > 6) drag.moved = true;
  }

  function rowUp(e: PointerEvent, entry: FileEntry) {
    const d = drag;
    drag = null;
    if (!d) return;
    if (!d.moved) {
      openEntry(entry);
      return;
    }
    const r = rootEl?.getBoundingClientRect();
    const inside = r && e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
    if (inside) return;
    const l = engine.tools.local(e);
    const w = engine.camera.screenToWorld(l.x, l.y);
    const n = nodeForPath(engine, joinPath(root, entry.path), entry.isDir, { x: w.x, y: w.y }, root);
    engine.selection.set([n.id]);
  }

  function pinnedEntry(path: string): FileEntry {
    const dir = dirName(path);
    const known = children[dir]?.find((c) => c.path === path);
    return known ?? { name: baseName(path), path, isDir: expanded.has(path), isSymlink: false, hidden: false, size: 0 };
  }

  /** Moves the drag ghost to <body> (the world layer is transformed). */
  function portal(el: HTMLElement) {
    document.body.appendChild(el);
    return { destroy: () => el.remove() };
  }

  function when(at: number): string {
    const ms = at > 1e12 ? at : at * 1000;
    return new Date(ms).toLocaleString();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="tree" class:focused class:selected bind:this={rootEl} oncontextmenu={(e) => openMenu(e, null)}>
  <div class="bar" data-canvas-chrome>
    <div class="seg" role="tablist">
      <button type="button" role="tab" aria-selected={view === "list"} class:on={view === "list"} onclick={() => setView("list")}>{tr("canvas.files.viewFiles", "Files")}</button>
      {#if !remote}
      <button type="button" role="tab" aria-selected={view === "diff"} class:on={view === "diff"} onclick={() => setView("diff")}>
        {tr("canvas.files.viewChanges", "Changes")}{#if status?.is_repo && status.files.length}<span class="n">{status.files.length}</span>{/if}
      </button>
      <button type="button" role="tab" aria-selected={view === "log"} class:on={view === "log"} onclick={() => setView("log")}>{tr("canvas.files.viewHistory", "History")}</button>
      {/if}
    </div>
    {#if remote}<span class="branch" title={tr("canvas.files.remoteHint", "Files on a remote environment")}>⇄ {typeof envData === "string" ? envData : String((envData as { kind?: string; host?: string })?.host ?? (envData as { kind?: string })?.kind ?? "")}</span>{/if}
    {#if status?.is_repo && status.branch}<span class="branch" title={status.branch}>⎇ {status.branch}</span>{/if}
    <span class="spacer"></span>
    <button type="button" class="tool" class:on={hidden} title={tr("canvas.files.hidden", "Show hidden files")} aria-pressed={hidden} onclick={() => setData({ showsHiddenFiles: hidden ? null : true })}>.*</button>
    <button type="button" class="tool" title={tr("canvas.files.refresh", "Refresh")} onclick={() => void refresh()}>↻</button>
    {#if !remote}<button type="button" class="tool" title={tr("canvas.files.chooseRoot", "Choose folder…")} onclick={() => void chooseRoot()}>⋯</button>{/if}
    {#if status?.is_repo && root}
      <GitActionsButton path={root} compact ops={gitOps} onchanged={() => void refresh()} />
    {/if}
  </div>

  {#if !root}
    <div class="empty">
      <p>{tr("canvas.files.noRoot", "This workspace has no folder yet.")}</p>
      <button type="button" data-canvas-chrome onclick={() => void chooseRoot()}>{tr("canvas.files.chooseRoot", "Choose folder…")}</button>
    </div>
  {:else if view === "diff"}
    <div class="diff" data-canvas-chrome>
      {#if diffLoading && !diffFiles.length}
        <p class="muted">{tr("canvas.files.loading", "Loading…")}</p>
      {:else}
        {#if changes.length}
          <div class="changes" role="list">
            {#each changes as c (c.rel)}
              <div class="chg" role="listitem" class:busy={gitBusy === c.rel}>
                <span class="git g-{statusLetter(c.kind)}">{statusLetter(c.kind)}</span>
                <button type="button" class="cname" title={c.rel} onclick={() => openEntry({ name: baseName(c.rel), path: c.rel, isDir: false, isSymlink: false, hidden: false, size: 0 })}>{c.rel}</button>
                {#if c.staged}<span class="stg" title={tr("canvas.files.staged", "Staged")}>●</span>{/if}
                {#if c.kind === "conflict"}
                  <button type="button" class="mini" onclick={() => void showConflict(c.rel)}>{tr("canvas.files.conflictView", "Current / Incoming")}</button>
                {/if}
                {#if c.unstaged || c.kind === "conflict"}
                  <button type="button" class="mini" title={tr("canvas.files.stageHint", "Stage this file")} onclick={() => void fileGit("stage", c.rel)}>{tr("canvas.files.stage", "Stage")}</button>
                {/if}
                {#if c.staged}
                  <button type="button" class="mini" title={tr("canvas.files.unstageHint", "Unstage this file")} onclick={() => void fileGit("unstage", c.rel)}>{tr("canvas.files.unstage", "Unstage")}</button>
                {/if}
                {#if confirmDiscard === c.rel}
                  <button type="button" class="mini danger" onclick={() => { confirmDiscard = null; void fileGit("discard", c.rel); }}>{tr("canvas.files.confirmDiscard", "Discard? Click again")}</button>
                {:else}
                  <button type="button" class="mini danger" title={tr("canvas.files.discardHint", "Drop the changes of this file (new files go to the trash)")} onclick={() => (confirmDiscard = c.rel)}>{tr("canvas.files.discard", "Discard")}</button>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
        {#if conflict}
          <div class="conflict">
            <div class="chead">
              <span class="cpath">{conflict.path}</span>
              <button type="button" class="mini" onclick={() => (conflict = null)}>×</button>
            </div>
            <div class="sides">
              <div class="side"><div class="slabel cur">{conflict.currentLabel}</div><pre>{conflict.current ?? tr("canvas.files.conflictMissing", "(deleted on this side)")}</pre></div>
              <div class="side"><div class="slabel inc">{conflict.incomingLabel}</div><pre>{conflict.incoming ?? tr("canvas.files.conflictMissing", "(deleted on this side)")}</pre></div>
            </div>
          </div>
        {/if}
        <div class="dv" data-follow={followPath ?? ""}>
          <DiffViewer files={diffFiles} height="100%" collapsedByDefault={diffFiles.length > 8} revealPath={selected ? null : followPath} emptyText={tr("canvas.files.noChanges", "No changes")} />
        </div>
      {/if}
    </div>
  {:else if view === "log"}
    <div class="log" data-canvas-chrome>
      {#each log as c (c.oid)}
        <div class="commit" title={`${c.oid}\n${c.author} <${c.email}>`}>
          <span class="sha">{c.short}</span>
          <span class="subject">{c.subject}</span>
          {#if c.refs}<span class="refs">{c.refs}</span>{/if}
          <span class="when">{c.author} · {when(c.at)}</span>
        </div>
      {:else}
        <p class="muted">{status?.is_repo ? tr("canvas.files.loading", "Loading…") : tr("canvas.files.notRepo", "Not a git repository")}</p>
      {/each}
    </div>
  {:else}
    <div class="search" data-canvas-chrome>
      <input
        bind:value={query}
        onkeydown={searchKey}
        placeholder={tr("canvas.files.search", "Search (> searches inside files)")}
        spellcheck="false"
      />
    </div>
    <div class="list" data-canvas-chrome role="tree" aria-label={baseName(root)}>
      {#if query.trim()}
        {#each hits as h (h.path + ":" + (h.line ?? ""))}
          <button
            type="button"
            class="row hit"
            onclick={() => openEntry({ name: baseName(h.path), path: h.path, isDir: h.isDir, isSymlink: false, hidden: false, size: 0 }, h.line)}
          >
            <span class="ico">{h.isDir ? "▸" : "·"}</span>
            <span class="name">{h.path}{h.line ? `:${h.line}` : ""}</span>
            {#if h.preview}<span class="preview">{h.preview}</span>{/if}
          </button>
        {:else}
          <p class="muted">{searching ? tr("canvas.files.searching", "Searching…") : tr("canvas.files.noResults", "Nothing found")}</p>
        {/each}
      {:else}
        {#if pinned.length}
          <div class="section">{tr("canvas.files.pinned", "Pinned")}</div>
          {#each pinned as p (p)}
            {@const e = pinnedEntry(p)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="row" role="treeitem" aria-selected={active === p} tabindex="-1" onclick={() => openEntry(e)} oncontextmenu={(ev) => openMenu(ev, e)}>
              <span class="ico">📌</span>
              <span class="name">{p}</span>
            </div>
          {/each}
          <div class="section">{baseName(root)}</div>
        {/if}
        {#if editing && editing.mode !== "rename" && editing.dir === ""}
          <input class="edit" style:margin-left="8px" bind:this={editInput} bind:value={editing.text} onkeydown={editKey} onblur={() => void commitEdit()} placeholder={editing.mode === "new-dir" ? tr("canvas.files.folderName", "Folder name") : tr("canvas.files.fileName", "File name")} />
        {/if}
        {#each rows as { entry, depth } (entry.path)}
          {@const letter = badges.map.get(entry.path)}
          {@const dirty = entry.isDir && badges.dirs.has(entry.path)}
          {#if editing?.mode === "rename" && editing.entry?.path === entry.path}
            <input class="edit" style:margin-left="{8 + depth * 14}px" bind:this={editInput} bind:value={editing.text} onkeydown={editKey} onblur={() => void commitEdit()} />
          {:else}
            <div
              class="row"
              class:active={active === entry.path}
              class:hidden-file={entry.hidden}
              role="treeitem"
              aria-selected={active === entry.path}
              aria-expanded={entry.isDir ? expanded.has(entry.path) : undefined}
              tabindex="-1"
              style:padding-left="{8 + depth * 14}px"
              onpointerdown={(e) => rowDown(e, entry)}
              onpointermove={rowMove}
              onpointerup={(e) => rowUp(e, entry)}
              oncontextmenu={(e) => openMenu(e, entry)}
              onkeydown={(e) => {
                if (e.key === "Enter") openEntry(entry);
              }}
            >
              <span class="ico">{entry.isDir ? (expanded.has(entry.path) ? "▾" : "▸") : ""}</span>
              <span class="name" class:dir={entry.isDir}>{entry.name}</span>
              {#if loadingDirs[entry.path]}<span class="muted small">…</span>{/if}
              {#if letter}<span class="git g-{letter}">{letter}</span>{:else if dirty}<span class="git dotted">•</span>{/if}
            </div>
          {/if}
          {#if editing && editing.mode !== "rename" && entry.isDir && editing.dir === entry.path}
            <input class="edit" style:margin-left="{8 + (depth + 1) * 14}px" bind:this={editInput} bind:value={editing.text} onkeydown={editKey} onblur={() => void commitEdit()} placeholder={editing.mode === "new-dir" ? tr("canvas.files.folderName", "Folder name") : tr("canvas.files.fileName", "File name")} />
          {/if}
        {:else}
          {#if !loadingDirs[""]}<p class="muted">{tr("canvas.files.emptyFolder", "Empty folder")}</p>{/if}
        {/each}
      {/if}
    </div>
  {/if}

  {#if error}
    <div class="error" data-canvas-chrome role="status">
      <span>{error}</span>
      <button type="button" onclick={() => (error = null)}>×</button>
    </div>
  {/if}

  {#if menu}
    {@const entry = menu.entry}
    <div class="menu" data-canvas-chrome role="menu" tabindex="-1" use:portal bind:this={menuEl} style:left="{menu.x}px" style:top="{menu.y}px" onpointerdown={(e) => e.stopPropagation()}>
      {#if entry && !entry.isDir}
        <button type="button" role="menuitem" onclick={() => { const target = entry; menu = null; openEntry(target); }}>{tr("canvas.files.open", "Open")}</button>
      {/if}
      <button type="button" role="menuitem" onclick={() => void startEdit("new-file", entry)}>{tr("canvas.files.newFile", "New File")}</button>
      <button type="button" role="menuitem" onclick={() => void startEdit("new-dir", entry)}>{tr("canvas.files.newFolder", "New Folder")}</button>
      {#if entry}
        <button type="button" role="menuitem" onclick={() => void startEdit("rename", entry)}>{tr("canvas.files.rename", "Rename")}</button>
        <button type="button" role="menuitem" onclick={() => { const path = entry.path; menu = null; togglePin(path); }}>
          {pinned.includes(entry.path) ? tr("canvas.files.unpin", "Unpin") : tr("canvas.files.pin", "Pin")}
        </button>
        <button type="button" role="menuitem" onclick={() => void copyPath(entry)}>{tr("canvas.files.copyPath", "Copy Path")}</button>
        <div class="sep"></div>
        {#if confirmDelete === entry.path}
          <button type="button" role="menuitem" class="danger" onclick={() => void doDelete(entry)}>{remote ? tr("canvas.files.deletePermanently", "Delete Permanently") : tr("canvas.files.confirmDelete", "Move to trash? Click again")}</button>
        {:else}
          <button type="button" role="menuitem" class="danger" onclick={() => (confirmDelete = entry.path)}>{tr("canvas.files.delete", "Delete…")}</button>
        {/if}
      {/if}
      <div class="sep"></div>
      <div class="mlabel">{tr("canvas.files.sortBy", "Sort by")}</div>
      {#each [["name", "Name"], ["type", "Type"], ["modified", "Date Modified"], ["size", "Size"]] as [k, label] (k)}
        <button type="button" role="menuitemradio" aria-checked={sortBy === k} onclick={() => setSort(k as SortBy)}>
          <span class="tick">{sortBy === k ? "✓" : ""}</span>{tr(`canvas.files.sort.${k}`, label)}
        </button>
      {/each}
      <button type="button" role="menuitemcheckbox" aria-checked={sortReverse} onclick={() => { menu = null; setData({ sortReverse: sortReverse ? null : true }); }}>
        <span class="tick">{sortReverse ? "✓" : ""}</span>{tr("canvas.files.sortReverse", "Reverse Order")}
      </button>
      <button type="button" role="menuitemcheckbox" aria-checked={foldersFirst} onclick={() => { menu = null; setData({ foldersFirst: foldersFirst ? false : null }); }}>
        <span class="tick">{foldersFirst ? "✓" : ""}</span>{tr("canvas.files.foldersFirst", "Folders on Top")}
      </button>
      <div class="sep"></div>
      <button type="button" role="menuitem" onclick={() => { menu = null; void refresh(); }}>{tr("canvas.files.reload", "Reload")}</button>
    </div>
  {/if}
</div>

{#if drag?.moved}
  <div class="ghost" use:portal style:left="{drag.x + 12}px" style:top="{drag.y + 8}px">{drag.entry.name}</div>
{/if}

<svelte:window onpointerdown={(e) => {
  if (menu && !(e.target as HTMLElement).closest(".menu")) {
    menu = null;
    confirmDelete = null;
  }
}} />

<style>
  .tree {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: var(--text);
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    border-bottom: 1px solid var(--border);
    user-select: none;
  }
  .seg {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    border: none;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 11px;
    padding: 2px 7px;
    cursor: pointer;
  }
  .seg button.on {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--text);
  }
  .n {
    margin-left: 4px;
    font-size: 10px;
    opacity: 0.8;
  }
  .branch {
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 10px var(--font-mono, monospace);
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .tool {
    min-width: 22px;
    height: 20px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .tool:hover,
  .tool.on {
    border-color: var(--border);
    color: var(--text);
  }
  .search {
    flex: none;
    padding: 5px 6px;
  }
  .search input {
    width: 100%;
    box-sizing: border-box;
    font: inherit;
    padding: 4px 7px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--input-bg, transparent);
    color: var(--text);
    outline: none;
  }
  .search input:focus {
    border-color: var(--accent);
  }
  .list,
  .log,
  .diff {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .list {
    padding: 2px 0 8px;
  }
  .section {
    padding: 6px 8px 2px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    box-sizing: border-box;
    padding: 2px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: default;
    white-space: nowrap;
    outline: none;
  }
  .row:hover {
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .row.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .row.hidden-file .name {
    opacity: 0.6;
  }
  .row.hit {
    flex-wrap: wrap;
    white-space: normal;
  }
  .ico {
    flex: none;
    width: 12px;
    color: var(--text-muted);
    font-size: 10px;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name.dir {
    font-weight: 500;
  }
  .preview {
    width: 100%;
    padding-left: 17px;
    color: var(--text-muted);
    font: 11px var(--font-mono, monospace);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .git {
    flex: none;
    font: 600 10px var(--font-mono, monospace);
  }
  .g-M {
    color: #dcab3c;
  }
  .g-A,
  .g-U {
    color: #5ca97a;
  }
  .g-D,
  .g-C {
    color: #d96a62;
  }
  .g-R {
    color: #6cb6ff;
  }
  .dotted {
    color: #dcab3c;
  }
  .edit {
    display: block;
    width: calc(100% - 24px);
    font: inherit;
    padding: 2px 5px;
    margin: 1px 0;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--input-bg, transparent);
    color: var(--text);
    outline: none;
  }
  .commit {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 8px;
    padding: 5px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .sha {
    font: 11px var(--font-mono, monospace);
    color: var(--accent);
  }
  .subject {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .refs {
    font-size: 10px;
    color: #5ca97a;
  }
  .when {
    width: 100%;
    color: var(--text-muted);
    font-size: 10px;
  }
  .muted {
    color: var(--text-muted);
    padding: 8px;
    margin: 0;
  }
  .small {
    padding: 0;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-muted);
  }
  .empty button {
    font: inherit;
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .error {
    flex: none;
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 4px 8px;
    color: var(--danger, #d96a62);
    font-size: 11px;
    border-top: 1px solid var(--border);
  }
  .error span {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .error button {
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .menu {
    position: fixed;
    z-index: 1000;
    font-size: 12px;
    max-height: calc(100vh - 16px);
    overflow: auto;
    min-width: 150px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    border-radius: 9px;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.3);
  }
  .menu button {
    text-align: left;
    padding: 5px 9px;
    border: none;
    border-radius: 5px;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .menu button:hover {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .menu .danger {
    color: var(--danger, #d96a62);
  }
  .mlabel {
    padding: 4px 9px 2px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .tick {
    display: inline-block;
    width: 14px;
  }
  .dv {
    height: 100%;
    min-height: 200px;
  }
  .changes {
    border-bottom: 1px solid var(--border);
    padding: 4px 0;
  }
  .chg {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
  }
  .chg.busy {
    opacity: 0.5;
  }
  .chg:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .cname {
    flex: 1;
    min-width: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
    padding: 0;
  }
  .stg {
    color: var(--accent);
    font-size: 9px;
  }
  .mini {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: 10px;
    padding: 0 5px;
    border-radius: 4px;
    cursor: pointer;
  }
  .mini:hover {
    color: var(--text);
  }
  .mini.danger {
    color: var(--danger, #d96a62);
  }
  .conflict {
    border-bottom: 1px solid var(--border);
    padding: 4px 8px 8px;
  }
  .chead {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .cpath {
    flex: 1;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
  }
  .sides {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    margin-top: 4px;
  }
  .side pre {
    margin: 0;
    max-height: 200px;
    overflow: auto;
    font-size: 10px;
    padding: 6px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--text) 5%, transparent);
    user-select: text;
  }
  .slabel {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 0;
  }
  .slabel.cur {
    color: #3b82f6;
  }
  .slabel.inc {
    color: #10b981;
  }
  .sep {
    height: 1px;
    margin: 3px 5px;
    background: var(--border);
  }
  .ghost {
    position: fixed;
    z-index: 1000;
    pointer-events: none;
    padding: 3px 8px;
    border-radius: 6px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--accent);
    font-size: 12px;
    color: var(--text);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.3);
  }
</style>
