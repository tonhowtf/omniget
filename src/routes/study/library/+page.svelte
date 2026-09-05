<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { goto, beforeNavigate, replaceState } from "$app/navigation";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import { page } from "$app/stores";
  import SegmentedControl from "$lib/study-components/SegmentedControl.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";
  import LibraryHomeShelves from "$lib/study-components/shelves/LibraryHomeShelves.svelte";
  import SortDropdown from "$lib/study-components/shelves/SortDropdown.svelte";
  import SearchInput from "$lib/study-components/shelves/SearchInput.svelte";
  import type { CatalogSort, CatalogSortDirection } from "$lib/study-bridge";
  import TelegramAuthGate from "$lib/study-components/TelegramAuthGate.svelte";
  import TelegramBrowser from "$lib/study-components/TelegramBrowser.svelte";
  import {
    telegramSessionState,
    studyDefaultOutputDir,
    type TelegramSessionState,
  } from "$lib/study-telegram-bridge";

  type Course = {
    id: number;
    title: string;
    source_path: string;
    thumbnail_path: string | null;
    added_at: string;
    last_scan_at: string;
    favorite: boolean;
    total_lessons: number;
    completed_lessons: number;
    progress_pct: number;
  };

  type SortKey = CatalogSort;
  type ViewMode = "courses" | "browse" | "telegram";
  type CoursesView = "home" | "catalog";

  type RootEntry = {
    path: string;
    enabled: boolean;
    added_at: number;
    is_default: boolean;
    exists?: boolean;
  };

  type BrowseFolder = {
    path: string;
    name: string;
    video_count: number;
    preview_videos?: string[];
    latest_mtime?: number | null;
  };
  type BrowseVideo = {
    path: string;
    name: string;
    stem: string;
    size: number;
    subtitle_path: string | null;
    thumbnail_path: string | null;
  };
  type BrowseDocument = {
    path: string;
    name: string;
    ext: string;
    size: number;
  };
  type BrowseCrumb = { path: string; name: string };
  type BrowseData = {
    path: string;
    parent: string | null;
    is_root_list: boolean;
    folders: BrowseFolder[];
    videos: BrowseVideo[];
    documents: BrowseDocument[];
    breadcrumb: BrowseCrumb[];
  };

  type LibraryCourse = {
    id: number;
    title: string;
    tags: string[];
    lesson_count: number;
    completed_lessons: number;
    total_duration_ms: number;
    last_opened_at: number | null;
  };

  type TagSummary = { tag: string; course_count: number };
  type PlaylistSummary = {
    id: number;
    name: string;
    description: string | null;
    color: string | null;
    course_count: number;
    updated_at: number;
  };

  let courses = $state<Course[]>([]);
  let loading = $state(true);
  let error = $state("");
  let search = $state("");
  let sortKey = $state<SortKey>("last_watched");
  let sortDirection = $state<CatalogSortDirection>("desc");
  let favoritesOnly = $state(false);
  let coursesView = $state<CoursesView>("home");
  let gridPageSize = $state(30);
  let gridSentinel = $state<HTMLElement | null>(null);

  $effect(() => {
    if (!gridSentinel) return;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting && gridPageSize < courses.length) {
            gridPageSize = Math.min(gridPageSize + 30, courses.length);
          }
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(gridSentinel);
    return () => observer.disconnect();
  });

  $effect(() => {
    void search;
    void statusFilter;
    void sortKey;
    void sortDirection;
    gridPageSize = 30;
  });

  let allTags = $state<TagSummary[]>([]);
  let courseTagsMap = $state<Map<number, string[]>>(new Map());
  let totalDurationByCourse = $state<Map<number, number>>(new Map());
  let lastOpenedByCourse = $state<Map<number, number>>(new Map());
  let playlists = $state<PlaylistSummary[]>([]);
  let playlistCoursesCache = $state<Map<number, number[]>>(new Map());

  let includeTags = $state<Set<string>>(new Set());
  let excludeTags = $state<Set<string>>(new Set());
  let statusFilter = $state<"all" | "not_started" | "in_progress" | "completed">("all");
  let playlistFilter = $state<number | null>(null);
  let filtersOpen = $state(true);

  let mode = $state<ViewMode>("courses");

  function buildLibraryUrlSearch(): string {
    const params = new URLSearchParams();
    if (mode !== "courses") params.set("mode", mode);
    if (mode === "courses" && coursesView !== "home") {
      params.set("view", coursesView);
    }
    if (mode === "browse" && browsePath.length > 0) params.set("bp", browsePath);
    if (sortKey !== "last_watched" || sortDirection !== "desc") {
      params.set("sort", sortKey);
      if (sortDirection !== "desc") params.set("dir", sortDirection);
    }
    if (search.trim().length > 0) params.set("q", search.trim());
    if (statusFilter !== "all") params.set("status", statusFilter);
    if (playlistFilter != null) params.set("pl", String(playlistFilter));
    if (includeTags.size > 0) params.set("inc", [...includeTags].join(","));
    if (excludeTags.size > 0) params.set("exc", [...excludeTags].join(","));
    if (favoritesOnly) params.set("fav", "1");
    const qs = params.toString();
    return qs.length > 0 ? `?${qs}` : "";
  }

  function buildCurrentLibraryUrl(): string {
    if (typeof window === "undefined") return "/study/library";
    return `${window.location.pathname}${buildLibraryUrlSearch()}`;
  }

  function syncLibraryUrl() {
    if (typeof window === "undefined") return;
    const next = buildLibraryUrlSearch();
    if (window.location.search === next) return;
    try {
      replaceState(`${window.location.pathname}${next}`, {});
    } catch {
      window.history.replaceState(null, "", `${window.location.pathname}${next}`);
    }
  }

  function readUrlParams() {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    if (import.meta.env.DEV) {
      console.log("[lib] readUrlParams from", window.location.search);
    }
    let bpFromUrl = params.get("bp");
    if (!params.get("mode") && !bpFromUrl) {
      try {
        const raw = sessionStorage.getItem("study.library.return_state");
        if (raw) {
          const fallback = JSON.parse(raw) as {
            mode?: string;
            browsePath?: string;
            search?: string;
          };
          if (
            fallback.mode === "browse" ||
            fallback.mode === "courses" ||
            fallback.mode === "telegram"
          ) {
            params.set("mode", fallback.mode);
          }
          if (fallback.browsePath) {
            params.set("bp", fallback.browsePath);
            bpFromUrl = fallback.browsePath;
          }
          if (fallback.search) params.set("q", fallback.search);
          if (import.meta.env.DEV) {
            console.log("[lib] used return_state fallback", fallback);
          }
        }
      } catch {
        /* malformed; ignore */
      }
    }
    const m = params.get("mode");
    if (m === "courses" || m === "browse" || m === "telegram") mode = m;
    const v = params.get("view");
    if (v === "catalog" || v === "home") coursesView = v;
    const bp = params.get("bp");
    if (bp) browsePath = bp;
    const s = params.get("sort");
    if (s) sortKey = s as SortKey;
    const d = params.get("dir");
    if (d === "asc" || d === "desc") sortDirection = d;
    const q = params.get("q");
    if (q) search = q;
    const st = params.get("status");
    if (
      st === "all" ||
      st === "not_started" ||
      st === "in_progress" ||
      st === "completed"
    ) {
      statusFilter = st;
    }
    const pl = params.get("pl");
    if (pl) {
      const n = Number(pl);
      if (Number.isFinite(n)) playlistFilter = n;
    }
    const inc = params.get("inc");
    if (inc) includeTags = new Set(inc.split(",").filter(Boolean));
    const exc = params.get("exc");
    if (exc) excludeTags = new Set(exc.split(",").filter(Boolean));
    if (params.get("fav") === "1") favoritesOnly = true;
  }

  $effect(() => {
    void mode;
    void coursesView;
    void browsePath;
    void sortKey;
    void sortDirection;
    void search;
    void statusFilter;
    void playlistFilter;
    void includeTags;
    void excludeTags;
    void favoritesOnly;
    syncLibraryUrl();
  });

  function onSortChange(s: CatalogSort, d: CatalogSortDirection) {
    sortKey = s;
    sortDirection = d;
  }

  function onSearchChange(next: string) {
    search = next;
  }

  let rootsOpen = $state(false);
  let roots = $state<RootEntry[]>([]);
  let browsePath = $state<string>("");
  let browseData = $state<BrowseData | null>(null);
  let browseLoading = $state(false);
  let rescanning = $state(false);
  let rescanMsg = $state("");

  type PinnedPath = {
    id: number;
    path: string;
    label: string | null;
    color: string | null;
    position: number;
    added_at: number;
  };
  let pins = $state<PinnedPath[]>([]);

  let openCrumbIndex = $state<number | null>(null);
  let crumbSiblingsCache = $state<Map<string, BrowseFolder[]>>(new Map());
  let crumbSiblingsLoading = $state<string | null>(null);

  let browseViewMode = $state<"grid" | "list">("grid");
  let lastWatchedInFolder = $state<string | null>(null);

  type BrowseSort = "name_asc" | "name_desc" | "size_desc";
  let browseSort = $state<BrowseSort>("name_asc");
  let sortMenuOpen = $state(false);

  let selectedPaths = $state<Set<string>>(new Set());
  let lastSelectionAnchor = $state<string | null>(null);
  let bulkDeleteOpen = $state(false);

  type Tab = { path: string };
  let tabs = $state<Tab[]>([]);
  let activeTabIndex = $state(0);

  function tabLabel(path: string): string {
    if (!path) return "Raízes";
    const norm = path.replace(/\\/g, "/").replace(/\/+$/, "");
    const seg = norm.split("/").pop();
    return seg && seg.length > 0 ? seg : path;
  }

  function saveTabs() {
    if (typeof window === "undefined") return;
    try {
      sessionStorage.setItem(
        "study.library.tabs.v1",
        JSON.stringify({ tabs, activeTabIndex }),
      );
    } catch {
      /* ignore */
    }
  }

  function loadTabsFromStorage(): { tabs: Tab[]; activeTabIndex: number } | null {
    if (typeof window === "undefined") return null;
    try {
      const raw = sessionStorage.getItem("study.library.tabs.v1");
      if (!raw) return null;
      const parsed = JSON.parse(raw) as { tabs?: Tab[]; activeTabIndex?: number };
      if (!Array.isArray(parsed.tabs) || parsed.tabs.length === 0) return null;
      const idx = Math.max(0, Math.min(parsed.tabs.length - 1, parsed.activeTabIndex ?? 0));
      return { tabs: parsed.tabs, activeTabIndex: idx };
    } catch {
      return null;
    }
  }

  function switchTab(idx: number) {
    if (idx < 0 || idx >= tabs.length) return;
    activeTabIndex = idx;
    const target = tabs[idx].path;
    saveTabs();
    if (target !== browsePath) {
      void loadBrowse(target);
    }
  }

  function openInNewTab(path: string) {
    tabs = [...tabs, { path }];
    activeTabIndex = tabs.length - 1;
    saveTabs();
    if (mode !== "browse") mode = "browse";
    void loadBrowse(path);
  }

  function closeTab(idx: number) {
    if (tabs.length <= 1) {
      tabs = [];
      activeTabIndex = 0;
      saveTabs();
      return;
    }
    const wasActive = idx === activeTabIndex;
    const next = tabs.filter((_, i) => i !== idx);
    let nextActive = activeTabIndex;
    if (wasActive) {
      nextActive = Math.max(0, idx - 1);
    } else if (idx < activeTabIndex) {
      nextActive = activeTabIndex - 1;
    }
    tabs = next;
    activeTabIndex = nextActive;
    saveTabs();
    if (wasActive && tabs[nextActive]) {
      void loadBrowse(tabs[nextActive].path);
    }
  }

  function selectionAllPaths(): string[] {
    if (!visibleBrowse) return [];
    return [
      ...visibleBrowse.folders.map((f) => f.path),
      ...visibleBrowse.videos.map((v) => v.path),
      ...(visibleBrowse.documents ?? []).map((d) => d.path),
    ];
  }

  function clearSelection() {
    selectedPaths = new Set();
    lastSelectionAnchor = null;
  }

  function toggleSelection(path: string) {
    const next = new Set(selectedPaths);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selectedPaths = next;
    lastSelectionAnchor = path;
  }

  function selectRange(toPath: string) {
    if (!lastSelectionAnchor || !visibleBrowse) {
      toggleSelection(toPath);
      return;
    }
    const all = selectionAllPaths();
    const a = all.indexOf(lastSelectionAnchor);
    const b = all.indexOf(toPath);
    if (a < 0 || b < 0) {
      toggleSelection(toPath);
      return;
    }
    const [from, to] = a <= b ? [a, b] : [b, a];
    const next = new Set(selectedPaths);
    for (let i = from; i <= to; i++) next.add(all[i]);
    selectedPaths = next;
  }

  function selectAllVisible() {
    selectedPaths = new Set(selectionAllPaths());
    lastSelectionAnchor = null;
  }

  function handleFolderClick(e: MouseEvent, f: BrowseFolder) {
    if (renamingPath === f.path) return;
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      toggleSelection(f.path);
      return;
    }
    if (e.shiftKey) {
      e.preventDefault();
      selectRange(f.path);
      return;
    }
    clearSelection();
    void loadBrowse(f.path);
  }

  function handleVideoClick(e: MouseEvent, v: BrowseVideo) {
    if (renamingPath === v.path) return;
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      toggleSelection(v.path);
      return;
    }
    if (e.shiftKey) {
      e.preventDefault();
      selectRange(v.path);
      return;
    }
    clearSelection();
    openBrowseVideo(v);
  }

  function handleDocClick(e: MouseEvent, d: BrowseDocument) {
    if (renamingPath === d.path) return;
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      toggleSelection(d.path);
      return;
    }
    if (e.shiftKey) {
      e.preventDefault();
      selectRange(d.path);
      return;
    }
    clearSelection();
    void openBrowseDocument(d);
  }

  async function performBulkDelete() {
    const targets = [...selectedPaths];
    if (targets.length === 0) {
      bulkDeleteOpen = false;
      return;
    }
    let ok = 0;
    let fail = 0;
    for (const path of targets) {
      try {
        await pluginInvoke("study", "study:browse:delete", { path });
        ok += 1;
      } catch (e) {
        fail += 1;
        if (import.meta.env.DEV) {
          console.warn("[library] bulk delete failed for", path, e);
        }
      }
    }
    bulkDeleteOpen = false;
    clearSelection();
    await loadBrowse(browsePath);
    if (fail === 0) {
      showToast(
        "success",
        $t("study.library.selection_delete_done", { count: ok }) as string,
      );
    } else {
      showToast(
        "info",
        $t("study.library.selection_delete_partial", { ok, fail }) as string,
      );
    }
  }

  function setBrowseSort(s: BrowseSort) {
    browseSort = s;
    sortMenuOpen = false;
  }

  $effect(() => {
    if (!sortMenuOpen) return;
    function onDoc(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (t && t.closest(".sort-menu, .sort-btn")) return;
      sortMenuOpen = false;
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  });

  let addressBarOpen = $state(false);
  let addressBarValue = $state("");
  let addressBarRef = $state<HTMLInputElement | null>(null);

  let creatingFolder = $state(false);
  let newFolderName = $state("");
  let newFolderRef = $state<HTMLInputElement | null>(null);

  let renamingPath = $state<string | null>(null);
  let renameValue = $state("");

  type CtxMenuTarget =
    | { kind: "folder"; path: string; name: string }
    | { kind: "video"; path: string; name: string }
    | { kind: "doc"; path: string; name: string };

  let contextMenu = $state<{
    x: number;
    y: number;
    target: CtxMenuTarget;
  } | null>(null);

  let confirmDeleteOpen = $state(false);
  let pendingDelete = $state<CtxMenuTarget | null>(null);

  function openContextMenu(event: MouseEvent, target: CtxMenuTarget) {
    event.preventDefault();
    contextMenu = { x: event.clientX, y: event.clientY, target };
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  $effect(() => {
    if (!contextMenu) return;
    function onDoc(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (t && t.closest(".ctx-menu")) return;
      closeContextMenu();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") closeContextMenu();
    }
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  });

  async function showInExplorer(path: string) {
    try {
      await pluginInvoke("study", "study:shell:show_in_folder", { path });
    } catch (e) {
      console.error("show_in_folder failed", e);
    }
    closeContextMenu();
  }

  function startRename(target: CtxMenuTarget) {
    renamingPath = target.path;
    renameValue = target.name;
    closeContextMenu();
  }

  async function confirmRename(targetPath: string) {
    const trimmed = renameValue.trim();
    if (!trimmed) {
      cancelRename();
      return;
    }
    try {
      await pluginInvoke("study", "study:browse:rename", {
        path: targetPath,
        newName: trimmed,
      });
      renamingPath = null;
      renameValue = "";
      await loadBrowse(browsePath);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  function cancelRename() {
    renamingPath = null;
    renameValue = "";
  }

  function startDelete(target: CtxMenuTarget) {
    pendingDelete = target;
    confirmDeleteOpen = true;
    closeContextMenu();
  }

  async function performDelete() {
    if (!pendingDelete) return;
    const target = pendingDelete;
    try {
      await pluginInvoke("study", "study:browse:delete", { path: target.path });
      await loadBrowse(browsePath);
      showToast(
        "success",
        $t("study.library.delete_toast_done", { name: target.name }) as string,
      );
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    } finally {
      pendingDelete = null;
      confirmDeleteOpen = false;
    }
  }

  function startCreateFolder() {
    creatingFolder = true;
    newFolderName = "";
    queueMicrotask(() => newFolderRef?.focus());
  }

  async function confirmCreateFolder() {
    const trimmed = newFolderName.trim();
    if (!trimmed) {
      cancelCreateFolder();
      return;
    }
    try {
      await pluginInvoke("study", "study:browse:create_folder", {
        parent: browsePath,
        name: trimmed,
      });
      creatingFolder = false;
      newFolderName = "";
      await loadBrowse(browsePath);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  function cancelCreateFolder() {
    creatingFolder = false;
    newFolderName = "";
  }

  function openAddressBar() {
    addressBarValue = browsePath;
    addressBarOpen = true;
    queueMicrotask(() => {
      addressBarRef?.focus();
      addressBarRef?.select();
    });
  }

  function submitAddressBar() {
    const v = addressBarValue.trim();
    addressBarOpen = false;
    if (v && v !== browsePath) {
      void loadBrowse(v);
    }
  }

  function closeAddressBar() {
    addressBarOpen = false;
    addressBarValue = "";
  }

  function autofocusOnRender(node: HTMLInputElement) {
    node.focus();
    if (node.value.length > 0) {
      node.select();
    }
  }

  function isTypingTarget(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    const tag = target.tagName;
    return (
      tag === "INPUT" ||
      tag === "TEXTAREA" ||
      tag === "SELECT" ||
      target.isContentEditable
    );
  }

  $effect(() => {
    if (mode !== "browse") return;
    function onKey(e: KeyboardEvent) {
      const isAddressBarShortcut =
        (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "l";
      if (isAddressBarShortcut) {
        e.preventDefault();
        openAddressBar();
        return;
      }
      if (isTypingTarget(e.target)) return;
      if (openCrumbIndex !== null) return;
      if (contextMenu) return;
      if (renamingPath !== null) return;
      if (e.key === "Escape" && selectedPaths.size > 0) {
        e.preventDefault();
        clearSelection();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
        if (visibleBrowse) {
          e.preventDefault();
          selectAllVisible();
        }
        return;
      }
      const isBack =
        e.key === "Backspace" ||
        (e.altKey && e.key === "ArrowLeft") ||
        (e.metaKey && e.key === "ArrowLeft") ||
        (e.metaKey && e.key === "ArrowUp") ||
        (e.altKey && e.key === "ArrowUp");
      if (!isBack) return;
      const parent = browseData?.parent ?? null;
      if (!parent) return;
      e.preventDefault();
      void loadBrowse(parent);
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });

  async function toggleSiblings(idx: number, parentPath: string) {
    if (openCrumbIndex === idx) {
      openCrumbIndex = null;
      return;
    }
    if (!crumbSiblingsCache.has(parentPath)) {
      crumbSiblingsLoading = parentPath;
      try {
        const data = await pluginInvoke<BrowseData>(
          "study",
          "study:browse:list",
          { path: parentPath },
        );
        const next = new Map(crumbSiblingsCache);
        next.set(parentPath, data.folders ?? []);
        crumbSiblingsCache = next;
      } catch (e) {
        if (import.meta.env.DEV) {
          console.warn("[library] siblings fetch failed", e);
        }
        const next = new Map(crumbSiblingsCache);
        next.set(parentPath, []);
        crumbSiblingsCache = next;
      } finally {
        crumbSiblingsLoading = null;
      }
    }
    openCrumbIndex = idx;
  }

  function closeSiblings() {
    openCrumbIndex = null;
  }

  $effect(() => {
    if (openCrumbIndex == null) return;
    function onDocPointer(e: MouseEvent) {
      const target = e.target as HTMLElement | null;
      if (target && target.closest(".crumb-chevron-wrap")) return;
      closeSiblings();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") closeSiblings();
    }
    document.addEventListener("mousedown", onDocPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocPointer);
      document.removeEventListener("keydown", onKey);
    };
  });

  $effect(() => {
    void browsePath;
    crumbSiblingsCache = new Map();
    openCrumbIndex = null;
  });

  async function loadPins() {
    try {
      const res = await pluginInvoke<{ pins: PinnedPath[] }>(
        "study",
        "study:library:pins:list",
      );
      pins = res.pins ?? [];
    } catch (e) {
      if (import.meta.env.DEV) console.warn("[library] pins:list failed", e);
    }
  }

  async function togglePin(path: string, suggestedLabel?: string) {
    const isPinned = pins.some((p) => p.path === path);
    try {
      if (isPinned) {
        await pluginInvoke("study", "study:library:pins:remove", { path });
      } else {
        await pluginInvoke("study", "study:library:pins:add", {
          path,
          label: suggestedLabel ?? null,
        });
      }
      await loadPins();
      showToast(
        "success",
        $t(
          isPinned
            ? "study.library.pin_removed_toast"
            : "study.library.pin_added_toast",
          { name: suggestedLabel ?? path },
        ) as string,
      );
    } catch (e) {
      console.error("toggle pin failed", e);
      const msg = e instanceof Error ? e.message : String(e);
      showToast(
        "error",
        $t(
          isPinned
            ? "study.library.unpin_failed"
            : "study.library.pin_failed",
          { msg },
        ) as string,
      );
    }
  }

  function openPinnedPath(path: string) {
    if (mode !== "browse") {
      mode = "browse";
    }
    void loadBrowse(path);
  }


  let tgSession = $state<TelegramSessionState>({ kind: "unauthenticated" });
  let tgChecking = $state(false);
  let tgOutputDir = $state("");
  let downloadsActive = $state(0);

  let tgConnected = $derived(tgSession.kind === "authenticated");
  let telegramLabel = $derived(
    tgConnected
      ? ($t("study.library.mode_telegram") as string)
      : `${$t("study.library.mode_telegram") as string} ${$t("study.library.mode_telegram_disconnected_suffix") as string}`,
  );

  async function checkTelegramSession() {
    tgChecking = true;
    try {
      tgSession = await telegramSessionState();
      if (tgSession.kind === "authenticated" && !tgOutputDir) {
        try {
          const r = await studyDefaultOutputDir();
          tgOutputDir = r.path;
        } catch {
          /* leave empty; UI tells user */
        }
      }
    } finally {
      tgChecking = false;
    }
  }

  function onTelegramAuthenticated(_e: CustomEvent<{ phone: string }>) {
    void checkTelegramSession();
  }

  function onDownloadEnqueued() {
    goto("/downloads");
  }

  function onTelegramLogout() {
    void checkTelegramSession();
  }

  const playlistMembers = $derived.by(() => {
    if (playlistFilter == null) return null;
    const ids = playlistCoursesCache.get(playlistFilter);
    return ids ? new Set(ids) : new Set<number>();
  });

  const visibleCourses = $derived.by(() => {
    const term = search.trim().toLowerCase();
    const inc = includeTags;
    const exc = excludeTags;
    const members = playlistMembers;
    let list = courses.filter((c) => {
      if (favoritesOnly && !c.favorite) return false;
      if (term && !c.title.toLowerCase().includes(term)) return false;
      if (members && !members.has(c.id)) return false;
      if (statusFilter === "completed" && c.progress_pct < 100) return false;
      if (statusFilter === "not_started" && c.progress_pct > 0) return false;
      if (
        statusFilter === "in_progress" &&
        (c.progress_pct === 0 || c.progress_pct >= 100)
      )
        return false;
      const myTags = courseTagsMap.get(c.id) ?? [];
      if (inc.size > 0 && ![...inc].some((t) => myTags.includes(t)))
        return false;
      if (exc.size > 0 && [...exc].some((t) => myTags.includes(t)))
        return false;
      return true;
    });
    list = [...list].sort((a, b) => sortCompare(a, b, sortKey, sortDirection));
    return list;
  });

  function sortCompare(
    a: Course,
    b: Course,
    key: SortKey,
    dir: CatalogSortDirection,
  ): number {
    const sign = dir === "asc" ? 1 : -1;
    switch (key) {
      case "name":
        return sign * a.title.localeCompare(b.title);
      case "added":
        return sign * a.added_at.localeCompare(b.added_at);
      case "progress":
        return sign * (a.progress_pct - b.progress_pct);
      case "last_watched": {
        const la = lastOpenedByCourse.get(a.id) ?? 0;
        const lb = lastOpenedByCourse.get(b.id) ?? 0;
        return sign * (la - lb);
      }
      case "times_watched":
        return sign * (a.completed_lessons - b.completed_lessons);
      case "watched": {
        const wa = a.completed_lessons >= a.total_lessons && a.total_lessons > 0 ? 1 : 0;
        const wb = b.completed_lessons >= b.total_lessons && b.total_lessons > 0 ? 1 : 0;
        return sign * (wa - wb);
      }
      case "not_watched": {
        const wa = a.completed_lessons === 0 ? 1 : 0;
        const wb = b.completed_lessons === 0 ? 1 : 0;
        return sign * (wa - wb);
      }
      case "platform":
        return sign * a.title.localeCompare(b.title);
      default:
        return 0;
    }
  }

  function toggleIncludeTag(tag: string) {
    excludeTags.delete(tag);
    if (includeTags.has(tag)) includeTags.delete(tag);
    else includeTags.add(tag);
    includeTags = new Set(includeTags);
    excludeTags = new Set(excludeTags);
  }

  function toggleExcludeTag(tag: string) {
    includeTags.delete(tag);
    if (excludeTags.has(tag)) excludeTags.delete(tag);
    else excludeTags.add(tag);
    includeTags = new Set(includeTags);
    excludeTags = new Set(excludeTags);
  }

  function clearTagFilters() {
    includeTags = new Set();
    excludeTags = new Set();
  }

  async function loadEnrichment() {
    try {
      const [lib, tags, pls] = await Promise.all([
        pluginInvoke<LibraryCourse[]>("study", "study:courses:library", { filter: {} }),
        pluginInvoke<TagSummary[]>("study", "study:courses:tags:list_all"),
        pluginInvoke<PlaylistSummary[]>("study", "study:courses:playlists:list"),
      ]);
      const tagMap = new Map<number, string[]>();
      const durMap = new Map<number, number>();
      const openedMap = new Map<number, number>();
      for (const c of lib) {
        tagMap.set(c.id, c.tags);
        durMap.set(c.id, c.total_duration_ms);
        if (c.last_opened_at != null) {
          openedMap.set(c.id, c.last_opened_at);
        }
      }
      courseTagsMap = tagMap;
      totalDurationByCourse = durMap;
      lastOpenedByCourse = openedMap;
      allTags = tags;
      playlists = pls;
    } catch (e) {
      console.error("loadEnrichment failed", e);
    }
  }

  async function selectPlaylist(id: number | null) {
    playlistFilter = id;
    if (id != null && !playlistCoursesCache.has(id)) {
      try {
        const items = await pluginInvoke<{ course_id: number }[]>(
          "study",
          "study:courses:playlists:courses",
          { playlistId: id },
        );
        const map = new Map(playlistCoursesCache);
        map.set(
          id,
          items.map((i) => i.course_id),
        );
        playlistCoursesCache = map;
      } catch (e) {
        console.error("playlist courses failed", e);
      }
    }
  }

  const visibleBrowse = $derived.by<BrowseData | null>(() => {
    if (!browseData) return null;
    const term = search.trim().toLowerCase();
    let folders = browseData.folders;
    let videos = browseData.videos;
    let documents = browseData.documents ?? [];
    if (term) {
      folders = folders.filter((f) => f.name.toLowerCase().includes(term));
      videos = videos.filter((v) => v.name.toLowerCase().includes(term));
      documents = documents.filter((d) => d.name.toLowerCase().includes(term));
    }
    const dir = browseSort === "name_desc" ? -1 : 1;
    const cmpName = (a: { name: string }, b: { name: string }) =>
      dir * a.name.localeCompare(b.name, undefined, { numeric: true });
    const cmpSize = (a: { size?: number; name: string }, b: { size?: number; name: string }) => {
      const aS = a.size ?? 0;
      const bS = b.size ?? 0;
      if (aS === bS) return a.name.localeCompare(b.name, undefined, { numeric: true });
      return bS - aS;
    };
    const sortFn = browseSort === "size_desc" ? cmpSize : cmpName;
    return {
      ...browseData,
      folders: [...folders].sort(cmpName),
      videos: [...videos].sort(sortFn),
      documents: [...documents].sort(sortFn),
    };
  });

  async function loadCourses() {
    try {
      courses = await pluginInvoke<Course[]>("study", "study:courses:list");
      await loadEnrichment();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function loadRoots() {
    try {
      const res = await pluginInvoke<{ roots: RootEntry[] }>(
        "study",
        "study:library:roots_list",
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("roots_list failed", e);
    }
  }

  function captureScrollRatio(): number {
    if (typeof window === "undefined") return 0;
    const max = Math.max(
      1,
      document.documentElement.scrollHeight - window.innerHeight,
    );
    return Math.min(1, Math.max(0, window.scrollY / max));
  }

  async function saveBrowseStateFor(
    path: string,
    lastVideoPath: string | null = null,
  ) {
    if (!path || typeof window === "undefined") return;
    const ratio = captureScrollRatio();
    try {
      const args: {
        path: string;
        scrollRatio: number;
        viewMode: string;
        lastVideoPath?: string;
      } = { path, scrollRatio: ratio, viewMode: browseViewMode };
      if (lastVideoPath) args.lastVideoPath = lastVideoPath;
      await pluginInvoke("study", "study:library:browse_state:set", args);
    } catch (e) {
      if (import.meta.env.DEV) {
        console.warn("[library] save browse state failed", e);
      }
    }
  }

  async function restoreScrollFor(path: string) {
    if (!path || typeof window === "undefined") return;
    try {
      const state = await pluginInvoke<{
        scroll_ratio: number;
        view_mode: string;
        last_video_path: string | null;
      }>("study", "study:library:browse_state:get", { path });
      const restoredView = state?.view_mode;
      if (restoredView === "list" || restoredView === "grid") {
        browseViewMode = restoredView;
      }
      lastWatchedInFolder = state?.last_video_path ?? null;
      const ratio = state?.scroll_ratio ?? 0;
      if (ratio <= 0) return;
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          const max = Math.max(
            0,
            document.documentElement.scrollHeight - window.innerHeight,
          );
          window.scrollTo({ top: max * ratio, behavior: "auto" });
        });
      });
    } catch (e) {
      if (import.meta.env.DEV) {
        console.warn("[library] restore browse scroll failed", e);
      }
    }
  }

  async function setBrowseViewMode(next: "grid" | "list") {
    if (browseViewMode === next) return;
    browseViewMode = next;
    if (!browsePath) return;
    try {
      await pluginInvoke("study", "study:library:browse_state:set", {
        path: browsePath,
        viewMode: next,
        scrollRatio: captureScrollRatio(),
      });
    } catch (e) {
      if (import.meta.env.DEV) {
        console.warn("[library] save view_mode failed", e);
      }
    }
  }

  const triedBrowseThumbs = new Set<string>();
  let videoThumbCache = $state<Map<string, string>>(new Map());

  function seedThumbCacheFromData() {
    if (!browseData) return;
    const next = new Map(videoThumbCache);
    for (const v of browseData.videos) {
      if (v.thumbnail_path) next.set(v.path, v.thumbnail_path);
    }
    videoThumbCache = next;
  }

  function applyBrowseThumb(videoPath: string, thumbPath: string) {
    const next = new Map(videoThumbCache);
    next.set(videoPath, thumbPath);
    videoThumbCache = next;
    if (!browseData) return;
    browseData = {
      ...browseData,
      videos: browseData.videos.map((v) =>
        v.path === videoPath ? { ...v, thumbnail_path: thumbPath } : v,
      ),
    };
  }

  function collectThumbExtractTargets(): string[] {
    if (!browseData) return [];
    const targets = new Set<string>();
    for (const v of browseData.videos) {
      if (!videoThumbCache.has(v.path) && !triedBrowseThumbs.has(v.path)) {
        targets.add(v.path);
      }
    }
    for (const f of browseData.folders) {
      const previews = f.preview_videos ?? [];
      for (const vp of previews) {
        if (!videoThumbCache.has(vp) && !triedBrowseThumbs.has(vp)) {
          targets.add(vp);
        }
      }
    }
    return [...targets];
  }

  async function extractMissingBrowseThumbs() {
    const targets = collectThumbExtractTargets();
    for (const path of targets) {
      triedBrowseThumbs.add(path);
      try {
        const res = await pluginInvoke<{
          video_path: string;
          thumbnail_path: string;
        }>("study", "study:browse:extract_thumbnail", { path });
        applyBrowseThumb(res.video_path, res.thumbnail_path);
      } catch (e) {
        if (import.meta.env.DEV) {
          console.warn("[library] thumb extract failed for", path, e);
        }
      }
    }
  }

  async function loadBrowse(path: string) {
    const previous = browsePath;
    if (previous && previous !== path) {
      void saveBrowseStateFor(previous);
    }
    browseLoading = true;
    try {
      browseData = await pluginInvoke<BrowseData>("study", "study:browse:list", {
        path,
      });
      browsePath = browseData.path;
      if (tabs.length === 0) {
        tabs = [{ path: browsePath }];
        activeTabIndex = 0;
        saveTabs();
      } else if (activeTabIndex < tabs.length && tabs[activeTabIndex].path !== browsePath) {
        const next = [...tabs];
        next[activeTabIndex] = { path: browsePath };
        tabs = next;
        saveTabs();
      }
      clearSelection();
      seedThumbCacheFromData();
      void restoreScrollFor(browsePath);
      void extractMissingBrowseThumbs();
    } catch (e) {
      console.error("browse failed", e);
      browseData = null;
    } finally {
      browseLoading = false;
    }
  }

  type HealthReport = {
    zero_byte_videos: { lesson_id: number; course_id: number; lesson_title: string; course_title: string; video_path: string }[];
    missing_videos: { lesson_id: number; course_id: number; lesson_title: string; course_title: string; video_path: string }[];
    orphan_descriptions: { dir: string; description_path: string }[];
    total_issues: number;
  };

  let healthReport = $state<HealthReport | null>(null);
  let healthOpen = $state(false);
  let healthLoading = $state(false);
  let cleanupBusy = $state(false);
  let cleanupMsg = $state("");
  let cleanupConfirmOpen = $state(false);
  let cleanupForceConfirmOpen = $state(false);

  async function runCleanupMissing(force: boolean = false) {
    if (cleanupBusy) return;
    cleanupBusy = true;
    cleanupMsg = "";
    try {
      const r = await pluginInvoke<{
        removed_lessons: number;
        removed_courses: number;
        kept_unmounted: number;
      }>("study", "study:library:cleanup_missing", { forceUnmounted: force });
      cleanupMsg = $t("study.library.cleanup_done", {
        lessons: r.removed_lessons,
        courses: r.removed_courses,
        kept: r.kept_unmounted,
      });
      await loadHealth();
      await loadCourses();
    } catch (e) {
      cleanupMsg = e instanceof Error ? e.message : String(e);
    } finally {
      cleanupBusy = false;
      setTimeout(() => (cleanupMsg = ""), 8000);
    }
  }

  function runCleanupSafe() {
    void runCleanupMissing(false);
  }
  function runCleanupForce() {
    void runCleanupMissing(true);
  }

  async function deleteOrphanDescription(path: string) {
    try {
      await pluginInvoke("study", "study:library:delete_orphan_description", {
        path,
      });
      const name = path.split(/[\\/]/).pop() ?? path;
      showToast("success", $t("study.library_health.removed_toast", { name }) as string);
      await loadHealth();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("delete orphan description failed", e);
      showToast("error", $t("study.library_health.remove_failed_toast", { msg }) as string);
    }
  }

  async function loadHealth(opts?: { announce?: boolean }) {
    try {
      const r = await pluginInvoke<HealthReport>(
        "study",
        "study:library:health",
      );
      healthReport = r;
      if (opts?.announce) {
        if (r.total_issues === 0) {
          showToast("success", $t("study.library_health.no_issues_toast") as string);
        } else {
          showToast(
            "info",
            $t("study.library_health.issues_found_toast", { count: r.total_issues }) as string,
          );
        }
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("library health failed", e);
      healthReport = null;
      if (opts?.announce) {
        showToast("error", $t("study.library_health.verify_failed_toast", { msg }) as string);
      }
    }
  }

  async function openHealth() {
    healthOpen = true;
    if (!healthReport) {
      healthLoading = true;
      await loadHealth();
      healthLoading = false;
    }
  }

  function consumeReturnToast() {
    if (typeof window === "undefined") return;
    try {
      const raw = sessionStorage.getItem("study.library.return_toast");
      if (!raw) return;
      sessionStorage.removeItem("study.library.return_toast");
      const data = JSON.parse(raw) as { folderName?: string };
      if (data.folderName) {
        showToast(
          "info",
          $t("study.library.back_toast", { folder: data.folderName }) as string,
        );
      }
    } catch {
      /* malformed or sessionStorage unavailable */
    }
  }

  onMount(async () => {
    readUrlParams();
    consumeReturnToast();
    const savedTabs = loadTabsFromStorage();
    if (savedTabs && savedTabs.tabs.length > 0) {
      tabs = savedTabs.tabs;
      activeTabIndex = savedTabs.activeTabIndex;
      if (mode === "browse") {
        const target = tabs[activeTabIndex]?.path ?? "";
        if (target !== browsePath) browsePath = target;
      }
    }
    await Promise.all([loadCourses(), loadRoots(), loadPins()]);
    loadHealth();
    void checkTelegramSession();
    loading = false;
  });

  $effect(() => {
    const m = mode;
    if (m === "browse" && untrack(() => !browseData)) {
      loadBrowse(untrack(() => browsePath));
    }
    if (m === "telegram" && untrack(() => !tgChecking && tgSession.kind !== "authenticated")) {
      void checkTelegramSession();
    }
  });

  async function toggleFavorite(course: Course, ev: MouseEvent) {
    ev.stopPropagation();
    ev.preventDefault();
    try {
      const res = await pluginInvoke<{ id: number; favorite: boolean }>(
        "study",
        "study:courses:toggle_favorite",
        { courseId: course.id },
      );
      courses = courses.map((c) =>
        c.id === res.id ? { ...c, favorite: res.favorite } : c,
      );
    } catch (e) {
      console.error("toggle favorite failed", e);
    }
  }

  async function toggleRoot(r: RootEntry) {
    try {
      const res = await pluginInvoke<{ roots: RootEntry[] }>(
        "study",
        "study:library:roots_toggle",
        { path: r.path, enabled: !r.enabled },
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("toggle root failed", e);
    }
  }

  async function removeRoot(r: RootEntry) {
    try {
      const res = await pluginInvoke<{ roots: RootEntry[] }>(
        "study",
        "study:library:roots_remove",
        { path: r.path },
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("remove root failed", e);
    }
  }

  async function addRoot() {
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.open({ directory: true, multiple: false });
      if (typeof picked !== "string" || !picked) return;
      const res = await pluginInvoke<{ roots: RootEntry[] }>(
        "study",
        "study:library:roots_add",
        { path: picked },
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("add root failed", e);
    }
  }

  async function rescan() {
    if (rescanning) return;
    rescanning = true;
    rescanMsg = "";
    try {
      const res = await pluginInvoke<{ courses_found?: number; lessons_found?: number }>(
        "study",
        "study:rescan",
      );
      rescanMsg = $t("study.library.rescan_ok", {
        courses: res?.courses_found ?? 0,
        lessons: res?.lessons_found ?? 0,
      });
      await Promise.all([loadCourses(), loadRoots()]);
      if (mode === "browse") await loadBrowse(browsePath);
    } catch (e) {
      rescanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      rescanning = false;
    }
  }

  function fmtBytes(n: number): string {
    if (!n) return "";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i += 1;
    }
    return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function compactPath(path: string, max = 48): string {
    if (path.length <= max) return path;
    return `…${path.slice(path.length - max + 1)}`;
  }

  async function openPath(path: string) {
    try {
      await invoke("open_path_default", { path });
    } catch (e) {
      console.error("open failed", e);
      try {
        const shell = await import("@tauri-apps/plugin-shell");
        await shell.open(path);
      } catch (e2) {
        console.error("shell fallback failed", e2);
      }
    }
  }

  function openBrowseVideo(v: BrowseVideo) {
    if (typeof window !== "undefined") {
      const returnUrl = buildCurrentLibraryUrl();
      try {
        sessionStorage.setItem("study.library.return_url", returnUrl);
        sessionStorage.setItem(
          "study.library.return_state",
          JSON.stringify({ mode, browsePath, search }),
        );
      } catch {
        /* sessionStorage may be unavailable; back-button falls back to history */
      }
      if (import.meta.env.DEV) {
        console.log("[lib] saving return_url", returnUrl, "browsePath", browsePath);
      }
    }
    if (browsePath) {
      void saveBrowseStateFor(browsePath, v.path);
    }
    const params = new URLSearchParams({ path: v.path, name: v.name });
    if (v.subtitle_path) params.set("subtitle", v.subtitle_path);
    goto(`/study/watch?${params.toString()}`);
  }

  beforeNavigate(() => {
    if (mode === "browse" && browsePath) {
      void saveBrowseStateFor(browsePath);
    }
  });

  async function openBrowseDocument(d: BrowseDocument) {
    await openPath(d.path);
  }
</script>

<section class="study-page">
  <header class="head">
    <div class="row-primary">
      <div class="title-block">
        <h1 class="page-title">{$t("study.library.title")}</h1>
        <p class="page-subtitle">{$t("study.library.subtitle")}</p>
      </div>
      <div class="right">
        <button
          type="button"
          class="ghost-btn"
          onclick={() => (rootsOpen = true)}
          aria-label={$t("study.library.roots_button")}
          title={$t("study.library.roots_button_hint") as string}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M3 7v13h18V9h-9l-2-2H3z"></path>
          </svg>
          <span>{$t("study.library.roots_button")}</span>
          {#if roots.length > 0}
            <span class="badge">{roots.filter((r) => r.enabled).length}</span>
          {/if}
        </button>
      </div>
    </div>

    <div class="row-secondary">
      {#snippet iconCourses()}
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M22 10L12 5 2 10l10 5 10-5z"/>
          <path d="M6 12v5c0 1 3 3 6 3s6-2 6-3v-5"/>
        </svg>
      {/snippet}
      {#snippet iconBrowse()}
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M3 7v13h18V9h-9l-2-2H3z"/>
        </svg>
      {/snippet}
      {#snippet iconTelegram()}
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
          <polyline points="7 10 12 15 17 10"/>
          <line x1="12" y1="15" x2="12" y2="3"/>
        </svg>
      {/snippet}
      <SegmentedControl
        bind:value={mode}
        options={[
          { value: "courses", label: $t("study.library.mode_courses") as string, icon: iconCourses },
          { value: "browse", label: $t("study.library.mode_browse") as string, icon: iconBrowse },
          { value: "telegram", label: telegramLabel, icon: iconTelegram },
        ]}
        ariaLabel="mode"
      />
      {#if mode !== "courses"}
        <input
          type="search"
          placeholder={$t("study.library.search_placeholder")}
          bind:value={search}
          class="search"
        />
      {:else}
        <SearchInput
          value={search}
          onChange={onSearchChange}
          placeholder={$t("study.library.search_placeholder")}
        />
        <div class="view-toggle" role="tablist" aria-label={$t("study.library.view_mode_aria")}>
          <button
            type="button"
            class="view-btn"
            class:active={coursesView === "home"}
            role="tab"
            aria-selected={coursesView === "home"}
            onclick={() => (coursesView = "home")}
          >
            {$t("study.library.view_home")}
          </button>
          <button
            type="button"
            class="view-btn"
            class:active={coursesView === "catalog"}
            role="tab"
            aria-selected={coursesView === "catalog"}
            onclick={() => (coursesView = "catalog")}
          >
            {$t("study.library.view_catalog")}
          </button>
        </div>
        {#if coursesView === "catalog"}
          <SortDropdown
            value={sortKey}
            direction={sortDirection}
            onChange={onSortChange}
          />
          <label class="fav-toggle">
            <input type="checkbox" bind:checked={favoritesOnly} />
            <span>{$t("study.library.favorites_only")}</span>
          </label>
        {/if}
      {/if}
    </div>
  </header>

  {#if healthReport && healthReport.total_issues > 0}
    {@const parts = [
      healthReport.zero_byte_videos.length > 0
        ? $t("study.library.health_banner_part_empty", { count: healthReport.zero_byte_videos.length })
        : null,
      healthReport.missing_videos.length > 0
        ? $t("study.library.health_banner_part_missing", { count: healthReport.missing_videos.length })
        : null,
      healthReport.orphan_descriptions.length > 0
        ? $t("study.library.health_banner_part_orphan", { count: healthReport.orphan_descriptions.length })
        : null,
    ].filter(Boolean)}
    <button
      class="health-banner"
      type="button"
      onclick={openHealth}
      aria-label={$t("study.library.health_banner_aria")}
    >
      <span class="health-dot" aria-hidden="true"></span>
      <span class="health-text">
        <strong>
          {healthReport.total_issues === 1
            ? $t("study.library.health_banner_one")
            : $t("study.library.health_banner_many", { count: healthReport.total_issues })}
        </strong>
        {#if parts.length > 0}
          <span class="health-parts">{parts.join(" · ")}</span>
        {/if}
      </span>
      <span class="health-cta">{$t("study.library.health_banner_cta")}</span>
    </button>
  {/if}

  {#if pins.length > 0}
    <nav class="pinned-strip" aria-label={$t("study.library.pinned_strip_aria")}>
      {#each pins as pin (pin.id)}
        <div class="pin-chip-wrap">
          <button
            type="button"
            class="pin-chip"
            onclick={() => openPinnedPath(pin.path)}
            title={pin.path}
          >
            <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor" aria-hidden="true">
              <path d="M14 2l-1.5 1.5L14 5l-2 2v3l-3 3 1.5 1.5L13 11l3 3 1.5-1.5L14 9l2-2 1.5 1.5L19 7z M9 16l-5 5"/>
            </svg>
            <span class="pin-label">{pin.label ?? pin.path}</span>
          </button>
          <button
            type="button"
            class="pin-remove"
            onclick={() => togglePin(pin.path)}
            aria-label={$t("study.library.unpin_action")}
            title={$t("study.library.unpin_action") as string}
          >×</button>
        </div>
      {/each}
    </nav>
  {/if}

  {#if mode === "courses" && coursesView === "home"}
    <section class="home-shelves">
      <LibraryHomeShelves />
    </section>
  {/if}

  {#if mode === "courses" && coursesView === "catalog"}
    <div class="status-row">
      <SegmentedControl
        bind:value={statusFilter}
        options={[
          { value: "all", label: $t("study.library.status_all") },
          { value: "in_progress", label: $t("study.library.status_in_progress") },
          { value: "not_started", label: $t("study.library.status_not_started") },
          { value: "completed", label: $t("study.library.status_completed") },
        ]}
        ariaLabel={$t("study.library.status_aria")}
      />
      <span class="result-count-inline">
        {visibleCourses.length} de {courses.length}
      </span>
    </div>
    <div class="content-cols">
      <aside class="filters" class:open={filtersOpen}>
        <button
          type="button"
          class="filters-toggle"
          onclick={() => (filtersOpen = !filtersOpen)}
        >
          <span>{$t("study.library.filters")}</span>
          <span class="count" aria-hidden="true">
            {includeTags.size + excludeTags.size +
              (statusFilter !== "all" ? 1 : 0) +
              (playlistFilter != null ? 1 : 0)}
          </span>
        </button>

        {#if filtersOpen}
          <section class="filter-section">
            <h3>{$t("study.library.filter_status")}</h3>
            <div class="status-buttons">
              {#each [
                { v: "all", label: $t("study.library.status_all") },
                { v: "not_started", label: $t("study.library.status_not_started") },
                { v: "in_progress", label: $t("study.library.status_in_progress") },
                { v: "completed", label: $t("study.library.status_completed") },
              ] as opt (opt.v)}
                <button
                  type="button"
                  class="status-btn"
                  class:active={statusFilter === opt.v}
                  onclick={() => (statusFilter = opt.v as typeof statusFilter)}
                >
                  {opt.label}
                </button>
              {/each}
            </div>
          </section>

          {#if playlists.length > 0}
            <section class="filter-section">
              <h3>{$t("study.library.filter_playlist")}</h3>
              <div class="playlist-list">
                <button
                  type="button"
                  class="pl-row"
                  class:active={playlistFilter == null}
                  onclick={() => selectPlaylist(null)}
                >
                  <span>{$t("study.library.all_courses")}</span>
                  <span class="pl-count">{courses.length}</span>
                </button>
                {#each playlists as p (p.id)}
                  <button
                    type="button"
                    class="pl-row"
                    class:active={playlistFilter === p.id}
                    onclick={() => selectPlaylist(p.id)}
                    style:--pl-color={p.color ?? "var(--accent)"}
                  >
                    <span class="pl-dot"></span>
                    <span>{p.name}</span>
                    <span class="pl-count">{p.course_count}</span>
                  </button>
                {/each}
              </div>
              <a href="/study/library/playlists" class="manage">{$t("study.library.manage_playlists")}</a>
            </section>
          {:else}
            <section class="filter-section">
              <h3>Playlist</h3>
              <a href="/study/library/playlists" class="empty-cta">
                + Criar primeira playlist
              </a>
            </section>
          {/if}

          {#if allTags.length > 0}
            <section class="filter-section">
              <header class="tags-head">
                <h3>{$t("study.library.filter_tags")}</h3>
                {#if includeTags.size + excludeTags.size > 0}
                  <button
                    type="button"
                    class="clear-link"
                    onclick={clearTagFilters}
                  >{$t("study.library.clear_filters")}</button>
                {/if}
              </header>
              <p class="hint">{$t("study.library.tags_hint")}</p>
              <div class="tag-grid">
                {#each allTags.slice(0, 40) as t (t.tag)}
                  {@const inc = includeTags.has(t.tag)}
                  {@const exc = excludeTags.has(t.tag)}
                  <button
                    type="button"
                    class="tag-pill"
                    class:include={inc}
                    class:exclude={exc}
                    onclick={(e) =>
                      e.shiftKey
                        ? toggleExcludeTag(t.tag)
                        : toggleIncludeTag(t.tag)}
                  >
                    {#if exc}<span class="prefix">−</span>{/if}
                    {#if inc}<span class="prefix">+</span>{/if}
                    #{t.tag}
                    <span class="t-count">{t.course_count}</span>
                  </button>
                {/each}
              </div>
            </section>
          {/if}
        {/if}
      </aside>

      <div class="content-area">
    {#if loading}
      <p class="muted">{$t("study.common.loading")}</p>
    {:else if error}
      <p class="error">{error}</p>
    {:else if visibleCourses.length === 0}
      {@const enabledRoots = roots.filter((r) => r.enabled).length}
      {#if courses.length === 0 && enabledRoots > 0}
        <div class="empty-with-roots">
          <p>
            {$t(
              enabledRoots === 1
                ? "study.library.empty_with_roots_one"
                : "study.library.empty_with_roots_other",
              { count: enabledRoots },
            )}
          </p>
          <button
            type="button"
            class="empty-card-cta"
            onclick={rescan}
            disabled={rescanning}
          >
            {rescanning
              ? $t("study.library.rescanning")
              : $t("study.library.empty_with_roots_cta")}
          </button>
        </div>
      {:else}
        <p class="muted">{$t("study.library.empty")}</p>
      {/if}
    {:else}
      <p class="result-count">
        {visibleCourses.length} de {courses.length} cursos
      </p>
      <div class="grid">
        {#each visibleCourses.slice(0, gridPageSize) as c (c.id)}
          <a class="card course-card" href="/study/course/{c.id}">
            <div
              class="thumb course-thumb"
              style={c.thumbnail_path
                ? `background-image: url('${convertFileSrc(c.thumbnail_path)}');`
                : ""}
            >
              {#if !c.thumbnail_path}
                <svg viewBox="0 0 64 64" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="M12 10h40a2 2 0 0 1 2 2v36a2 2 0 0 1-2 2H12a2 2 0 0 1-2-2V12a2 2 0 0 1 2-2z"/>
                  <path d="M10 22h44"/>
                  <path d="M22 10v12"/>
                  <path d="M42 10v12"/>
                </svg>
              {/if}
              <span class="kind-badge">{$t("study.library.kind_course")}</span>
              <div class="progress" style:--pct="{c.progress_pct}%"></div>
            </div>
            <div class="meta">
              <div class="title-row">
                <span class="title">{c.title}</span>
                <button
                  type="button"
                  class="star"
                  class:active={c.favorite}
                  onclick={(e) => toggleFavorite(c, e)}
                  aria-label={c.favorite
                    ? $t("study.library.favorite_on")
                    : $t("study.library.favorite_off")}
                >
                  {c.favorite ? "★" : "☆"}
                </button>
              </div>
              <div class="sub">
                <span>{$t("study.library.lessons_count", { count: c.total_lessons })}</span>
                <span>·</span>
                <span>{$t("study.library.progress_pct", { pct: Math.round(c.progress_pct) })}</span>
              </div>
            </div>
          </a>
        {/each}
      </div>
      {#if gridPageSize < visibleCourses.length}
        <div class="grid-sentinel" bind:this={gridSentinel} aria-hidden="true">
          <span class="muted">{$t("study.library.loading_more")}</span>
        </div>
      {/if}
    {/if}
      </div>
    </div>
  {:else if mode === "browse"}
    {#snippet emptyBrowse()}
      <div class="empty-browse">
        <div class="empty-browse-head">
          <h2>{$t("study.library.empty_browse_title")}</h2>
          <p>{$t("study.library.empty_browse_description")}</p>
        </div>
        <div class="empty-cards">
          <div class="empty-card">
            <span class="empty-card-icon">
              <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 7v13h18V9h-9l-2-2H3z"/>
                <line x1="12" y1="13" x2="12" y2="17"/>
                <line x1="10" y1="15" x2="14" y2="15"/>
              </svg>
            </span>
            <h3>{$t("study.library.empty_add_folder_title")}</h3>
            <p>{$t("study.library.empty_add_folder_desc")}</p>
            <button type="button" class="empty-card-cta" onclick={addRoot}>
              {$t("study.library.empty_add_folder_cta")} →
            </button>
          </div>
          <div class="empty-card">
            <span class="empty-card-icon">
              <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
                <polyline points="7 10 12 15 17 10"/>
                <line x1="12" y1="15" x2="12" y2="3"/>
              </svg>
            </span>
            <h3>{$t("study.library.empty_telegram_title")}</h3>
            <p>{$t("study.library.empty_telegram_desc")}</p>
            <button type="button" class="empty-card-cta" onclick={() => (mode = "telegram")}>
              {$t("study.library.empty_telegram_cta")} →
            </button>
          </div>
        </div>
      </div>
    {/snippet}
    {#if tabs.length >= 2}
      <nav class="tab-bar" aria-label="Browse tabs">
        {#each tabs as tab, i (i + ":" + tab.path)}
          <div class="tab-wrap" class:active={i === activeTabIndex}>
            <button
              type="button"
              class="tab"
              onclick={() => switchTab(i)}
              title={tab.path || "Raízes"}
            >
              <svg viewBox="0 0 24 24" width="11" height="11" fill="currentColor" aria-hidden="true">
                <path d="M3 7v13h18V9h-9l-2-2H3z" opacity="0.85"/>
              </svg>
              <span class="tab-label">{tabLabel(tab.path)}</span>
            </button>
            <button
              type="button"
              class="tab-close"
              onclick={(e) => { e.stopPropagation(); closeTab(i); }}
              aria-label={$t("study.library.tab_close")}
              title={$t("study.library.tab_close") as string}
            >×</button>
          </div>
        {/each}
      </nav>
    {/if}
    {#if browseLoading && !browseData}
      <p class="muted">{$t("study.common.loading")}</p>
    {:else if !browseData}
      {@render emptyBrowse()}
    {:else}
      {#if browseData.breadcrumb.length > 0 || !browseData.is_root_list}
        {@const segments = [
          { path: "", name: $t("study.library.browse_breadcrumb_root") as string },
          ...browseData.breadcrumb,
        ]}
        <div class="crumbs-row">
          {#if addressBarOpen}
            <input
              bind:this={addressBarRef}
              bind:value={addressBarValue}
              class="address-bar"
              type="text"
              spellcheck="false"
              autocomplete="off"
              placeholder={$t("study.library.address_bar_placeholder")}
              onkeydown={(e) => {
                if (e.key === "Enter") submitAddressBar();
                else if (e.key === "Escape") closeAddressBar();
              }}
              onblur={() => closeAddressBar()}
            />
          {:else}
            <button
              type="button"
              class="address-bar-toggle"
              onclick={openAddressBar}
              aria-label={$t("study.library.address_bar_open")}
              title={$t("study.library.address_bar_open") as string}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <polyline points="16 18 22 12 16 6"/>
                <polyline points="8 6 2 12 8 18"/>
              </svg>
            </button>
          {/if}
          <nav class="crumbs" aria-label="breadcrumb" class:hidden={addressBarOpen}>
            {#each segments as seg, i (i + ":" + seg.path)}
              {@const isLast = i === segments.length - 1}
              {@const parentPath =
                i >= 2 ? browseData.breadcrumb[i - 2].path : ""}
              {@const cached = crumbSiblingsCache.get(parentPath) ?? []}
              {#if isLast}
                <span class="crumb-current">{seg.name}</span>
              {:else if i === 0}
                <button
                  type="button"
                  class="crumb"
                  onclick={() => loadBrowse("")}
                >{seg.name}</button>
              {:else}
                <button
                  type="button"
                  class="crumb"
                  onclick={() => loadBrowse(seg.path)}
                >{seg.name}</button>
              {/if}
              <span class="crumb-chevron-wrap">
                <button
                  type="button"
                  class="crumb-chevron"
                  class:open={openCrumbIndex === i}
                  onclick={() => toggleSiblings(i, parentPath)}
                  aria-haspopup="menu"
                  aria-expanded={openCrumbIndex === i}
                  aria-label={$t("study.library.siblings_aria")}
                >
                  <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <polyline points="6 9 12 15 18 9"/>
                  </svg>
                </button>
                {#if openCrumbIndex === i}
                  <div class="crumb-popover" role="menu">
                    {#if crumbSiblingsLoading === parentPath}
                      <span class="crumb-popover-empty">…</span>
                    {:else if cached.length === 0}
                      <span class="crumb-popover-empty">{$t("study.library.siblings_empty")}</span>
                    {:else}
                      {#each cached as sib (sib.path)}
                        {@const isCurrent = sib.path === seg.path}
                        <button
                          type="button"
                          class="crumb-popover-item"
                          class:current={isCurrent}
                          role="menuitem"
                          onclick={() => {
                            closeSiblings();
                            loadBrowse(sib.path);
                          }}
                        >
                          <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor" aria-hidden="true">
                            <path d="M3 7v13h18V9h-9l-2-2H3z" opacity="0.85"/>
                          </svg>
                          <span class="crumb-popover-name">{sib.name}</span>
                          {#if sib.video_count > 0}
                            <span class="crumb-popover-count">{sib.video_count}</span>
                          {/if}
                        </button>
                      {/each}
                    {/if}
                  </div>
                {/if}
              </span>
              {#if !isLast}
                <span class="crumb-sep" aria-hidden="true">›</span>
              {/if}
            {/each}
          </nav>
          {#if !browseData.is_root_list && browsePath.length > 0}
            {@const currentName = browseData.breadcrumb.at(-1)?.name ?? ""}
            {@const isPinned = pins.some((p) => p.path === browsePath)}
            <button
              type="button"
              class="ghost-btn-mini"
              onclick={startCreateFolder}
              disabled={creatingFolder}
              title={$t("study.library.new_folder") as string}
            >
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 7v13h18V9h-9l-2-2H3z"/>
                <line x1="12" y1="13" x2="12" y2="17"/>
                <line x1="10" y1="15" x2="14" y2="15"/>
              </svg>
              <span>{$t("study.library.new_folder")}</span>
            </button>
            <button
              type="button"
              class="pin-current"
              class:active={isPinned}
              onclick={() => togglePin(browsePath, currentName)}
              title={(isPinned
                ? $t("study.library.unpin_action_title")
                : $t("study.library.pin_action_title")) as string}
            >
              <svg viewBox="0 0 24 24" width="12" height="12" fill={isPinned ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M14 2l-1.5 1.5L14 5l-2 2v3l-3 3 1.5 1.5L13 11l3 3 1.5-1.5L14 9l2-2 1.5 1.5L19 7z M9 16l-5 5"/>
              </svg>
              <span>{$t(isPinned ? "study.library.pin_action_active" : "study.library.pin_action")}</span>
            </button>
          {/if}
          <div class="sort-wrap">
            <button
              type="button"
              class="ghost-btn-mini sort-btn"
              onclick={() => (sortMenuOpen = !sortMenuOpen)}
              aria-haspopup="menu"
              aria-expanded={sortMenuOpen}
              title={$t("study.library.browse_sort_label") as string}
            >
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 6h18M6 12h12M10 18h4"/>
              </svg>
              <span>
                {browseSort === "name_asc"
                  ? $t("study.library.browse_sort_name_asc")
                  : browseSort === "name_desc"
                    ? $t("study.library.browse_sort_name_desc")
                    : $t("study.library.browse_sort_size_desc")}
              </span>
            </button>
            {#if sortMenuOpen}
              <div class="sort-menu" role="menu">
                <button
                  type="button"
                  class="ctx-item"
                  class:current={browseSort === "name_asc"}
                  role="menuitem"
                  onclick={() => setBrowseSort("name_asc")}
                >{$t("study.library.browse_sort_name_asc")}</button>
                <button
                  type="button"
                  class="ctx-item"
                  class:current={browseSort === "name_desc"}
                  role="menuitem"
                  onclick={() => setBrowseSort("name_desc")}
                >{$t("study.library.browse_sort_name_desc")}</button>
                <button
                  type="button"
                  class="ctx-item"
                  class:current={browseSort === "size_desc"}
                  role="menuitem"
                  onclick={() => setBrowseSort("size_desc")}
                >{$t("study.library.browse_sort_size_desc")}</button>
              </div>
            {/if}
          </div>
          <div class="view-mode-toggle" role="tablist" aria-label={$t("study.library.view_mode_aria")}>
            <button
              type="button"
              class="vm-btn"
              class:active={browseViewMode === "grid"}
              role="tab"
              aria-selected={browseViewMode === "grid"}
              onclick={() => setBrowseViewMode("grid")}
              title={$t("study.library.view_mode_grid") as string}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <rect x="3" y="3" width="7" height="7"/>
                <rect x="14" y="3" width="7" height="7"/>
                <rect x="3" y="14" width="7" height="7"/>
                <rect x="14" y="14" width="7" height="7"/>
              </svg>
            </button>
            <button
              type="button"
              class="vm-btn"
              class:active={browseViewMode === "list"}
              role="tab"
              aria-selected={browseViewMode === "list"}
              onclick={() => setBrowseViewMode("list")}
              title={$t("study.library.view_mode_list") as string}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <line x1="3" y1="6" x2="21" y2="6"/>
                <line x1="3" y1="12" x2="21" y2="12"/>
                <line x1="3" y1="18" x2="21" y2="18"/>
              </svg>
            </button>
          </div>
        </div>
      {/if}

      {#if visibleBrowse && (visibleBrowse.folders.length > 0 || visibleBrowse.videos.length > 0 || (visibleBrowse.documents?.length ?? 0) > 0) || creatingFolder}
        <div class="grid" class:browse-list={browseViewMode === "list" && mode === "browse"}>
          {#if creatingFolder}
            <div class="card folder-card folder-card-creating">
              <div class="thumb folder-thumb">
                <svg viewBox="0 0 64 64" width="56" height="56" fill="currentColor" aria-hidden="true" opacity="0.4">
                  <path d="M6 14a4 4 0 0 1 4-4h14l6 6h24a4 4 0 0 1 4 4v30a4 4 0 0 1-4 4H10a4 4 0 0 1-4-4V14z"/>
                </svg>
                <span class="kind-badge">{$t("study.library.folder")}</span>
              </div>
              <div class="meta">
                <input
                  bind:this={newFolderRef}
                  bind:value={newFolderName}
                  class="rename-input"
                  type="text"
                  placeholder={$t("study.library.new_folder_placeholder")}
                  onkeydown={(e) => {
                    if (e.key === "Enter") confirmCreateFolder();
                    else if (e.key === "Escape") cancelCreateFolder();
                  }}
                  onblur={() => confirmCreateFolder()}
                />
              </div>
            </div>
          {/if}
          {#each visibleBrowse?.folders ?? [] as f (f.path)}
            {@const previews = (f.preview_videos ?? []).slice(0, 4)}
            {@const isRenaming = renamingPath === f.path}
            {@const isSelected = selectedPaths.has(f.path)}
            <button
              type="button"
              class="card folder-card"
              class:renaming={isRenaming}
              class:selected={isSelected}
              onclick={(e) => handleFolderClick(e, f)}
              oncontextmenu={(e) =>
                openContextMenu(e, { kind: "folder", path: f.path, name: f.name })}
            >
              <div class="thumb folder-thumb">
                {#if previews.length > 0}
                  <div class="folder-mosaic" data-tiles={previews.length}>
                    {#each previews as vp (vp)}
                      {@const tp = videoThumbCache.get(vp)}
                      <span class="folder-mosaic-tile">
                        {#if tp}
                          <img src={convertFileSrc(tp)} alt="" loading="lazy" />
                        {/if}
                      </span>
                    {/each}
                  </div>
                {:else}
                  <svg viewBox="0 0 64 64" width="56" height="56" fill="currentColor" aria-hidden="true">
                    <path d="M6 14a4 4 0 0 1 4-4h14l6 6h24a4 4 0 0 1 4 4v30a4 4 0 0 1-4 4H10a4 4 0 0 1-4-4V14z" opacity="0.9"/>
                    <path d="M6 22h52v2H6z" opacity="0.3"/>
                  </svg>
                {/if}
                <span class="kind-badge">{$t("study.library.folder")}</span>
              </div>
              <div class="meta">
                <div class="title-row">
                  {#if isRenaming}
                    <input
                      class="rename-input"
                      type="text"
                      bind:value={renameValue}
                      onclick={(e) => e.stopPropagation()}
                      onkeydown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") confirmRename(f.path);
                        else if (e.key === "Escape") cancelRename();
                      }}
                      onblur={() => confirmRename(f.path)}
                      use:autofocusOnRender
                    />
                  {:else}
                    <span class="title">{f.name}</span>
                  {/if}
                </div>
                <div class="sub">
                  <span>{$t("study.library.videos_count", { count: f.video_count })}</span>
                </div>
              </div>
            </button>
          {/each}
          {#each visibleBrowse?.videos ?? [] as v (v.path)}
            {@const isRenaming = renamingPath === v.path}
            {@const isLastWatched = v.path === lastWatchedInFolder}
            {@const isSelected = selectedPaths.has(v.path)}
            <button
              type="button"
              class="card video-card"
              class:renaming={isRenaming}
              class:last-watched={isLastWatched}
              class:selected={isSelected}
              onclick={(e) => handleVideoClick(e, v)}
              oncontextmenu={(e) =>
                openContextMenu(e, { kind: "video", path: v.path, name: v.name })}
            >
              <div class="thumb video-thumb">
                {#if isLastWatched}
                  <span class="last-watched-badge">{$t("study.library.last_watched_label")}</span>
                {/if}
                {#if v.thumbnail_path}
                  <img
                    class="video-thumb-img"
                    src={convertFileSrc(v.thumbnail_path)}
                    alt=""
                    loading="lazy"
                  />
                {:else}
                  <svg viewBox="0 0 64 64" width="56" height="56" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <rect x="6" y="12" width="52" height="40" rx="4" fill="currentColor" opacity="0.12"/>
                    <rect x="6" y="12" width="52" height="40" rx="4"/>
                    <path d="M27 24l14 8-14 8z" fill="currentColor"/>
                  </svg>
                {/if}
                <span class="kind-badge">{$t("study.library.video")}</span>
              </div>
              <div class="meta">
                <div class="title-row">
                  {#if isRenaming}
                    <input
                      class="rename-input"
                      type="text"
                      bind:value={renameValue}
                      onclick={(e) => e.stopPropagation()}
                      onkeydown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") confirmRename(v.path);
                        else if (e.key === "Escape") cancelRename();
                      }}
                      onblur={() => confirmRename(v.path)}
                      use:autofocusOnRender
                    />
                  {:else}
                    <span class="title">{v.name}</span>
                  {/if}
                </div>
                <div class="sub">
                  {#if v.size}<span>{fmtBytes(v.size)}</span>{/if}
                  {#if v.subtitle_path}<span>·</span><span>CC</span>{/if}
                </div>
              </div>
            </button>
          {/each}
          {#each (visibleBrowse?.documents ?? []) as d (d.path)}
            {@const isRenaming = renamingPath === d.path}
            {@const isSelected = selectedPaths.has(d.path)}
            <button
              type="button"
              class="card doc-card"
              class:renaming={isRenaming}
              class:selected={isSelected}
              data-ext={d.ext}
              onclick={(e) => handleDocClick(e, d)}
              oncontextmenu={(e) =>
                openContextMenu(e, { kind: "doc", path: d.path, name: d.name })}
            >
              <div class="thumb doc-thumb">
                <svg viewBox="0 0 64 64" width="52" height="64" fill="none" aria-hidden="true">
                  <path d="M14 4h28l10 10v42a4 4 0 0 1-4 4H14a4 4 0 0 1-4-4V8a4 4 0 0 1 4-4z" fill="currentColor" opacity="0.08"/>
                  <path d="M14 4h28l10 10v42a4 4 0 0 1-4 4H14a4 4 0 0 1-4-4V8a4 4 0 0 1 4-4z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/>
                  <path d="M42 4v10h10" stroke="currentColor" stroke-width="2" stroke-linejoin="round" fill="currentColor" fill-opacity="0.15"/>
                </svg>
                <span class="ext-stamp">{d.ext.toUpperCase()}</span>
              </div>
              <div class="meta">
                <div class="title-row">
                  {#if isRenaming}
                    <input
                      class="rename-input"
                      type="text"
                      bind:value={renameValue}
                      onclick={(e) => e.stopPropagation()}
                      onkeydown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") confirmRename(d.path);
                        else if (e.key === "Escape") cancelRename();
                      }}
                      onblur={() => confirmRename(d.path)}
                      use:autofocusOnRender
                    />
                  {:else}
                    <span class="title">{d.name}</span>
                  {/if}
                </div>
                <div class="sub">
                  {#if d.size}<span>{fmtBytes(d.size)}</span>{/if}
                </div>
              </div>
            </button>
          {/each}
        </div>
      {:else if browseData.is_root_list}
        {@render emptyBrowse()}
      {:else}
        <p class="muted">{$t("study.library.browse_empty")}</p>
      {/if}
    {/if}
  {:else if mode === "telegram"}
    {#if tgChecking}
      <p class="muted">{$t("study.common.loading")}</p>
    {:else if tgSession.kind === "plugin_missing"}
      <div class="tg-empty">
        <h2>{$t("study.library.telegram.plugin_missing_title")}</h2>
        <p class="muted">{$t("study.library.telegram.plugin_missing_body")}</p>
      </div>
    {:else if tgSession.kind === "unauthenticated"}
      <TelegramAuthGate on:authenticated={onTelegramAuthenticated} />
    {:else}
      <TelegramBrowser
        outputDir={tgOutputDir}
        onDownloadEnqueued={onDownloadEnqueued}
        onLogout={onTelegramLogout}
      />
    {/if}
  {/if}
</section>

{#if mode === "telegram" && tgSession.kind === "authenticated"}
  <button
    type="button"
    class="downloads-fab"
    class:has-active={downloadsActive > 0}
    onclick={() => goto("/downloads")}
    aria-label={$t("study.library.telegram.downloads.toggle")}
    title={$t("study.library.telegram.downloads_view_panel")}
  >
    <span aria-hidden="true">↓</span>
    {#if downloadsActive > 0}
      <span class="badge">{downloadsActive}</span>
    {/if}
  </button>
{/if}

{#if healthOpen}
  <div
    class="drawer-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) healthOpen = false;
    }}
    onkeydown={(e) => {
      if (e.key === "Escape") healthOpen = false;
    }}
  >
    <div class="health-drawer" role="dialog" aria-modal="true" aria-labelledby="health-title" tabindex="-1">
      <header class="drawer-head">
        <h2 id="health-title">{$t("study.library.health_title")}</h2>
        <button
          type="button"
          class="ghost-btn"
          onclick={() => {
            healthLoading = true;
            loadHealth({ announce: true }).finally(() => (healthLoading = false));
          }}
          disabled={healthLoading}
        >
          {healthLoading ? $t("study.library.health_checking") : $t("study.library.health_recheck")}
        </button>
        <button class="close-btn" onclick={() => (healthOpen = false)} aria-label={$t("study.common.close")}>×</button>
      </header>

      {#if !healthReport}
        <p class="muted small">{$t("study.common.loading")}</p>
      {:else if healthReport.total_issues === 0}
        <p class="health-ok">
          <span class="health-ok-dot" aria-hidden="true"></span>
          {$t("study.library.health_ok")}
        </p>
      {:else}
        {#if healthReport.zero_byte_videos.length > 0}
          <section class="health-section">
            <h3>
              {$t("study.library.health_zero_byte")}
              <span class="health-count">{healthReport.zero_byte_videos.length}</span>
            </h3>
            <p class="muted small">{$t("study.library.health_zero_byte_hint")}</p>
            <ul class="health-list">
              {#each healthReport.zero_byte_videos as v (v.lesson_id)}
                <li>
                  <strong>{v.course_title}</strong> · {v.lesson_title}
                  <code class="path">{v.video_path}</code>
                </li>
              {/each}
            </ul>
          </section>
        {/if}

        {#if healthReport.missing_videos.length > 0}
          <section class="health-section">
            <h3>
              {$t("study.library.health_missing")}
              <span class="health-count">{healthReport.missing_videos.length}</span>
            </h3>
            <p class="muted small">{$t("study.library.health_missing_hint")}</p>
            <div class="health-actions">
              <button
                type="button"
                class="ghost-btn"
                disabled={cleanupBusy}
                onclick={() => (cleanupConfirmOpen = true)}
              >
                {cleanupBusy ? $t("study.library.cleanup_busy") : $t("study.library.cleanup_btn")}
              </button>
              <button
                type="button"
                class="ghost-btn danger"
                disabled={cleanupBusy}
                onclick={() => (cleanupForceConfirmOpen = true)}
                title={$t("study.library.cleanup_force_hint")}
              >
                {$t("study.library.cleanup_force_btn")}
              </button>
              {#if cleanupMsg}
                <span class="muted small">{cleanupMsg}</span>
              {/if}
            </div>
            <ul class="health-list">
              {#each healthReport.missing_videos as v (v.lesson_id)}
                <li>
                  <strong>{v.course_title}</strong> · {v.lesson_title}
                  <code class="path">{v.video_path}</code>
                </li>
              {/each}
            </ul>
          </section>
        {/if}

        {#if healthReport.orphan_descriptions.length > 0}
          <section class="health-section">
            <h3>
              {$t("study.library.health_orphan_desc")}
              <span class="health-count">{healthReport.orphan_descriptions.length}</span>
            </h3>
            <p class="muted small">{$t("study.library.health_orphan_hint")}</p>
            <ul class="health-list">
              {#each healthReport.orphan_descriptions as o, i (i)}
                <li>
                  <code class="path">{o.dir}</code>
                  <button
                    type="button"
                    class="link danger"
                    onclick={() => deleteOrphanDescription(o.description_path)}
                    title={o.description_path}
                  >
                    {$t("study.library.cleanup_orphan_delete")}
                  </button>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<ConfirmDialog
  bind:open={cleanupConfirmOpen}
  title={$t("study.library.cleanup_confirm_title")}
  message={$t("study.library.cleanup_confirm_message", {
    count: healthReport?.missing_videos.length ?? 0,
  })}
  confirmLabel={$t("study.library.cleanup_btn")}
  variant="danger"
  onConfirm={runCleanupSafe}
/>

<ConfirmDialog
  bind:open={cleanupForceConfirmOpen}
  title={$t("study.library.cleanup_force_confirm_title")}
  message={$t("study.library.cleanup_force_confirm_message", {
    count: healthReport?.missing_videos.length ?? 0,
  })}
  confirmLabel={$t("study.library.cleanup_force_btn")}
  variant="danger"
  onConfirm={runCleanupForce}
/>

<ConfirmDialog
  bind:open={confirmDeleteOpen}
  title={$t("study.library.delete_confirm_title")}
  message={pendingDelete
    ? ($t(
        pendingDelete.kind === "folder"
          ? "study.library.delete_confirm_msg_folder"
          : "study.library.delete_confirm_msg_file",
        { name: pendingDelete.name },
      ) as string)
    : ""}
  confirmLabel={$t("study.library.delete_confirm_btn")}
  variant="danger"
  onConfirm={performDelete}
/>

{#if mode === "browse" && selectedPaths.size > 0}
  <div class="bulk-action-bar" role="region" aria-label="Bulk actions">
    <span class="bulk-count">
      {$t(
        selectedPaths.size === 1
          ? "study.library.selection_count_one"
          : "study.library.selection_count_other",
        { count: selectedPaths.size },
      )}
    </span>
    <button
      type="button"
      class="bulk-btn"
      onclick={clearSelection}
    >{$t("study.library.selection_clear")}</button>
    <button
      type="button"
      class="bulk-btn danger"
      onclick={() => (bulkDeleteOpen = true)}
    >{$t("study.library.selection_delete")}</button>
  </div>
{/if}

<ConfirmDialog
  bind:open={bulkDeleteOpen}
  title={$t("study.library.selection_delete_confirm_title", { count: selectedPaths.size })}
  message={$t("study.library.selection_delete_confirm_msg")}
  confirmLabel={$t("study.library.selection_delete")}
  variant="danger"
  onConfirm={performBulkDelete}
/>

{#if contextMenu}
  <div
    class="ctx-menu"
    role="menu"
    style:left="{contextMenu.x}px"
    style:top="{contextMenu.y}px"
  >
    {#if contextMenu.target.kind === "folder"}
      <button
        type="button"
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          if (contextMenu) {
            const t = contextMenu.target;
            closeContextMenu();
            void loadBrowse(t.path);
          }
        }}
      >{$t("study.library.ctx_open")}</button>
      <button
        type="button"
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          if (contextMenu) {
            const t = contextMenu.target;
            closeContextMenu();
            openInNewTab(t.path);
          }
        }}
      >{$t("study.library.open_in_new_tab")}</button>
    {:else}
      <button
        type="button"
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          if (contextMenu) {
            const t = contextMenu.target;
            closeContextMenu();
            if (t.kind === "video") {
              const fake = visibleBrowse?.videos.find((vv) => vv.path === t.path);
              if (fake) openBrowseVideo(fake);
            } else if (t.kind === "doc") {
              void openPath(t.path);
            }
          }
        }}
      >{$t("study.library.ctx_open")}</button>
    {/if}
    <button
      type="button"
      class="ctx-item"
      role="menuitem"
      onclick={() => contextMenu && showInExplorer(contextMenu.target.path)}
    >{$t("study.library.ctx_show_in_explorer")}</button>
    <button
      type="button"
      class="ctx-item"
      role="menuitem"
      onclick={() => contextMenu && startRename(contextMenu.target)}
    >{$t("study.library.ctx_rename")}</button>
    <button
      type="button"
      class="ctx-item danger"
      role="menuitem"
      onclick={() => contextMenu && startDelete(contextMenu.target)}
    >{$t("study.library.ctx_delete")}</button>
  </div>
{/if}

{#if rootsOpen}
  <div
    class="drawer-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) rootsOpen = false;
    }}
    onkeydown={(e) => {
      if (e.key === "Escape") rootsOpen = false;
    }}
  >
    <div class="drawer" role="dialog" aria-modal="true" tabindex="-1">
      <header class="drawer-head">
        <h3>{$t("study.library.roots_button")}</h3>
        <button type="button" class="icon-btn" aria-label="close" onclick={() => (rootsOpen = false)}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
            <path d="M18 6L6 18"></path>
            <path d="M6 6l12 12"></path>
          </svg>
        </button>
      </header>
      <p class="drawer-hint">{$t("study.library.roots_hint")}</p>

      {#if roots.length === 0}
        <p class="muted drawer-empty">{$t("study.library.roots_empty")}</p>
      {:else}
        <ul class="roots-list">
          {#each roots as r (r.path)}
            <li class="root-row" class:disabled={!r.enabled} class:missing={r.exists === false}>
              <label class="root-toggle">
                <input
                  type="checkbox"
                  checked={r.enabled}
                  onchange={() => toggleRoot(r)}
                />
              </label>
              <div class="root-info">
                <span class="root-path" title={r.path}>{compactPath(r.path)}</span>
                {#if r.is_default}
                  <span class="root-tag">{$t("study.library.roots_default")}</span>
                {/if}
                {#if r.exists === false}
                  <span class="root-tag missing-tag" title={$t("study.library.root_missing_hint") as string}>
                    {$t("study.library.root_missing_label")}
                  </span>
                {/if}
              </div>
              {#if !r.is_default}
                <button
                  type="button"
                  class="root-remove"
                  onclick={() => removeRoot(r)}
                  aria-label={$t("study.library.roots_remove")}
                  title={$t("study.library.roots_remove")}
                >×</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      <div class="drawer-actions">
        <button type="button" class="btn" onclick={addRoot}>
          + {$t("study.library.roots_add_btn")}
        </button>
        <button type="button" class="btn primary" onclick={rescan} disabled={rescanning}>
          {rescanning ? $t("study.library.rescanning") : $t("study.library.rescan_now")}
        </button>
      </div>
      {#if rescanMsg}
        <p class="drawer-msg">{rescanMsg}</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .home-shelves {
    margin: 0 0 24px;
  }
  .status-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .result-count-inline {
    font-size: 12px;
    color: color-mix(in oklab, currentColor 60%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .grid-sentinel {
    padding: 24px;
    text-align: center;
    font-size: 12px;
  }
  .view-toggle {
    display: inline-flex;
    border-radius: 8px;
    background: color-mix(in oklab, var(--content-bg) 80%, var(--accent) 4%);
    border: none;
    padding: 2px;
    gap: 2px;
  }
  .view-btn {
    padding: 6px 12px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background var(--tg-duration-fast, 150ms);
  }
  .view-btn:hover:not(.active) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .view-btn.active {
    background: var(--accent);
    color: var(--accent-contrast, white);
  }
  .content-cols {
    display: grid;
    grid-template-columns: 240px 1fr;
    gap: 16px;
  }
  @media (max-width: 760px) {
    .content-cols {
      grid-template-columns: 1fr;
    }
    .filters {
      max-height: 220px;
      overflow-y: auto;
    }
  }
  .filters {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 12px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    height: fit-content;
    position: sticky;
    top: 12px;
  }
  .filters-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 6px;
    border: 0;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .filters-toggle .count {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
    border-radius: 999px;
    padding: 1px 7px;
    font-size: 10px;
    font-weight: 700;
    text-transform: none;
    letter-spacing: 0;
    min-width: 18px;
    text-align: center;
  }
  .filter-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .filter-section h3 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--tertiary);
  }
  .tags-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0;
    margin: 0;
  }
  .tags-head h3 {
    margin: 0;
  }
  .clear-link {
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    padding: 0;
  }
  .clear-link:hover {
    text-decoration: underline;
  }
  .filter-section .hint {
    margin: 0;
    color: var(--tertiary);
    font-size: 10px;
  }

  .status-buttons {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .status-btn {
    padding: 5px 8px;
    border: 0;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .status-btn:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .status-btn.active {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    font-weight: 600;
  }

  .playlist-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pl-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border: 0;
    border-radius: var(--border-radius);
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .pl-row:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .pl-row.active {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--text);
    font-weight: 600;
  }
  .pl-dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--pl-color, var(--accent));
    flex-shrink: 0;
  }
  .pl-row > span:nth-of-type(1):not(.pl-dot) {
    flex: 1;
  }
  .pl-row > span:not(.pl-dot) {
    flex: 1;
  }
  .pl-count {
    flex: 0 !important;
    color: var(--tertiary);
    font-weight: 500;
    font-size: 11px;
  }
  .manage,
  .empty-cta {
    color: var(--accent);
    text-decoration: none;
    font-size: 11px;
    padding: 4px 6px;
  }
  .manage:hover,
  .empty-cta:hover {
    text-decoration: underline;
  }

  .tag-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .tag-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    border: none;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .tag-pill:hover {
    border-color: var(--accent);
  }
  .tag-pill.include {
    background: color-mix(in oklab, var(--accent) 18%, transparent);
    color: var(--accent);
    border-color: var(--accent);
    font-weight: 600;
  }
  .tag-pill.exclude {
    background: color-mix(
      in oklab,
      var(--error, var(--accent)) 14%,
      transparent
    );
    color: var(--error, var(--accent));
    border-color: var(--error, var(--accent));
    text-decoration: line-through;
  }
  .tag-pill .prefix {
    font-weight: 700;
  }
  .t-count {
    color: var(--tertiary);
    font-weight: 500;
    font-size: 10px;
  }

  .content-area {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .result-count {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
  }

  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 2);
    width: 100%;
    max-width: 1200px;
    margin-inline: auto;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }
  .row-primary {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }
  .right {
    display: flex;
    gap: 0.5rem;
  }
  h1 {
    font-size: 22px;
    font-weight: 500;
    margin: 0;
    letter-spacing: -0.5px;
    color: var(--secondary);
  }
  .ghost-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.45rem 0.75rem;
    background: transparent;
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    transition:
      border-color 150ms ease,
      background 150ms ease;
  }
  .ghost-btn:hover {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .ghost-btn .badge {
    padding: 0 0.4rem;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 20%, transparent);
    color: var(--accent);
    font-size: 11px;
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
  }
  .row-secondary {
    display: flex;
    flex-wrap: wrap;
    gap: var(--padding);
    align-items: center;
  }
  .search {
    flex: 1 1 220px;
    padding: 8px 12px;
    border-radius: var(--border-radius);
    background: var(--input-bg);
    border: none;
    color: var(--secondary);
    font-size: 13px;
    font-family: inherit;
  }
  .search:focus-visible {
    outline: var(--focus-ring, 2px solid var(--accent));
    outline-offset: 2px;
  }
  .sort {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--tertiary);
  }
  .sort select {
    background: var(--input-bg);
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    padding: 6px 8px;
    font-size: 13px;
    font-family: inherit;
  }
  .fav-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--tertiary);
    cursor: pointer;
  }
  .muted {
    color: var(--tertiary);
    font-size: 14px;
  }
  .error {
    color: var(--error);
    font-size: 14px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: calc(var(--padding) * 1.5);
  }
  .card {
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: none;
    border-radius: var(--radius-md);
    box-shadow: var(--elev-1);
    overflow: hidden;
    text-decoration: none;
    color: inherit;
    cursor: pointer;
    font-family: inherit;
    transition: transform var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
    text-align: left;
    padding: 0;
    transition:
      border-color 150ms ease,
      transform 150ms ease;
  }
  .card:hover {
    border-color: var(--accent);
    transform: translateY(-1px);
  }
  .card:focus-visible {
    outline: var(--focus-ring, 2px solid var(--accent));
    outline-offset: 2px;
  }
  .thumb {
    aspect-ratio: 16 / 9;
    background: var(--input-bg);
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tertiary);
  }
  .course-thumb {
    background-size: cover;
    background-position: center;
    background-color: color-mix(in oklab, var(--accent) 6%, var(--input-bg));
    color: var(--accent);
  }
  .course-thumb svg {
    opacity: 0.4;
  }
  .kind-badge {
    position: absolute;
    top: 8px;
    left: 8px;
    padding: 0.15rem 0.5rem;
    border-radius: 999px;
    background: color-mix(in oklab, var(--primary) 70%, transparent);
    color: var(--secondary);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 500;
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
  }
  .folder-thumb {
    background:
      radial-gradient(
        circle at 30% 30%,
        color-mix(in oklab, var(--accent) 15%, transparent),
        color-mix(in oklab, var(--accent) 4%, var(--input-bg))
      );
    color: var(--accent);
  }
  .video-thumb {
    overflow: hidden;
    background:
      radial-gradient(
        circle at 50% 50%,
        color-mix(in oklab, var(--accent) 12%, transparent),
        color-mix(in oklab, var(--primary) 50%, var(--input-bg))
      );
    color: var(--accent);
  }
  .video-thumb-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .doc-thumb {
    position: relative;
    background:
      linear-gradient(
        135deg,
        color-mix(in oklab, var(--accent) 10%, var(--input-bg)),
        var(--input-bg)
      );
    color: var(--tertiary);
  }
  .doc-card[data-ext="pdf"] .doc-thumb {
    color: var(--red);
  }
  .doc-card[data-ext="epub"] .doc-thumb,
  .doc-card[data-ext="mobi"] .doc-thumb,
  .doc-card[data-ext="azw3"] .doc-thumb {
    color: var(--blue);
  }
  .doc-card[data-ext="djvu"] .doc-thumb {
    color: var(--orange);
  }
  .doc-card[data-ext="cbz"] .doc-thumb,
  .doc-card[data-ext="cbr"] .doc-thumb {
    color: var(--green);
  }
  .ext-stamp {
    position: absolute;
    bottom: 14px;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.15rem 0.6rem;
    background: currentColor;
    color: var(--text);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.05em;
    border-radius: 3px;
  }
  .doc-card[data-ext="pdf"] .ext-stamp {
    color: white;
  }
  .progress {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 3px;
    background: color-mix(in oklab, var(--content-border) 60%, transparent);
  }
  .progress::after {
    content: "";
    position: absolute;
    inset: 0;
    width: var(--pct, 0%);
    background: var(--accent);
  }
  .meta {
    padding: calc(var(--padding) * 1.2);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .title-row {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 8px;
  }
  .title {
    color: var(--secondary);
    font-size: 14px;
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .star {
    background: transparent;
    border: none;
    color: var(--tertiary);
    font-size: 16px;
    cursor: pointer;
    padding: 0;
    line-height: 1;
  }
  .star.active {
    color: var(--accent);
  }
  .star:focus-visible {
    outline: var(--focus-ring, 2px solid var(--accent));
    outline-offset: 2px;
    border-radius: 3px;
  }
  .sub {
    display: flex;
    gap: 6px;
    color: var(--tertiary);
    font-size: 12px;
  }

  .crumbs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    font-size: 13px;
    color: var(--tertiary);
  }
  .crumb {
    background: transparent;
    border: 0;
    padding: 0.2rem 0.5rem;
    border-radius: var(--border-radius);
    color: var(--tertiary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    transition:
      color 150ms ease,
      background 150ms ease;
  }
  .crumb:hover {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .crumb-current {
    padding: 0.2rem 0.5rem;
    color: var(--secondary);
    font-weight: 500;
  }
  .crumb-sep {
    opacity: 0.6;
  }

  .drawer-overlay {
    position: fixed;
    inset: 0;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.45));
    backdrop-filter: blur(8px) saturate(140%);
    -webkit-backdrop-filter: blur(8px) saturate(140%);
    display: flex;
    justify-content: flex-end;
    z-index: 1100;
    animation: fade-in 180ms ease-out;
  }
  .drawer {
    width: 420px;
    max-width: 100%;
    height: 100%;
    background: var(--button-elevated);
    border-left: none;
    display: flex;
    flex-direction: column;
    padding: calc(var(--padding) * 2);
    gap: 0.75rem;
    animation: slide-in 260ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .drawer-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .drawer-head h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 500;
    color: var(--secondary);
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: transparent;
    border: 0;
    color: var(--tertiary);
    cursor: pointer;
    border-radius: var(--border-radius);
    font-family: inherit;
  }
  .icon-btn:hover {
    color: var(--secondary);
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .drawer-hint {
    margin: 0;
    font-size: 12px;
    color: var(--tertiary);
    line-height: 1.4;
  }
  .drawer-empty {
    padding: 1rem 0;
  }
  .roots-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    overflow-y: auto;
    flex: 1;
  }
  .root-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.6rem;
    background: var(--input-bg);
    border: none;
    border-radius: var(--border-radius);
    transition: opacity 150ms ease;
  }
  .root-row.disabled {
    opacity: 0.55;
  }
  .root-toggle input {
    cursor: pointer;
  }
  .root-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .root-path {
    color: var(--secondary);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .root-tag {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--tertiary);
  }
  .root-remove {
    width: 22px;
    height: 22px;
    border: 0;
    background: transparent;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    border-radius: 50%;
    font-family: inherit;
  }
  .root-remove:hover {
    color: var(--error);
    background: color-mix(in oklab, var(--error) 10%, transparent);
  }
  .drawer-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .btn {
    padding: 0.5rem 0.85rem;
    border-radius: var(--border-radius);
    background: transparent;
    border: none;
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    transition: border-color 150ms ease;
  }
  .btn:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .btn.primary:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 85%, black);
  }
  .drawer-msg {
    margin: 0;
    font-size: 12px;
    color: var(--tertiary);
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  @keyframes slide-in {
    from {
      transform: translateX(100%);
    }
    to {
      transform: translateX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer-overlay,
    .drawer,
    .card {
      animation: none;
      transition: none;
    }
  }

  .health-banner {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border: none;
    background: color-mix(in oklab, var(--surface, var(--primary)) 60%, transparent);
    border-radius: var(--border-radius);
    color: var(--secondary);
    cursor: pointer;
    font: inherit;
    font-size: 13px;
    text-align: left;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .health-banner:hover {
    background: color-mix(in oklab, var(--surface, var(--primary)) 90%, transparent);
    border-color: color-mix(in oklab, var(--input-border) 70%, transparent);
  }
  .health-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warning, var(--accent));
    flex-shrink: 0;
  }
  .health-text {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
  }
  .health-text strong {
    font-weight: 500;
  }
  .health-parts {
    color: var(--tertiary);
    font-size: 12px;
  }
  .health-cta {
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
  }
  .health-count {
    display: inline-block;
    margin-left: 8px;
    padding: 1px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--input-border) 50%, transparent);
    color: var(--secondary);
    font-size: 11px;
    font-weight: 500;
    vertical-align: middle;
  }

  .health-drawer {
    width: min(720px, 90vw);
    max-height: 85vh;
    overflow-y: auto;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    padding: calc(var(--padding) * 1.25);
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-shadow: 0 20px 50px color-mix(in oklab, black 35%, transparent);
  }
  .health-drawer .drawer-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .health-drawer .drawer-head h2 {
    margin: 0;
    flex: 1;
    font-size: 16px;
  }
  .close-btn {
    background: transparent;
    border: 0;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
    padding: 0 6px;
  }
  .close-btn:hover {
    color: var(--text);
  }
  .health-ok {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    margin: 0;
    padding: 12px;
    color: var(--secondary);
    font-size: 14px;
  }
  .health-ok-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success, var(--accent));
  }
  .health-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 8px;
    border-top: none;
  }
  .health-section h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  .health-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 320px;
    overflow-y: auto;
  }
  .health-list li {
    padding: 6px 8px;
    border-radius: 4px;
    background: color-mix(in oklab, var(--input-border) 18%, transparent);
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .health-list .path {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--tertiary);
    overflow-x: auto;
    white-space: nowrap;
  }
  .tg-empty {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    align-items: center;
    justify-content: center;
    padding: calc(var(--padding) * 4) var(--padding);
    text-align: center;
    min-height: 40vh;
  }
  .downloads-fab {
    position: fixed;
    right: 16px;
    bottom: 16px;
    width: 44px;
    height: 44px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--on-accent);
    border: none;
    cursor: pointer;
    font-size: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.18);
    z-index: 49;
  }
  .downloads-fab .badge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 18px;
    height: 18px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--success);
    color: var(--on-accent);
    font-size: 10px;
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .downloads-fab.has-active {
    animation: fab-pulse 1.6s ease-in-out infinite;
  }
  @keyframes fab-pulse {
    0%, 100% { box-shadow: 0 4px 16px rgba(0,0,0,0.18); }
    50% { box-shadow: 0 4px 24px color-mix(in oklab, var(--accent) 50%, transparent); }
  }
  @media (prefers-reduced-motion: reduce) {
    .downloads-fab.has-active {
      animation: none;
    }
  }
  .tg-empty h2 {
    margin: 0;
    font-size: 1.1rem;
    font-weight: 600;
    color: var(--secondary);
  }

  .title-block {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .page-subtitle {
    margin: 0;
    font-size: 13px;
    color: var(--tertiary);
    line-height: 1.4;
    max-width: 60ch;
  }

  .empty-browse {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 28px;
    padding: 56px 24px 24px;
    text-align: center;
  }
  .empty-browse-head {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 50ch;
  }
  .empty-browse-head h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.3px;
    color: var(--secondary);
  }
  .empty-browse-head p {
    margin: 0;
    font-size: 14px;
    color: var(--tertiary);
    line-height: 1.5;
  }
  .empty-cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 16px;
    width: 100%;
    max-width: 600px;
  }
  .empty-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    padding: 22px 20px;
    border-radius: 12px;
    border: none;
    background: var(--surface);
    text-align: left;
    transition: border-color 150ms ease, transform 150ms ease;
  }
  .empty-card:hover {
    border-color: color-mix(in oklab, var(--accent) 60%, var(--content-border));
    transform: translateY(-2px);
  }
  .empty-card-icon {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
    display: grid;
    place-items: center;
  }
  .empty-card h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--secondary);
  }
  .empty-card > p {
    margin: 0;
    font-size: 13px;
    color: var(--tertiary);
    line-height: 1.45;
    flex: 1;
  }
  .empty-card-cta {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
    padding: 8px 14px;
    background: var(--accent);
    color: var(--on-accent, white);
    border: 0;
    border-radius: 8px;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 150ms ease;
  }
  .empty-card-cta:hover {
    background: color-mix(in oklab, var(--accent) 88%, black);
  }
  .empty-card-cta:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  @media (prefers-reduced-motion: reduce) {
    .empty-card,
    .empty-card-cta {
      transition: none;
    }
    .empty-card:hover {
      transform: none;
    }
  }

  .pinned-strip {
    display: flex;
    flex-wrap: nowrap;
    overflow-x: auto;
    gap: 6px;
    padding: 4px 2px;
    scrollbar-width: thin;
  }
  .pin-chip-wrap {
    display: inline-flex;
    align-items: stretch;
    flex-shrink: 0;
    border-radius: 999px;
    border: 1px solid color-mix(in oklab, var(--accent) 30%, transparent);
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    transition: background 150ms ease, border-color 150ms ease;
  }
  .pin-chip-wrap:hover {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    border-color: color-mix(in oklab, var(--accent) 50%, transparent);
  }
  .pin-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px 5px 9px;
    border: 0;
    background: transparent;
    color: var(--secondary);
    font-family: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    border-radius: 999px 0 0 999px;
    max-width: 240px;
  }
  .pin-chip svg {
    color: var(--accent);
    flex-shrink: 0;
  }
  .pin-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pin-remove {
    border: 0;
    background: transparent;
    color: color-mix(in oklab, currentColor 50%, transparent);
    font-family: inherit;
    font-size: 14px;
    line-height: 1;
    padding: 0 9px;
    cursor: pointer;
    border-radius: 0 999px 999px 0;
    border-left: 1px solid color-mix(in oklab, var(--accent) 18%, transparent);
    transition: color 150ms ease, background 150ms ease;
  }
  .pin-remove:hover {
    color: var(--error);
    background: color-mix(in oklab, var(--error) 12%, transparent);
  }

  .crumbs-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .crumbs-row .crumbs {
    flex: 1 1 auto;
    min-width: 0;
  }
  .pin-current {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border-radius: 999px;
    border: none;
    background: transparent;
    color: var(--tertiary);
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    flex-shrink: 0;
    transition: border-color 150ms ease, color 150ms ease, background 150ms ease;
  }
  .pin-current:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .pin-current.active {
    border-color: color-mix(in oklab, var(--accent) 60%, transparent);
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }

  .crumb-chevron-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  .crumb-chevron {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    margin-left: 1px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 4px;
    color: color-mix(in oklab, currentColor 45%, transparent);
    cursor: pointer;
    transition: color 120ms ease, background 120ms ease, transform 150ms ease;
  }
  .crumb-chevron:hover {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .crumb-chevron.open {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    transform: rotate(180deg);
  }
  .crumb-popover {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 50;
    min-width: 200px;
    max-width: 320px;
    max-height: 300px;
    overflow-y: auto;
    padding: 4px;
    background: var(--surface, var(--button-elevated));
    border: none;
    border-radius: 8px;
    box-shadow: 0 8px 24px color-mix(in oklab, black 18%, transparent);
    display: flex;
    flex-direction: column;
    gap: 1px;
    scrollbar-width: thin;
  }
  .crumb-popover-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: transparent;
    border: 0;
    border-radius: 5px;
    color: var(--secondary);
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .crumb-popover-item:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .crumb-popover-item.current {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    font-weight: 600;
  }
  .crumb-popover-item svg {
    color: color-mix(in oklab, var(--accent) 70%, currentColor);
    flex-shrink: 0;
  }
  .crumb-popover-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .crumb-popover-count {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 10px;
    color: var(--tertiary);
    font-variant-numeric: tabular-nums;
  }
  .crumb-popover-empty {
    padding: 8px 10px;
    color: var(--tertiary);
    font-size: 12px;
  }
  @media (prefers-reduced-motion: reduce) {
    .crumb-chevron,
    .crumb-chevron.open {
      transition: none;
      transform: none;
    }
  }

  .view-mode-toggle {
    display: inline-flex;
    border: none;
    border-radius: 7px;
    padding: 1px;
    background: color-mix(in oklab, var(--content-bg) 80%, transparent);
    flex-shrink: 0;
  }
  .vm-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 26px;
    background: transparent;
    border: 0;
    border-radius: 5px;
    color: color-mix(in oklab, currentColor 55%, transparent);
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease;
  }
  .vm-btn:hover:not(.active) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    color: var(--secondary);
  }
  .vm-btn.active {
    background: var(--accent);
    color: var(--on-accent, white);
  }

  .grid.browse-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grid.browse-list .card {
    flex-direction: row;
    align-items: center;
    width: 100%;
    padding: 6px;
    border-radius: 8px;
    transition: background 150ms ease;
  }
  .grid.browse-list .card:hover {
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .grid.browse-list .thumb {
    width: 160px;
    flex-shrink: 0;
    border-radius: 6px;
  }
  .grid.browse-list .meta {
    flex: 1;
    min-width: 0;
    padding: 0 12px;
  }
  .grid.browse-list .video-thumb,
  .grid.browse-list .folder-thumb,
  .grid.browse-list .doc-thumb {
    aspect-ratio: 16 / 9;
  }
  .grid.browse-list .kind-badge {
    top: 4px;
    left: 4px;
    font-size: 9px;
    padding: 0.1rem 0.4rem;
  }

  .folder-thumb {
    overflow: hidden;
  }
  .folder-mosaic {
    width: 100%;
    height: 100%;
    display: grid;
    gap: 1px;
    background: color-mix(in oklab, var(--accent) 8%, var(--input-bg));
  }
  .folder-mosaic[data-tiles="1"] {
    grid-template-columns: 1fr;
    grid-template-rows: 1fr;
  }
  .folder-mosaic[data-tiles="2"] {
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr;
  }
  .folder-mosaic[data-tiles="3"] {
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
  }
  .folder-mosaic[data-tiles="3"] .folder-mosaic-tile:first-child {
    grid-row: span 2;
  }
  .folder-mosaic[data-tiles="4"] {
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
  }
  .folder-mosaic-tile {
    position: relative;
    overflow: hidden;
    background: color-mix(in oklab, var(--accent) 12%, var(--input-bg));
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .folder-mosaic-tile img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .address-bar {
    flex: 1 1 auto;
    min-width: 0;
    padding: 5px 10px;
    border: 1px solid color-mix(in oklab, var(--accent) 60%, transparent);
    border-radius: 7px;
    background: var(--input-bg);
    color: var(--secondary);
    font-family: ui-monospace, "IBM Plex Mono", monospace;
    font-size: 12px;
    outline: none;
  }
  .address-bar:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--accent) 18%, transparent);
  }
  .address-bar-toggle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: color-mix(in oklab, currentColor 55%, transparent);
    cursor: pointer;
    flex-shrink: 0;
    transition: border-color 120ms ease, color 120ms ease;
  }
  .address-bar-toggle:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .crumbs.hidden {
    display: none;
  }

  .ghost-btn-mini {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border-radius: 999px;
    border: none;
    background: transparent;
    color: var(--tertiary);
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    flex-shrink: 0;
    transition: border-color 150ms ease, color 150ms ease, background 150ms ease;
  }
  .ghost-btn-mini:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .ghost-btn-mini:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .rename-input {
    width: 100%;
    padding: 2px 4px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--input-bg);
    color: var(--secondary);
    font-family: inherit;
    font-size: inherit;
    outline: none;
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--accent) 18%, transparent);
  }

  .card.renaming {
    cursor: default;
  }
  .card.renaming:hover {
    transform: none;
  }

  .folder-card-creating {
    border: 1px dashed color-mix(in oklab, var(--accent) 50%, transparent);
    border-radius: 10px;
    padding: 4px;
  }

  .ctx-menu {
    position: fixed;
    z-index: 200;
    min-width: 180px;
    max-width: 280px;
    padding: 4px;
    background: var(--surface, var(--button-elevated));
    border: none;
    border-radius: 8px;
    box-shadow: 0 8px 28px color-mix(in oklab, black 22%, transparent);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ctx-item {
    display: flex;
    align-items: center;
    padding: 7px 10px;
    background: transparent;
    border: 0;
    border-radius: 5px;
    color: var(--secondary);
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease;
  }
  .ctx-item:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .ctx-item.danger {
    color: var(--error);
  }
  .ctx-item.danger:hover {
    background: color-mix(in oklab, var(--error) 12%, transparent);
  }

  .video-card.last-watched .video-thumb {
    box-shadow: 0 0 0 2px var(--accent), 0 0 0 4px color-mix(in oklab, var(--accent) 30%, transparent);
  }
  .last-watched-badge {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 2;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--on-accent, white);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.02em;
    box-shadow: 0 2px 6px color-mix(in oklab, black 25%, transparent);
  }

  .doc-card[data-ext="pdf"] .doc-thumb {
    background: linear-gradient(
      135deg,
      color-mix(in oklab, oklch(0.6 0.2 30) 14%, var(--input-bg)),
      color-mix(in oklab, oklch(0.5 0.18 25) 6%, var(--input-bg))
    );
    color: oklch(0.7 0.18 28);
  }
  .doc-card[data-ext="epub"] .doc-thumb {
    background: linear-gradient(
      135deg,
      color-mix(in oklab, oklch(0.65 0.18 145) 14%, var(--input-bg)),
      color-mix(in oklab, oklch(0.55 0.16 150) 6%, var(--input-bg))
    );
    color: oklch(0.7 0.16 145);
  }
  .doc-card[data-ext="cbz"] .doc-thumb,
  .doc-card[data-ext="cbr"] .doc-thumb {
    background: linear-gradient(
      135deg,
      color-mix(in oklab, oklch(0.6 0.2 300) 14%, var(--input-bg)),
      color-mix(in oklab, oklch(0.5 0.18 305) 6%, var(--input-bg))
    );
    color: oklch(0.7 0.18 300);
  }
  .doc-card[data-ext="mobi"] .doc-thumb,
  .doc-card[data-ext="azw3"] .doc-thumb {
    background: linear-gradient(
      135deg,
      color-mix(in oklab, oklch(0.7 0.16 75) 14%, var(--input-bg)),
      color-mix(in oklab, oklch(0.6 0.14 80) 6%, var(--input-bg))
    );
    color: oklch(0.72 0.16 75);
  }
  .doc-card[data-ext="djvu"] .doc-thumb {
    background: linear-gradient(
      135deg,
      color-mix(in oklab, oklch(0.6 0.16 240) 14%, var(--input-bg)),
      color-mix(in oklab, oklch(0.5 0.14 245) 6%, var(--input-bg))
    );
    color: oklch(0.7 0.16 240);
  }

  .empty-with-roots {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    padding: 48px 24px 24px;
    text-align: center;
    max-width: 50ch;
    margin: 0 auto;
  }
  .empty-with-roots p {
    margin: 0;
    color: var(--secondary);
    font-size: 14px;
    line-height: 1.5;
  }

  .root-row.missing {
    background: color-mix(in oklab, var(--warning) 8%, transparent);
    border-radius: 6px;
  }
  .root-row.missing .root-path {
    color: color-mix(in oklab, var(--warning) 80%, currentColor);
    text-decoration: line-through;
    text-decoration-color: color-mix(in oklab, currentColor 40%, transparent);
  }
  .missing-tag {
    background: color-mix(in oklab, var(--warning) 18%, transparent) !important;
    color: var(--warning) !important;
    border-color: color-mix(in oklab, var(--warning) 40%, transparent) !important;
    cursor: help;
  }

  .sort-wrap {
    position: relative;
    display: inline-flex;
    flex-shrink: 0;
  }
  .sort-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 60;
    min-width: 160px;
    padding: 4px;
    background: var(--surface, var(--button-elevated));
    border: none;
    border-radius: 8px;
    box-shadow: 0 8px 24px color-mix(in oklab, black 22%, transparent);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ctx-item.current {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    font-weight: 600;
  }

  .card.selected {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .card.selected.last-watched .video-thumb {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .tab-bar {
    display: flex;
    flex-wrap: nowrap;
    overflow-x: auto;
    gap: 2px;
    padding: 0;
    margin: 0;
    border-bottom: none;
    scrollbar-width: thin;
  }
  .tab-wrap {
    display: inline-flex;
    align-items: stretch;
    flex-shrink: 0;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: 7px 7px 0 0;
    background: transparent;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .tab-wrap:hover:not(.active) {
    background: color-mix(in oklab, var(--accent) 5%, transparent);
  }
  .tab-wrap.active {
    background: var(--surface, var(--button-elevated));
    border-color: color-mix(in oklab, var(--content-border) 60%, transparent);
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 4px 6px 10px;
    background: transparent;
    border: 0;
    color: var(--secondary);
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    max-width: 200px;
  }
  .tab svg {
    color: var(--accent);
    flex-shrink: 0;
  }
  .tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-wrap.active .tab {
    color: var(--text, var(--secondary));
    font-weight: 500;
  }
  .tab-close {
    background: transparent;
    border: 0;
    color: color-mix(in oklab, currentColor 50%, transparent);
    font-family: inherit;
    font-size: 14px;
    line-height: 1;
    padding: 0 8px;
    cursor: pointer;
    border-radius: 4px;
    transition: color 120ms ease, background 120ms ease;
  }
  .tab-close:hover {
    color: var(--error);
    background: color-mix(in oklab, var(--error) 12%, transparent);
  }

  .bulk-action-bar {
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 90;
    display: inline-flex;
    align-items: center;
    gap: 12px;
    padding: 10px 18px;
    background: var(--surface, var(--button-elevated));
    border: none;
    border-radius: 999px;
    box-shadow: 0 8px 28px color-mix(in oklab, black 30%, transparent);
  }
  .bulk-count {
    font-size: 13px;
    font-weight: 600;
    color: var(--secondary);
  }
  .bulk-btn {
    padding: 6px 14px;
    background: transparent;
    border: none;
    border-radius: 999px;
    color: var(--secondary);
    font-family: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: border-color 120ms ease, color 120ms ease, background 120ms ease;
  }
  .bulk-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .bulk-btn.danger {
    background: var(--error);
    border-color: var(--error);
    color: white;
  }
  .bulk-btn.danger:hover {
    background: color-mix(in oklab, var(--error) 88%, black);
    color: white;
  }
</style>
