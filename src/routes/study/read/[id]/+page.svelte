<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { rpcSetSource, rpcClearSource } from "$lib/rpc";
  import { awardXp } from "$lib/study-gamification";
  import { t } from "$lib/i18n";
  import SegmentedControl from "$lib/study-components/SegmentedControl.svelte";
  import { createReadingSession, type ReadingSession } from "$lib/reading-session";
  import {
    applyFocusMode,
    getCursorLine,
    getPaperFilter,
    getReadingTheme,
    setCursorLine,
    setPaperFilter,
    pushReadingTheme,
    popReadingTheme,
  } from "$lib/reader-theme";
  import ReaderThemeMenu from "$lib/reader-components/ReaderThemeMenu.svelte";
  import ReaderZoomIndicator from "$lib/reader-components/ReaderZoomIndicator.svelte";
  import { wheelZoomDirection } from "$lib/reader-zoom";
  import "$lib/reader-theme.css";
  import EpubReader from "./EpubReader.svelte";
  import type {
    EpubAnchorJson,
    EpubChapterHighlight,
    EpubSelectionEvent,
  } from "$lib/study-epub-anchor";
  import TxtReader from "./TxtReader.svelte";
  import HtmlReader from "./HtmlReader.svelte";
  import CbzReader from "./CbzReader.svelte";

  type Book = {
    id: number;
    file_path: string;
    format: string;
    title: string | null;
    author: string | null;
    publisher: string | null;
    language: string | null;
    page_count: number | null;
    reading_pct: number;
    last_location: string | null;
  };

  type OutlineNode = {
    title: string;
    page: number | null;
    children: OutlineNode[];
  };

  type OpenResult = {
    book_id: number;
    format: string;
    page_count: number;
    title: string | null;
    author: string | null;
    outline: OutlineNode[];
  };

  type RenderResult = {
    page: number;
    width: number;
    height: number;
    png_b64: string;
  };

  type Bookmark = {
    id: number;
    page_hint: number | null;
    text: string | null;
    note: string | null;
    created_at: number;
  };

  type SearchHit = {
    page: number;
    offset: number;
    snippet: string;
  };

  type EpubSearchHit = {
    chapter_index: number;
    chapter_id: string | null;
    chapter_href: string | null;
    char_offset_start: number;
    char_offset_end: number;
    match: string;
    snippet: string;
  };

  type PdfChar = { c: string; x: number; y: number; w: number; h: number };
  type TextRects = {
    page: number;
    page_width_pt: number;
    page_height_pt: number;
    chars: PdfChar[];
  };

  type HighlightRectPt = { x: number; y: number; w: number; h: number };
  type HighlightAnchor = {
    page: number;
    rects_pt: HighlightRectPt[];
    page_width_pt: number;
    page_height_pt: number;
  };

  type Highlight = {
    id: number;
    kind: string;
    anchor_json: string;
    text: string | null;
    note: string | null;
    color: string | null;
    drawer: string | null;
    ink_json: string | null;
    chapter: string | null;
    page_hint: number | null;
  };

  let book = $state<Book | null>(null);
  let meta = $state<OpenResult | null>(null);
  let pageImg = $state<string>("");
  let pageSize = $state<{ w: number; h: number }>({ w: 0, h: 0 });
  let currentPage = $state(1);
  let zoomDpi = $state(120);
  let loadingBook = $state(true);
  let loadingPage = $state(false);
  let errorMsg = $state("");
  let sidebarOpen = $state(true);
  let sidebarTab = $state<
    "outline" | "bookmarks" | "highlights" | "notes" | "search"
  >("outline");
  let gotoInput = $state("");

  let bookmarks = $state<Bookmark[]>([]);
  let session: ReadingSession | null = null;

  type ViewMode = "paged" | "scroll";
  function getViewMode(): ViewMode {
    if (typeof localStorage === "undefined") return "paged";
    return localStorage.getItem("study.read.view_mode") === "scroll" ? "scroll" : "paged";
  }
  let viewMode = $state<ViewMode>("paged");
  let scrollImgs = $state<Record<number, string>>({});
  let scrollRenderedSet = new Set<number>();
  let scrollObserver: IntersectionObserver | null = null;
  $effect(() => {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem("study.read.view_mode", viewMode);
    }
  });
  let readerTheme = $state<string>("app");
  let paperFilter = $state(false);
  let cursorLine = $state(false);
  let focusMode = $state(false);
  let themeMenuOpen = $state(false);
  let cursorY = $state(-1);

  $effect(() => { setPaperFilter(paperFilter); });
  $effect(() => { setCursorLine(cursorLine); });
  $effect(() => { applyFocusMode(focusMode); });

  $effect(() => {
    void textRects;
    void pageImg;
    void zoomDpi;
    if (!textLayerEl) return;
    const el = textLayerEl;
    requestAnimationFrame(() => {
      const spans = el.querySelectorAll<HTMLSpanElement>(".char-span");
      if (spans.length === 0) return;
      const N = spans.length;
      const targets = new Float32Array(N);
      const naturals = new Float32Array(N);
      for (let i = 0; i < N; i++) {
        const s = spans[i];
        s.style.transform = "";
        targets[i] = parseFloat(s.dataset.tw ?? "0");
      }
      for (let i = 0; i < N; i++) {
        naturals[i] = spans[i].getBoundingClientRect().width;
      }
      for (let i = 0; i < N; i++) {
        const t = targets[i];
        const n = naturals[i];
        if (t > 0 && n > 0) {
          spans[i].style.transform = `scaleX(${t / n})`;
        }
      }
    });
  });
  let searchQuery = $state("");
  let searchHits = $state<SearchHit[]>([]);
  let epubSearchHits = $state<EpubSearchHit[]>([]);
  let searchLoading = $state(false);
  let searchCaseSensitive = $state(false);

  let metadataOpen = $state(false);
  let metadataDraft = $state<{
    title: string;
    author: string;
    publisher: string;
    language: string;
  }>({ title: "", author: "", publisher: "", language: "" });
  let savingMetadata = $state(false);
  let metadataError = $state("");
  let searchTruncated = $state(false);

  let textRects = $state<TextRects | null>(null);
  let highlights = $state<Highlight[]>([]);
  let pageFrameEl: HTMLDivElement | null = $state(null);
  let textLayerEl: HTMLDivElement | null = $state(null);
  let pageFrameWidth = $state(0);
  let pageFrameHeight = $state(0);
  const MAX_CHAR_LAYER = 3000;
  const textLayerEnabled = $derived.by(() => {
    if (!textRects) return false;
    return textRects.chars.length <= MAX_CHAR_LAYER;
  });

  $effect(() => {
    void textRects;
    void pageImg;
    void zoomDpi;
    if (!pageFrameEl) {
      pageFrameWidth = 0;
      pageFrameHeight = 0;
      return;
    }
    const el = pageFrameEl;
    requestAnimationFrame(() => {
      const rect = el.getBoundingClientRect();
      pageFrameWidth = rect.width;
      pageFrameHeight = rect.height;
    });
  });

  $effect(() => {
    if (!pageFrameEl) return;
    const el = pageFrameEl;
    const observer = new ResizeObserver(() => {
      const rect = el.getBoundingClientRect();
      pageFrameWidth = rect.width;
      pageFrameHeight = rect.height;
    });
    observer.observe(el);
    return () => observer.disconnect();
  });
  let pageImgEl: HTMLImageElement | null = $state(null);
  let popupPos = $state<{ x: number; y: number } | null>(null);
  let pendingSelection = $state<{ text: string; rects_pt: HighlightRectPt[] } | null>(null);

  type EditableHighlight = {
    id: number;
    color: string | null;
    drawer: string | null;
    note: string | null;
    text: string | null;
  };
  let editPopupPos = $state<{ x: number; y: number } | null>(null);
  let editingHighlight = $state<EditableHighlight | null>(null);
  let editingNote = $state("");
  let sendingToNotes = $state(false);
  let creatingFlashcard = $state(false);

  let inkMode = $state(false);
  let inkColor = $state("#1f2937");
  let inkStrokePt = $state(2);
  let inkEraser = $state(false);
  let drawing = $state(false);
  let currentPath = $state<{ x: number; y: number }[]>([]);
  const INK_PALETTE = [
    { key: "black", css: "#1f2937" },
    { key: "red", css: "#dc2626" },
    { key: "blue", css: "#2563eb" },
    { key: "green", css: "#16a34a" },
    { key: "yellow", css: "#eab308" },
  ];
  const INK_STROKE_MIN = 1;
  const INK_STROKE_MAX = 8;
  const INK_COLOR_KEY = "study.read.ink_color";
  const INK_STROKE_KEY = "study.read.ink_stroke";

  function loadInkPreferences() {
    if (typeof localStorage === "undefined") return;
    const c = localStorage.getItem(INK_COLOR_KEY);
    if (c && INK_PALETTE.some((entry) => entry.css === c)) {
      inkColor = c;
    }
    const s = localStorage.getItem(INK_STROKE_KEY);
    const parsed = s ? Number(s) : NaN;
    if (Number.isFinite(parsed) && parsed >= INK_STROKE_MIN && parsed <= INK_STROKE_MAX) {
      inkStrokePt = parsed;
    }
  }

  function persistInkColor(value: string) {
    inkColor = value;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(INK_COLOR_KEY, value);
    }
  }

  function persistInkStroke(value: number) {
    const bounded = Math.min(INK_STROKE_MAX, Math.max(INK_STROKE_MIN, value));
    inkStrokePt = bounded;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(INK_STROKE_KEY, String(bounded));
    }
  }

  const COLOR_PALETTE = [
    { key: "yellow", css: "rgba(250, 204, 21, 0.42)" },
    { key: "green", css: "rgba(134, 239, 172, 0.45)" },
    { key: "pink", css: "rgba(249, 168, 212, 0.45)" },
    { key: "blue", css: "rgba(147, 197, 253, 0.45)" },
    { key: "orange", css: "rgba(253, 186, 116, 0.45)" },
    { key: "purple", css: "rgba(196, 181, 253, 0.45)" },
    { key: "cyan", css: "rgba(165, 243, 252, 0.45)" },
    { key: "red", css: "rgba(252, 165, 165, 0.45)" },
  ];

  const DRAWERS = [
    { key: "lighten", label: "marca-texto" },
    { key: "underscore", label: "sublinhar" },
    { key: "strikeout", label: "tachar" },
    { key: "invert", label: "invert" },
  ];

  const sidebarTabOptions = $derived([
    { value: "outline", label: $t("study.read.panel_outline") },
    { value: "bookmarks", label: $t("study.read.panel_bookmarks") },
    { value: "highlights", label: $t("study.read.panel_highlights") },
    { value: "notes", label: "Notas" },
    { value: "search", label: $t("study.read.panel_search") },
  ]);

  const annotationsWithNotes = $derived(
    highlights.filter((h) => {
      if (h.kind === "note") return true;
      const trimmed = (h.note ?? "").trim();
      return trimmed.length > 0;
    }),
  );

  const pageHighlights = $derived(
    highlights.filter((h) => {
      if (h.kind !== "highlight") return false;
      try {
        const a = JSON.parse(h.anchor_json) as HighlightAnchor;
        return a.page === currentPage;
      } catch {
        return false;
      }
    }),
  );

  const pageInks = $derived(
    highlights.filter((h) => {
      if (h.kind !== "ink") return false;
      try {
        const a = JSON.parse(h.anchor_json);
        return a?.page === currentPage;
      } catch {
        return false;
      }
    }),
  );

  function colorCss(key: string | null | undefined): string {
    const entry = COLOR_PALETTE.find((c) => c.key === key) ?? COLOR_PALETTE[0];
    return entry.css;
  }

  function ptToScreen(
    px: number,
    py: number,
    pw: number,
    ph: number,
    pageW: number,
    pageH: number,
    imgW: number,
    imgH: number,
  ): { left: number; top: number; width: number; height: number } {
    const scaleX = imgW / pageW;
    const scaleY = imgH / pageH;
    return {
      left: px * scaleX,
      top: (pageH - py - ph) * scaleY,
      width: pw * scaleX,
      height: ph * scaleY,
    };
  }

  const bookId = $derived(Number($page.params.id));
  const isEpub = $derived(book?.format === "epub");
  const isTxt = $derived(book?.format === "txt");
  const isHtml = $derived(book?.format === "html");
  const isCbz = $derived(book?.format === "cbz");
  const customReader = $derived(isEpub || isTxt || isHtml || isCbz);
  const unsupported = $derived(
    customReader
      ? false
      : meta
        ? meta.format !== "pdf"
        : book
          ? book.format !== "pdf"
          : false,
  );

  function maybeAwardBookFinished() {
    if (!book || !meta) return;
    const total = meta.page_count;
    if (!total || total < 1) return;
    if (currentPage < total) return;
    if (typeof localStorage === "undefined") return;
    const key = `study.read.book_finished.${bookId}`;
    if (localStorage.getItem(key)) return;
    localStorage.setItem(key, String(Date.now()));
    void awardXp("book_finished", 100, {
      book_id: bookId,
      title: book.title ?? null,
    });
    void pluginInvoke("study", "study:gamification:counter:bump", {
      key: "books_read",
      delta: 1,
    });
  }

  $effect(() => {
    if (currentPage && meta) maybeAwardBookFinished();
  });

  async function loadBook() {
    if (!Number.isFinite(bookId) || bookId <= 0) {
      errorMsg = $t("study.read.no_book");
      loadingBook = false;
      return;
    }
    try {
      book = await pluginInvoke<Book>("study", "study:read:books:get", {
        bookId,
      });
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
      loadingBook = false;
      return;
    }

    if (
      book!.format === "epub" ||
      book!.format === "txt" ||
      book!.format === "html" ||
      book!.format === "cbz"
    ) {
      loadingBook = false;
      return;
    }

    if (book!.format !== "pdf") {
      loadingBook = false;
      return;
    }

    try {
      meta = await pluginInvoke<OpenResult>("study", "study:read:pdf:open", {
        bookId,
      });
      const saved = parseLastLocation(book!.last_location);
      currentPage = Math.max(
        1,
        Math.min(meta!.page_count, saved ?? 1),
      );
      await Promise.all([renderCurrent(), loadBookmarks(), loadHighlights()]);
      loadTextRects();
      session = createReadingSession({
        bookId,
        getLocation: () => String(currentPage),
      });
      await session.start();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      loadingBook = false;
    }
  }

  const SCROLL_KEEP_RADIUS = 6;

  function pruneScrollCache(around: number) {
    if (!meta) return;
    const keep = new Set<number>();
    for (
      let p = Math.max(1, around - SCROLL_KEEP_RADIUS);
      p <= Math.min(meta.page_count, around + SCROLL_KEEP_RADIUS);
      p++
    ) {
      keep.add(p);
    }
    const next: Record<number, string> = {};
    for (const k of Object.keys(scrollImgs)) {
      const n = Number(k);
      if (keep.has(n)) next[n] = scrollImgs[n];
    }
    scrollImgs = next;
    scrollRenderedSet = new Set(Object.keys(next).map(Number));
  }

  async function renderScrollPage(pageNum: number) {
    if (scrollRenderedSet.has(pageNum)) return;
    scrollRenderedSet.add(pageNum);
    try {
      const res = await pluginInvoke<RenderResult>(
        "study",
        "study:read:pdf:render_page",
        { bookId, page: pageNum, dpi: zoomDpi },
      );
      scrollImgs = { ...scrollImgs, [pageNum]: `data:image/png;base64,${res.png_b64}` };
      pruneScrollCache(currentPage);
    } catch (e) {
      scrollRenderedSet.delete(pageNum);
      console.error("scroll render failed for page", pageNum, e);
    }
  }

  let lastZoomForScroll = 0;
  $effect(() => {
    if (viewMode !== "scroll") {
      lastZoomForScroll = 0;
      return;
    }
    if (lastZoomForScroll === 0) {
      lastZoomForScroll = zoomDpi;
      return;
    }
    if (lastZoomForScroll !== zoomDpi) {
      lastZoomForScroll = zoomDpi;
      scrollImgs = {};
      scrollRenderedSet = new Set();
      void renderScrollPage(currentPage);
    }
  });

  function setupScrollObserver() {
    teardownScrollObserver();
    scrollObserver = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const el = entry.target as HTMLElement;
          const pg = Number(el.dataset.page);
          if (!Number.isFinite(pg)) continue;
          if (entry.isIntersecting) {
            void renderScrollPage(pg);
            if (entry.intersectionRatio > 0.5 && currentPage !== pg) {
              currentPage = pg;
              void persistLocation();
              session?.notePageChange();
            }
          }
        }
      },
      { rootMargin: "200px 0px", threshold: [0, 0.5] },
    );
    queueMicrotask(() => {
      document.querySelectorAll<HTMLElement>(".scroll-page").forEach((el) => {
        scrollObserver?.observe(el);
      });
    });
  }

  function teardownScrollObserver() {
    scrollObserver?.disconnect();
    scrollObserver = null;
  }

  $effect(() => {
    if (viewMode === "scroll" && meta) {
      setupScrollObserver();
      queueMicrotask(() => {
        void renderScrollPage(currentPage);
        const el = document.querySelector<HTMLElement>(
          `.scroll-page[data-page="${currentPage}"]`,
        );
        el?.scrollIntoView({ block: "start", behavior: "instant" as ScrollBehavior });
      });
    } else {
      teardownScrollObserver();
    }
    return () => teardownScrollObserver();
  });

  async function preloadAroundCurrent() {
    if (!meta) return;
    const around = currentPage;
    const tasks: Promise<void>[] = [];
    for (let offset = 0; offset <= 2; offset++) {
      for (const dir of [1, -1]) {
        const p = around + offset * dir;
        if (p >= 1 && p <= meta.page_count && !scrollRenderedSet.has(p)) {
          tasks.push(renderScrollPage(p));
        }
        if (offset === 0) break;
      }
    }
    await Promise.allSettled(tasks);
  }

  async function toggleViewMode() {
    const next: ViewMode = viewMode === "paged" ? "scroll" : "paged";
    viewMode = next;
    if (next === "scroll") {
      scrollImgs = {};
      scrollRenderedSet = new Set();
      await renderScrollPage(currentPage);
      queueMicrotask(() => {
        const el = document.querySelector<HTMLElement>(
          `.scroll-page[data-page="${currentPage}"]`,
        );
        el?.scrollIntoView({ block: "start", behavior: "instant" as ScrollBehavior });
      });
      void preloadAroundCurrent();
    } else {
      await renderCurrent();
    }
  }

  async function loadTextRects() {
    if (!meta) return;
    try {
      textRects = await pluginInvoke<TextRects>(
        "study",
        "study:read:pdf:extract_text_rects",
        { bookId, page: currentPage },
      );
    } catch (e) {
      console.error("text rects failed", e);
      textRects = null;
    }
  }

  async function loadHighlights() {
    try {
      const arr = await pluginInvoke<Highlight[]>("study", "study:read:annot:list", {
        bookId,
      });
      highlights = Array.isArray(arr) ? arr.filter((a) => a.kind === "highlight" || a.kind === "ink") : [];
    } catch (e) {
      console.error("highlights load failed", e);
      highlights = [];
    }
  }

  function openMetadataEditor() {
    if (!book) return;
    metadataDraft = {
      title: book.title ?? "",
      author: book.author ?? "",
      publisher: book.publisher ?? "",
      language: book.language ?? "",
    };
    metadataError = "";
    metadataOpen = true;
  }

  async function saveMetadata() {
    if (!book) return;
    savingMetadata = true;
    metadataError = "";
    try {
      const args: Record<string, unknown> = { bookId: book.id };
      const t = metadataDraft.title.trim();
      const a = metadataDraft.author.trim();
      const p = metadataDraft.publisher.trim();
      const l = metadataDraft.language.trim();
      args.title = t === "" ? null : t;
      args.author = a === "" ? null : a;
      args.publisher = p === "" ? null : p;
      args.language = l === "" ? null : l;
      await pluginInvoke("study", "study:read:books:update_metadata", args);
      book = {
        ...book,
        title: t === "" ? null : t,
        author: a === "" ? null : a,
        publisher: p === "" ? null : p,
        language: l === "" ? null : l,
      };
      metadataOpen = false;
    } catch (e) {
      metadataError = e instanceof Error ? e.message : String(e);
    } finally {
      savingMetadata = false;
    }
  }

  async function createOrphanNote() {
    if (!textRects) return;
    const note = window.prompt("Nota da página:", "");
    if (note === null) return;
    const trimmed = note.trim();
    if (trimmed === "") return;
    try {
      await pluginInvoke("study", "study:read:annot:create", {
        bookId,
        kind: "note",
        anchor: {
          page: currentPage,
          page_width_pt: textRects.page_width_pt,
          page_height_pt: textRects.page_height_pt,
        },
        note: trimmed,
        pageHint: currentPage,
      });
      await loadHighlights();
    } catch (err) {
      console.error("orphan note create failed", err);
    }
  }

  function onNoteShortcut() {
    if (pendingSelection && popupPos) {
      saveHighlight("yellow").then(async () => {
        await loadHighlights();
        const fresh = highlights
          .filter((h) => h.kind === "highlight")
          .sort((a, b) => b.id - a.id)[0];
        if (!fresh || !pageFrameEl) return;
        const frame = pageFrameEl.getBoundingClientRect();
        editingHighlight = {
          id: fresh.id,
          color: fresh.color ?? null,
          drawer: fresh.drawer ?? null,
          note: fresh.note ?? null,
          text: fresh.text ?? null,
        };
        editingNote = fresh.note ?? "";
        editPopupPos = { x: 16, y: 16 };
        queueMicrotask(() => {
          const ta = document.querySelector<HTMLTextAreaElement>(".ep-note");
          ta?.focus();
        });
        void frame;
      });
      return;
    }
    void createOrphanNote();
  }

  function ptFromScreen(e: PointerEvent): { x: number; y: number } | null {
    if (!textRects || !pageFrameEl) return null;
    const frame = pageFrameEl.getBoundingClientRect();
    const sx = e.clientX - frame.left;
    const sy = e.clientY - frame.top;
    const scaleX = frame.width / textRects.page_width_pt;
    const scaleY = frame.height / textRects.page_height_pt;
    return { x: sx / scaleX, y: sy / scaleY };
  }

  function onInkDown(e: PointerEvent) {
    if (!inkMode) return;
    const p = ptFromScreen(e);
    if (!p) return;
    e.preventDefault();
    if (inkEraser) {
      eraseInkAt(p);
      return;
    }
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drawing = true;
    currentPath = [p];
  }

  async function eraseInkAt(p: { x: number; y: number }) {
    const tolerance = Math.max(6, inkStrokePt * 3);
    const tol2 = tolerance * tolerance;
    const candidate = pageInks.find((h) => {
      try {
        const data = JSON.parse(h.ink_json ?? "null") as
          | { paths?: { x: number; y: number }[][] }
          | null;
        if (!data || !Array.isArray(data.paths)) return false;
        for (const path of data.paths) {
          for (const pt of path) {
            const dx = pt.x - p.x;
            const dy = pt.y - p.y;
            if (dx * dx + dy * dy <= tol2) return true;
          }
        }
      } catch {
        return false;
      }
      return false;
    });
    if (!candidate) return;
    try {
      await pluginInvoke("study", "study:read:annot:delete", {
        annotId: candidate.id,
      });
      highlights = highlights.filter((x) => x.id !== candidate.id);
    } catch (err) {
      console.error("erase failed", err);
    }
  }

  function onInkMove(e: PointerEvent) {
    if (!drawing) return;
    const p = ptFromScreen(e);
    if (!p) return;
    currentPath = [...currentPath, p];
  }

  async function onInkUp(e: PointerEvent) {
    if (!drawing) return;
    drawing = false;
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
    const pts = currentPath;
    currentPath = [];
    if (pts.length < 2 || !textRects) return;
    const anchor = {
      page: currentPage,
      page_width_pt: textRects.page_width_pt,
      page_height_pt: textRects.page_height_pt,
    };
    const ink = { paths: [pts], color: inkColor, stroke_pt: inkStrokePt };
    try {
      await pluginInvoke("study", "study:read:annot:create", {
        bookId,
        kind: "ink",
        anchor,
        inkJson: JSON.stringify(ink),
        pageHint: currentPage,
      });
      await loadHighlights();
    } catch (err) {
      console.error(err);
    }
  }

  async function undoLastInk() {
    const mine = pageInks
      .slice()
      .sort((a, b) => b.id - a.id);
    if (mine.length === 0) return;
    const last = mine[0];
    try {
      await pluginInvoke("study", "study:read:annot:delete", { annotId: last.id });
      highlights = highlights.filter((x) => x.id !== last.id);
    } catch (e) {
      console.error(e);
    }
  }

  function buildPathD(pts: { x: number; y: number }[]): string {
    if (pts.length === 0) return "";
    let d = `M ${pts[0].x.toFixed(2)} ${pts[0].y.toFixed(2)}`;
    for (let i = 1; i < pts.length; i++) {
      d += ` L ${pts[i].x.toFixed(2)} ${pts[i].y.toFixed(2)}`;
    }
    return d;
  }

  async function saveHighlight(colorKey: string) {
    if (!pendingSelection || !textRects) return;
    const anchor: HighlightAnchor = {
      page: currentPage,
      rects_pt: pendingSelection.rects_pt,
      page_width_pt: textRects.page_width_pt,
      page_height_pt: textRects.page_height_pt,
    };
    try {
      await pluginInvoke("study", "study:read:annot:create", {
        bookId,
        kind: "highlight",
        anchor,
        text: pendingSelection.text,
        color: colorKey,
        drawer: "lighten",
        pageHint: currentPage,
      });
      await loadHighlights();
      void awardXp("highlight", 3, {
        book_id: bookId,
        page: currentPage,
        color: colorKey,
      });
    } catch (e) {
      console.error(e);
    } finally {
      popupPos = null;
      pendingSelection = null;
      window.getSelection()?.removeAllRanges();
    }
  }

  async function deleteHighlight(h: Highlight) {
    try {
      await pluginInvoke("study", "study:read:annot:delete", { annotId: h.id });
      highlights = highlights.filter((x) => x.id !== h.id);
    } catch (e) {
      console.error(e);
    }
  }

  function openHighlightEditor(h: Highlight, e: MouseEvent) {
    e.stopPropagation();
    e.preventDefault();
    if (!pageFrameEl) return;
    const frameRect = pageFrameEl.getBoundingClientRect();
    editingHighlight = {
      id: h.id,
      color: h.color ?? null,
      drawer: h.drawer ?? null,
      note: h.note ?? null,
      text: h.text ?? null,
    };
    editingNote = h.note ?? "";
    editPopupPos = {
      x: e.clientX - frameRect.left + 8,
      y: e.clientY - frameRect.top + 8,
    };
  }

  function closeHighlightEditor() {
    editPopupPos = null;
    editingHighlight = null;
    editingNote = "";
  }

  async function updateHighlightField(
    field: "color" | "drawer",
    value: string,
  ) {
    if (!editingHighlight) return;
    try {
      const args: Record<string, unknown> = {
        annotId: editingHighlight.id,
      };
      args[field] = value;
      await pluginInvoke("study", "study:read:annot:update", args);
      editingHighlight = { ...editingHighlight, [field]: value };
      highlights = highlights.map((x) =>
        x.id === editingHighlight!.id ? { ...x, [field]: value } : x,
      );
    } catch (e) {
      console.error(e);
    }
  }

  async function saveNote() {
    if (!editingHighlight) return;
    try {
      await pluginInvoke("study", "study:read:annot:update", {
        annotId: editingHighlight.id,
        note: editingNote,
      });
      editingHighlight = { ...editingHighlight, note: editingNote };
      highlights = highlights.map((x) =>
        x.id === editingHighlight!.id ? { ...x, note: editingNote } : x,
      );
    } catch (e) {
      console.error(e);
    }
  }

  async function sendHighlightToNotes() {
    if (!editingHighlight) return;
    sendingToNotes = true;
    try {
      const r = await pluginInvoke<{ page_id: number; page_name: string }>(
        "study",
        "study:read:annot:send_to_notes",
        { annotId: editingHighlight.id },
      );
      const url = `/study/notes?page=${encodeURIComponent(r.page_name)}`;
      window.open(url, "_blank");
      closeHighlightEditor();
    } catch (e) {
      console.error(e);
    } finally {
      sendingToNotes = false;
    }
  }

  let flashcardToast = $state("");

  async function createFlashcardFromHighlight() {
    if (!editingHighlight || !book) return;
    const text = (editingHighlight.text ?? "").trim();
    if (!text) {
      flashcardToast = "Highlight sem texto";
      setTimeout(() => (flashcardToast = ""), 2400);
      return;
    }
    creatingFlashcard = true;
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      type NotetypeSummary = { id: number; name: string; kind: string };
      const types = await pluginInvoke<NotetypeSummary[]>(
        "study",
        "study:anki:notetypes:list",
      );
      let basic = (types || []).find(
        (t) => t.kind === "normal" && /basic/i.test(t.name),
      );
      if (!basic) {
        basic = (types || []).find((t) => t.kind === "normal");
      }
      if (!basic) {
        const r = await pluginInvoke<{ id: number }>(
          "study",
          "study:anki:notetypes:create_stock",
          { kind: "basic", name: "Basic" },
        );
        basic = { id: r.id, name: "Basic", kind: "normal" };
      }
      const note = (editingHighlight.note ?? "").trim();
      const front = text.length > 500 ? text.slice(0, 497) + "…" : text;
      const back =
        note.length > 0
          ? `${note}\n\nDe: ${book.title ?? "livro"}`
          : `De: ${book.title ?? "livro"}`;
      await pluginInvoke("study", "study:anki:notes:create", {
        notetypeId: basic.id,
        deckId: 1,
        fields: [front, back],
        tags: [
          "from-book",
          `book:${book.id}`,
          ...(editingHighlight.color
            ? [`color:${editingHighlight.color}`]
            : []),
        ],
      });
      void awardXp("anki_card_from_highlight", 3, {
        book_id: book.id,
        annot_id: editingHighlight.id,
      });
      flashcardToast = "Flashcard criado no Anki";
      setTimeout(() => (flashcardToast = ""), 2800);
      closeHighlightEditor();
    } catch (e) {
      flashcardToast = `Falhou: ${e instanceof Error ? e.message : String(e)}`;
      setTimeout(() => (flashcardToast = ""), 4000);
    } finally {
      creatingFlashcard = false;
    }
  }

  async function deleteFromEditor() {
    if (!editingHighlight) return;
    const id = editingHighlight.id;
    closeHighlightEditor();
    try {
      await pluginInvoke("study", "study:read:annot:delete", { annotId: id });
      highlights = highlights.filter((x) => x.id !== id);
    } catch (e) {
      console.error(e);
    }
  }

  function onEditorKey(e: KeyboardEvent) {
    if (e.key === "Escape") closeHighlightEditor();
  }

  function onTextSelectionEnd() {
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0 || sel.isCollapsed) {
      popupPos = null;
      pendingSelection = null;
      return;
    }
    if (!textRects || !pageFrameEl) return;
    const range = sel.getRangeAt(0);
    const commonAncestor = range.commonAncestorContainer;
    if (
      commonAncestor.nodeType === Node.ELEMENT_NODE &&
      !(commonAncestor as Element).closest(".text-layer")
    ) {
      popupPos = null;
      return;
    }
    if (
      commonAncestor.nodeType !== Node.ELEMENT_NODE &&
      !(commonAncestor.parentElement?.closest(".text-layer"))
    ) {
      popupPos = null;
      return;
    }
    const text = sel.toString().trim();
    if (!text) return;

    const frameRect = pageFrameEl.getBoundingClientRect();
    const imgW = frameRect.width;
    const imgH = frameRect.height;
    const pageW = textRects.page_width_pt;
    const pageH = textRects.page_height_pt;
    const scaleX = imgW / pageW;
    const scaleY = imgH / pageH;

    const clientRects = Array.from(range.getClientRects());
    if (clientRects.length === 0) return;
    const rects_pt: HighlightRectPt[] = clientRects
      .map((r) => {
        const screenX = r.left - frameRect.left;
        const screenY = r.top - frameRect.top;
        return {
          x: screenX / scaleX,
          y: pageH - (screenY + r.height) / scaleY,
          w: r.width / scaleX,
          h: r.height / scaleY,
        };
      })
      .filter((r) => r.w > 0.1 && r.h > 0.1);

    if (rects_pt.length === 0) {
      popupPos = null;
      return;
    }

    pendingSelection = { text, rects_pt };
    const last = clientRects[clientRects.length - 1];
    popupPos = {
      x: last.right - frameRect.left + 8,
      y: last.bottom - frameRect.top - 6,
    };
  }

  function cancelHighlight() {
    popupPos = null;
    pendingSelection = null;
    window.getSelection()?.removeAllRanges();
  }

  async function loadBookmarks() {
    try {
      const list = await pluginInvoke<Bookmark[]>("study", "study:read:annot:list", {
        bookId,
        kind: "bookmark",
      });
      bookmarks = Array.isArray(list) ? list : [];
    } catch (e) {
      console.error("load bookmarks failed", e);
      bookmarks = [];
    }
  }

  async function toggleBookmark() {
    const existing = bookmarks.find((b) => b.page_hint === currentPage);
    if (existing) {
      try {
        await pluginInvoke("study", "study:read:annot:delete", {
          annotId: existing.id,
        });
        bookmarks = bookmarks.filter((b) => b.id !== existing.id);
      } catch (e) {
        console.error(e);
      }
      return;
    }
    try {
      await pluginInvoke<{ id: number }>("study", "study:read:annot:create", {
        bookId,
        kind: "bookmark",
        anchor: { page: currentPage },
        pageHint: currentPage,
      });
      await loadBookmarks();
    } catch (e) {
      console.error(e);
    }
  }

  async function runSearch() {
    const q = searchQuery.trim();
    if (!q) {
      searchHits = [];
      epubSearchHits = [];
      searchTruncated = false;
      return;
    }
    searchLoading = true;
    try {
      if (isEpub) {
        const res = await pluginInvoke<{ hits: EpubSearchHit[]; truncated: boolean }>(
          "study",
          "study:read:epub:search_text",
          {
            bookId,
            query: q,
            caseSensitive: searchCaseSensitive,
            maxHits: 200,
          },
        );
        epubSearchHits = Array.isArray(res?.hits) ? res.hits : [];
        searchHits = [];
        searchTruncated = !!res?.truncated;
      } else {
        const res = await pluginInvoke<{ hits: SearchHit[]; truncated: boolean }>(
          "study",
          "study:read:pdf:search_text",
          { bookId, query: q, maxHits: 200 },
        );
        searchHits = Array.isArray(res?.hits) ? res.hits : [];
        epubSearchHits = [];
        searchTruncated = !!res?.truncated;
      }
    } catch (e) {
      console.error("search failed", e);
      searchHits = [];
      epubSearchHits = [];
    } finally {
      searchLoading = false;
    }
  }

  function handleSearchKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      runSearch();
    }
  }

  const currentIsBookmarked = $derived(
    bookmarks.some((b) => b.page_hint === currentPage),
  );

  function parseLastLocation(raw: string | null): number | null {
    if (!raw) return null;
    try {
      const parsed = JSON.parse(raw);
      if (typeof parsed === "number") return parsed;
      if (parsed && typeof parsed.page === "number") return parsed.page;
    } catch {
      const asNum = Number(raw);
      if (Number.isFinite(asNum)) return asNum;
    }
    return null;
  }

  async function renderCurrent() {
    if (!meta) return;
    loadingPage = true;
    popupPos = null;
    pendingSelection = null;
    textRects = null;
    try {
      const res = await pluginInvoke<RenderResult>(
        "study",
        "study:read:pdf:render_page",
        { bookId, page: currentPage, dpi: zoomDpi },
      );
      pageImg = `data:image/png;base64,${res.png_b64}`;
      pageSize = { w: res.width, h: res.height };
      await persistLocation();
      session?.notePageChange();
      loadTextRects();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      loadingPage = false;
    }
  }

  async function persistLocation() {
    if (!meta) return;
    const pct = (currentPage / meta.page_count) * 100;
    try {
      await pluginInvoke("study", "study:read:books:update_location", {
        bookId,
        location: JSON.stringify({ page: currentPage }),
        readingPct: pct,
      });
    } catch (_) {
      // ignore
    }
  }

  function scrollToPage(pageNum: number, behavior: ScrollBehavior = "smooth") {
    queueMicrotask(() => {
      const el = document.querySelector<HTMLElement>(
        `.scroll-page[data-page="${pageNum}"]`,
      );
      el?.scrollIntoView({ block: "start", behavior });
    });
  }

  function goPrev() {
    if (!meta) return;
    if (currentPage > 1) {
      currentPage -= 1;
      if (viewMode === "scroll") {
        void renderScrollPage(currentPage);
        scrollToPage(currentPage);
      } else {
        renderCurrent();
      }
    }
  }

  function goNext() {
    if (!meta) return;
    if (currentPage < meta.page_count) {
      currentPage += 1;
      if (viewMode === "scroll") {
        void renderScrollPage(currentPage);
        scrollToPage(currentPage);
      } else {
        renderCurrent();
      }
    }
  }

  function jumpTo(page: number) {
    if (!meta) return;
    const clamped = Math.max(1, Math.min(meta.page_count, page));
    if (clamped !== currentPage) {
      currentPage = clamped;
      if (viewMode === "scroll") {
        void renderScrollPage(currentPage);
        scrollToPage(currentPage, "instant" as ScrollBehavior);
      } else {
        renderCurrent();
      }
    }
  }

  function handleGoto(e: KeyboardEvent) {
    if (e.key === "Enter") {
      const n = Number(gotoInput);
      if (Number.isFinite(n)) jumpTo(n);
      gotoInput = "";
    }
  }

  const PDF_ZOOM_KEY = "study.read.pdf_zoom_dpi";
  const PDF_ZOOM_MIN = 48;
  const PDF_ZOOM_MAX = 300;
  const PDF_ZOOM_BASE = 120;
  let zoomLabel = $state("");
  let zoomLabelTimer: ReturnType<typeof setTimeout> | null = null;
  let wheelZoomTarget: number | null = null;
  let wheelZoomTimer: ReturnType<typeof setTimeout> | null = null;

  function loadZoomDpi() {
    if (typeof localStorage === "undefined") return;
    const raw = Number(localStorage.getItem(PDF_ZOOM_KEY));
    if (Number.isFinite(raw) && raw >= PDF_ZOOM_MIN && raw <= PDF_ZOOM_MAX) {
      zoomDpi = raw;
    }
  }

  $effect(() => {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(PDF_ZOOM_KEY, String(zoomDpi));
    }
  });

  function flashZoomLabel(text: string) {
    zoomLabel = text;
    if (zoomLabelTimer) clearTimeout(zoomLabelTimer);
    zoomLabelTimer = setTimeout(() => (zoomLabel = ""), 1200);
  }

  function onWheelZoom(e: WheelEvent) {
    if (!meta) return;
    const dir = wheelZoomDirection(e);
    if (dir === 0) return;
    e.preventDefault();
    const base = wheelZoomTarget ?? zoomDpi;
    const next = Math.max(PDF_ZOOM_MIN, Math.min(PDF_ZOOM_MAX, base + dir * 15));
    wheelZoomTarget = next;
    flashZoomLabel(`${Math.round((next / PDF_ZOOM_BASE) * 100)}%`);
    if (wheelZoomTimer) clearTimeout(wheelZoomTimer);
    wheelZoomTimer = setTimeout(() => {
      const target = wheelZoomTarget;
      wheelZoomTarget = null;
      if (target === null || target === zoomDpi) return;
      zoomDpi = target;
      if (viewMode === "paged") void renderCurrent();
    }, 160);
  }

  function zoomIn() {
    zoomDpi = Math.min(PDF_ZOOM_MAX, zoomDpi + 30);
    renderCurrent();
  }

  function zoomOut() {
    zoomDpi = Math.max(PDF_ZOOM_MIN, zoomDpi - 30);
    renderCurrent();
  }

  function fitWidth() {
    zoomDpi = 150;
    renderCurrent();
  }

  async function openExternal() {
    if (!book) return;
    try {
      await invoke("open_path_default", { path: book.file_path });
    } catch (e) {
      console.error(e);
    }
  }

  let exportToast = $state("");

  async function exportAs(format: "md" | "json" | "md_v2" | "pdf_burn_in") {
    if (!book) return;
    if (format === "pdf_burn_in" && book.format !== "pdf") {
      exportToast = $t("study.read.export_pdf_only");
      setTimeout(() => (exportToast = ""), 4000);
      return;
    }
    const dialog = await import("@tauri-apps/plugin-dialog");
    const baseName = (book.title || book.file_path.split(/[\\/]/).pop() || "book").replace(
      /[^\w\- .()]+/g,
      "_",
    );
    const ext =
      format === "json" ? "json" : format === "pdf_burn_in" ? "pdf" : "md";
    const suffix =
      format === "md_v2"
        ? " — anotações.v2"
        : format === "pdf_burn_in"
          ? " — anotado"
          : " — anotações";
    const filterName =
      format === "json"
        ? "JSON"
        : format === "pdf_burn_in"
          ? "PDF"
          : "Markdown";
    const targetPath = await dialog.save({
      defaultPath: `${baseName}${suffix}.${ext}`,
      filters: [{ name: filterName, extensions: [ext] }],
    });
    if (typeof targetPath !== "string" || !targetPath) return;
    const cmd =
      format === "md"
        ? "study:read:export:markdown"
        : format === "md_v2"
          ? "study:read:export:markdown_v2"
          : format === "pdf_burn_in"
            ? "study:read:export:pdf_burn_in"
            : "study:read:export:json";
    try {
      const r = await pluginInvoke<{
        path: string;
        highlights?: number;
        bookmarks?: number;
        notes?: number;
        inks?: number;
      }>("study", cmd, { bookId, targetPath });
      if (format === "pdf_burn_in" && r) {
        exportToast = $t("study.read.export_pdf_ok", {
          highlights: r.highlights ?? 0,
          bookmarks: r.bookmarks ?? 0,
          notes: r.notes ?? 0,
          inks: r.inks ?? 0,
        });
      } else {
        exportToast = $t("study.read.export_ok", { path: targetPath });
      }
      setTimeout(() => (exportToast = ""), 5000);
    } catch (e) {
      exportToast = $t("study.read.export_failed", {
        err: e instanceof Error ? e.message : String(e),
      });
      setTimeout(() => (exportToast = ""), 6000);
    }
  }

  async function importAnnotationsJson() {
    if (!book) return;
    const dialog = await import("@tauri-apps/plugin-dialog");
    const sourcePath = await dialog.open({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (typeof sourcePath !== "string" || !sourcePath) return;
    try {
      const r = await pluginInvoke<{
        imported: number;
        skipped: number;
        errors: number;
      }>("study", "study:read:import:json", { bookId, sourcePath });
      exportToast = $t("study.read.import_ok", {
        imported: r.imported,
        skipped: r.skipped,
        errors: r.errors,
      });
      setTimeout(() => (exportToast = ""), 6000);
      await loadHighlights();
    } catch (e) {
      exportToast = $t("study.read.import_failed", {
        err: e instanceof Error ? e.message : String(e),
      });
      setTimeout(() => (exportToast = ""), 6000);
    }
  }

  function back() {
    if (history.length > 1) history.back();
    else goto("/study/read");
  }

  function onKey(e: KeyboardEvent) {
    const tag = (e.target as HTMLElement)?.tagName?.toLowerCase();
    if (tag === "input" || tag === "textarea") return;
    if (e.key === "ArrowLeft" || e.key === "PageUp") {
      e.preventDefault();
      goPrev();
    } else if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === " ") {
      e.preventDefault();
      goNext();
    } else if (e.key === "Home") {
      e.preventDefault();
      jumpTo(1);
    } else if (e.key === "End" && meta) {
      e.preventDefault();
      jumpTo(meta.page_count);
    } else if (e.key === "+" || e.key === "=") {
      e.preventDefault();
      zoomIn();
    } else if (e.key === "-") {
      e.preventDefault();
      zoomOut();
    } else if (e.key === "b" || e.key === "B") {
      e.preventDefault();
      toggleBookmark();
    } else if (e.key === "n" || e.key === "N") {
      e.preventDefault();
      onNoteShortcut();
    } else if (e.key === "/") {
      e.preventDefault();
      sidebarTab = "search";
      sidebarOpen = true;
      queueMicrotask(() => {
        const el = document.querySelector<HTMLInputElement>(".search-input");
        el?.focus();
      });
    } else if (e.key === "f" || e.key === "F") {
      e.preventDefault();
      focusMode = !focusMode;
    } else if (e.key === "Escape" && focusMode) {
      e.preventDefault();
      focusMode = false;
    }
  }

  function onMouseMove(e: MouseEvent) {
    if (!cursorLine) return;
    cursorY = e.clientY;
  }

  onMount(() => {
    readerTheme = getReadingTheme();
    paperFilter = getPaperFilter();
    cursorLine = getCursorLine();
    viewMode = getViewMode();
    loadInkPreferences();
    loadZoomDpi();
    pushReadingTheme();
    loadBook();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("wheel", onWheelZoom, { passive: false });
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("wheel", onWheelZoom);
    };
  });

  onDestroy(() => {
    window.removeEventListener("keydown", onKey);
    window.removeEventListener("mousemove", onMouseMove);
    window.removeEventListener("wheel", onWheelZoom);
    applyFocusMode(false);
    popReadingTheme();
    void session?.stop(false);
    void rpcClearSource("reading");
  });

  $effect(() => {
    if (!book || !meta) return;
    const total = meta.page_count;
    const author = book.author?.trim();
    const stateText = author
      ? `${author} · pág. ${currentPage}/${total}`
      : `pág. ${currentPage}/${total}`;
    void rpcSetSource({
      source: "reading",
      details: book.title ?? "Lendo",
      state: stateText,
      duration: 0,
      position: 0,
      paused: false,
    });
  });

  let pendingEpubSelection = $state<EpubSelectionEvent | null>(null);
  let epubPopupPos = $state<{ x: number; y: number } | null>(null);
  let currentEpubChapter = $state<{ id: string; href: string; idx: number } | null>(null);

  const epubChapterHighlights = $derived.by<EpubChapterHighlight[]>(() => {
    const ch = currentEpubChapter;
    if (!ch) return [];
    const out: EpubChapterHighlight[] = [];
    for (const h of highlights) {
      if (h.kind !== "highlight") continue;
      if (!h.chapter || h.chapter !== ch.id) continue;
      try {
        const a = JSON.parse(h.anchor_json ?? "{}") as Partial<EpubAnchorJson>;
        if (typeof a.char_offset_start !== "number" || typeof a.char_offset_end !== "number") continue;
        out.push({
          id: h.id,
          color: h.color ?? null,
          char_offset_start: a.char_offset_start,
          char_offset_end: a.char_offset_end,
        });
      } catch {
        // skip malformed anchor
      }
    }
    return out;
  });

  function handleEpubSelection(ev: EpubSelectionEvent) {
    pendingEpubSelection = ev;
    const iframe = document.querySelector<HTMLIFrameElement>(".chapter-frame");
    if (iframe) {
      const ir = iframe.getBoundingClientRect();
      epubPopupPos = {
        x: ir.left + ev.rectInIframe.x,
        y: ir.top + ev.rectInIframe.y + ev.rectInIframe.height + 6,
      };
    } else {
      epubPopupPos = { x: 80, y: 80 };
    }
  }

  function handleEpubChapterChange(info: {
    chapterIdx: number;
    chapterId: string;
    chapterHref: string;
  }) {
    currentEpubChapter = {
      id: info.chapterId,
      href: info.chapterHref,
      idx: info.chapterIdx,
    };
    cancelEpubHighlight();
  }

  function cancelEpubHighlight() {
    pendingEpubSelection = null;
    epubPopupPos = null;
  }

  async function saveEpubHighlight(colorKey: string) {
    if (!pendingEpubSelection) return;
    const sel = pendingEpubSelection;
    const anchor: EpubAnchorJson = {
      version: 1,
      chapter_id: sel.chapterId,
      chapter_href: sel.chapterHref,
      chapter_index: sel.chapterIndex,
      char_offset_start: sel.charOffsetStart,
      char_offset_end: sel.charOffsetEnd,
      selected_text: sel.selectedText,
      context_before: sel.contextBefore,
      context_after: sel.contextAfter,
    };
    try {
      await pluginInvoke("study", "study:read:annot:create", {
        bookId,
        kind: "highlight",
        anchor,
        text: sel.selectedText,
        color: colorKey,
        chapter: sel.chapterId,
      });
      await loadHighlights();
      void awardXp("highlight", 3, {
        book_id: bookId,
        color: colorKey,
        format: "epub",
      });
    } catch (e) {
      console.error("epub highlight save failed", e);
    } finally {
      pendingEpubSelection = null;
      epubPopupPos = null;
      const iframe = document.querySelector<HTMLIFrameElement>(".chapter-frame");
      iframe?.contentWindow?.getSelection()?.removeAllRanges();
    }
  }
</script>

{#if book && isEpub}
  <EpubReader
    {book}
    onBack={back}
    chapterHighlights={epubChapterHighlights}
    onSelection={handleEpubSelection}
    onChapterChange={handleEpubChapterChange}
  />
  {#if epubPopupPos && pendingEpubSelection}
    <div
      class="hl-popup epub-popup"
      role="toolbar"
      tabindex="-1"
      aria-label="highlight"
      style:left="{epubPopupPos.x}px" style:top="{epubPopupPos.y}px"
      onmousedown={(e) => e.stopPropagation()}
    >
      {#each COLOR_PALETTE as c (c.key)}
        <button
          type="button"
          class="hl-color"
          style:background="{c.css}"
          title={c.key}
          onclick={() => saveEpubHighlight(c.key)}
        ></button>
      {/each}
      <button type="button" class="hl-cancel" onclick={cancelEpubHighlight}>×</button>
    </div>
  {/if}
{:else if book && isTxt}
  <TxtReader {book} onBack={back} />
{:else if book && isHtml}
  <HtmlReader {book} onBack={back} />
{:else if book && isCbz}
  <CbzReader {book} onBack={back} />
{:else}
<section
  class="reader-page"
  data-read-paper={paperFilter ? "1" : "0"}
  data-read-focus={focusMode ? "1" : "0"}
>
  <header class="head">
    <button type="button" class="back-btn" onclick={back}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M15 18l-6-6 6-6"></path>
      </svg>
      <span>{$t("study.read.back_to_library")}</span>
    </button>
    {#if book}
      <button
        type="button"
        class="title-btn"
        title="Editar metadados"
        onclick={openMetadataEditor}
      >
        <h1 class="title">{book.title ?? book.file_path.split(/[\\/]/).pop()}</h1>
        {#if book.author}
          <span class="title-author">— {book.author}</span>
        {/if}
      </button>
    {/if}
    {#if meta}
      <div class="toolbar">
        <button type="button" class="tool-btn" onclick={goPrev} disabled={currentPage <= 1} title={$t("study.read.prev_page")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6"></path></svg>
        </button>
        <span class="page-label mono">{$t("study.read.page_of", { current: currentPage, total: meta.page_count })}</span>
        <button type="button" class="tool-btn" onclick={goNext} disabled={currentPage >= meta.page_count} title={$t("study.read.next_page")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6"></path></svg>
        </button>
        <input
          type="text"
          class="goto-input mono"
          placeholder="#"
          bind:value={gotoInput}
          onkeydown={handleGoto}
          title={$t("study.read.go_to")}
        />
        <span class="sep"></span>
        <button type="button" class="tool-btn" onclick={zoomOut} title={$t("study.read.zoom_out")}>−</button>
        <button type="button" class="tool-btn tool-label" onclick={fitWidth}>{zoomDpi}</button>
        <button type="button" class="tool-btn" onclick={zoomIn} title={$t("study.read.zoom_in")}>+</button>
        <span class="sep"></span>
        <button
          type="button"
          class="tool-btn"
          class:active={currentIsBookmarked}
          onclick={toggleBookmark}
          title={currentIsBookmarked ? $t("study.read.remove_bookmark") : $t("study.read.add_bookmark")}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill={currentIsBookmarked ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"></path>
          </svg>
        </button>
        <button
          type="button"
          class="tool-btn"
          class:active={inkMode}
          onclick={() => (inkMode = !inkMode)}
          title={$t("study.read.ink_toggle")}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M12 19l7-7 3 3-7 7-3-3z"></path>
            <path d="M18 13l-1.5-7.5L2 2l3.5 14.5L13 18"></path>
            <path d="M2 2l7.586 7.586"></path>
            <circle cx="11" cy="11" r="2"></circle>
          </svg>
        </button>
        {#if inkMode}
          <div class="ink-palette" aria-label="ink color">
            {#each INK_PALETTE as c (c.key)}
              <button
                type="button"
                class="ink-swatch"
                class:selected={inkColor === c.css && !inkEraser}
                style:background="{c.css}"
                onclick={() => {
                  inkEraser = false;
                  persistInkColor(c.css);
                }}
                title={c.key}
              ></button>
            {/each}
            <label class="ink-stroke" title="Espessura">
              <input
                type="range"
                min={INK_STROKE_MIN}
                max={INK_STROKE_MAX}
                step="1"
                value={inkStrokePt}
                onchange={(e) =>
                  persistInkStroke(Number((e.target as HTMLInputElement).value))}
                oninput={(e) =>
                  persistInkStroke(Number((e.target as HTMLInputElement).value))}
              />
              <span class="ink-stroke-value mono">{inkStrokePt}</span>
            </label>
            <button
              type="button"
              class="tool-btn"
              class:active={inkEraser}
              onclick={() => (inkEraser = !inkEraser)}
              title="Borracha"
              aria-pressed={inkEraser}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M16 3l5 5L9 20H4v-5L16 3z"></path>
                <path d="M14 5l5 5"></path>
              </svg>
            </button>
            <button type="button" class="tool-btn" onclick={undoLastInk} title={$t("study.read.ink_undo")}>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 7v6h6"></path>
                <path d="M21 17a9 9 0 0 0-15-6.7L3 13"></path>
              </svg>
            </button>
          </div>
        {/if}
        <div class="theme-wrap">
          <button
            type="button"
            class="tool-btn"
            class:active={themeMenuOpen}
            onclick={() => (themeMenuOpen = !themeMenuOpen)}
            title={$t("study.read.reading_style")}
            aria-label={$t("study.read.reading_style")}
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <circle cx="12" cy="12" r="9"></circle>
              <path d="M12 3a9 9 0 0 0 0 18"></path>
            </svg>
          </button>
          {#if themeMenuOpen}
            <ReaderThemeMenu
              bind:theme={readerTheme}
              bind:paper={paperFilter}
              bind:cursor={cursorLine}
              onClose={() => (themeMenuOpen = false)}
            />
          {/if}
        </div>
        <button
          type="button"
          class="tool-btn"
          class:active={focusMode}
          onclick={() => (focusMode = !focusMode)}
          title={$t("study.read.focus_mode")}
          aria-label={$t("study.read.focus_mode")}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M4 8V4h4"></path>
            <path d="M20 8V4h-4"></path>
            <path d="M4 16v4h4"></path>
            <path d="M20 16v4h-4"></path>
          </svg>
        </button>
        <button
          type="button"
          class="tool-btn"
          class:active={viewMode === "scroll"}
          onclick={toggleViewMode}
          title={viewMode === "scroll" ? $t("study.read.view_paged") : $t("study.read.view_scroll")}
          aria-label={viewMode === "scroll" ? $t("study.read.view_paged") : $t("study.read.view_scroll")}
        >
          {#if viewMode === "scroll"}
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <rect x="4" y="5" width="16" height="14" rx="1"></rect>
              <path d="M9 5v14"></path>
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M8 6h12"></path>
              <path d="M8 12h12"></path>
              <path d="M8 18h12"></path>
              <path d="M3 6h2"></path>
              <path d="M3 12h2"></path>
              <path d="M3 18h2"></path>
            </svg>
          {/if}
        </button>
        <button type="button" class="tool-btn" onclick={openExternal} title={$t("study.read.open_external")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"></path>
            <path d="M15 3h6v6"></path>
            <path d="M10 14L21 3"></path>
          </svg>
        </button>
        <details class="tool-menu">
          <summary class="tool-btn" title={$t("study.read.menu")}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
              <circle cx="5" cy="12" r="1"></circle>
              <circle cx="12" cy="12" r="1"></circle>
              <circle cx="19" cy="12" r="1"></circle>
            </svg>
          </summary>
          <ul class="menu-list" role="menu">
            <li><button type="button" onclick={() => exportAs("md")}>{$t("study.read.export_md")}</button></li>
            <li><button type="button" onclick={() => exportAs("md_v2")}>{$t("study.read.export_md_v2")}</button></li>
            <li><button type="button" onclick={() => exportAs("json")}>{$t("study.read.export_json")}</button></li>
            {#if book?.format === "pdf"}
              <li><button type="button" onclick={() => exportAs("pdf_burn_in")}>{$t("study.read.export_pdf_burn_in")}</button></li>
            {/if}
            <li><button type="button" onclick={importAnnotationsJson}>{$t("study.read.import_json")}</button></li>
          </ul>
        </details>
      </div>
    {/if}
  </header>
  {#if exportToast}
    <div class="export-toast">{exportToast}</div>
  {/if}
  <ReaderZoomIndicator label={zoomLabel} />

  {#if loadingBook}
    <p class="state">{$t("study.read.loading_book")}</p>
  {:else if errorMsg}
    <div class="state">
      <p class="error">{errorMsg}</p>
      {#if errorMsg.toLowerCase().includes("pdfium")}
        <p class="muted small">{$t("study.read.pdfium_missing")}</p>
      {/if}
      {#if book}
        <button type="button" class="btn" onclick={openExternal}>{$t("study.read.open_external")}</button>
      {/if}
    </div>
  {:else if unsupported}
    <div class="state">
      <p class="muted">{$t("study.read.unsupported_format", { format: (book?.format ?? "").toUpperCase() })}</p>
      <button type="button" class="btn primary" onclick={openExternal}>{$t("study.read.open_external")}</button>
    </div>
  {:else if meta}
    <div class="layout">
      <aside class="sidebar" class:open={sidebarOpen}>
        <div class="sidebar-head">
          {#if sidebarOpen}
            <SegmentedControl
              bind:value={sidebarTab}
              options={sidebarTabOptions}
              ariaLabel="panel"
            />
          {/if}
          <button type="button" class="icon-btn" onclick={() => (sidebarOpen = !sidebarOpen)}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
              {#if sidebarOpen}
                <path d="M15 6l-6 6 6 6"></path>
              {:else}
                <path d="M9 6l6 6-6 6"></path>
              {/if}
            </svg>
          </button>
        </div>

        {#if sidebarOpen}
          {#if sidebarTab === "outline"}
            {#if meta.outline.length === 0}
              <p class="muted small">{$t("study.read.outline_empty")}</p>
            {:else}
              <ul class="outline-tree">
                {#each meta.outline as node, i (i)}
                  {@render outlineNode(node, 0)}
                {/each}
              </ul>
            {/if}
          {:else if sidebarTab === "bookmarks"}
            {#if bookmarks.length === 0}
              <p class="muted small">{$t("study.read.bookmarks_empty")}</p>
            {:else}
              <ul class="outline-tree">
                {#each bookmarks as bm (bm.id)}
                  <li class="outline-item">
                    <button
                      type="button"
                      class="outline-link"
                      class:active={bm.page_hint === currentPage}
                      onclick={() => bm.page_hint && jumpTo(bm.page_hint)}
                    >
                      <span class="outline-title">
                        <svg viewBox="0 0 24 24" width="11" height="11" fill="var(--accent)" aria-hidden="true" style="margin-right: 4px; vertical-align: -1px;">
                          <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/>
                        </svg>
                        {bm.text ?? ($t("study.read.page_of", { current: bm.page_hint ?? 0, total: meta.page_count }))}
                      </span>
                      <span class="outline-page mono">{bm.page_hint ?? "?"}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if sidebarTab === "highlights"}
            {#if highlights.length === 0}
              <p class="muted small">{$t("study.read.highlights_empty")}</p>
            {:else}
              <ul class="outline-tree">
                {#each highlights as h (h.id)}
                  {@const a = (() => { try { return JSON.parse(h.anchor_json); } catch { return null; } })()}
                  <li class="outline-item">
                    <button
                      type="button"
                      class="outline-link"
                      class:active={a && a.page === currentPage}
                      onclick={() => a && jumpTo(a.page)}
                    >
                      <span class="hl-dot" style:background="{colorCss(h.color)}"></span>
                      <span class="outline-title hl-text-clamp">{h.text ?? "—"}</span>
                      <span class="outline-page mono">{a?.page ?? "?"}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if sidebarTab === "notes"}
            {#if annotationsWithNotes.length === 0}
              <p class="muted small">
                Nenhuma nota ainda. Selecione um trecho e tecle N pra anexar
                uma nota, ou tecle N sem seleção pra criar uma nota solta.
              </p>
            {:else}
              <ul class="outline-tree">
                {#each annotationsWithNotes as h (h.id)}
                  {@const a = (() => { try { return JSON.parse(h.anchor_json); } catch { return null; } })()}
                  <li class="outline-item">
                    <button
                      type="button"
                      class="outline-link note-link"
                      class:active={a && a.page === currentPage}
                      onclick={() => a && jumpTo(a.page)}
                    >
                      {#if h.kind === "note"}
                        <span class="hl-dot" style="background: var(--accent);"></span>
                      {:else}
                        <span class="hl-dot" style:background="{colorCss(h.color)}"></span>
                      {/if}
                      <span class="note-body">
                        {#if h.text}
                          <span class="note-quote">"{h.text}"</span>
                        {/if}
                        {#if h.note}
                          <span class="note-text">{h.note}</span>
                        {/if}
                      </span>
                      <span class="outline-page mono">{a?.page ?? "?"}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if sidebarTab === "search"}
            <input
              type="search"
              class="search-input"
              placeholder={$t("study.read.search_input")}
              bind:value={searchQuery}
              onkeydown={handleSearchKey}
            />
            {#if isEpub}
              <label class="search-flag">
                <input type="checkbox" bind:checked={searchCaseSensitive} />
                <span>case-sensitive</span>
              </label>
            {/if}
            {#if searchLoading}
              <p class="muted small">{$t("study.common.loading")}</p>
            {:else if searchQuery.trim() === ""}
              <p class="muted small">{$t("study.read.search_input")}</p>
            {:else if isEpub}
              {#if epubSearchHits.length === 0}
                <p class="muted small">{$t("study.read.search_no_hits")}</p>
              {:else}
                <p class="muted small">
                  {$t("study.read.search_hits_count", { n: epubSearchHits.length })}
                  {#if searchTruncated}+{/if}
                </p>
                <ul class="hits-list">
                  {#each epubSearchHits as hit, i (i)}
                    <li>
                      <button
                        type="button"
                        class="hit-btn"
                        onclick={() => jumpTo(hit.chapter_index + 1)}
                      >
                        <span class="hit-page mono">cap {hit.chapter_index + 1}</span>
                        <span class="hit-snippet">{hit.snippet}</span>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            {:else if searchHits.length === 0}
              <p class="muted small">{$t("study.read.search_no_hits")}</p>
            {:else}
              <p class="muted small">
                {$t("study.read.search_hits_count", { n: searchHits.length })}
                {#if searchTruncated}+{/if}
              </p>
              <ul class="hits-list">
                {#each searchHits as hit, i (i)}
                  <li>
                    <button
                      type="button"
                      class="hit-btn"
                      onclick={() => jumpTo(hit.page)}
                    >
                      <span class="hit-page mono">{hit.page}</span>
                      <span class="hit-snippet">{hit.snippet}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          {/if}
        {/if}
      </aside>

      <div class="canvas-wrap">
        {#if viewMode === "scroll" && meta}
          <div class="scroll-page-pill" aria-live="polite">
            {currentPage} / {meta.page_count}
          </div>
          <div class="scroll-stack">
            {#each Array.from({ length: meta.page_count }, (_, i) => i + 1) as p (p)}
              <div
                class="canvas-frame scroll-page"
                data-page={p}
                style:aspect-ratio="{pageSize.w > 0 ? `${pageSize.w} / ${pageSize.h}` : '612 / 792'}"
              >
                {#if scrollImgs[p]}
                  <img class="page-img" src={scrollImgs[p]} alt="page {p}" loading="lazy" />
                {:else}
                  <div class="scroll-placeholder" style="position: absolute; inset: 0;">
                    {$t("study.read.page_of", { current: p, total: meta.page_count })}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {:else}
        {#if loadingPage && !pageImg}
          <p class="muted">{$t("study.common.loading")}</p>
        {/if}
        {#if pageImg}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            class="canvas-frame"
            class:ink-mode={inkMode}
            role="region"
            aria-label="page"
            bind:this={pageFrameEl}
            style:aspect-ratio="{pageSize.w} / {pageSize.h}"
            onmouseup={onTextSelectionEnd}
            onpointerdown={onInkDown}
            onpointermove={onInkMove}
            onpointerup={onInkUp}
            onpointercancel={onInkUp}
          >
            <img class="page-img" bind:this={pageImgEl} src={pageImg} alt="page {currentPage}" />

            {#if textRects && pageFrameEl && pageFrameWidth > 0}
              {@const fw = pageFrameWidth}
              {@const fh = pageFrameHeight}
              <div class="hl-layer" aria-hidden="true">
                {#each pageHighlights as h (h.id)}
                  {@const a = JSON.parse(h.anchor_json)}
                  {#each a.rects_pt as r, ri (ri)}
                    {@const s = ptToScreen(r.x, r.y, r.w, r.h, a.page_width_pt, a.page_height_pt, fw, fh)}
                    <button
                      type="button"
                      class="hl-rect"
                      data-drawer={h.drawer ?? "lighten"}
                      style:left="{s.left}px" style:top="{s.top}px" style:width="{s.width}px" style:height="{s.height}px" style:--hl-color="{colorCss(h.color)}"
                      onclick={(e) => openHighlightEditor(h, e)}
                      title={h.note ? `nota: ${h.note}` : "editar"}
                    >
                      {#if h.note}
                        <span class="hl-note-pin" aria-hidden="true">●</span>
                      {/if}
                    </button>
                  {/each}
                {/each}
              </div>
              <svg
                class="ink-layer"
                class:pointer-off={!inkMode}
                viewBox="0 0 {textRects.page_width_pt} {textRects.page_height_pt}"
                preserveAspectRatio="none"
                aria-hidden="true"
              >
                {#each pageInks as ink (ink.id)}
                  {@const data = (() => { try { return JSON.parse(ink.ink_json ?? "{}"); } catch { return null; } })()}
                  {#if data}
                    {#each data.paths ?? [] as path, pi (pi)}
                      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                      <path
                        d={buildPathD(path)}
                        stroke={data.color ?? "#1f2937"}
                        stroke-width={data.stroke_pt ?? 2}
                        fill="none"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        class="ink-path"
                        onclick={() => deleteHighlight(ink)}
                      ></path>
                    {/each}
                  {/if}
                {/each}
                {#if drawing && currentPath.length > 0}
                  <path
                    d={buildPathD(currentPath)}
                    stroke={inkColor}
                    stroke-width={inkStrokePt}
                    fill="none"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  ></path>
                {/if}
              </svg>
              {#if textLayerEnabled}
                <div class="text-layer" class:disabled={inkMode} bind:this={textLayerEl}>
                  {#each textRects.chars as c, i (i)}
                    {@const s = ptToScreen(c.x, c.y, c.w, c.h, textRects.page_width_pt, textRects.page_height_pt, fw, fh)}
                    <span
                      class="char-span"
                      data-tw={s.width}
                      style:left="{s.left}px" style:top="{s.top}px" style:height="{s.height}px" style:font-size="{Math.max(6, s.height * 0.95)}px"
                    >{c.c}</span>
                  {/each}
                </div>
              {/if}
            {/if}

            {#if popupPos}
              <div
                class="hl-popup"
                role="toolbar"
                tabindex="-1"
                aria-label="highlight"
                style:left="{popupPos.x}px" style:top="{popupPos.y}px"
                onmousedown={(e) => e.stopPropagation()}
              >
                {#each COLOR_PALETTE as c (c.key)}
                  <button
                    type="button"
                    class="hl-color"
                    style:background="{c.css}"
                    title={c.key}
                    onclick={() => saveHighlight(c.key)}
                  ></button>
                {/each}
                <button type="button" class="hl-cancel" onclick={cancelHighlight}>×</button>
              </div>
            {/if}

            {#if editPopupPos && editingHighlight}
              <div
                class="hl-edit-popup"
                role="dialog"
                tabindex="-1"
                aria-label="Editar highlight"
                style:left="{editPopupPos.x}px" style:top="{editPopupPos.y}px"
                onmousedown={(e) => e.stopPropagation()}
                onkeydown={onEditorKey}
              >
                <header class="ep-head">
                  <span class="ep-title">Highlight</span>
                  <button
                    type="button"
                    class="ep-close"
                    onclick={closeHighlightEditor}
                    aria-label="Fechar"
                  >×</button>
                </header>

                {#if editingHighlight.text}
                  <blockquote class="ep-quote">"{editingHighlight.text}"</blockquote>
                {/if}

                <div class="ep-section">
                  <span class="ep-label">Cor</span>
                  <div class="ep-colors">
                    {#each COLOR_PALETTE as c (c.key)}
                      <button
                        type="button"
                        class="ep-color"
                        class:active={editingHighlight.color === c.key}
                        style:background="{c.css}"
                        title={c.key}
                        onclick={() => updateHighlightField("color", c.key)}
                      ></button>
                    {/each}
                  </div>
                </div>

                <div class="ep-section">
                  <span class="ep-label">Estilo</span>
                  <div class="ep-drawers">
                    {#each DRAWERS as d (d.key)}
                      <button
                        type="button"
                        class="ep-drawer"
                        class:active={(editingHighlight.drawer ?? "lighten") === d.key}
                        onclick={() => updateHighlightField("drawer", d.key)}
                      >
                        {d.label}
                      </button>
                    {/each}
                  </div>
                </div>

                <div class="ep-section">
                  <label class="ep-label" for="ep-note">Nota</label>
                  <textarea
                    id="ep-note"
                    class="ep-note"
                    placeholder="adicionar nota…"
                    bind:value={editingNote}
                    onblur={saveNote}
                  ></textarea>
                </div>

                {#if flashcardToast}
                  <p class="ep-toast">{flashcardToast}</p>
                {/if}
                <footer class="ep-foot">
                  <button
                    type="button"
                    class="ep-btn primary"
                    onclick={sendHighlightToNotes}
                    disabled={sendingToNotes}
                  >
                    {sendingToNotes ? "Enviando…" : "→ Notas"}
                  </button>
                  <button
                    type="button"
                    class="ep-btn ghost"
                    onclick={createFlashcardFromHighlight}
                    disabled={creatingFlashcard}
                  >
                    {creatingFlashcard ? "Criando…" : "→ Flashcard"}
                  </button>
                  <button
                    type="button"
                    class="ep-btn danger"
                    onclick={deleteFromEditor}
                  >
                    Excluir
                  </button>
                </footer>
              </div>
            {/if}

            {#if loadingPage}
              <div class="page-loading-overlay"></div>
            {/if}
          </div>
        {/if}
        {/if}
      </div>
    </div>
  {/if}
  {#if cursorLine && cursorY > 0}
    <div class="reader-cursor-line" style:top="{cursorY}px"></div>
  {/if}
</section>
{/if}

{#if metadataOpen}
  <div
    class="meta-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget && !savingMetadata) metadataOpen = false;
    }}
    onkeydown={(e) => {
      if (e.key === "Escape" && !savingMetadata) metadataOpen = false;
    }}
  >
    <div class="meta-modal" role="dialog" aria-modal="true" aria-labelledby="meta-title">
      <h3 id="meta-title">Editar metadados</h3>
      <p class="meta-hint">
        Atualiza apenas o registro local da biblioteca. O arquivo no disco
        não é modificado.
      </p>

      <label class="meta-field">
        <span>Título</span>
        <input
          type="text"
          bind:value={metadataDraft.title}
          disabled={savingMetadata}
          placeholder="(sem título)"
        />
      </label>

      <label class="meta-field">
        <span>Autor</span>
        <input
          type="text"
          bind:value={metadataDraft.author}
          disabled={savingMetadata}
          placeholder="(desconhecido)"
        />
      </label>

      <label class="meta-field">
        <span>Editora</span>
        <input
          type="text"
          bind:value={metadataDraft.publisher}
          disabled={savingMetadata}
          placeholder="—"
        />
      </label>

      <label class="meta-field">
        <span>Idioma</span>
        <input
          type="text"
          bind:value={metadataDraft.language}
          disabled={savingMetadata}
          placeholder="ex.: pt, en, fr"
        />
      </label>

      {#if metadataError}
        <p class="meta-error">{metadataError}</p>
      {/if}

      <footer class="meta-foot">
        <button
          type="button"
          class="meta-btn ghost"
          onclick={() => (metadataOpen = false)}
          disabled={savingMetadata}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="meta-btn primary"
          onclick={saveMetadata}
          disabled={savingMetadata}
        >
          {savingMetadata ? "Salvando…" : "Salvar"}
        </button>
      </footer>
    </div>
  </div>
{/if}

{#snippet outlineNode(node: OutlineNode, depth: number)}
  <li class="outline-item" style:--depth="{depth}">
    {#if node.page != null}
      <button type="button" class="outline-link" onclick={() => jumpTo(node.page!)}>
        <span class="outline-title">{node.title}</span>
        <span class="outline-page mono">{node.page}</span>
      </button>
    {:else}
      <span class="outline-link no-link">
        <span class="outline-title">{node.title}</span>
      </span>
    {/if}
    {#if node.children.length > 0}
      <ul class="outline-tree">
        {#each node.children as child, i (i)}
          {@render outlineNode(child, depth + 1)}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<style>
  .reader-page {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    width: 100%;
    max-width: 1400px;
    margin-inline: auto;
  }
  .reader-page[data-read-focus="1"] {
    max-width: none;
    padding: 0;
  }
  .head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.75rem;
    padding: 0.5rem 0;
    border-bottom: none;
    position: sticky;
    top: 0;
    z-index: 20;
    background: color-mix(in oklab, var(--primary) 94%, transparent);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }
  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.4rem 0.75rem;
    background: transparent;
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    transition: border-color 150ms ease;
  }
  .back-btn:hover {
    border-color: var(--accent);
  }
  .title {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 180px;
  }
  .toolbar {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .tool-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    height: 28px;
    padding: 0 0.4rem;
    background: transparent;
    border: 0;
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    border-radius: 4px;
    transition: background 120ms ease;
  }
  .tool-btn:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .theme-wrap {
    position: relative;
    display: inline-flex;
  }
  .tool-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .tool-label {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
  }
  .page-label {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    padding: 0 0.5rem;
    color: var(--tertiary);
    font-size: 12px;
  }
  .goto-input {
    width: 48px;
    padding: 4px 6px;
    background: var(--input-bg);
    border: none;
    border-radius: 4px;
    color: var(--secondary);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    font-size: 12px;
    text-align: center;
  }
  .sep {
    width: 1px;
    height: 18px;
    background: var(--content-border);
    margin: 0 0.25rem;
  }
  .tool-menu {
    position: relative;
  }
  .tool-menu summary {
    list-style: none;
    cursor: pointer;
  }
  .tool-menu summary::-webkit-details-marker {
    display: none;
  }
  .tool-menu .menu-list {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    list-style: none;
    margin: 0;
    padding: 0.3rem;
    background: var(--popup-bg, var(--button-elevated));
    border: none;
    border-radius: var(--border-radius);
    min-width: 220px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    z-index: 30;
  }
  .tool-menu .menu-list button {
    width: 100%;
    text-align: left;
    background: transparent;
    border: 0;
    color: var(--secondary);
    padding: 0.5rem 0.75rem;
    border-radius: 6px;
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .tool-menu .menu-list button:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .export-toast {
    margin-top: 0.5rem;
    padding: 0.5rem 0.75rem;
    background: color-mix(in oklab, var(--accent) 12%, var(--button-elevated));
    border: 1px solid color-mix(in oklab, var(--accent) 35%, var(--content-border));
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-size: 13px;
  }
  .mono {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
  }
  .layout {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--padding);
    align-items: start;
  }
  .sidebar {
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    padding: 0.75rem;
    width: 300px;
    max-height: calc(100vh - 10rem);
    overflow-y: auto;
    position: sticky;
    top: var(--padding);
    transition: width 180ms ease;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .sidebar:not(.open) {
    width: 44px;
    padding: 0.75rem 0.25rem;
  }
  .sidebar:not(.open) .outline-tree,
  .sidebar:not(.open) .hits-list,
  .sidebar:not(.open) .muted,
  .sidebar:not(.open) .search-input {
    display: none;
  }
  .sidebar-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
  }
  .search-input {
    padding: 0.4rem 0.6rem;
    background: var(--input-bg);
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    font-family: inherit;
    font-size: 12px;
    width: 100%;
  }
  .hits-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .hit-btn {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    width: 100%;
    padding: 0.4rem 0.5rem;
    background: transparent;
    border: 0;
    border-radius: 4px;
    text-align: left;
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 11px;
    line-height: 1.3;
    transition: background 120ms ease;
  }
  .hit-btn:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .hit-page {
    flex-shrink: 0;
    color: var(--accent);
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    font-size: 10px;
    padding: 2px 6px;
    border: none;
    border-radius: 3px;
    min-width: 28px;
    text-align: center;
  }
  .hit-snippet {
    color: var(--tertiary);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .tool-btn.active {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 15%, transparent);
  }
  .outline-link.active {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    background: transparent;
    border: 0;
    color: var(--tertiary);
    cursor: pointer;
    border-radius: 4px;
  }
  .icon-btn:hover {
    color: var(--secondary);
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .outline-tree {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .outline-item {
    padding-left: calc(var(--depth, 0) * 0.75rem);
  }
  .outline-link {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.3rem 0.5rem;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
    border-radius: 4px;
    transition: background 120ms ease, color 120ms ease;
  }
  .outline-link:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
    color: var(--accent);
  }
  .outline-link.no-link {
    cursor: default;
    color: var(--tertiary);
  }
  .outline-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .outline-page {
    font-size: 10px;
    color: var(--tertiary);
    flex-shrink: 0;
  }
  .canvas-wrap {
    min-width: 0;
    display: flex;
    justify-content: center;
    padding: var(--padding) 0;
  }
  .canvas-frame {
    position: relative;
    width: 100%;
    max-width: 900px;
    background: white;
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
    box-shadow: 0 6px 28px rgba(0, 0, 0, 0.18);
  }
  .sidebar:not(.open) + .canvas-wrap .canvas-frame {
    max-width: 1100px;
  }
  .reader-page[data-read-focus="1"] .canvas-frame {
    max-width: 1200px;
  }
  .scroll-page-pill {
    position: sticky;
    top: 12px;
    align-self: center;
    margin: 0 auto 12px;
    z-index: 5;
    padding: 4px 12px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--bg-elevated) 92%, transparent);
    color: var(--text-muted);
    border: none;
    font-size: 11px;
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
    backdrop-filter: blur(6px);
    pointer-events: none;
    width: max-content;
    animation: pill-fade-in 200ms ease-out both;
  }
  @keyframes pill-fade-in {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
  @media (prefers-reduced-motion: reduce) {
    .scroll-page-pill { animation: none; }
  }
  .scroll-stack {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
    align-items: center;
    width: 100%;
  }
  .scroll-stack .canvas-frame {
    max-width: 900px;
  }
  .sidebar:not(.open) + .canvas-wrap .scroll-stack .canvas-frame {
    max-width: 1100px;
  }
  .reader-page[data-read-focus="1"] .scroll-stack .canvas-frame {
    max-width: 1200px;
  }
  .scroll-placeholder {
    position: relative;
    width: 100%;
    max-width: 900px;
    background: color-mix(in oklab, var(--content-border) 20%, transparent);
    border: none;
    border-radius: var(--border-radius);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tertiary);
    font-size: 11px;
  }
  .page-img {
    display: block;
    width: 100%;
    height: auto;
    user-select: none;
    pointer-events: none;
  }
  .hl-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .hl-rect {
    position: absolute;
    border: 0;
    padding: 0;
    cursor: pointer;
    pointer-events: auto;
    transition: filter 120ms ease;
    background: var(--hl-color);
    mix-blend-mode: multiply;
  }
  .hl-rect[data-drawer="lighten"] {
    background: var(--hl-color);
  }
  .hl-rect[data-drawer="underscore"] {
    background: transparent;
    border-bottom: 2px solid var(--hl-color);
    mix-blend-mode: normal;
  }
  .hl-rect[data-drawer="strikeout"] {
    background: transparent;
    mix-blend-mode: normal;
    box-shadow: 0 -1px 0 var(--hl-color) inset;
    position: relative;
  }
  .hl-rect[data-drawer="strikeout"]::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 2px;
    background: var(--hl-color);
    pointer-events: none;
  }
  .hl-rect[data-drawer="invert"] {
    background: var(--hl-color);
    mix-blend-mode: difference;
  }
  .hl-rect:hover {
    filter: brightness(0.9);
  }
  .hl-note-pin {
    position: absolute;
    top: -3px;
    right: -3px;
    color: var(--accent);
    font-size: 10px;
    pointer-events: none;
  }
  .text-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    line-height: 1;
    user-select: none;
    cursor: default;
    pointer-events: none;
  }
  .text-layer.disabled {
    pointer-events: none;
    user-select: none;
  }
  .ink-layer {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .ink-layer.pointer-off {
    pointer-events: none;
  }
  .ink-path {
    cursor: pointer;
    pointer-events: stroke;
    transition: stroke-width 120ms ease;
  }
  .ink-path:hover {
    stroke-width: 3.5;
  }
  .canvas-frame.ink-mode {
    cursor: crosshair;
  }
  .canvas-frame.ink-mode .text-layer {
    pointer-events: none;
  }
  .ink-palette {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 0 2px;
  }
  .ink-swatch {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
    transition: transform 120ms ease, border-color 120ms ease;
  }
  .ink-swatch:hover {
    transform: scale(1.1);
  }
  .ink-swatch.selected {
    border-color: var(--accent);
    transform: scale(1.15);
  }
  .search-flag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-muted, var(--secondary));
    margin: 4px 0 8px;
  }
  .search-flag input {
    accent-color: var(--accent);
  }

  .title-btn {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    background: transparent;
    border: 0;
    padding: 0;
    color: inherit;
    font: inherit;
    cursor: pointer;
    text-align: left;
    border-radius: 4px;
    transition: background 120ms ease;
  }
  .title-btn:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .title-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .title-author {
    font-size: 12px;
    color: var(--text-muted, var(--secondary));
    font-weight: 400;
  }

  .meta-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in oklab, black 50%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    padding: 16px;
  }
  .meta-modal {
    width: 100%;
    max-width: 480px;
    background: var(--bg-elevated, var(--surface, var(--bg)));
    border: none;
    border-radius: 10px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-shadow: 0 20px 50px color-mix(in oklab, black 40%, transparent);
  }
  .meta-modal h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .meta-hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted, var(--secondary));
  }
  .meta-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-muted, var(--secondary));
  }
  .meta-field input {
    padding: 8px 10px;
    border: none;
    border-radius: 6px;
    background: var(--bg, var(--surface));
    color: var(--text, var(--primary));
    font: inherit;
    font-size: 13px;
  }
  .meta-field input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in oklab, var(--accent) 20%, transparent);
  }
  .meta-error {
    margin: 0;
    font-size: 12px;
    color: var(--error);
  }
  .meta-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding-top: 4px;
  }
  .meta-btn {
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid transparent;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  .meta-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .meta-btn.primary {
    background: var(--accent);
    color: var(--on-accent, white);
  }
  .meta-btn.ghost {
    background: transparent;
    border-color: var(--border, var(--input-border));
    color: var(--text, var(--primary));
  }
  .meta-btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }

  .ink-stroke {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: 4px;
    padding: 0 4px;
  }
  .ink-stroke input[type="range"] {
    width: 70px;
    accent-color: var(--accent);
  }
  .ink-stroke-value {
    font-size: 11px;
    color: var(--text-muted, var(--secondary));
    min-width: 12px;
    text-align: right;
  }

  .note-link {
    align-items: flex-start;
  }
  .note-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow: hidden;
    text-align: left;
  }
  .note-quote {
    font-size: 11px;
    color: var(--text-muted, var(--secondary));
    font-style: italic;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 1;
    line-clamp: 1;
    -webkit-box-orient: vertical;
  }
  .note-text {
    font-size: 12px;
    color: var(--text, var(--primary));
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .char-span {
    position: absolute;
    color: transparent;
    white-space: pre;
    transform-origin: 0 0;
    line-height: 1;
    font-family: serif;
    user-select: text;
    pointer-events: auto;
    cursor: text;
  }
  .char-span::selection {
    background: color-mix(in oklab, var(--accent) 35%, transparent);
    color: transparent;
  }
  .hl-popup {
    position: absolute;
    z-index: 40;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    background: var(--button-elevated);
    border: none;
    border-radius: 999px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.28);
  }
  .hl-color {
    width: 22px;
    height: 22px;
    border: none;
    border-radius: 50%;
    cursor: pointer;
    padding: 0;
  }
  .hl-color:hover {
    transform: scale(1.1);
  }
  .hl-cancel {
    width: 22px;
    height: 22px;
    border: 0;
    background: transparent;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
    padding: 0;
  }
  .hl-cancel:hover {
    color: var(--secondary);
  }

  .hl-edit-popup {
    position: absolute;
    z-index: 60;
    width: 280px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.32);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    color: var(--text);
  }
  .ep-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .ep-title {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--tertiary);
    font-weight: 600;
  }
  .ep-close {
    width: 22px;
    height: 22px;
    border: 0;
    background: transparent;
    color: var(--tertiary);
    cursor: pointer;
    font-size: 16px;
    border-radius: 999px;
  }
  .ep-close:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
    color: var(--text);
  }
  .ep-quote {
    margin: 0;
    padding: 8px 10px;
    background: color-mix(in oklab, var(--input-border) 30%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 4px;
    font-size: 11px;
    color: var(--secondary);
    line-height: 1.45;
    max-height: 60px;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
  }
  .ep-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ep-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--tertiary);
  }
  .ep-colors {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 4px;
  }
  .ep-color {
    aspect-ratio: 1;
    border: 2px solid transparent;
    border-radius: 50%;
    cursor: pointer;
    padding: 0;
    transition: transform 120ms ease, border-color 120ms ease;
  }
  .ep-color:hover {
    transform: scale(1.1);
  }
  .ep-color.active {
    border-color: var(--text);
    box-shadow: 0 0 0 2px var(--surface);
  }
  .ep-drawers {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
  }
  .ep-drawer {
    padding: 4px 6px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--secondary);
    font: inherit;
    font-size: 10px;
    cursor: pointer;
    text-transform: lowercase;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .ep-drawer:hover {
    border-color: var(--accent);
  }
  .ep-drawer.active {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  .ep-note {
    width: 100%;
    min-height: 60px;
    padding: 6px 8px;
    border: none;
    border-radius: 4px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    line-height: 1.5;
    resize: vertical;
  }
  .ep-note:focus {
    outline: none;
    border-color: var(--accent);
  }
  .ep-foot {
    display: flex;
    justify-content: space-between;
    gap: 6px;
    padding-top: 4px;
    border-top: none;
  }
  .ep-btn {
    padding: 6px 12px;
    border-radius: 4px;
    border: 1px solid transparent;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .ep-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .ep-btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .ep-btn.danger {
    background: transparent;
    border-color: color-mix(
      in oklab,
      var(--error, var(--accent)) 40%,
      var(--input-border)
    );
    color: var(--error, var(--accent));
  }
  .ep-btn.danger:hover {
    background: color-mix(
      in oklab,
      var(--error, var(--accent)) 10%,
      transparent
    );
  }
  .ep-btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .ep-btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
    border-color: var(--accent);
  }
  .ep-toast {
    margin: 4px 0;
    padding: 4px 8px;
    font-size: 11px;
    color: var(--success, var(--accent));
    text-align: center;
  }

  .hl-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    margin-right: 4px;
  }
  .hl-text-clamp {
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    white-space: normal !important;
  }
  .page-loading-overlay {
    position: absolute;
    inset: 0;
    background: color-mix(in oklab, var(--primary) 30%, transparent);
    pointer-events: none;
  }
  .state {
    padding: 4rem 2rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    text-align: center;
  }
  .muted {
    color: var(--tertiary);
    font-size: 14px;
    margin: 0;
  }
  .small {
    font-size: 12px;
  }
  .error {
    color: var(--error);
    font-size: 14px;
    font-weight: 500;
    max-width: 60ch;
    word-break: break-word;
  }
  .btn {
    padding: 0.5rem 1rem;
    border-radius: var(--border-radius);
    border: none;
    background: transparent;
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  @media (max-width: 720px) {
    .layout {
      grid-template-columns: 1fr;
    }
    .sidebar {
      position: static;
      width: 100%;
      max-height: 280px;
    }
  }
</style>
