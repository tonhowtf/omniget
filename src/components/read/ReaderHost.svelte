<script lang="ts">
  // Opens whatever the reader was asked for: a library book (ebook or comic)
  // or a manga chapter (downloaded, or streamed page by page). Logs the
  // reading session when it closes.
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openUrl } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { bookById, ensureStatus, pageUrl } from "$lib/read/library-store.svelte";
  import { resetReader } from "$lib/read/reader-state.svelte";
  import { prefs } from "$lib/read/prefs.svelte";
  import { chapterLabel, tr, type ReaderTarget } from "$lib/read/ui-store.svelte";
  import { isComic, type Book, type Chapter, type ChapterOpen, type SeriesFull } from "$lib/read/types";
  import EbookReader from "./EbookReader.svelte";
  import ComicReader from "./ComicReader.svelte";
  import ReadIcon from "./ReadIcon.svelte";

  let { target, onclose }: { target: ReaderTarget; onclose: () => void } = $props();

  let book = $state<Book | null>(null);
  let pages = $state<string[]>([]);
  let startPage = $state(0);
  let title = $state("");
  let rtl = $state(false);
  let seriesMode = $state("");
  let chapter = $state<Chapter | null>(null);
  let series = $state<SeriesFull | null>(null);
  let error = $state("");
  let loading = $state(true);

  // Session log: time actually spent with the window visible.
  let started = Date.now();
  let activeMs = 0;
  let lastTick = Date.now();
  let fromProgress = 0;
  let toProgress = 0;
  let pagesTurned = 0;
  const tick = setInterval(() => {
    const now = Date.now();
    if (document.visibilityState === "visible" && now - lastTick < 120_000) activeMs += now - lastTick;
    lastTick = now;
  }, 5000);

  function onProgress(p: number) {
    toProgress = p;
    pagesTurned++;
    lastTick = Date.now();
  }

  async function flushSession() {
    const now = Date.now();
    if (document.visibilityState === "visible") activeMs += Math.min(now - lastTick, 120_000);
    lastTick = now;
    if (activeMs < 5000 || prefs.incognito) return;
    const ms = activeMs;
    activeMs = 0;
    await invoke("read_log_session", {
      bookId: book?.id ?? null,
      chapterId: chapter?.id ?? null,
      startedAt: started,
      ms,
      fromProgress,
      toProgress,
      pages: pagesTurned,
    }).catch(() => undefined);
    started = now;
    fromProgress = toProgress;
    pagesTurned = 0;
  }

  async function loadBook(id: number) {
    const b = bookById(id) ?? (await invoke<Book>("read_book", { id }));
    book = b;
    title = b.title;
    rtl = b.rtl;
    fromProgress = toProgress = b.progress;
    if (isComic(b)) {
      const n = await invoke<number>("read_comic_pages", { id: b.id });
      if (b.format === "cbr" || b.format === "cb7") throw new Error(tr("read.reader.needs_convert"));
      pages = Array.from({ length: n }, (_, i) => pageUrl(b.id, i));
      startPage = Number(b.location) || 0;
    }
  }

  async function loadChapter(chapterId: number, seriesId: number) {
    series = await invoke<SeriesFull>("read_series_get", { id: seriesId });
    chapter = series.chapters.find((c) => c.id === chapterId) ?? null;
    if (!chapter) throw new Error("chapter not found");
    seriesMode = (series.series.reader?.mode as string) ?? "";
    title = `${series.series.title} · ${chapterLabel(chapter)}`;
    const o = await invoke<ChapterOpen>("read_chapter_open", { id: chapterId, dataSaver: false });
    if (o.external_url) {
      await openUrl(o.external_url);
      onclose();
      return;
    }
    startPage = o.start_page;
    if (o.book_id) {
      book = null;
      const n = await invoke<number>("read_comic_pages", { id: o.book_id });
      pages = Array.from({ length: n }, (_, i) => pageUrl(o.book_id!, i));
    } else {
      pages = o.pages;
    }
    void invoke("read_download_ahead", { id: chapterId }).catch(() => undefined);
  }

  async function load() {
    loading = true;
    error = "";
    pages = [];
    book = null;
    try {
      await ensureStatus();
      if (target.kind === "book") await loadBook(target.id);
      else await loadChapter(target.chapterId, target.seriesId);
    } catch (e) {
      error = String(e instanceof Error ? e.message : e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  onDestroy(() => {
    clearInterval(tick);
    void flushSession();
    if (book && !prefs.incognito) void invoke("read_kosync_push", { id: book.id }).catch(() => undefined);
    resetReader();
  });

  // Chapter navigation for manga.
  let neighbors = $derived.by(() => {
    if (!series || !chapter) return { prev: null as Chapter | null, next: null as Chapter | null };
    const list = series.chapters.filter((c) => !chapter!.lang || c.lang === chapter!.lang);
    const i = list.findIndex((c) => c.id === chapter!.id);
    return { prev: i > 0 ? list[i - 1] : null, next: i >= 0 && i < list.length - 1 ? list[i + 1] : null };
  });

  async function gotoChapter(c: Chapter | null) {
    if (!c || !series) return;
    await flushSession();
    target = { kind: "chapter", chapterId: c.id, seriesId: series.series.id };
    await load();
  }

  async function onComicPage(page: number, total: number) {
    onProgress(total ? (page + 1) / total : 0);
    if (prefs.incognito) return;
    if (chapter) {
      const done = await invoke<boolean>("read_chapter_progress", { id: chapter.id, page, pages: total }).catch(() => false);
      if (done && chapter) chapter.read = true;
    } else if (book) {
      await invoke("read_save_progress", { id: book.id, progress: total ? (page + 1) / total : 0, location: String(page), pages: total }).catch(() => undefined);
    }
  }

  let comicMode = $derived.by(() => {
    if (seriesMode) return seriesMode;
    const pref = (book?.prefs?.mode as string) ?? "";
    if (pref) return pref;
    if (book && !book.rtl && book.chapter_id == null) return "";
    return rtl ? "rtl" : "";
  });
</script>

<section class="rh" aria-label={title}>
  {#if loading}
    <div class="center"><ReadIcon name="refresh" size={26} spin /></div>
  {:else if error}
    <div class="center">
      <ReadIcon name="alert" size={30} />
      <p>{error}</p>
      <button class="btn" onclick={onclose}>{$t("read.close")}</button>
    </div>
  {:else if pages.length}
    <ComicReader
      {title}
      {pages}
      {startPage}
      bookId={book?.id ?? null}
      modeOverride={comicMode}
      hasPrev={!!neighbors.prev}
      hasNext={!!neighbors.next}
      onprevchapter={() => gotoChapter(neighbors.prev)}
      onnextchapter={() => gotoChapter(neighbors.next)}
      onpage={onComicPage}
      onmode={async (m) => {
        if (series) await invoke("read_series_update", { id: series.series.id, patch: { reader: { ...series.series.reader, mode: m } } });
        else if (book) await invoke("read_update_book", { id: book.id, patch: { prefs: { ...book.prefs, mode: m } } });
      }}
      onsetcover={async (dataUrl) => {
        if (!book) return;
        await invoke("read_set_cover", { id: book.id, dataBase64: dataUrl });
        showToast("success", tr("read.reader.cover_set"));
      }}
      {onclose}
    />
  {:else if book}
    <EbookReader {book} at={target.kind === "book" ? target.at : undefined} onprogress={onProgress} {onclose} />
  {/if}
</section>

<style>
  .rh {
    position: fixed;
    inset: 0;
    z-index: 900;
    background: var(--bg);
    display: flex;
    flex-direction: column;
  }
  .center {
    display: grid;
    place-items: center;
    align-content: center;
    gap: var(--space-3);
    height: 100%;
    color: var(--text-dim);
    text-align: center;
    padding: var(--space-5);
  }
</style>
