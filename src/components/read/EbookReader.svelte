<script lang="ts">
  // Reflowable and fixed-layout books (EPUB, MOBI/AZW3, FB2, PDF, and
  // TXT/Markdown/HTML converted on the fly) through the vendored foliate-js.
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openUrl } from "@tauri-apps/plugin-shell";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { bookUrl, ensureStatus, patchBookLocal } from "$lib/read/library-store.svelte";
  import { applyBionic, isDarkTheme, loadFoliate, openBookFile, readerCss, THEMES } from "$lib/read/foliate";
  import { prefs, savePrefs } from "$lib/read/prefs.svelte";
  import { reader, resetReader } from "$lib/read/reader-state.svelte";
  import { tr } from "$lib/read/ui-store.svelte";
  import type { Annotation, Book } from "$lib/read/types";
  import ReaderSettings from "./ReaderSettings.svelte";
  import ReadIcon from "./ReadIcon.svelte";
  import { agentActions } from "$lib/native-agent/actions";

  let { book, at, onprogress, onclose }: { book: Book; at?: string; onprogress?: (p: number) => void; onclose: () => void } = $props();

  const COLORS: Record<string, string> = {
    yellow: "#f5d000",
    green: "#5cc26b",
    blue: "#4aa3ff",
    pink: "#ff6fa8",
    purple: "#a77bff",
    orange: "#ff9a3d",
  };
  const STYLES = ["highlight", "underline", "squiggly", "strikethrough"];

  let host: HTMLDivElement | undefined = $state();
  let view: any = null;
  let Overlayer: any = null;
  let loading = $state(true);
  let error = $state("");
  let ui = $state(true);
  let panel = $state<"" | "toc" | "search" | "notes" | "settings" | "ai">("");
  let toc = $state<any[]>([]);
  let loc = $state<{ fraction: number; cfi: string; chapter: string; href: string; page: number; pages: number; left: number; leftBook: number; index: number }>({
    fraction: untrack(() => book.progress),
    cfi: "",
    chapter: "",
    href: "",
    page: 0,
    pages: 0,
    left: 0,
    leftBook: 0,
    index: 0,
  });
  let notes = $state<Annotation[]>([]);
  let sel = $state<{ x: number; y: number; text: string; cfi: string; index: number; context: string } | null>(null);
  let popup = $state<{ x: number; y: number; ann: Annotation } | null>(null);
  let dict = $state<{ word: string; html: string; loading: boolean; wiki: string } | null>(null);
  let aiOut = $state<{ task: string; text: string; loading: boolean } | null>(null);
  let aiQuestion = $state("");
  let searchQ = $state("");
  let searchOpts = $state({ matchCase: false, matchWholeWords: false, matchDiacritics: false, regex: false });
  let results = $state<{ label: string; items: { cfi: string; pre: string; match: string; post: string }[] }[]>([]);
  let searching = $state(0);
  let speaking = $state(false);
  let fullscreen = $state(false);
  let fixed = $state(false);
  let remoteSync = $state<{ percentage: number; device: string } | null>(null);
  let footnote = $state<HTMLElement | null>(null);
  let footnoteOpen = $state(false);
  let footHost: HTMLDivElement | undefined = $state();
  let clock = $state("");
  let dir = $state<"ltr" | "rtl">("ltr");

  const isBookmarked = $derived(notes.some((n) => n.kind === "bookmark" && n.progress != null && Math.abs(n.progress - loc.fraction) < 0.004));

  // ---------- open ----------

  async function fileFor(b: Book): Promise<{ url: string; name: string }> {
    await ensureStatus();
    if (["txt", "md", "html"].includes(b.format)) {
      const url = await invoke<string>("read_prepare", { id: b.id });
      return { url, name: `${b.title}.epub` };
    }
    const ext = b.format === "kepub" ? "epub" : b.format === "fbz" ? "fbz" : b.format;
    return { url: bookUrl(b.id), name: `${b.id}.${ext}` };
  }

  function applyStyles() {
    if (!view?.renderer) return;
    const r = view.renderer;
    r.setStyles?.(readerCss(prefs));
    if (!fixed) {
      r.setAttribute("flow", prefs.flow);
      r.setAttribute("max-column-count", String(prefs.maxColumns));
      r.setAttribute("max-inline-size", `${prefs.maxWidth}px`);
      r.setAttribute("margin", `${prefs.margin}px`);
      r.setAttribute("gap", `${prefs.gap}%`);
      if (prefs.animated) r.setAttribute("animated", "");
      else r.removeAttribute("animated");
    }
    host?.style.setProperty("--reader-bg", (THEMES[prefs.theme] ?? THEMES.light).bg);
    host?.style.setProperty("--reader-fg", (THEMES[prefs.theme] ?? THEMES.light).fg);
    host?.style.setProperty("--reader-filter", prefs.brightness !== 100 ? `brightness(${prefs.brightness / 100})` : "none");
  }

  async function open() {
    loading = true;
    error = "";
    try {
      reader.stage = "fetch";
      await loadFoliate();
      ({ Overlayer } = await import("$lib/vendor/foliate-js/overlayer.js"));
      const f = await fileFor(book);
      reader.stage = "parse";
      const bk = await openBookFile(f.url, f.name);
      reader.stage = "render";
      view = document.createElement("foliate-view");
      view.setAttribute("autohide-cursor", "");
      host!.append(view);
      view.addEventListener("relocate", onRelocate);
      view.addEventListener("load", onLoad);
      view.addEventListener("create-overlay", onOverlay);
      view.addEventListener("draw-annotation", onDraw);
      view.addEventListener("show-annotation", onShowAnnotation);
      view.addEventListener("external-link", (e: CustomEvent) => {
        e.preventDefault();
        void openUrl(e.detail.href);
      });
      view.addEventListener("link", onLink);
      await view.open(bk);
      fixed = !!view.isFixedLayout;
      dir = bk.dir === "rtl" ? "rtl" : "ltr";
      toc = bk.toc ?? [];
      applyStyles();
      notes = await invoke<Annotation[]>("read_annotations", { bookId: book.id }).catch(() => []);
      const last = at || book.location || undefined;
      if (last && last.startsWith("epubcfi")) await view.init({ lastLocation: last });
      else if (book.progress > 0) await view.init({ lastLocation: null }).then(() => view.goToFraction(book.progress));
      else await view.init({ showTextStart: true });
      reader.kind = "ebook";
      reader.title = book.title;
      reader.stage = "ready";
      void pullSync();
    } catch (e) {
      error = String(e instanceof Error ? e.message : e);
      reader.stage = `error: ${error}`;
    } finally {
      loading = false;
    }
  }

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  function onRelocate(e: CustomEvent) {
    const d = e.detail;
    loc = {
      // foliate can report past 1 on a book that fits in one screen.
      fraction: Math.min(1, Math.max(0, d.fraction ?? 0)),
      cfi: d.cfi ?? "",
      chapter: d.tocItem?.label ?? "",
      href: d.tocItem?.href ?? "",
      page: d.location?.current ?? 0,
      pages: d.location?.total ?? 0,
      left: d.time?.section ?? 0,
      leftBook: d.time?.total ?? 0,
      index: d.section?.current ?? 0,
    };
    reader.location = loc.cfi;
    reader.progress = loc.fraction;
    reader.chapter = loc.chapter;
    onprogress?.(loc.fraction);
    sel = null;
    if (saveTimer) clearTimeout(saveTimer);
    if (prefs.incognito) return;
    saveTimer = setTimeout(async () => {
      try {
        const pages = d.location?.total ? d.location.total : null;
        await invoke("read_save_progress", { id: book.id, progress: loc.fraction, location: loc.cfi, pages });
        patchBookLocal({ ...book, progress: loc.fraction, location: loc.cfi, status: book.status === "unread" ? "reading" : book.status, last_read_at: Date.now() });
      } catch {
        /* next relocate retries */
      }
    }, 900);
  }

  function onLoad(e: CustomEvent) {
    const { doc, index } = e.detail as { doc: Document; index: number };
    doc.addEventListener("keydown", keys);
    doc.addEventListener("pointerup", () => setTimeout(() => checkSelection(doc, index), 10));
    doc.addEventListener("keyup", (ev: KeyboardEvent) => {
      if (ev.shiftKey) checkSelection(doc, index);
    });
    doc.addEventListener("wheel", wheel, { passive: false });
    if (prefs.bionic) applyBionic(doc);
    if (prefs.rulerLines > 0) addRuler(doc);
  }

  function onOverlay(e: CustomEvent) {
    const { index } = e.detail;
    for (const n of notes) {
      if (n.kind === "bookmark") continue;
      void view.addAnnotation({ value: n.locator, color: n.color, style: n.style, id: n.id }).then((r: any) => {
        if (r && r.index !== index) return;
      });
    }
  }

  function onDraw(e: CustomEvent) {
    const { draw, annotation } = e.detail;
    const color = COLORS[annotation.color] ?? annotation.color ?? COLORS.yellow;
    const style = STYLES.includes(annotation.style) ? annotation.style : "highlight";
    draw(Overlayer[style], { color });
  }

  function onShowAnnotation(e: CustomEvent) {
    const { value, range } = e.detail;
    const ann = notes.find((n) => n.locator === value);
    if (!ann) return;
    const r = frameRect(range);
    popup = { x: r.x + r.width / 2, y: r.bottom + 8, ann };
  }

  let footnoteHandler: any = null;
  async function onLink(e: CustomEvent) {
    if (!footnoteHandler) {
      const { FootnoteHandler } = await import("$lib/vendor/foliate-js/footnotes.js");
      footnoteHandler = new FootnoteHandler();
      footnoteHandler.addEventListener("before-render", (ev: CustomEvent) => {
        const v = ev.detail.view;
        v.addEventListener("link", (le: CustomEvent) => {
          le.preventDefault();
          footnoteOpen = false;
          void view.goTo(le.detail.href);
        });
        footnote?.remove();
        footnote = v;
        footHost?.replaceChildren(v);
      });
      footnoteHandler.addEventListener("render", (ev: CustomEvent) => {
        const v = ev.detail.view;
        v.renderer?.setStyles?.(readerCss({ ...prefs, flow: "scrolled" } as typeof prefs));
        v.renderer?.setAttribute("flow", "scrolled");
        v.renderer?.setAttribute("margin", "8px");
        footnoteOpen = true;
      });
    }
    footnoteHandler.handle(view.book, e)?.catch(() => undefined);
  }

  // ---------- selection & annotations ----------

  function frameRect(range: Range) {
    const r = range.getBoundingClientRect();
    const frame = (range.startContainer.ownerDocument?.defaultView as any)?.frameElement as HTMLElement | null;
    const f = frame?.getBoundingClientRect() ?? { left: 0, top: 0 };
    return { x: r.left + f.left, y: r.top + f.top, width: r.width, bottom: r.bottom + f.top, top: r.top + f.top };
  }

  function checkSelection(doc: Document, index: number) {
    const s = doc.getSelection();
    if (!s || s.isCollapsed || !s.rangeCount) {
      sel = null;
      reader.selection = "";
      return;
    }
    const range = s.getRangeAt(0);
    const text = s.toString().trim();
    if (!text) return;
    const cfi = view.getCFI(index, range);
    const r = frameRect(range);
    const block = (range.startContainer.parentElement?.closest("p, li, blockquote, div") as HTMLElement | null)?.textContent ?? text;
    sel = { x: r.x + r.width / 2, y: r.top, text, cfi, index, context: block.slice(0, 1200) };
    reader.selection = text;
  }

  function clearSelection() {
    for (const c of view?.renderer?.getContents?.() ?? []) c.doc?.getSelection()?.removeAllRanges();
    sel = null;
  }

  async function annotate(color: string, style = "highlight", note = "") {
    if (!sel) return;
    const a = await invoke<Annotation>("read_annotation_save", {
      a: {
        id: 0,
        book_id: book.id,
        kind: note ? "note" : "highlight",
        locator: sel.cfi,
        text: sel.text,
        note,
        color,
        style,
        chapter: loc.chapter,
        progress: loc.fraction,
      },
    });
    notes = [...notes, a];
    await view.addAnnotation({ value: a.locator, color: a.color, style: a.style });
    clearSelection();
  }

  async function updateAnnotation(a: Annotation, patch: Partial<Annotation>) {
    const saved = await invoke<Annotation>("read_annotation_save", { a: { ...a, ...patch } });
    notes = notes.map((n) => (n.id === a.id ? saved : n));
    await view.addAnnotation({ value: saved.locator, color: saved.color, style: saved.style });
    if (popup) popup = { ...popup, ann: saved };
  }

  async function removeAnnotation(a: Annotation) {
    await invoke("read_annotation_delete", { id: a.id });
    notes = notes.filter((n) => n.id !== a.id);
    if (a.kind !== "bookmark") await view.deleteAnnotation({ value: a.locator });
    popup = null;
  }

  async function toggleBookmark() {
    const existing = notes.find((n) => n.kind === "bookmark" && n.progress != null && Math.abs(n.progress - loc.fraction) < 0.004);
    if (existing) return removeAnnotation(existing);
    const a = await invoke<Annotation>("read_annotation_save", {
      a: { id: 0, book_id: book.id, kind: "bookmark", locator: loc.cfi, text: loc.chapter, note: "", color: "", style: "", chapter: loc.chapter, progress: loc.fraction },
    });
    notes = [...notes, a];
  }

  async function copySel() {
    if (!sel) return;
    await navigator.clipboard.writeText(sel.text).catch(() => undefined);
    showToast("success", tr("read.reader.copied"));
    clearSelection();
  }

  async function copyQuote() {
    if (!sel) return;
    const q = `“${sel.text}”\n— ${book.authors.join(", ")}, ${book.title}`;
    await navigator.clipboard.writeText(q).catch(() => undefined);
    showToast("success", tr("read.reader.copied"));
    clearSelection();
  }

  // ---------- dictionary & AI ----------

  async function lookup(word: string, context: string) {
    const w = word.trim().split(/\s+/).slice(0, 4).join(" ");
    dict = { word: w, html: "", loading: true, wiki: "" };
    const lang = (book.language || prefs.dictionaryLang || "en").slice(0, 2);
    try {
      const r = await fetch(`https://en.wiktionary.org/api/rest_v1/page/definition/${encodeURIComponent(w.toLowerCase())}`);
      if (r.ok) {
        const j = await r.json();
        const entries = (j[lang] ?? j.en ?? Object.values(j)[0] ?? []) as any[];
        dict.html = entries
          .slice(0, 4)
          .map((en) => `<p><i>${en.partOfSpeech}</i></p><ol>${(en.definitions ?? []).slice(0, 4).map((d: any) => `<li>${d.definition}</li>`).join("")}</ol>`)
          .join("");
      }
    } catch {
      /* offline */
    }
    try {
      const r = await fetch(`https://${lang}.wikipedia.org/api/rest_v1/page/summary/${encodeURIComponent(w)}`);
      if (r.ok) {
        const j = await r.json();
        if (j.type !== "disambiguation") dict.wiki = j.extract ?? "";
      }
    } catch {
      /* offline */
    }
    if (!dict.html && !dict.wiki) {
      try {
        dict.html = `<p>${escapeHtml(await invoke<string>("read_ai", { task: "define", text: w, context, lang: navigator.language }))}</p>`;
      } catch {
        dict.html = `<p>${escapeHtml(tr("read.reader.no_definition"))}</p>`;
      }
    }
    dict.loading = false;
    // Sanitize third-party HTML to plain structure.
    dict.html = dict.html.replace(/<(?!\/?(p|i|b|ol|ul|li|em|strong)\b)[^>]*>/gi, "");
  }

  function escapeHtml(s: string) {
    return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
  }

  async function saveWord() {
    if (!dict) return;
    await invoke("read_vocab_add", { word: dict.word, context: sel?.context ?? "", definition: dict.html.replace(/<[^>]+>/g, " ").trim().slice(0, 600), lang: book.language, bookId: book.id }).catch(() => undefined);
    showToast("success", tr("read.reader.word_saved"));
  }

  /** Text from the start of the book up to where the reader is. */
  async function textSoFar(maxChars = 120_000) {
    const secs = view.book.sections as any[];
    const parts: string[] = [];
    let total = 0;
    for (let i = loc.index; i >= 0 && total < maxChars; i--) {
      if (!secs[i]?.createDocument || secs[i].linear === "no") continue;
      const doc: Document = await secs[i].createDocument();
      let txt = doc.body?.textContent ?? "";
      if (i === loc.index) {
        const range = view.lastLocation?.range as Range | undefined;
        if (range) {
          const live = view.renderer.getContents?.().find((c: any) => c.index === i)?.doc as Document | undefined;
          if (live) {
            const r = live.createRange();
            r.setStart(live.body, 0);
            try {
              r.setEnd(range.endContainer, range.endOffset);
              txt = r.toString();
            } catch {
              /* keep whole section */
            }
          }
        }
      }
      txt = txt.replace(/\s+/g, " ").trim();
      parts.unshift(txt);
      total += txt.length;
    }
    return parts.join("\n\n").slice(-maxChars);
  }

  async function chapterText() {
    const s = view.book.sections[loc.index];
    const doc: Document = await s.createDocument();
    return (doc.body?.textContent ?? "").replace(/\s+/g, " ").trim();
  }

  async function ai(task: string, text = "") {
    panel = "ai";
    aiOut = { task, text: "", loading: true };
    try {
      const context = task === "summarize" ? await chapterText() : ["recap", "ask", "characters", "quiz"].includes(task) ? await textSoFar() : (sel?.context ?? "");
      aiOut.text = await invoke<string>("read_ai", {
        task,
        text,
        context,
        lang: task === "translate" ? prefs.translateTo || navigator.language : navigator.language,
        question: aiQuestion,
        title: book.title,
      });
    } catch (e) {
      aiOut.text = String(e);
    } finally {
      if (aiOut) aiOut.loading = false;
    }
  }

  // ---------- search ----------

  let searchRun = 0;
  async function search() {
    const q = searchQ.trim();
    const run = ++searchRun;
    results = [];
    view?.clearSearch?.();
    if (!q) return;
    searching = 0.001;
    try {
      const iter = view.search({ query: q, ...searchOpts, mode: searchOpts.regex ? "regex" : undefined });
      for await (const r of iter) {
        if (run !== searchRun) return;
        if (r === "done") break;
        if (r.progress != null) searching = r.progress;
        if (r.subitems) {
          results = [
            ...results,
            {
              label: r.label || `§${r.index + 1}`,
              items: r.subitems.map((s: any) => ({ cfi: s.cfi, pre: s.excerpt?.pre ?? "", match: s.excerpt?.match ?? "", post: s.excerpt?.post ?? "" })),
            },
          ];
        }
      }
    } catch (e) {
      showToast("error", String(e));
    } finally {
      if (run === searchRun) searching = 0;
    }
  }

  // ---------- TTS ----------

  let ttsGen = 0;
  function speakText(text: string): Promise<void> {
    return new Promise((resolve) => {
      if (!text.trim()) return resolve();
      const u = new SpeechSynthesisUtterance(text);
      u.rate = prefs.ttsRate;
      u.pitch = prefs.ttsPitch;
      u.lang = book.language || navigator.language;
      const v = speechSynthesis.getVoices().find((x) => x.voiceURI === prefs.ttsVoice);
      if (v) u.voice = v;
      u.onend = () => resolve();
      u.onerror = () => resolve();
      speechSynthesis.speak(u);
    });
  }

  function ssmlSegments(ssml: string) {
    const doc = new DOMParser().parseFromString(ssml, "application/xml");
    const segs: { mark: string | null; text: string }[] = [{ mark: null, text: "" }];
    const walk = (n: Node) => {
      for (const c of Array.from(n.childNodes)) {
        if (c.nodeType === 3) segs[segs.length - 1].text += c.textContent ?? "";
        else if ((c as Element).localName === "mark") segs.push({ mark: (c as Element).getAttribute("name"), text: "" });
        else walk(c);
      }
    };
    walk(doc.documentElement);
    return segs.filter((s) => s.text.trim() || s.mark);
  }

  async function speak(fromSelection = false) {
    if (speaking) return stopSpeaking();
    if (!("speechSynthesis" in window)) return showToast("error", tr("read.reader.no_tts"));
    speaking = true;
    const gen = ++ttsGen;
    await view.initTTS("sentence");
    const range = fromSelection && sel ? view.renderer.getContents?.()[0]?.doc?.getSelection()?.getRangeAt(0) : null;
    let ssml: string | undefined = range ? view.tts.from(range) : view.tts.start();
    clearSelection();
    while (speaking && gen === ttsGen) {
      if (!ssml) {
        // End of section: move on and keep reading.
        const before = loc.index;
        await view.renderer.nextSection?.();
        await tick();
        if (loc.index === before) break;
        view.tts = null;
        await view.initTTS("sentence");
        ssml = view.tts?.start();
        if (!ssml) break;
      }
      for (const s of ssmlSegments(ssml)) {
        if (!speaking || gen !== ttsGen) break;
        if (s.mark) view.tts.setMark(s.mark);
        await speakText(s.text);
      }
      ssml = speaking && gen === ttsGen ? view.tts.next() : undefined;
    }
    if (gen === ttsGen) speaking = false;
  }

  function stopSpeaking() {
    speaking = false;
    ttsGen++;
    speechSynthesis.cancel();
  }

  // ---------- speed reading (RSVP) ----------

  let rsvp = $state<{ words: string[]; i: number; playing: boolean } | null>(null);
  let rsvpTimer: ReturnType<typeof setTimeout> | null = null;

  async function startRsvp() {
    const live = view.renderer.getContents?.().find((c: any) => c.index === loc.index)?.doc as Document | undefined;
    let text = "";
    const range = view.lastLocation?.range as Range | undefined;
    if (live && range) {
      const r = live.createRange();
      r.setStart(range.startContainer, range.startOffset);
      r.setEnd(live.body, live.body.childNodes.length);
      text = r.toString();
    } else {
      text = await chapterText();
    }
    const words = text.split(/\s+/).filter(Boolean);
    if (!words.length) return;
    rsvp = { words, i: 0, playing: true };
    tickRsvp();
  }

  function tickRsvp() {
    if (!rsvp || !rsvp.playing) return;
    const w = rsvp.words[rsvp.i] ?? "";
    // Longer words and sentence ends get a little more time.
    const base = 60000 / Math.max(100, prefs.rsvpWpm);
    const extra = /[.!?;:]$/.test(w) ? 2 : /,$/.test(w) ? 1.4 : w.length > 9 ? 1.3 : 1;
    rsvpTimer = setTimeout(() => {
      if (!rsvp) return;
      if (rsvp.i >= rsvp.words.length - 1) {
        rsvp.playing = false;
        void view.renderer?.nextSection?.();
        return;
      }
      rsvp.i++;
      tickRsvp();
    }, base * extra);
  }

  function toggleRsvp() {
    if (!rsvp) return;
    rsvp.playing = !rsvp.playing;
    if (rsvp.playing) tickRsvp();
    else if (rsvpTimer) clearTimeout(rsvpTimer);
  }

  function stopRsvp() {
    if (rsvpTimer) clearTimeout(rsvpTimer);
    rsvp = null;
  }

  /** Optimal recognition point: the letter the eye should land on. */
  function orp(w: string) {
    const k = w.length <= 1 ? 0 : w.length <= 5 ? 1 : w.length <= 9 ? 2 : 3;
    return [w.slice(0, k), w[k] ?? "", w.slice(k + 1)];
  }

  // ---------- misc ----------

  function addRuler(doc: Document) {
    const bar = doc.createElement("div");
    bar.setAttribute("aria-hidden", "true");
    bar.style.cssText = `position:fixed;left:0;right:0;height:${prefs.rulerLines * 1.6}em;pointer-events:none;background:rgb(255 220 0 / .14);border-top:1px solid rgb(255 180 0 / .5);border-bottom:1px solid rgb(255 180 0 / .5);z-index:9999;display:none`;
    doc.body.append(bar);
    doc.addEventListener("mousemove", (e) => {
      bar.style.display = "block";
      bar.style.top = `${e.clientY - 12}px`;
    });
  }

  async function pullSync() {
    try {
      const r = await invoke<{ percentage: number; device: string } | null>("read_kosync_pull", { id: book.id });
      if (r && r.percentage > loc.fraction + 0.005) remoteSync = r;
    } catch {
      /* sync off or offline */
    }
  }

  async function toggleFullscreen() {
    fullscreen = !fullscreen;
    await getCurrentWindow().setFullscreen(fullscreen).catch(() => undefined);
  }

  let wheelLock = false;
  function wheel(e: WheelEvent) {
    if (prefs.flow === "scrolled" || fixed) return;
    if (Math.abs(e.deltaY) < 20 && Math.abs(e.deltaX) < 20) return;
    e.preventDefault();
    if (wheelLock) return;
    wheelLock = true;
    setTimeout(() => (wheelLock = false), 260);
    if (e.deltaY > 0 || e.deltaX > 0) view.next();
    else view.prev();
  }

  function keys(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    if (target?.closest?.("input, textarea, select")) return;
    const k = e.key;
    const mod = e.metaKey || e.ctrlKey;
    if (rsvp && k === " ") {
      e.preventDefault();
      toggleRsvp();
      return;
    }
    if (k === "Escape") {
      if (rsvp) stopRsvp();
      else if (dict) dict = null;
      else if (popup) popup = null;
      else if (footnoteOpen) footnoteOpen = false;
      else if (sel) clearSelection();
      else if (panel) panel = "";
      else if (fullscreen) void toggleFullscreen();
      else onclose();
    } else if (mod && k === "f") {
      e.preventDefault();
      panel = "search";
    } else if (mod && k === "b") {
      e.preventDefault();
      void toggleBookmark();
    } else if (k === "ArrowRight" || k === "l") view?.goRight();
    else if (k === "ArrowLeft" || k === "h") view?.goLeft();
    else if (k === " " || k === "PageDown" || (k === "ArrowDown" && prefs.flow === "paginated")) {
      e.preventDefault();
      if (e.shiftKey) view?.prev();
      else view?.next();
    } else if (k === "PageUp" || (k === "ArrowUp" && prefs.flow === "paginated")) {
      e.preventDefault();
      view?.prev();
    } else if (k === "Home") view?.goToFraction(0);
    else if (k === "End") view?.goToFraction(1);
    else if (k === "]") view?.renderer?.nextSection?.();
    else if (k === "[") view?.renderer?.prevSection?.();
    else if (k === "t") panel = panel === "toc" ? "" : "toc";
    else if (k === "n") panel = panel === "notes" ? "" : "notes";
    else if (k === "s") panel = panel === "settings" ? "" : "settings";
    else if (k === "r") void speak();
    else if (k === "f") void toggleFullscreen();
    else if (k === "u") ui = !ui;
    else if (e.altKey && k === "ArrowLeft") view?.history?.back();
    else if (e.altKey && k === "ArrowRight") view?.history?.forward();
    else if (k === "+" || k === "=") {
      prefs.fontSize = Math.min(40, prefs.fontSize + 1);
      savePrefs();
      applyStyles();
    } else if (k === "-") {
      prefs.fontSize = Math.max(10, prefs.fontSize - 1);
      savePrefs();
      applyStyles();
    }
  }

  let clockTimer: ReturnType<typeof setInterval>;
  onMount(() => {
    void open();
    const upd = () => (clock = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
    upd();
    clockTimer = setInterval(upd, 30_000);
  });

  onDestroy(() => {
    resetReader();
    stopSpeaking();
    stopRsvp();
    clearInterval(clockTimer);
    if (saveTimer) clearTimeout(saveTimer);
    try {
      view?.close?.();
      view?.remove();
      footnote?.remove();
    } catch {
      /* already gone */
    }
    if (fullscreen) void getCurrentWindow().setFullscreen(false).catch(() => undefined);
  });

  function tocClick(item: any) {
    if (item.href) void view.goTo(item.href);
    if (window.innerWidth < 900) panel = "";
  }

  let sortedNotes = $derived([...notes].sort((a, b) => (a.progress ?? 0) - (b.progress ?? 0)));
  let voices = $state<SpeechSynthesisVoice[]>([]);
  onMount(() => {
    const load = () => (voices = speechSynthesis?.getVoices?.() ?? []);
    load();
    speechSynthesis?.addEventListener?.("voiceschanged", load);
    return () => speechSynthesis?.removeEventListener?.("voiceschanged", load);
  });
  const fmtMin = (m: number) => (m < 60 ? `${Math.max(1, Math.round(m))} min` : `${Math.floor(m / 60)} h ${Math.round(m % 60)} min`);
</script>

<svelte:window onkeydown={keys} />

<div class="er" class:dark={isDarkTheme(prefs)} style:--rbg={(THEMES[prefs.theme] ?? THEMES.light).bg} style:--rfg={(THEMES[prefs.theme] ?? THEMES.light).fg}>
  {#if ui}
    <header class="top">
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" onclick={onclose} aria-label={$t("read.close") as string}><ReadIcon name="back" size={18} /></button>
      <div class="ttl"><b>{book.title}</b>{#if loc.chapter}<span>{loc.chapter}</span>{/if}</div>
      <span class="grow"></span>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={panel === "toc"} onclick={() => (panel = panel === "toc" ? "" : "toc")} title={$t("read.reader.toc") as string} aria-label={$t("read.reader.toc") as string}><ReadIcon name="toc" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={panel === "search"} onclick={() => (panel = panel === "search" ? "" : "search")} title={$t("read.reader.search") as string} aria-label={$t("read.reader.search") as string}><ReadIcon name="search" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={panel === "notes"} onclick={() => (panel = panel === "notes" ? "" : "notes")} title={$t("read.reader.notes") as string} aria-label={$t("read.reader.notes") as string}><ReadIcon name="highlighter" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={isBookmarked} onclick={toggleBookmark} title={$t("read.reader.bookmark") as string} aria-label={$t("read.reader.bookmark") as string}><ReadIcon name={isBookmarked ? "bookmark-fill" : "bookmark"} size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={speaking} onclick={() => speak()} title={$t("read.reader.read_aloud") as string} aria-label={$t("read.reader.read_aloud") as string}><ReadIcon name={speaking ? "stop" : "speak"} size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={!!rsvp} onclick={() => (rsvp ? stopRsvp() : startRsvp())} title={$t("read.reader.rsvp") as string} aria-label={$t("read.reader.rsvp") as string}><ReadIcon name="flame" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={panel === "ai"} onclick={() => (panel = panel === "ai" ? "" : "ai")} title={$t("read.reader.ai") as string} aria-label={$t("read.reader.ai") as string}><ReadIcon name="sparkles" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={panel === "settings"} onclick={() => (panel = panel === "settings" ? "" : "settings")} title={$t("read.reader.settings") as string} aria-label={$t("read.reader.settings") as string}><ReadIcon name="type" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" onclick={toggleFullscreen} title={$t("read.reader.fullscreen") as string} aria-label={$t("read.reader.fullscreen") as string}><ReadIcon name={fullscreen ? "minimize" : "maximize"} size={17} /></button>
    </header>
  {/if}

  <div class="body">
    {#if panel === "toc" || panel === "search" || panel === "notes"}
      <aside class="side left">
        {#if panel === "toc"}
          <h3>{$t("read.reader.toc")}</h3>
          {#snippet tree(items: any[], depth: number)}
            <ul class="toc" style:--d={depth}>
              {#each items as it, i (i)}
                <li>
                  <button class:cur={it.href && loc.href && it.href === loc.href} onclick={() => tocClick(it)}>{it.label}</button>
                  {#if it.subitems?.length}{@render tree(it.subitems, depth + 1)}{/if}
                </li>
              {/each}
            </ul>
          {/snippet}
          {#if toc.length}{@render tree(toc, 0)}{:else}<p class="muted">{$t("read.reader.no_toc")}</p>{/if}
        {:else if panel === "search"}
          <form onsubmit={(e) => { e.preventDefault(); void search(); }}>
            <input class="input" bind:value={searchQ} placeholder={$t("read.reader.search_book") as string} />
          </form>
          <div class="opts">
            <label><input type="checkbox" bind:checked={searchOpts.matchCase} /> Aa</label>
            <label><input type="checkbox" bind:checked={searchOpts.matchWholeWords} /> {$t("read.reader.whole_words")}</label>
            <label><input type="checkbox" bind:checked={searchOpts.matchDiacritics} /> {$t("read.reader.diacritics")}</label>
            <label><input type="checkbox" bind:checked={searchOpts.regex} /> .*</label>
          </div>
          {#if searching}<div class="r-bar"><i style:width="{Math.round(searching * 100)}%"></i></div>{/if}
          <p class="muted">{$t("read.reader.results", { count: results.reduce((a, r) => a + r.items.length, 0) })}</p>
          <div class="results">
            {#each results as g, gi (gi)}
              <h4>{g.label}</h4>
              {#each g.items as it (it.cfi)}
                <button onclick={() => view.goTo(it.cfi)}>…{it.pre}<mark>{it.match}</mark>{it.post}…</button>
              {/each}
            {/each}
          </div>
        {:else}
          <h3>{$t("read.reader.notes")} <span class="muted">{notes.length}</span></h3>
          {#each sortedNotes as n (n.id)}
            <div class="note" style:--c={COLORS[n.color] ?? n.color}>
              <button onclick={() => view.goTo(n.locator)}>
                {#if n.kind === "bookmark"}<span class="bm"><ReadIcon name="bookmark-fill" size={12} /> {n.text || $t("read.reader.bookmark")}</span>{:else}<q>{n.text}</q>{/if}
                {#if n.note}<p>{n.note}</p>{/if}
                <span class="muted">{n.chapter} · {Math.round((n.progress ?? 0) * 100)}%</span>
              </button>
              <button class="x" onclick={() => removeAnnotation(n)} aria-label={$t("read.delete") as string}><ReadIcon name="trash" size={13} /></button>
            </div>
          {:else}
            <p class="muted">{$t("read.notes.none")}</p>
          {/each}
        {/if}
      </aside>
    {/if}

    <div class="stage" bind:this={host} style:filter="var(--reader-filter)"></div>

    {#if panel === "settings" || panel === "ai"}
      <aside class="side right">
        {#if panel === "settings"}
          <ReaderSettings onchange={applyStyles} {fixed} />
          <h4 class="h4">{$t("read.reader.read_aloud")}</h4>
          <label class="rowx">{$t("read.reader.voice")}
            <select class="input" bind:value={prefs.ttsVoice} onchange={savePrefs}>
              <option value="">{$t("read.reader.default_voice")}</option>
              {#each voices as v (v.voiceURI)}<option value={v.voiceURI}>{v.name} ({v.lang})</option>{/each}
            </select>
          </label>
          <label class="rowx">{$t("read.reader.speed")} {prefs.ttsRate.toFixed(1)}×
            <input type="range" min="0.5" max="2.5" step="0.1" bind:value={prefs.ttsRate} onchange={savePrefs} />
          </label>
          <label class="rowx ck2"><input type="checkbox" bind:checked={prefs.incognito} onchange={savePrefs} /> {$t("read.reader.incognito")}</label>
          <label class="rowx">{$t("read.reader.translate_to")}
            <input class="input" bind:value={prefs.translateTo} onchange={savePrefs} placeholder={navigator.language} />
          </label>
        {:else}
          <h3>{$t("read.reader.ai")}</h3>
          <p class="muted">{$t("read.reader.ai_hint")}</p>
          <div class="aib">
            <button class="btn" onclick={() => ai("recap")}>{$t("read.ai.recap")}</button>
            <button class="btn" onclick={() => ai("summarize")}>{$t("read.ai.summarize")}</button>
            <button class="btn" onclick={() => ai("characters")}>{$t("read.ai.characters")}</button>
            <button class="btn" onclick={() => ai("quiz")}>{$t("read.ai.quiz")}</button>
          </div>
          <form class="ask" onsubmit={(e) => { e.preventDefault(); if (aiQuestion.trim()) void ai("ask"); }}>
            <input class="input" bind:value={aiQuestion} placeholder={$t("read.ai.ask_placeholder") as string} />
            <button class="btn btn-primary" type="submit">{$t("read.ai.ask")}</button>
          </form>
          {#if aiOut}
            <div class="aiout">
              {#if aiOut.loading}<ReadIcon name="refresh" size={16} spin />{:else}<p>{aiOut.text}</p>
                <button class="r-chip" onclick={() => navigator.clipboard.writeText(aiOut?.text ?? "")}>{$t("read.reader.copy")}</button>{/if}
            </div>
          {/if}
        {/if}
      </aside>
    {/if}
  </div>

  {#if ui}
    <footer class="bottom">
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib sm" onclick={() => view?.goLeft()} aria-label={$t("read.reader.prev") as string}><ReadIcon name="left" size={16} /></button>
      <input
        class="slider"
        class:rtl={dir === "rtl"}
        type="range"
        min="0"
        max="1000"
        value={Math.round(loc.fraction * 1000)}
        onchange={(e) => view?.goToFraction(Number((e.target as HTMLInputElement).value) / 1000)}
        aria-label={$t("read.reader.progress") as string}
      />
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib sm" onclick={() => view?.goRight()} aria-label={$t("read.reader.next") as string}><ReadIcon name="right" size={16} /></button>
      <span class="st">{Math.round(loc.fraction * 100)}%</span>
      {#if loc.pages}<span class="st">{$t("read.reader.loc", { current: loc.page + 1, total: loc.pages })}</span>{/if}
      {#if loc.left}<span class="st" title={$t("read.reader.left_book", { time: fmtMin(loc.leftBook) }) as string}>{$t("read.reader.left_chapter", { time: fmtMin(loc.left) })}</span>{/if}
      {#if prefs.showClock}<span class="st">{clock}</span>{/if}
    </footer>
  {/if}

  {#if loading}
    <div class="cover"><ReadIcon name="refresh" size={26} spin /></div>
  {:else if error}
    <div class="cover">
      <ReadIcon name="alert" size={28} />
      <p>{error}</p>
      <button class="btn" onclick={onclose}>{$t("read.close")}</button>
    </div>
  {/if}

  {#if rsvp}
    {@const w = orp(rsvp.words[rsvp.i] ?? "")}
    <div class="rsvp" role="region" aria-label={$t("read.reader.rsvp") as string}>
      <div class="word"><span class="a">{w[0]}</span><span class="p">{w[1]}</span><span class="b">{w[2]}</span></div>
      <div class="r-bar"><i style:width="{Math.round((rsvp.i / rsvp.words.length) * 100)}%"></i></div>
      <div class="ctl">
        <button class="btn" onclick={() => { if (rsvp) rsvp.i = Math.max(0, rsvp.i - 10); }}>−10</button>
        <button class="btn btn-primary" onclick={toggleRsvp}>{rsvp.playing ? $t("read.reader.pause") : $t("read.reader.play")}</button>
        <label>{prefs.rsvpWpm} wpm <input type="range" min="150" max="1000" step="25" bind:value={prefs.rsvpWpm} onchange={savePrefs} /></label>
        <button class="btn" onclick={stopRsvp}>{$t("read.close")}</button>
      </div>
    </div>
  {/if}

  {#if remoteSync}
    <div class="sync">
      <span>{$t("read.reader.sync_ahead", { pct: Math.round(remoteSync.percentage * 100), device: remoteSync.device })}</span>
      <button class="btn btn-primary" onclick={() => { view.goToFraction(remoteSync!.percentage); remoteSync = null; }}>{$t("read.reader.sync_jump")}</button>
      <button class="btn" onclick={() => (remoteSync = null)}>{$t("read.reader.sync_stay")}</button>
    </div>
  {/if}

  {#if sel}
    <div class="selpop" style:left="{Math.max(160, Math.min(window.innerWidth - 160, sel.x))}px" style:top="{Math.max(60, sel.y - 58)}px" role="toolbar">
      <div class="sw">
        {#each Object.entries(COLORS) as [name, c] (name)}
          <button style:background={c} onclick={() => annotate(name)} aria-label={name}></button>
        {/each}
        <button class="u" onclick={() => annotate("blue", "underline")} title={$t("read.reader.underline") as string} aria-label={$t("read.reader.underline") as string}>U</button>
      </div>
      <div class="acts">
        <button onclick={() => { const n = prompt(tr("read.reader.note_prompt")); if (n) void annotate("yellow", "highlight", n); }}><ReadIcon name="note" size={14} />{$t("read.reader.note")}</button>
        <button onclick={copySel}><ReadIcon name="copy" size={14} />{$t("read.reader.copy")}</button>
        <button onclick={copyQuote}><ReadIcon name="quote" size={14} />{$t("read.reader.quote")}</button>
        <button onclick={() => lookup(sel!.text, sel!.context)}><ReadIcon name="book" size={14} />{$t("read.reader.define")}</button>
        <button onclick={() => ai("translate", sel!.text)}><ReadIcon name="translate" size={14} />{$t("read.reader.translate")}</button>
        <button onclick={() => ai("explain", sel!.text)}><ReadIcon name="sparkles" size={14} />{$t("read.reader.explain")}</button>
        <button onclick={() => { searchQ = sel!.text; panel = "search"; void search(); }}><ReadIcon name="search" size={14} />{$t("read.reader.search")}</button>
        <button onclick={() => speak(true)}><ReadIcon name="speak" size={14} />{$t("read.reader.speak")}</button>
      </div>
    </div>
  {/if}

  {#if popup}
    {@const a = popup.ann}
    <div class="annpop" style:left="{Math.max(170, Math.min(window.innerWidth - 170, popup.x))}px" style:top="{Math.min(window.innerHeight - 220, popup.y)}px">
      <div class="sw">
        {#each Object.entries(COLORS) as [name, c] (name)}
          <button style:background={c} class:on={a.color === name} onclick={() => updateAnnotation(a, { color: name })} aria-label={name}></button>
        {/each}
      </div>
      <div class="seg">
        {#each STYLES as s (s)}<button class:on={a.style === s} onclick={() => updateAnnotation(a, { style: s })}>{$t(`read.reader.style_${s}`)}</button>{/each}
      </div>
      <textarea class="input" rows="3" value={a.note} placeholder={$t("read.reader.note_prompt") as string} onchange={(e) => updateAnnotation(a, { note: (e.target as HTMLTextAreaElement).value, kind: (e.target as HTMLTextAreaElement).value ? "note" : "highlight" })}></textarea>
      <div class="r-line">
        <button class="btn" onclick={() => navigator.clipboard.writeText(a.text)}>{$t("read.reader.copy")}</button>
        <button class="btn btn-destructive" onclick={() => removeAnnotation(a)}>{$t("read.delete")}</button>
        <button class="btn" onclick={() => (popup = null)}>{$t("read.close")}</button>
      </div>
    </div>
  {/if}

  {#if dict}
    <div class="dict" role="region" aria-label={dict.word}>
      <header><b>{dict.word}</b><span class="grow"></span>
        <button class="r-chip" onclick={saveWord}><ReadIcon name="plus" size={12} /> {$t("read.reader.save_word")}</button>
        <button data-agent-actions="click" use:agentActions={["click"]} class="ib sm" onclick={() => (dict = null)} aria-label={$t("read.close") as string}><ReadIcon name="close" size={14} /></button>
      </header>
      {#if dict.loading}<ReadIcon name="refresh" size={16} spin />{:else}
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <div class="defs">{@html dict.html}</div>
        {#if dict.wiki}<h5>Wikipedia</h5><p>{dict.wiki}</p>{/if}
      {/if}
    </div>
  {/if}

  <div class="foot" class:open={footnoteOpen}>
    <header><b>{$t("read.reader.footnote")}</b><span class="grow"></span><button data-agent-actions="click" use:agentActions={["click"]} class="ib sm" onclick={() => (footnoteOpen = false)} aria-label={$t("read.close") as string}><ReadIcon name="close" size={14} /></button></header>
    <div class="fh" bind:this={footHost}></div>
  </div>
</div>

<style>
  .er {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--rbg);
    color: var(--rfg);
  }
  .top,
  .bottom {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--rbg) 92%, var(--rfg) 8%);
    border-bottom: 1px solid color-mix(in srgb, var(--rfg) 12%, transparent);
    z-index: 3;
  }
  .bottom {
    border-bottom: none;
    border-top: 1px solid color-mix(in srgb, var(--rfg) 12%, transparent);
  }
  .ttl {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.2;
  }
  .ttl b,
  .ttl span {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 44vw;
  }
  .ttl span {
    font-size: 12px;
    opacity: 0.7;
  }
  .grow {
    flex: 1;
  }
  .ib {
    all: unset;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 8px;
    cursor: pointer;
    color: inherit;
  }
  .ib.sm {
    width: 28px;
    height: 28px;
  }
  .ib:hover,
  .ib.on {
    background: color-mix(in srgb, var(--rfg) 12%, transparent);
  }
  .ib:focus-visible {
    outline: 2px solid var(--accent);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .stage {
    flex: 1;
    min-width: 0;
    position: relative;
  }
  .stage :global(foliate-view) {
    display: block;
    width: 100%;
    height: 100%;
  }
  .side {
    width: 320px;
    flex-shrink: 0;
    overflow-y: auto;
    padding: 12px 14px;
    background: var(--surface);
    color: var(--text);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .side.left {
    border-right: 1px solid var(--content-border);
  }
  .side.right {
    border-left: 1px solid var(--content-border);
  }
  .side h3 {
    margin: 0;
    font-size: var(--text-base);
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .toc {
    list-style: none;
    margin: 0;
    padding: 0 0 0 calc(var(--d) * 10px);
  }
  .toc button {
    all: unset;
    display: block;
    width: 100%;
    box-sizing: border-box;
    padding: 5px 6px;
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
  }
  .toc button:hover {
    background: var(--fill-1);
  }
  .toc button.cur {
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-weight: 600;
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 12px;
  }
  .results h4 {
    margin: 8px 0 2px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .results button {
    all: unset;
    display: block;
    padding: 5px 6px;
    border-radius: 6px;
    font-size: 12.5px;
    line-height: 1.4;
    cursor: pointer;
  }
  .results button:hover {
    background: var(--fill-1);
  }
  .results mark {
    background: #f5d00088;
    color: inherit;
  }
  .note {
    display: flex;
    gap: 4px;
    border-left: 3px solid var(--c, #f5d000);
    padding-left: 8px;
  }
  .note > button:first-child {
    all: unset;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
    cursor: pointer;
    font-size: 13px;
    min-width: 0;
  }
  .note q {
    font-family: Georgia, serif;
  }
  .note p {
    margin: 0;
    font-size: 12px;
  }
  .note .x {
    all: unset;
    cursor: pointer;
    color: var(--text-muted);
    padding: 2px;
  }
  .bm {
    display: flex;
    gap: 4px;
    align-items: center;
    font-weight: 600;
  }
  .h4 {
    margin: 10px 0 0;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .rowx {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
  .aib {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .ask {
    display: flex;
    gap: 6px;
  }
  .ask .input {
    flex: 1;
  }
  .aiout {
    white-space: pre-wrap;
    line-height: 1.5;
    font-size: 13px;
    padding: 10px;
    border-radius: 8px;
    background: var(--fill-1);
  }
  .slider {
    flex: 1;
    accent-color: var(--accent);
  }
  .slider.rtl {
    direction: rtl;
  }
  .st {
    margin-left: 6px;
    font-size: 12px;
    opacity: 0.75;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .cover {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    align-content: center;
    gap: 10px;
    background: var(--rbg);
    z-index: 4;
    text-align: center;
    padding: 20px;
  }
  .rsvp {
    position: absolute;
    inset: 20% 10%;
    z-index: 7;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 18px;
    padding: 24px;
    border-radius: 16px;
    background: var(--rbg);
    box-shadow: 0 10px 40px rgb(0 0 0 / 0.35);
  }
  .rsvp .word {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    font-family: Georgia, serif;
    font-size: clamp(28px, 5vw, 56px);
  }
  .rsvp .a {
    text-align: right;
  }
  .rsvp .p {
    color: #e0452e;
  }
  .rsvp .ctl {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
    justify-content: center;
    font-size: 13px;
  }
  .ck2 {
    flex-direction: row !important;
    align-items: center;
  }
  .sync {
    position: absolute;
    left: 50%;
    bottom: 56px;
    transform: translateX(-50%);
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 6px 24px rgb(0 0 0 / 0.3);
    font-size: 13px;
  }
  .selpop,
  .annpop {
    position: fixed;
    z-index: 10;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 8px 28px rgb(0 0 0 / 0.35);
  }
  .annpop {
    width: 320px;
  }
  .sw {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .sw button {
    all: unset;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    cursor: pointer;
    box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.15);
  }
  .sw button.on {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .sw .u {
    width: auto;
    padding: 0 6px;
    font-weight: 700;
    text-decoration: underline;
    background: var(--fill-1);
    border-radius: 6px;
    line-height: 20px;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    max-width: 440px;
  }
  .acts button {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 7px;
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
  }
  .acts button:hover {
    background: var(--fill-1);
  }
  .seg {
    display: flex;
    gap: 4px;
  }
  .seg button {
    all: unset;
    flex: 1;
    text-align: center;
    padding: 4px;
    font-size: 11px;
    border-radius: 6px;
    background: var(--fill-1);
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent);
    color: #fff;
  }
  .dict,
  .foot {
    position: fixed;
    right: 16px;
    bottom: 60px;
    z-index: 11;
    width: 380px;
    max-height: 60vh;
    overflow-y: auto;
    padding: 10px 14px;
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 8px 28px rgb(0 0 0 / 0.35);
    font-size: 13px;
    line-height: 1.5;
  }
  .dict header,
  .foot header {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .defs :global(ol) {
    margin: 4px 0 8px;
    padding-left: 18px;
  }
  .defs :global(p) {
    margin: 4px 0 0;
  }
  .foot {
    display: none;
    left: 50%;
    right: auto;
    transform: translateX(-50%);
    width: min(620px, 90vw);
    height: 40vh;
  }
  .foot.open {
    display: flex;
    flex-direction: column;
  }
  .fh {
    flex: 1;
    min-height: 0;
  }
  .fh :global(foliate-view) {
    display: block;
    width: 100%;
    height: 100%;
  }
  @media (max-width: 900px) {
    .side {
      position: absolute;
      top: 47px;
      bottom: 45px;
      z-index: 5;
    }
    .side.right {
      right: 0;
    }
  }
</style>
