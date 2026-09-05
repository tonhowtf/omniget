<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";
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
  import ReaderTypographyMenu from "$lib/reader-components/ReaderTypographyMenu.svelte";
  import {
    DEFAULT_TYPOGRAPHY,
    FONT_LIMITS,
    applyIframeTypography,
    loadBookSettings,
    loadGlobal,
    saveBookSettings,
    saveGlobal,
    type Typography,
  } from "$lib/reader-typography";
  import ReaderZoomIndicator from "$lib/reader-components/ReaderZoomIndicator.svelte";
  import { wheelZoomDirection } from "$lib/reader-zoom";
  import {
    applyHighlight,
    captureSelection,
    clearHighlights,
    ensureHighlightStyles,
    type EpubChapterHighlight,
    type EpubSelectionEvent,
  } from "$lib/study-epub-anchor";
  import "$lib/reader-theme.css";

  type Book = {
    id: number;
    file_path: string;
    format: string;
    title: string | null;
    author: string | null;
    last_location: string | null;
  };

  type Chapter = {
    index: number;
    id: string;
    href: string;
    abs_path: string;
  };

  type TocItem = {
    label: string;
    href: string;
  };

  type OpenResult = {
    book_id: number;
    format: string;
    chapter_count: number;
    title: string;
    author: string;
    publisher: string;
    language: string;
    chapters: Chapter[];
    toc: TocItem[];
    cover_path: string | null;
    extract_dir: string;
  };

  let {
    book,
    onBack,
    chapterHighlights = [],
    onSelection,
    onChapterChange,
  }: {
    book: Book;
    onBack: () => void;
    chapterHighlights?: EpubChapterHighlight[];
    onSelection?: (event: EpubSelectionEvent) => void;
    onChapterChange?: (info: { chapterIdx: number; chapterId: string; chapterHref: string }) => void;
  } = $props();

  let meta = $state<OpenResult | null>(null);
  let chapterIdx = $state(0);
  let chapterSrc = $state<string>("");
  let loadingBook = $state(true);
  let loadingChapter = $state(false);
  let errorMsg = $state("");
  let sidebarOpen = $state(true);
  let session: ReadingSession | null = null;
  let frameEl: HTMLIFrameElement | null = $state(null);
  let typography = $state<Typography>({ ...DEFAULT_TYPOGRAPHY });
  let typoMenuOpen = $state(false);
  $effect(() => {
    saveGlobal(typography);
    if (book) void saveBookSettings(book.id, typography);
    applyIframeTypography(frameEl, typography);
  });

  let iframeMouseUpHandler: ((e: MouseEvent) => void) | null = null;
  let iframeWheelHandler: ((e: WheelEvent) => void) | null = null;
  let zoomLabel = $state("");
  let zoomLabelTimer: ReturnType<typeof setTimeout> | null = null;

  function flashZoomLabel(size: number) {
    zoomLabel = `${Math.round((size / DEFAULT_TYPOGRAPHY.font_size) * 100)}%`;
    if (zoomLabelTimer) clearTimeout(zoomLabelTimer);
    zoomLabelTimer = setTimeout(() => (zoomLabel = ""), 1200);
  }

  function onWheelZoom(e: WheelEvent) {
    const dir = wheelZoomDirection(e);
    if (dir === 0) return;
    e.preventDefault();
    const next = Math.max(
      FONT_LIMITS.size.min,
      Math.min(FONT_LIMITS.size.max, typography.font_size + dir * FONT_LIMITS.size.step),
    );
    if (next !== typography.font_size) {
      typography = { ...typography, font_size: next };
    }
    flashZoomLabel(next);
  }

  function currentChapter() {
    if (!meta) return null;
    const ch = meta.chapters[chapterIdx];
    return ch ?? null;
  }

  function handleIframeMouseUp() {
    if (!frameEl || !onSelection) return;
    const win = frameEl.contentWindow;
    const doc = frameEl.contentDocument;
    if (!win || !doc) return;
    const sel = win.getSelection();
    if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return;
    const range = sel.getRangeAt(0);
    const ch = currentChapter();
    if (!ch) return;
    const anchor = captureSelection(doc, range, ch.id, ch.href, ch.index);
    if (!anchor) return;
    const r = range.getBoundingClientRect();
    onSelection({
      ...anchor,
      rectInIframe: { x: r.left, y: r.top, width: r.width, height: r.height },
    });
  }

  function reapplyHighlights() {
    if (!frameEl) return;
    const doc = frameEl.contentDocument;
    if (!doc || !doc.body) return;
    ensureHighlightStyles(doc);
    clearHighlights(doc);
    for (const h of chapterHighlights) {
      applyHighlight(doc, h.char_offset_start, h.char_offset_end, h.id, h.color ?? "yellow");
    }
  }

  function onIframeLoaded() {
    applyIframeTypography(frameEl, typography);
    if (frameEl?.contentDocument) {
      ensureHighlightStyles(frameEl.contentDocument);
      reapplyHighlights();
      if (iframeMouseUpHandler) {
        frameEl.contentDocument.removeEventListener("mouseup", iframeMouseUpHandler);
      }
      iframeMouseUpHandler = (_e) => handleIframeMouseUp();
      frameEl.contentDocument.addEventListener("mouseup", iframeMouseUpHandler);
      if (iframeWheelHandler) {
        frameEl.contentDocument.removeEventListener("wheel", iframeWheelHandler);
      }
      iframeWheelHandler = (e) => onWheelZoom(e);
      frameEl.contentDocument.addEventListener("wheel", iframeWheelHandler, {
        passive: false,
      });
    }
  }

  $effect(() => {
    void chapterHighlights;
    void chapterIdx;
    reapplyHighlights();
  });

  $effect(() => {
    const ch = currentChapter();
    if (ch && onChapterChange) {
      onChapterChange({ chapterIdx, chapterId: ch.id, chapterHref: ch.href });
    }
  });
  let readerTheme = $state<string>("app");
  let paperFilter = $state(false);
  let cursorLine = $state(false);
  let focusMode = $state(false);
  let themeMenuOpen = $state(false);
  let cursorY = $state(-1);

  $effect(() => {
    setPaperFilter(paperFilter);
  });
  $effect(() => {
    setCursorLine(cursorLine);
  });
  $effect(() => {
    applyFocusMode(focusMode);
  });

  const totalChapters = $derived(meta?.chapter_count ?? 0);

  const tocEntries = $derived.by<Array<{ label: string; chapter: number }>>(() => {
    if (!meta) return [];
    if (meta.toc.length > 0 && meta.chapters.length > 0) {
      return meta.toc.map((item) => {
        const cleanHref = item.href.split("#")[0].replace(/\\/g, "/");
        const match = meta!.chapters.findIndex((ch) => {
          const chHref = ch.href.split("#")[0].replace(/\\/g, "/");
          return chHref === cleanHref || chHref.endsWith("/" + cleanHref) || cleanHref.endsWith("/" + chHref);
        });
        return { label: item.label || `#${match + 1}`, chapter: match >= 0 ? match : 0 };
      });
    }
    return meta.chapters.map((ch, i) => ({
      label: ch.id || `${$t("study.read.chapter")} ${i + 1}`,
      chapter: i,
    }));
  });

  function parseLastLocation(raw: string | null): number | null {
    if (!raw) return null;
    try {
      const parsed = JSON.parse(raw);
      if (typeof parsed?.chapter === "number") return parsed.chapter;
    } catch {}
    const n = Number(raw);
    return Number.isFinite(n) ? n : null;
  }

  async function loadBook() {
    try {
      meta = await pluginInvoke<OpenResult>("study", "study:read:epub:open", {
        bookId: book.id,
      });
      const saved = parseLastLocation(book.last_location) ?? 0;
      chapterIdx = Math.max(0, Math.min(meta!.chapter_count - 1, saved));
      const perBook = await loadBookSettings(book.id);
      typography = perBook ?? loadGlobal();
      await renderChapter();
      session = createReadingSession({
        bookId: book.id,
        getLocation: () => JSON.stringify({ chapter: chapterIdx }),
      });
      await session.start();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      loadingBook = false;
    }
  }

  async function renderChapter() {
    if (!meta || chapterIdx < 0 || chapterIdx >= meta.chapter_count) return;
    loadingChapter = true;
    try {
      const ch = meta.chapters[chapterIdx];
      if (ch && ch.abs_path) {
        chapterSrc = convertFileSrc(ch.abs_path);
      } else {
        const out = await pluginInvoke<{ abs_path: string }>(
          "study",
          "study:read:epub:chapter_path",
          { bookId: book.id, index: chapterIdx },
        );
        chapterSrc = convertFileSrc(out.abs_path);
      }
      persistLocation();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      loadingChapter = false;
    }
  }

  async function persistLocation() {
    if (!meta) return;
    const pct = totalChapters > 0 ? (chapterIdx + 1) / totalChapters : 0;
    try {
      await pluginInvoke("study", "study:read:books:update_location", {
        bookId: book.id,
        location: JSON.stringify({ chapter: chapterIdx }),
        readingPct: pct,
      });
    } catch (e) {
      console.error("persist location failed", e);
    }
  }

  async function goTo(i: number) {
    if (!meta) return;
    const clamped = Math.max(0, Math.min(meta.chapter_count - 1, i));
    if (clamped === chapterIdx) return;
    chapterIdx = clamped;
    await renderChapter();
    session?.notePageChange();
  }

  async function goPrev() {
    await goTo(chapterIdx - 1);
  }

  async function goNext() {
    await goTo(chapterIdx + 1);
  }

  function onKey(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
    if (e.key === "ArrowLeft" || e.key === "PageUp") {
      e.preventDefault();
      goPrev();
    } else if (e.key === "ArrowRight" || e.key === "PageDown") {
      e.preventDefault();
      goNext();
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

  async function openExternal() {
    try {
      await invoke("open_path_default", { path: book.file_path });
    } catch (e) {
      console.error(e);
    }
  }

  onMount(() => {
    readerTheme = getReadingTheme();
    paperFilter = getPaperFilter();
    cursorLine = getCursorLine();
    pushReadingTheme();
    loadBook();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("wheel", onWheelZoom, { passive: false });
  });

  onDestroy(() => {
    window.removeEventListener("keydown", onKey);
    window.removeEventListener("mousemove", onMouseMove);
    window.removeEventListener("wheel", onWheelZoom);
    applyFocusMode(false);
    popReadingTheme();
    void session?.stop(false);
  });
</script>

<section
  class="reader-page"
  data-read-paper={paperFilter ? "1" : "0"}
  data-read-focus={focusMode ? "1" : "0"}
>
  <header class="head">
    <button type="button" class="back-btn" onclick={onBack}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M15 18l-6-6 6-6"></path>
      </svg>
      <span>{$t("study.read.back_to_library")}</span>
    </button>
    <h1 class="title">{book.title ?? book.file_path.split(/[\\/]/).pop()}</h1>
    {#if meta}
      <div class="toolbar">
        <button type="button" class="tool-btn" onclick={goPrev} disabled={chapterIdx <= 0} title={$t("study.read.prev_chapter")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6"></path></svg>
        </button>
        <span class="page-label mono">{$t("study.read.chapter_of", { current: chapterIdx + 1, total: meta.chapter_count })}</span>
        <button type="button" class="tool-btn" onclick={goNext} disabled={chapterIdx >= meta.chapter_count - 1} title={$t("study.read.next_chapter")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6"></path></svg>
        </button>
        <span class="sep"></span>
        <div class="theme-wrap">
          <button
            type="button"
            class="tool-btn"
            class:active={typoMenuOpen}
            onclick={() => (typoMenuOpen = !typoMenuOpen)}
            title={$t("study.read.typo_title")}
            aria-label={$t("study.read.typo_title")}
          >
            <span style="font-family: serif; font-weight: 600; font-size: 13px;">Aa</span>
          </button>
          {#if typoMenuOpen}
            <ReaderTypographyMenu
              bind:typography
              onClose={() => (typoMenuOpen = false)}
              onReset={() => (typography = { ...DEFAULT_TYPOGRAPHY })}
            />
          {/if}
        </div>
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
        <button type="button" class="tool-btn" onclick={openExternal} title={$t("study.read.open_external")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"></path>
            <path d="M15 3h6v6"></path>
            <path d="M10 14L21 3"></path>
          </svg>
        </button>
      </div>
    {/if}
  </header>

  <ReaderZoomIndicator label={zoomLabel} />

  {#if cursorLine && cursorY > 0}
    <div class="reader-cursor-line" style:top="{cursorY}px"></div>
  {/if}

  {#if loadingBook}
    <p class="state">{$t("study.read.loading_book")}</p>
  {:else if errorMsg}
    <div class="state">
      <p class="error">{errorMsg}</p>
      <button type="button" class="btn" onclick={openExternal}>{$t("study.read.open_external")}</button>
    </div>
  {:else if meta}
    <div class="layout">
      <aside class="sidebar" class:open={sidebarOpen}>
        <div class="sidebar-head">
          {#if sidebarOpen}
            <span class="panel-title">{$t("study.read.panel_outline")}</span>
          {/if}
          <button type="button" class="icon-btn" onclick={() => (sidebarOpen = !sidebarOpen)} aria-label="toggle">
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
          <div class="sidebar-body">
            {#if tocEntries.length === 0}
              <p class="muted small">{$t("study.read.no_outline")}</p>
            {:else}
              <ul class="toc-list">
                {#each tocEntries as entry, i (i)}
                  <li>
                    <button
                      type="button"
                      class="toc-item"
                      class:active={entry.chapter === chapterIdx}
                      onclick={() => goTo(entry.chapter)}
                    >
                      <span class="toc-label">{entry.label}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {/if}
      </aside>

      <div class="viewer">
        {#if loadingChapter}
          <p class="state small muted">{$t("study.read.loading_page")}</p>
        {/if}
        {#if chapterSrc}
          <iframe
            class="chapter-frame"
            src={chapterSrc}
            title={book.title ?? "chapter"}
            sandbox="allow-same-origin"
            bind:this={frameEl}
            onload={onIframeLoaded}
          ></iframe>
        {/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .reader-page {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg);
    color: var(--text);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: none;
    background: var(--bg-elevated);
    flex-shrink: 0;
  }

  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: transparent;
    color: var(--text-muted);
    border: none;
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
  }
  .back-btn:hover {
    color: var(--text);
    background: var(--surface);
  }

  .title {
    margin: 0;
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .tool-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    color: var(--text-muted);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    font-size: 12px;
  }
  .tool-btn:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface);
  }
  .tool-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .tool-btn.active {
    color: var(--text);
    background: var(--surface);
    border-color: var(--accent);
  }

  .theme-wrap {
    position: relative;
    display: inline-flex;
  }

  .page-label {
    font-size: 11px;
    color: var(--text-muted);
    padding: 0 6px;
  }

  .sep {
    display: inline-block;
    width: 1px;
    height: 18px;
    background: var(--border);
    margin: 0 4px;
  }

  .state {
    padding: 40px;
    text-align: center;
    color: var(--text-muted);
  }
  .state .error {
    color: var(--danger);
    font-size: 13px;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: 11px;
  }

  .btn {
    display: inline-block;
    padding: 6px 14px;
    background: transparent;
    color: var(--text);
    border: none;
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
    margin-top: 8px;
  }
  .btn:hover {
    background: var(--surface);
  }

  .layout {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .sidebar {
    width: 48px;
    background: var(--bg-elevated);
    border-right: none;
    display: flex;
    flex-direction: column;
    transition: width 0.15s ease;
    flex-shrink: 0;
    overflow: hidden;
  }
  .sidebar.open {
    width: 260px;
  }

  .sidebar-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: none;
    min-height: 38px;
  }

  .panel-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    flex: 1;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    background: transparent;
    color: var(--text-muted);
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  .icon-btn:hover {
    color: var(--text);
    background: var(--surface);
  }

  .sidebar-body {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
  }

  .toc-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .toc-item {
    display: block;
    width: 100%;
    padding: 6px 10px;
    background: transparent;
    color: var(--text);
    border: none;
    border-radius: 4px;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .toc-item:hover {
    background: var(--surface);
  }
  .toc-item.active {
    background: var(--accent);
    color: var(--on-accent);
  }

  .toc-label {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .viewer {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--bg);
    position: relative;
  }

  .chapter-frame {
    flex: 1;
    width: 100%;
    border: none;
    background: white;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
</style>
