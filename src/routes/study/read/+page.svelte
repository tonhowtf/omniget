<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import SegmentedControl from "$lib/study-components/SegmentedControl.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  let confirmOpen = $state(false);
  let confirmMessage = $state("");
  let confirmTitle = $state("");
  let confirmAction: (() => void) | null = null;

  function askConfirm(title: string, message: string, action: () => void) {
    confirmTitle = title;
    confirmMessage = message;
    confirmAction = action;
    confirmOpen = true;
  }

  function runConfirm() {
    if (confirmAction) confirmAction();
    confirmAction = null;
  }

  type Book = {
    id: number;
    file_path: string;
    file_hash: string;
    format: string;
    title: string | null;
    author: string | null;
    page_count: number | null;
    cover_path: string | null;
    added_at: number;
    last_opened_at: number | null;
    last_location: string | null;
    reading_pct: number;
    favorite: boolean;
    tags_json: string;
  };

  type SortKey = "last_opened" | "added" | "title" | "author";
  type FilterKey = "all" | "reading" | "finished" | "favorites";
  type FormatKey = "all" | "pdf" | "epub" | "txt" | "html" | "cbz";
  type Root = { path: string; enabled: boolean; added_at: number };

  type LibraryPage = { items: Book[]; total: number; page: number; page_size: number };

  let books = $state<Book[]>([]);
  let roots = $state<Root[]>([]);
  let loading = $state(true);
  let loadingMore = $state(false);
  let pageIndex = $state(0);
  let totalBooks = $state(0);
  let eofBooks = $state(false);
  let listSentinelEl: HTMLDivElement | null = $state(null);
  const PAGE_SIZE = 60;
  let scanning = $state(false);
  let scanMsg = $state("");
  let search = $state("");
  let sortKey = $state<SortKey>("last_opened");
  let filterKey = $state<FilterKey>("all");
  let formatKey = $state<FormatKey>("all");
  let rootKey = $state<string>("all");
  let rootsOpen = $state(false);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  const filterOptions = $derived([
    { value: "all", label: $t("study.read.filter_all") },
    { value: "reading", label: $t("study.read.filter_reading") },
    { value: "finished", label: $t("study.read.filter_finished") },
    { value: "favorites", label: $t("study.read.filter_favorites") },
  ]);

  const formatCounts = $derived.by(() => {
    const sumLoaded = Object.values(byFormatCounts).reduce(
      (a: number, b: number) => a + b,
      0,
    );
    const allCount = sumLoaded > 0 ? sumLoaded : totalBooks;
    return { ...byFormatCounts, all: allCount } as Record<string, number>;
  });

  const formatOptions = $derived([
    { value: "all", label: `${$t("study.read.format_all")} · ${formatCounts.all ?? 0}` },
    { value: "pdf", label: `PDF · ${formatCounts.pdf ?? 0}` },
    { value: "epub", label: `EPUB · ${formatCounts.epub ?? 0}` },
    { value: "txt", label: `TXT · ${formatCounts.txt ?? 0}` },
    { value: "html", label: `HTML · ${formatCounts.html ?? 0}` },
    { value: "cbz", label: `CBZ · ${formatCounts.cbz ?? 0}` },
  ]);

  function normPath(p: string): string {
    return p.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  }

  const visible = $derived.by(() => {
    const term = search.trim().toLowerCase();
    const rootNorm = rootKey === "all" ? "" : normPath(rootKey);
    const filterByCollection = collectionKey !== "all";
    let list = books.filter((b) => {
      if (filterKey === "favorites" && !b.favorite) return false;
      if (filterKey === "reading" && !(b.reading_pct > 0 && b.reading_pct < 100))
        return false;
      if (filterKey === "finished" && b.reading_pct < 100) return false;
      if (formatKey !== "all" && b.format !== formatKey) return false;
      if (filterByCollection && !collectionBookIds.has(b.id)) return false;
      if (rootNorm) {
        const fp = normPath(b.file_path);
        if (!fp.startsWith(rootNorm + "/") && fp !== rootNorm) return false;
      }
      if (term) {
        const hay = `${b.title ?? ""} ${b.author ?? ""} ${b.file_path}`.toLowerCase();
        if (!hay.includes(term)) return false;
      }
      return true;
    });
    const cmp = (a: Book, b: Book) => {
      if (sortKey === "title") return (a.title ?? "").localeCompare(b.title ?? "");
      if (sortKey === "author") return (a.author ?? "").localeCompare(b.author ?? "");
      if (sortKey === "added") return b.added_at - a.added_at;
      return (b.last_opened_at ?? 0) - (a.last_opened_at ?? 0);
    };
    return [...list].sort(cmp);
  });

  function buildBackendFilters(page: number) {
    const filters: Record<string, unknown> = {
      page,
      pageSize: PAGE_SIZE,
    };
    const term = search.trim();
    if (term) filters.search = term;
    if (formatKey !== "all") filters.format = formatKey;
    if (filterKey === "favorites") filters.favoritesOnly = true;
    return filters;
  }

  function unwrapLibraryResponse(res: unknown): {
    items: Book[];
    total: number;
    by_format: Record<string, number>;
  } {
    if (Array.isArray(res)) {
      const items = res as Book[];
      const by: Record<string, number> = {};
      for (const b of items) by[b.format] = (by[b.format] ?? 0) + 1;
      return { items, total: items.length, by_format: by };
    }
    const obj = (res ?? {}) as {
      items?: Book[];
      total?: number;
      by_format?: Record<string, number>;
    };
    const items = Array.isArray(obj.items) ? obj.items : [];
    return {
      items,
      total: typeof obj.total === "number" ? obj.total : items.length,
      by_format: obj.by_format ?? {},
    };
  }

  let byFormatCounts = $state<Record<string, number>>({});

  async function loadBooks() {
    pageIndex = 0;
    eofBooks = false;
    loading = true;
    try {
      const raw = await pluginInvoke<unknown>("study", "study:read:library:list", {
        filters: buildBackendFilters(0),
      });
      const { items, total, by_format } = unwrapLibraryResponse(raw);
      books = items;
      totalBooks = total;
      byFormatCounts = by_format;
      eofBooks = books.length >= totalBooks;
    } catch (e) {
      console.error("read library load failed", e);
      books = [];
      totalBooks = 0;
      byFormatCounts = {};
      eofBooks = true;
    } finally {
      loading = false;
    }
    ensureCovers();
  }

  async function loadMoreBooks() {
    if (eofBooks || loadingMore) return;
    loadingMore = true;
    try {
      const next = pageIndex + 1;
      const raw = await pluginInvoke<unknown>("study", "study:read:library:list", {
        filters: buildBackendFilters(next),
      });
      const { items, total, by_format } = unwrapLibraryResponse(raw);
      if (Object.keys(by_format).length > 0) byFormatCounts = by_format;
      if (items.length === 0) {
        eofBooks = true;
      } else {
        books = [...books, ...items];
        pageIndex = next;
        totalBooks = total > totalBooks ? total : totalBooks;
        eofBooks = books.length >= totalBooks;
      }
    } catch (e) {
      console.error("load more books failed", e);
      eofBooks = true;
    } finally {
      loadingMore = false;
    }
    ensureCovers();
  }

  $effect(() => {
    if (!listSentinelEl || eofBooks) return;
    const el = listSentinelEl;
    const obs = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) void loadMoreBooks();
      },
      { rootMargin: "400px" },
    );
    obs.observe(el);
    return () => obs.disconnect();
  });

  let initialized = $state(false);
  $effect(() => {
    void search;
    void formatKey;
    void filterKey;
    if (!initialized) return;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      void loadBooks();
    }, 250);
  });

  async function loadRoots() {
    try {
      const res = await pluginInvoke<{ roots: Root[] }>(
        "study",
        "study:read:roots:list",
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("read:roots:list failed", e);
      roots = [];
    }
  }

  async function addRoot() {
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.open({ directory: true, multiple: false });
      if (typeof picked !== "string" || !picked) return;
      const res = await pluginInvoke<{ roots: Root[] }>(
        "study",
        "study:read:roots:add",
        { path: picked },
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("add root failed", e);
    }
  }

  async function toggleRoot(r: Root) {
    try {
      const res = await pluginInvoke<{ roots: Root[] }>(
        "study",
        "study:read:roots:toggle",
        { path: r.path, enabled: !r.enabled },
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("toggle root failed", e);
    }
  }

  async function removeRoot(r: Root) {
    try {
      const res = await pluginInvoke<{ roots: Root[] }>(
        "study",
        "study:read:roots:remove",
        { path: r.path },
      );
      roots = res.roots ?? [];
    } catch (e) {
      console.error("remove root failed", e);
    }
  }

  function shortPath(p: string): string {
    const parts = p.replace(/\\/g, "/").split("/").filter(Boolean);
    if (parts.length <= 2) return p;
    return `…/${parts.slice(-2).join("/")}`;
  }

  let extractingCovers = false;
  async function ensureCovers() {
    if (extractingCovers) return;
    const cmdFor: Record<string, string> = {
      pdf: "study:read:pdf:extract_cover",
      epub: "study:read:epub:extract_cover",
      cbz: "study:read:cbz:extract_cover",
    };
    const pending = books.filter((b) => cmdFor[b.format] && !b.cover_path);
    if (pending.length === 0) {
      const formats = [...new Set(books.map((b) => b.format))];
      console.info(
        `[read/covers] no extractable covers pending. ${books.length} books, formats: ${formats.join(", ") || "(none)"}`,
      );
      return;
    }
    console.info(`[read/covers] extracting ${pending.length} cover(s)…`);
    extractingCovers = true;
    let okCount = 0;
    const failures: { id: number; format: string; error: string }[] = [];
    try {
      for (const b of pending) {
        const cmd = cmdFor[b.format];
        try {
          const res = await pluginInvoke<{ cover_path: string }>("study", cmd, {
            bookId: b.id,
          });
          books = books.map((x) =>
            x.id === b.id ? { ...x, cover_path: res.cover_path } : x,
          );
          okCount += 1;
        } catch (e) {
          failures.push({
            id: b.id,
            format: b.format,
            error: e instanceof Error ? e.message : String(e),
          });
        }
      }
    } finally {
      extractingCovers = false;
    }
    if (failures.length > 0) {
      console.warn(
        `[read/covers] ${okCount} ok, ${failures.length} failed:`,
        failures,
      );
    } else {
      console.info(`[read/covers] all ${okCount} cover(s) extracted ok`);
    }
  }

  let cleaningOrphans = $state(false);
  async function cleanOrphans() {
    if (cleaningOrphans) return;
    cleaningOrphans = true;
    try {
      const res = await pluginInvoke<{
        removed_missing: number;
        removed_member: number;
        removed_junk: number;
        total: number;
      }>("study", "study:read:books:clear_orphans");
      scanMsg = $t("study.read.orphans_cleared", {
        missing: res?.removed_missing ?? 0,
        member: res?.removed_member ?? 0,
        junk: res?.removed_junk ?? 0,
      });
      await loadBooks();
    } catch (e) {
      scanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      cleaningOrphans = false;
    }
  }

  async function rescan() {
    if (scanning) return;
    scanning = true;
    scanMsg = "";
    try {
      const res = await pluginInvoke<{
        books_found: number;
        added: number;
        updated: number;
        removed: number;
      }>("study", "study:read:scan");
      const removed = res?.removed ?? 0;
      scanMsg = $t("study.read.scan_done", {
        added: res?.added ?? 0,
        updated: res?.updated ?? 0,
        found: res?.books_found ?? 0,
      });
      if (removed > 0) {
        scanMsg += " · " + $t("study.read.scan_removed", { n: removed });
      }
      await loadBooks();
    } catch (e) {
      scanMsg = e instanceof Error ? e.message : String(e);
    } finally {
      scanning = false;
    }
  }

  let bulkExporting = $state(false);
  let bulkExportMsg = $state("");

  async function bulkExportMdV2() {
    if (bulkExporting) return;
    const dialog = await import("@tauri-apps/plugin-dialog");
    const targetDir = await dialog.open({ directory: true, multiple: false });
    if (typeof targetDir !== "string" || !targetDir) return;
    bulkExporting = true;
    bulkExportMsg = "";
    try {
      const r = await pluginInvoke<{
        directory: string;
        written: number;
        skipped: number;
        total_bytes: number;
      }>("study", "study:read:export:markdown_v2_all", { targetDir });
      bulkExportMsg = $t("study.read.bulk_export_done", {
        written: r.written,
        skipped: r.skipped,
        kb: Math.round(r.total_bytes / 1024),
      });
      setTimeout(() => (bulkExportMsg = ""), 6000);
    } catch (e) {
      bulkExportMsg =
        e instanceof Error ? e.message : String(e);
      setTimeout(() => (bulkExportMsg = ""), 8000);
    } finally {
      bulkExporting = false;
    }
  }

  async function toggleFavorite(b: Book, ev: MouseEvent) {
    ev.stopPropagation();
    ev.preventDefault();
    try {
      const res = await pluginInvoke<{ id: number; favorite: boolean }>(
        "study",
        "study:read:books:toggle_favorite",
        { bookId: b.id },
      );
      books = books.map((x) => (x.id === res.id ? { ...x, favorite: res.favorite } : x));
    } catch (e) {
      console.error(e);
    }
  }

  function openBook(b: Book) {
    goto(`/study/read/${b.id}`);
  }

  function confirmDeleteBook(b: Book, ev: Event) {
    ev.stopPropagation();
    ev.preventDefault();
    askConfirm(
      $t("study.read.delete_book_title"),
      $t("study.read.delete_book_confirm", { title: deriveTitle(b) }),
      () => void doDeleteBook(b),
    );
  }

  async function doDeleteBook(b: Book) {
    try {
      const res = await pluginInvoke<{ file_missing?: boolean }>(
        "study",
        "study:read:books:delete",
        { bookId: b.id, deleteFile: true },
      );
      books = books.filter((x) => x.id !== b.id);
      totalBooks = Math.max(0, totalBooks - 1);
      byFormatCounts = {
        ...byFormatCounts,
        [b.format]: Math.max(0, (byFormatCounts[b.format] ?? 1) - 1),
      };
      if (tagBook?.id === b.id) tagBook = null;
      if (enrichBook?.id === b.id) enrichBook = null;
      showToast(
        "success",
        $t(
          res?.file_missing
            ? "study.read.delete_book_done_missing"
            : "study.read.delete_book_done",
          { title: deriveTitle(b) },
        ),
      );
    } catch (e) {
      showToast(
        "error",
        $t("study.read.delete_book_failed", {
          err: e instanceof Error ? e.message : String(e),
        }),
      );
    }
  }

  type Collection = { id: number; name: string; sort_order: number };
  let collections = $state<Collection[]>([]);
  let collectionKey = $state<string>("all");
  let collectionBookIds = $state<Set<number>>(new Set());

  let tagBook = $state<Book | null>(null);
  let tagInput = $state("");

  type EnrichCandidate = {
    ol_key: string;
    title: string;
    author: string | null;
    year: number | null;
    cover_url: string | null;
    isbn: string | null;
    publisher: string | null;
  };

  let enrichBook = $state<Book | null>(null);
  let enrichLoading = $state(false);
  let enrichCandidates = $state<EnrichCandidate[]>([]);
  let enrichSelected = $state<EnrichCandidate | null>(null);
  let enrichApply = $state<{ title: boolean; author: boolean; cover: boolean; publisher: boolean }>({
    title: true,
    author: true,
    cover: true,
    publisher: true,
  });
  let enrichApplying = $state(false);
  let enrichError = $state("");

  function openEnrichEditor(b: Book, ev: MouseEvent) {
    ev.stopPropagation();
    ev.preventDefault();
    enrichBook = b;
    enrichCandidates = [];
    enrichSelected = null;
    enrichError = "";
  }

  async function searchEnrichment() {
    if (!enrichBook) return;
    enrichLoading = true;
    enrichError = "";
    enrichCandidates = [];
    enrichSelected = null;
    try {
      const title = (enrichBook.title ?? deriveTitle(enrichBook)).trim();
      const author = (enrichBook.author ?? "").trim() || undefined;
      const res = await pluginInvoke<{ candidates: EnrichCandidate[] }>(
        "study",
        "study:read:openlibrary:enrich_title",
        { title, author },
      );
      enrichCandidates = Array.isArray(res?.candidates) ? res.candidates : [];
      if (enrichCandidates.length === 0) {
        enrichError = $t("study.read.enrich_no_results") as string;
      } else {
        enrichSelected = enrichCandidates[0];
      }
    } catch (e) {
      enrichError = e instanceof Error ? e.message : String(e);
    } finally {
      enrichLoading = false;
    }
  }

  async function applyEnrichment() {
    if (!enrichBook || !enrichSelected) return;
    enrichApplying = true;
    try {
      const fields: Record<string, unknown> = {};
      if (enrichApply.title && enrichSelected.title) fields.title = enrichSelected.title;
      if (enrichApply.author && enrichSelected.author) fields.author = enrichSelected.author;
      if (enrichApply.cover && enrichSelected.cover_url) fields.cover_url = enrichSelected.cover_url;
      if (enrichApply.publisher && enrichSelected.publisher) fields.publisher = enrichSelected.publisher;
      await pluginInvoke("study", "study:read:books:apply_enrichment", {
        bookId: enrichBook.id,
        fields,
      });
      const targetId = enrichBook.id;
      books = books.map((x) => {
        if (x.id !== targetId) return x;
        return {
          ...x,
          title: enrichApply.title && enrichSelected!.title ? enrichSelected!.title : x.title,
          author: enrichApply.author && enrichSelected!.author ? enrichSelected!.author : x.author,
          publisher: enrichApply.publisher && enrichSelected!.publisher
            ? enrichSelected!.publisher
            : (x as Book & { publisher?: string }).publisher,
        } as Book;
      });
      enrichBook = null;
      okMsg = $t("study.read.enrich_done") as string;
      setTimeout(() => (okMsg = ""), 2500);
    } catch (e) {
      enrichError = e instanceof Error ? e.message : String(e);
    } finally {
      enrichApplying = false;
    }
  }

  let okMsg = $state<string>("");

  let collectionsOpen = $state(false);
  let renamingId = $state<number | null>(null);
  let renameInput = $state("");

  function parseTags(b: Book): string[] {
    try {
      const arr = JSON.parse(b.tags_json || "[]");
      return Array.isArray(arr) ? arr.filter((t) => typeof t === "string") : [];
    } catch {
      return [];
    }
  }

  async function loadCollections() {
    try {
      const arr = await pluginInvoke<Collection[]>(
        "study",
        "study:read:collections:list",
      );
      collections = Array.isArray(arr) ? arr : [];
    } catch (e) {
      console.error("collections:list failed", e);
      collections = [];
    }
  }

  async function loadCollectionMembers(collectionId: number) {
    try {
      const arr = await pluginInvoke<{ id: number }[]>(
        "study",
        "study:read:collections:books",
        { collectionId },
      );
      collectionBookIds = new Set((Array.isArray(arr) ? arr : []).map((b) => b.id));
    } catch (e) {
      console.error("collections:books failed", e);
      collectionBookIds = new Set();
    }
  }

  $effect(() => {
    if (collectionKey === "all") {
      collectionBookIds = new Set();
    } else {
      const id = Number(collectionKey);
      if (Number.isFinite(id)) {
        void loadCollectionMembers(id);
      }
    }
  });

  async function createCollection() {
    const name = window.prompt($t("study.read.collection_create_prompt"));
    if (!name || !name.trim()) return;
    try {
      await pluginInvoke("study", "study:read:collections:create", {
        name: name.trim(),
      });
      await loadCollections();
    } catch (e) {
      console.error(e);
    }
  }

  async function commitRename(c: Collection) {
    const trimmed = renameInput.trim();
    renamingId = null;
    if (!trimmed || trimmed === c.name) return;
    try {
      await pluginInvoke("study", "study:read:collections:rename", {
        collectionId: c.id,
        name: trimmed,
      });
      await loadCollections();
    } catch (e) {
      console.error(e);
    }
  }

  async function deleteCollection(c: Collection) {
    askConfirm(
      $t("study.common.confirm"),
      $t("study.read.collection_delete_confirm", { name: c.name }),
      async () => await doDeleteCollection(c),
    );
  }

  async function doDeleteCollection(c: Collection) {
    try {
      await pluginInvoke("study", "study:read:collections:delete", {
        collectionId: c.id,
      });
      if (collectionKey === String(c.id)) collectionKey = "all";
      await loadCollections();
    } catch (e) {
      console.error(e);
    }
  }

  async function toggleBookInActiveCollection(b: Book) {
    if (collectionKey === "all") return;
    const id = Number(collectionKey);
    if (!Number.isFinite(id)) return;
    const cmd = collectionBookIds.has(b.id)
      ? "study:read:collections:remove"
      : "study:read:collections:add";
    try {
      await pluginInvoke("study", cmd, { collectionId: id, bookId: b.id });
      await loadCollectionMembers(id);
    } catch (e) {
      console.error(e);
    }
  }

  async function saveTags(b: Book, newTags: string[]) {
    try {
      await pluginInvoke("study", "study:read:books:set_tags", {
        bookId: b.id,
        tags: newTags,
      });
      const tj = JSON.stringify(newTags);
      books = books.map((x) => (x.id === b.id ? { ...x, tags_json: tj } : x));
      if (tagBook && tagBook.id === b.id) {
        tagBook = { ...tagBook, tags_json: tj };
      }
    } catch (e) {
      console.error(e);
    }
  }

  function openTagEditor(b: Book, ev: MouseEvent) {
    ev.stopPropagation();
    ev.preventDefault();
    tagBook = b;
    tagInput = "";
  }

  async function addTagFromInput() {
    if (!tagBook) return;
    const t = tagInput.trim().toLowerCase();
    tagInput = "";
    if (!t) return;
    const current = parseTags(tagBook);
    if (current.includes(t)) return;
    await saveTags(tagBook, [...current, t]);
  }

  async function removeTag(b: Book, tag: string) {
    const current = parseTags(b).filter((t) => t !== tag);
    await saveTags(b, current);
  }

  function deriveTitle(b: Book): string {
    if (b.title && b.title.trim()) return b.title;
    const parts = b.file_path.replace(/\\/g, "/").split("/");
    return parts[parts.length - 1] ?? b.file_path;
  }

  function formatLabel(fmt: string): string {
    const key = `study.read.format_${fmt}`;
    const labelled = $t(key);
    return labelled === key ? fmt.toUpperCase() : labelled;
  }

  function statusLabel(b: Book): string {
    if (b.reading_pct >= 100) return $t("study.read.finished");
    if (b.reading_pct > 0) return $t("study.read.in_progress");
    return $t("study.read.not_started");
  }

  onMount(async () => {
    await Promise.all([loadBooks(), loadRoots(), loadCollections()]);
    initialized = true;
  });
</script>

<section class="read-page">
  <header class="head">
    <div class="row-primary">
      <h1 class="page-title">{$t("study.read.title")}</h1>
      <button
        type="button"
        class="ghost-btn"
        onclick={() => (rootsOpen = true)}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path>
        </svg>
        <span>{$t("study.read.roots_btn", { count: roots.filter((r) => r.enabled).length })}</span>
      </button>
      <button
        type="button"
        class="ghost-btn"
        onclick={() => (collectionsOpen = true)}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M4 6h16M4 12h16M4 18h12"></path>
        </svg>
        <span>{$t("study.read.collections_btn", { count: collections.length })}</span>
      </button>
      <a href="/study/read/manga" class="ghost-btn">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3" y="4" width="7" height="16" rx="1"></rect>
          <rect x="12" y="4" width="4" height="16" rx="1"></rect>
          <rect x="18" y="4" width="3" height="16" rx="1"></rect>
        </svg>
        <span>{$t("study.read.manga_nav_btn")}</span>
      </a>
      <a href="/study/read/books/discover" class="ghost-btn">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="11" cy="11" r="7"></circle>
          <path d="M21 21l-4.35-4.35"></path>
          <path d="M8 11h6"></path>
        </svg>
        <span>{$t("study.read.book_discover_nav")}</span>
      </a>
      <a href="/study/read/search" class="ghost-btn">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="11" cy="11" r="7"></circle>
          <path d="M21 21l-4.35-4.35"></path>
        </svg>
        <span>{$t("study.read.search_notes_btn")}</span>
      </a>
      <a href="/study/read/downloads" class="ghost-btn">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 3v12 M7 10l5 5 5-5 M5 21h14"></path>
        </svg>
        <span>{$t("study.read.downloads_btn")}</span>
      </a>
      <button
        type="button"
        class="ghost-btn"
        onclick={rescan}
        disabled={scanning}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 12a9 9 0 1 1-3-6.7"></path>
          <path d="M21 3v6h-6"></path>
        </svg>
        <span>{scanning ? $t("study.read.scanning") : $t("study.read.scan_now")}</span>
      </button>
      <button
        type="button"
        class="ghost-btn"
        onclick={cleanOrphans}
        disabled={cleaningOrphans || scanning}
        title={$t("study.read.clean_orphans_hint")}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M3 6h18"></path>
          <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
          <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"></path>
        </svg>
        <span>{cleaningOrphans ? $t("study.common.loading") : $t("study.read.clean_orphans_btn")}</span>
      </button>
      <button
        type="button"
        class="ghost-btn"
        onclick={bulkExportMdV2}
        disabled={bulkExporting}
        title={$t("study.read.bulk_export_hint")}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 3v12"></path>
          <path d="M7 10l5 5 5-5"></path>
          <path d="M5 21h14"></path>
        </svg>
        <span>{bulkExporting ? $t("study.common.loading") : $t("study.read.bulk_export_btn")}</span>
      </button>
    </div>
    <p class="subtitle">{$t("study.read.subtitle")}</p>
    <div class="row-secondary">
      <SegmentedControl bind:value={filterKey} options={filterOptions} ariaLabel="filter" />
      <input
        type="search"
        class="search"
        placeholder={$t("study.read.search_placeholder")}
        bind:value={search}
      />
      {#if roots.length > 0}
        <label class="sort">
          <span>{$t("study.read.folder_label")}</span>
          <select bind:value={rootKey}>
            <option value="all">{$t("study.read.folder_all")}</option>
            {#each roots as r (r.path)}
              <option value={r.path}>{shortPath(r.path)}</option>
            {/each}
          </select>
        </label>
      {/if}
      {#if collections.length > 0}
        <label class="sort">
          <span>{$t("study.read.collection_label")}</span>
          <select bind:value={collectionKey}>
            <option value="all">{$t("study.read.collection_all")}</option>
            {#each collections as c (c.id)}
              <option value={String(c.id)}>{c.name}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label class="sort">
        <span>{$t("study.library.sort_label")}</span>
        <select bind:value={sortKey}>
          <option value="last_opened">{$t("study.read.sort_last_opened")}</option>
          <option value="added">{$t("study.read.sort_added")}</option>
          <option value="title">{$t("study.read.sort_title")}</option>
          <option value="author">{$t("study.read.sort_author")}</option>
        </select>
      </label>
    </div>
    <div class="row-formats">
      <SegmentedControl bind:value={formatKey} options={formatOptions} ariaLabel="format" />
    </div>
    {#if scanMsg}
      <p class="muted small">{scanMsg}</p>
    {/if}
    {#if bulkExportMsg}
      <p class="muted small">{bulkExportMsg}</p>
    {/if}
  </header>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if books.length === 0 && roots.length === 0}
    <div class="empty-hint">
      <h3>{$t("study.read.empty_no_roots_title")}</h3>
      <p class="muted">{$t("study.read.empty_no_roots_hint")}</p>
      <button type="button" class="cta" onclick={() => (rootsOpen = true)}>
        {$t("study.read.empty_no_roots_cta")}
      </button>
    </div>
  {:else if visible.length === 0}
    <p class="muted">{$t("study.read.empty")}</p>
  {:else}
    <div class="grid">
      {#each visible as b (b.id)}
        <button
          type="button"
          class="card book-card"
          data-ext={b.format}
          onclick={() => openBook(b)}
        >
          <div
            class="thumb book-thumb"
            style={b.cover_path
              ? `background-image: url('${convertFileSrc(b.cover_path)}');`
              : ""}
          >
            {#if !b.cover_path}
              <svg viewBox="0 0 64 64" width="44" height="56" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M14 4h28l10 10v42a4 4 0 0 1-4 4H14a4 4 0 0 1-4-4V8a4 4 0 0 1 4-4z"/>
                <path d="M42 4v10h10"/>
              </svg>
              <span class="ext-stamp">{formatLabel(b.format)}</span>
            {/if}
            <span
              class="star"
              class:active={b.favorite}
              role="button"
              tabindex="0"
              onclick={(e) => toggleFavorite(b, e as MouseEvent)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  e.stopPropagation();
                  toggleFavorite(b, e as unknown as MouseEvent);
                }
              }}
              aria-label={b.favorite ? "unfavorite" : "favorite"}
            >
              <svg viewBox="0 0 24 24" width="18" height="18" fill={b.favorite ? "currentColor" : "none"} stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
              </svg>
            </span>
            <div class="progress" style:--pct="{Math.round(b.reading_pct)}%"></div>
          </div>
          <div class="meta">
            <div class="title">{deriveTitle(b)}</div>
            {#if b.author}
              <div class="author">{b.author}</div>
            {/if}
            <div class="sub">
              <span>{formatLabel(b.format)}</span>
              {#if b.page_count}
                <span>·</span>
                <span>{$t("study.read.page_count", { count: b.page_count })}</span>
              {/if}
              <span>·</span>
              <span class="status">{statusLabel(b)}</span>
            </div>
            {#if parseTags(b).length > 0}
              <div class="tag-row">
                {#each parseTags(b).slice(0, 4) as tag (tag)}
                  <span class="tag-chip">{tag}</span>
                {/each}
                {#if parseTags(b).length > 4}
                  <span class="tag-chip more">+{parseTags(b).length - 4}</span>
                {/if}
              </div>
            {/if}
          </div>
          <div class="card-actions">
            <span
              class="card-action-btn"
              role="button"
              tabindex="0"
              title={$t("study.read.tags_edit")}
              aria-label={$t("study.read.tags_edit")}
              onclick={(e) => openTagEditor(b, e as MouseEvent)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  e.stopPropagation();
                  openTagEditor(b, e as unknown as MouseEvent);
                }
              }}
            >
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M20.59 13.41L13.42 20.58a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z"></path>
                <circle cx="7" cy="7" r="1.5"></circle>
              </svg>
            </span>
            <span
              class="card-action-btn"
              role="button"
              tabindex="0"
              title={$t("study.read.enrich_metadata")}
              aria-label={$t("study.read.enrich_metadata")}
              onclick={(e) => openEnrichEditor(b, e as MouseEvent)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  e.stopPropagation();
                  openEnrichEditor(b, e as unknown as MouseEvent);
                }
              }}
            >
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="11" cy="11" r="7"></circle>
                <path d="M21 21l-4.35-4.35"></path>
                <path d="M11 7v4l3 2" />
              </svg>
            </span>
            <span
              class="card-action-btn danger"
              role="button"
              tabindex="0"
              title={$t("study.read.delete_book_title")}
              aria-label={$t("study.read.delete_book_title")}
              onclick={(e) => confirmDeleteBook(b, e)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  confirmDeleteBook(b, e);
                }
              }}
            >
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 6h18"></path>
                <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
                <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"></path>
              </svg>
            </span>
            {#if collectionKey !== "all"}
              <span
                class="card-action-btn"
                class:active={collectionBookIds.has(b.id)}
                role="button"
                tabindex="0"
                title={collectionBookIds.has(b.id)
                  ? $t("study.read.collection_remove_book")
                  : $t("study.read.collection_add_book")}
                aria-label="toggle collection membership"
                onclick={(e) => {
                  e.stopPropagation();
                  e.preventDefault();
                  void toggleBookInActiveCollection(b);
                }}
                onkeydown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    e.stopPropagation();
                    void toggleBookInActiveCollection(b);
                  }
                }}
              >
                {collectionBookIds.has(b.id) ? "−" : "+"}
              </span>
            {/if}
          </div>
        </button>
      {/each}
    </div>
    {#if !eofBooks}
      <div bind:this={listSentinelEl} class="list-sentinel">
        {#if loadingMore}
          <span class="muted small">{$t("study.common.loading")}</span>
        {/if}
      </div>
    {/if}
  {/if}
</section>

<ConfirmDialog
  bind:open={confirmOpen}
  title={confirmTitle}
  message={confirmMessage}
  variant="danger"
  onConfirm={runConfirm}
/>

{#if tagBook}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="drawer-overlay" onclick={() => (tagBook = null)}></div>
  <div class="drawer drawer-narrow" role="dialog" tabindex="-1" aria-label={$t("study.read.tags_edit")}>
    <header class="drawer-head">
      <h2>{$t("study.read.tags_edit")}</h2>
      <button type="button" class="icon-btn" onclick={() => (tagBook = null)} aria-label="close">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M18 6L6 18M6 6l12 12"></path>
        </svg>
      </button>
    </header>
    <p class="drawer-hint" title={tagBook.file_path}>{deriveTitle(tagBook)}</p>
    {#if parseTags(tagBook).length > 0}
      <div class="tag-row">
        {#each parseTags(tagBook) as t (t)}
          <span class="tag-chip removable">
            {t}
            <button type="button" class="tag-x" onclick={() => removeTag(tagBook!, t)} aria-label="remove">×</button>
          </span>
        {/each}
      </div>
    {:else}
      <p class="muted small">{$t("study.read.tags_empty")}</p>
    {/if}
    <input
      type="text"
      class="search"
      bind:value={tagInput}
      placeholder={$t("study.read.tags_input_placeholder")}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          void addTagFromInput();
        }
      }}
    />
    <button type="button" class="cta outline" onclick={addTagFromInput} disabled={!tagInput.trim()}>
      + {$t("study.read.tags_add_btn")}
    </button>
  </div>
{/if}

{#if enrichBook}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="drawer-overlay" onclick={() => (enrichBook = null)}></div>
  <div class="drawer" role="dialog" tabindex="-1" aria-label={$t("study.read.enrich_metadata")}>
    <header class="drawer-head">
      <h2>{$t("study.read.enrich_metadata")}</h2>
      <button type="button" class="icon-btn" onclick={() => (enrichBook = null)} aria-label="close">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M18 6L6 18M6 6l12 12"></path>
        </svg>
      </button>
    </header>
    <p class="drawer-hint" title={enrichBook.file_path}>{deriveTitle(enrichBook)}</p>
    {#if enrichCandidates.length === 0}
      <button type="button" class="cta" onclick={searchEnrichment} disabled={enrichLoading}>
        {enrichLoading ? $t("study.common.loading") : $t("study.read.enrich_search_btn")}
      </button>
      <p class="muted small">{$t("study.read.enrich_search_hint")}</p>
    {:else}
      <ul class="enrich-list">
        {#each enrichCandidates as c (c.ol_key)}
          <li
            class="enrich-row"
            class:selected={enrichSelected?.ol_key === c.ol_key}
          >
            <button
              type="button"
              class="enrich-pick"
              onclick={() => (enrichSelected = c)}
            >
              {#if c.cover_url}
                <img src={c.cover_url} alt="" class="enrich-cover" />
              {:else}
                <div class="enrich-cover placeholder"></div>
              {/if}
              <div class="enrich-info">
                <div class="enrich-title">{c.title}</div>
                {#if c.author}<div class="enrich-author small muted">{c.author}</div>{/if}
                <div class="enrich-meta small muted">
                  {#if c.year}{c.year} · {/if}
                  {#if c.publisher}{c.publisher}{/if}
                </div>
              </div>
            </button>
          </li>
        {/each}
      </ul>
      {#if enrichSelected}
        <div class="enrich-apply">
          <p class="muted small">{$t("study.read.enrich_apply_hint")}</p>
          <label><input type="checkbox" bind:checked={enrichApply.title} disabled={!enrichSelected.title} /> {$t("study.read.enrich_field_title")} <span class="mono">{enrichSelected.title}</span></label>
          <label><input type="checkbox" bind:checked={enrichApply.author} disabled={!enrichSelected.author} /> {$t("study.read.enrich_field_author")} <span class="mono">{enrichSelected.author ?? "—"}</span></label>
          <label><input type="checkbox" bind:checked={enrichApply.publisher} disabled={!enrichSelected.publisher} /> {$t("study.read.enrich_field_publisher")} <span class="mono">{enrichSelected.publisher ?? "—"}</span></label>
          <label><input type="checkbox" bind:checked={enrichApply.cover} disabled={!enrichSelected.cover_url} /> {$t("study.read.enrich_field_cover")}</label>
          <button type="button" class="cta" onclick={applyEnrichment} disabled={enrichApplying}>
            {enrichApplying ? $t("study.common.loading") : $t("study.read.enrich_apply_btn")}
          </button>
        </div>
      {/if}
    {/if}
    {#if enrichError}
      <p class="error small">{enrichError}</p>
    {/if}
  </div>
{/if}

{#if collectionsOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="drawer-overlay" onclick={() => (collectionsOpen = false)}></div>
  <div class="drawer" role="dialog" tabindex="-1" aria-label={$t("study.read.collections_title")}>
    <header class="drawer-head">
      <h2>{$t("study.read.collections_title")}</h2>
      <button type="button" class="icon-btn" onclick={() => (collectionsOpen = false)} aria-label="close">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M18 6L6 18M6 6l12 12"></path>
        </svg>
      </button>
    </header>
    <p class="drawer-hint">{$t("study.read.collections_hint")}</p>
    <ul class="roots-list">
      {#each collections as c (c.id)}
        <li class="root-row">
          <div class="root-info">
            {#if renamingId === c.id}
              <input
                type="text"
                class="search inline"
                bind:value={renameInput}
                onkeydown={(e) => {
                  if (e.key === "Enter") commitRename(c);
                  if (e.key === "Escape") (renamingId = null);
                }}
              />
            {:else}
              <span class="root-short">{c.name}</span>
            {/if}
          </div>
          {#if renamingId === c.id}
            <button type="button" class="icon-btn" onclick={() => commitRename(c)} aria-label="save">
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
                <path d="M20 6L9 17l-5-5"></path>
              </svg>
            </button>
          {:else}
            <button
              type="button"
              class="icon-btn"
              onclick={() => { renamingId = c.id; renameInput = c.name; }}
              aria-label="rename"
              title={$t("study.read.collection_rename")}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
                <path d="M11 4H4v16h16v-7"></path>
                <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
              </svg>
            </button>
            <button
              type="button"
              class="icon-btn danger"
              onclick={() => deleteCollection(c)}
              aria-label="delete"
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
                <path d="M3 6h18"></path>
                <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
                <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"></path>
              </svg>
            </button>
          {/if}
        </li>
      {/each}
      {#if collections.length === 0}
        <li class="muted small">{$t("study.read.collections_empty")}</li>
      {/if}
    </ul>
    <button type="button" class="cta outline" onclick={createCollection}>
      + {$t("study.read.collection_create")}
    </button>
  </div>
{/if}

{#if rootsOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="drawer-overlay" onclick={() => (rootsOpen = false)}></div>
  <div class="drawer" role="dialog" tabindex="-1" aria-label={$t("study.read.roots_title")}>
    <header class="drawer-head">
      <h2>{$t("study.read.roots_title")}</h2>
      <button type="button" class="icon-btn" onclick={() => (rootsOpen = false)} aria-label="close">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M18 6L6 18M6 6l12 12"></path>
        </svg>
      </button>
    </header>
    <p class="drawer-hint">{$t("study.read.roots_hint")}</p>
    <ul class="roots-list">
      {#each roots as r (r.path)}
        <li class="root-row" class:disabled={!r.enabled}>
          <label class="root-toggle">
            <input type="checkbox" checked={r.enabled} onchange={() => toggleRoot(r)} />
          </label>
          <div class="root-info">
            <span class="root-short" title={r.path}>{shortPath(r.path)}</span>
            <span class="root-full muted small" title={r.path}>{r.path}</span>
          </div>
          <button
            type="button"
            class="icon-btn danger"
            onclick={() => removeRoot(r)}
            aria-label="remove"
            title={$t("study.library.roots_remove_tooltip")}
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
              <path d="M3 6h18"></path>
              <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
              <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"></path>
            </svg>
          </button>
        </li>
      {/each}
      {#if roots.length === 0}
        <li class="muted small">{$t("study.read.roots_empty")}</li>
      {/if}
    </ul>
    <button type="button" class="cta outline" onclick={addRoot}>
      + {$t("study.read.roots_add_btn")}
    </button>
    <button
      type="button"
      class="cta"
      onclick={async () => {
        rootsOpen = false;
        await rescan();
      }}
      disabled={scanning || roots.filter((r) => r.enabled).length === 0}
    >
      {scanning ? $t("study.read.scanning") : $t("study.read.roots_rescan")}
    </button>
  </div>
{/if}

<style>
  .read-page {
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
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .row-primary h1 {
    flex: 0 0 auto;
    min-width: fit-content;
    margin-right: auto;
  }
  .row-formats {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .empty-hint {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 32px;
    text-align: center;
    color: var(--secondary);
  }
  .empty-hint h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 500;
  }
  .empty-hint .cta {
    margin-top: 8px;
  }
  .cta {
    display: inline-block;
    padding: 10px 20px;
    background: var(--cta, var(--accent));
    color: var(--on-cta, white);
    border: 1px solid transparent;
    border-radius: var(--border-radius);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
  }
  .cta.outline {
    background: transparent;
    color: var(--secondary);
    border-color: var(--input-border);
  }
  .cta:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .drawer-overlay {
    position: fixed;
    inset: 0;
    background: color-mix(in oklab, var(--bg) 60%, transparent);
    backdrop-filter: blur(2px);
    z-index: 40;
  }
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(420px, 90vw);
    background: var(--bg-elevated, var(--bg));
    border-left: none;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    z-index: 41;
    overflow-y: auto;
  }
  .drawer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .drawer-head h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 500;
    color: var(--secondary);
  }
  .drawer-hint {
    margin: 0;
    font-size: 12px;
    color: var(--tertiary);
  }
  .roots-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .root-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: none;
    border-radius: var(--border-radius);
    transition: opacity 120ms ease;
  }
  .root-row.disabled {
    opacity: 0.55;
  }
  .root-toggle {
    display: inline-flex;
    align-items: center;
  }
  .root-toggle input {
    accent-color: var(--accent);
  }
  .root-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .root-short {
    font-size: 13px;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .root-full {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    color: var(--tertiary);
    border: 1px solid transparent;
    border-radius: 6px;
    cursor: pointer;
  }
  .icon-btn:hover {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .icon-btn.danger:hover {
    color: var(--error);
  }

  .tag-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 6px;
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--secondary);
    border-radius: 999px;
    font-size: 10px;
    line-height: 1.4;
  }
  .tag-chip.more {
    background: transparent;
    color: var(--tertiary);
    border: none;
  }
  .tag-chip.removable {
    padding-right: 4px;
  }
  .tag-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    padding: 0;
    background: transparent;
    color: inherit;
    border: 0;
    border-radius: 50%;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }
  .tag-x:hover {
    background: color-mix(in oklab, var(--accent) 30%, transparent);
  }

  .card-actions {
    display: flex;
    gap: 4px;
    position: absolute;
    top: 8px;
    left: 8px;
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .book-card:hover .card-actions,
  .book-card:focus-within .card-actions {
    opacity: 1;
  }
  .card-action-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    background: color-mix(in oklab, var(--primary) 75%, transparent);
    color: var(--secondary);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    font-size: 12px;
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
  }
  .card-action-btn:hover {
    color: var(--accent);
    border-color: var(--accent);
  }
  .card-action-btn.active {
    color: var(--accent);
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 18%, var(--primary));
  }
  .card-action-btn.danger:hover {
    color: var(--error);
    border-color: var(--error);
  }

  .enrich-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 60vh;
    overflow-y: auto;
  }
  .enrich-row {
    border: none;
    border-radius: var(--border-radius);
    background: var(--button-elevated);
  }
  .enrich-row.selected {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 10%, var(--button-elevated));
  }
  .enrich-pick {
    width: 100%;
    display: flex;
    gap: 10px;
    padding: 8px;
    background: transparent;
    color: var(--secondary);
    border: 0;
    text-align: left;
    cursor: pointer;
    font-family: inherit;
  }
  .enrich-cover {
    width: 48px;
    height: 72px;
    object-fit: cover;
    border-radius: 4px;
    background: var(--input-bg);
    flex-shrink: 0;
  }
  .enrich-cover.placeholder {
    background: var(--input-bg);
  }
  .enrich-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .enrich-title {
    font-weight: 500;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .enrich-apply {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    margin-top: 12px;
  }
  .enrich-apply label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--tertiary);
  }
  .enrich-apply .mono {
    font-family: var(--font-mono, monospace);
    color: var(--secondary);
  }
  .drawer-narrow {
    width: min(380px, 90vw);
  }
  .search.inline {
    flex: 1;
    height: 24px;
    font-size: 12px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 500;
    letter-spacing: -0.5px;
    color: var(--secondary);
  }
  .subtitle {
    margin: 0;
    font-size: 14px;
    color: var(--tertiary);
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
    text-decoration: none;
    transition: border-color 150ms ease;
  }
  .ghost-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .ghost-btn:disabled {
    opacity: 0.55;
    cursor: default;
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
  .list-sentinel {
    display: flex;
    justify-content: center;
    padding: 24px 0;
    min-height: 24px;
  }
  .muted {
    color: var(--tertiary);
    font-size: 14px;
  }
  .small {
    font-size: 12px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
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
    color: inherit;
    cursor: pointer;
    font-family: inherit;
    transition: transform var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
    text-align: left;
    padding: 0;
    position: relative;
    transition:
      border-color 150ms ease,
      transform 150ms ease;
  }
  .card:hover {
    border-color: var(--accent);
    transform: translateY(-1px);
  }
  .book-thumb {
    position: relative;
    aspect-ratio: 3 / 4;
    background: color-mix(in oklab, var(--accent) 4%, var(--input-bg));
    color: var(--tertiary);
    display: flex;
    align-items: center;
    justify-content: center;
    background-size: cover;
    background-position: center;
  }
  .book-card[data-ext="pdf"] .book-thumb {
    color: var(--red);
  }
  .book-card[data-ext="epub"] .book-thumb {
    color: var(--blue);
  }
  .book-card[data-ext="djvu"] .book-thumb {
    color: var(--orange);
  }
  .book-card[data-ext="cbz"] .book-thumb,
  .book-card[data-ext="cbr"] .book-thumb {
    color: var(--green);
  }
  .ext-stamp {
    position: absolute;
    bottom: 1rem;
    left: 50%;
    transform: translateX(-50%);
    padding: 0.15rem 0.55rem;
    background: currentColor;
    color: var(--text);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.05em;
    border-radius: 3px;
  }
  .star {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 50%;
    background: color-mix(in oklab, var(--primary) 60%, transparent);
    color: var(--text);
    cursor: pointer;
    padding: 0;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
  }
  .star.active {
    color: var(--warning);
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
  .author {
    font-size: 12px;
    color: var(--tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    color: var(--tertiary);
    font-size: 11px;
  }
  .status {
    color: var(--accent);
    font-weight: 500;
  }
</style>
