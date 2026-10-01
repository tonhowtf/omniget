<script lang="ts">
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { invoke } from "@tauri-apps/api/core";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import { prefs, savePrefs } from "$lib/read/prefs.svelte";
  import { reader, resetReader } from "$lib/read/reader-state.svelte";
  import ReadIcon from "./ReadIcon.svelte";
  import { agentActions } from "$lib/native-agent/actions";

  let {
    title,
    pages,
    startPage = 0,
    bookId = null,
    modeOverride = "",
    hasPrev = false,
    hasNext = false,
    onprevchapter,
    onnextchapter,
    onpage,
    onmode,
    onsetcover,
    onclose,
  }: {
    title: string;
    pages: string[];
    startPage?: number;
    bookId?: number | null;
    modeOverride?: string;
    hasPrev?: boolean;
    hasNext?: boolean;
    onprevchapter?: () => void;
    onnextchapter?: () => void;
    onpage?: (page: number, total: number) => void;
    onmode?: (mode: string) => void;
    onsetcover?: (dataUrl: string) => void;
    onclose: () => void;
  } = $props();

  type Mode = "ltr" | "rtl" | "vertical" | "webtoon";
  let mode = $state<Mode>((untrack(() => modeOverride) as Mode) || prefs.comicMode);
  let page = $state(untrack(() => Math.min(Math.max(0, startPage), Math.max(0, pages.length - 1))));
  let ui = $state(true);
  let panel = $state<"" | "settings" | "pages">("");
  let zoom = $state(1);
  let pan = $state({ x: 0, y: 0 });
  let dims = $state<Record<number, { w: number; h: number }>>({});
  let cropped = $state<Record<number, string>>({});
  let atEnd = $state(false);
  let fullscreen = $state(false);
  let clock = $state("");
  let strip: HTMLDivElement | undefined = $state();
  let stage: HTMLDivElement | undefined = $state();
  let viewport = $state({ w: window.innerWidth, h: window.innerHeight });
  let translated = $state<Record<number, string>>({});
  let showTranslated = $state(true);
  let translating = $state(false);

  async function translateShown() {
    translating = true;
    try {
      for (const i of mode === "webtoon" ? [page] : shown) {
        if (translated[i]) continue;
        translated[i] = await invoke<string>("read_translate_page", {
          imageUrl: pages[i],
          server: prefs.translatorUrl,
          targetLang: prefs.translatorLang,
        });
      }
      showTranslated = true;
    } catch (e) {
      showToast("error", String(e));
    } finally {
      translating = false;
    }
  }

  $effect(() => {
    reader.kind = "comic";
    reader.title = title;
    reader.progress = pages.length ? (page + 1) / pages.length : 0;
    reader.location = `${page + 1}/${pages.length}`;
  });

  function isWide(i: number) {
    const d = dims[i];
    return !!d && d.w > d.h * 1.1;
  }

  let double = $derived(
    mode !== "webtoon" &&
      mode !== "vertical" &&
      (prefs.comicSpread === "double" || (prefs.comicSpread === "auto" && viewport.w > viewport.h * 1.15)),
  );

  let spreads = $derived.by(() => {
    const out: number[][] = [];
    let i = 0;
    while (i < pages.length) {
      if (!double || (prefs.comicCoverAlone && i === 0) || isWide(i) || i === pages.length - 1 || isWide(i + 1)) {
        out.push([i]);
        i += 1;
      } else {
        out.push([i, i + 1]);
        i += 2;
      }
    }
    return out;
  });
  let spreadIndex = $derived(Math.max(0, spreads.findIndex((s) => s.includes(page))));
  let shown = $derived(spreads[spreadIndex] ?? [page]);
  let rtl = $derived(mode === "rtl");

  function report() {
    onpage?.(page, pages.length);
  }

  function goSpread(n: number) {
    if (n < 0) {
      if (hasPrev) onprevchapter?.();
      return;
    }
    if (n >= spreads.length) {
      if (atEnd && hasNext) onnextchapter?.();
      else if (hasNext && prefs.comicAutoNext && atEnd) onnextchapter?.();
      else atEnd = true;
      return;
    }
    atEnd = false;
    page = spreads[n][0];
    zoom = 1;
    pan = { x: 0, y: 0 };
    report();
  }

  const next = () => (mode === "webtoon" ? scrollBy(0.85) : goSpread(spreadIndex + 1));
  const prev = () => (mode === "webtoon" ? scrollBy(-0.85) : goSpread(spreadIndex - 1));
  const left = () => (rtl ? next() : prev());
  const right = () => (rtl ? prev() : next());

  function scrollBy(f: number) {
    strip?.scrollBy({ top: strip.clientHeight * f, behavior: "smooth" });
  }

  function jump(p: number) {
    page = Math.max(0, Math.min(pages.length - 1, p));
    atEnd = false;
    if (mode === "webtoon") {
      void tick().then(() => strip?.querySelector(`[data-p="${page}"]`)?.scrollIntoView({ block: "start" }));
    }
    report();
  }

  function setMode(m: Mode) {
    mode = m;
    prefs.comicMode = m;
    savePrefs();
    onmode?.(m);
    zoom = 1;
    if (m === "webtoon") void tick().then(() => jump(page));
  }

  let io: IntersectionObserver | null = null;
  function observeStrip() {
    io?.disconnect();
    if (mode !== "webtoon" || !strip) return;
    io = new IntersectionObserver(
      (entries) => {
        const vis = entries.filter((e) => e.isIntersecting).map((e) => Number((e.target as HTMLElement).dataset.p));
        if (!vis.length) return;
        const p = Math.min(...vis);
        if (p !== page) {
          page = p;
          report();
        }
        if (vis.includes(pages.length - 1)) atEnd = true;
      },
      { root: strip, threshold: 0.35 },
    );
    strip.querySelectorAll("[data-p]").forEach((el) => io!.observe(el));
  }
  $effect(() => {
    void mode;
    void pages.length;
    void strip;
    void tick().then(observeStrip);
  });

  function measure(i: number, img: HTMLImageElement) {
    if (!dims[i]) dims[i] = { w: img.naturalWidth, h: img.naturalHeight };
    if (!modeOverride && i < 4 && mode !== "webtoon") {
      const tall = Object.values(dims).filter((d) => d.h > d.w * 2.4).length;
      if (tall >= 2) setMode("webtoon");
    }
  }

  const preloaded = new Map<number, HTMLImageElement>();
  $effect(() => {
    const n = prefs.comicPreload;
    for (let k = 1; k <= n; k++) {
      const i = page + k * (double ? 2 : 1);
      if (i < pages.length && !preloaded.has(i)) {
        const im = new Image();
        im.decoding = "async";
        im.src = pages[i];
        im.onload = () => measure(i, im);
        preloaded.set(i, im);
      }
    }
  });

  async function cropPage(i: number) {
    if (cropped[i] !== undefined) return;
    cropped[i] = "";
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.src = pages[i];
    try {
      await img.decode();
    } catch {
      return;
    }
    const w = img.naturalWidth;
    const h = img.naturalHeight;
    const sc = Math.min(1, 400 / Math.max(w, h));
    const cw = Math.max(1, Math.round(w * sc));
    const ch = Math.max(1, Math.round(h * sc));
    const c = document.createElement("canvas");
    c.width = cw;
    c.height = ch;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;
    ctx.drawImage(img, 0, 0, cw, ch);
    let data: Uint8ClampedArray;
    try {
      data = ctx.getImageData(0, 0, cw, ch).data;
    } catch {
      return;
    }
    const lum = (x: number, y: number) => {
      const o = (y * cw + x) * 4;
      return (data[o] * 3 + data[o + 1] * 6 + data[o + 2]) / 10;
    };
    const bg = lum(0, 0);
    const blankRow = (y: number) => {
      for (let x = 0; x < cw; x += 2) if (Math.abs(lum(x, y) - bg) > 24) return false;
      return true;
    };
    const blankCol = (x: number) => {
      for (let y = 0; y < ch; y += 2) if (Math.abs(lum(x, y) - bg) > 24) return false;
      return true;
    };
    let top = 0, bottom = 0, l = 0, r = 0;
    while (top < ch * 0.1 && blankRow(top)) top++;
    while (bottom < ch * 0.1 && blankRow(ch - 1 - bottom)) bottom++;
    while (l < cw * 0.1 && blankCol(l)) l++;
    while (r < cw * 0.1 && blankCol(cw - 1 - r)) r++;
    if (top + bottom + l + r < 3) return;
    const out = document.createElement("canvas");
    const sx = l / sc, sy = top / sc, sw = w - (l + r) / sc, sh = h - (top + bottom) / sc;
    out.width = Math.round(sw);
    out.height = Math.round(sh);
    out.getContext("2d")?.drawImage(img, sx, sy, sw, sh, 0, 0, sw, sh);
    out.toBlob((b) => {
      if (b) cropped[i] = URL.createObjectURL(b);
    }, "image/jpeg", 0.92);
  }

  function srcOf(i: number) {
    if (showTranslated && translated[i]) return translated[i];
    if (prefs.comicCrop) {
      void cropPage(i);
      return cropped[i] || pages[i];
    }
    return pages[i];
  }

  let filter = $derived(
    [
      prefs.comicFilter === "grayscale" ? "grayscale(1)" : "",
      prefs.comicFilter === "sepia" ? "sepia(0.7)" : "",
      prefs.comicFilter === "invert" ? "invert(1) hue-rotate(180deg)" : "",
      prefs.brightness !== 100 ? `brightness(${prefs.brightness / 100})` : "",
    ]
      .filter(Boolean)
      .join(" ") || "none",
  );

  function tap(e: MouseEvent) {
    if (e.button !== 0 || dragged) return;
    if (panel) {
      panel = "";
      return;
    }
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width;
    const y = (e.clientY - rect.top) / rect.height;
    const zones = prefs.tapZones;
    if (zones === "off") {
      ui = !ui;
      return;
    }
    if (mode === "vertical" || mode === "webtoon") {
      if (y < 0.3) prev();
      else if (y > 0.7) next();
      else ui = !ui;
      return;
    }
    const edge = zones === "edge" ? 0.15 : zones === "kindle" ? 0.25 : 0.33;
    if (zones === "kindle") {
      if (x < edge) left();
      else if (y < 0.18) ui = !ui;
      else right();
      return;
    }
    if (x < edge) left();
    else if (x > 1 - edge) right();
    else ui = !ui;
  }

  let dragging = false;
  let dragged = false;
  let dragStart = { x: 0, y: 0, px: 0, py: 0 };
  function down(e: PointerEvent) {
    if (zoom <= 1 || mode === "webtoon") return;
    dragging = true;
    dragged = false;
    dragStart = { x: e.clientX, y: e.clientY, px: pan.x, py: pan.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!dragging) return;
    const dx = e.clientX - dragStart.x;
    const dy = e.clientY - dragStart.y;
    if (Math.abs(dx) + Math.abs(dy) > 4) dragged = true;
    pan = { x: dragStart.px + dx, y: dragStart.py + dy };
  }
  function up() {
    dragging = false;
    setTimeout(() => (dragged = false), 0);
  }

  function wheel(e: WheelEvent) {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      zoom = Math.max(1, Math.min(6, zoom * (e.deltaY < 0 ? 1.12 : 0.89)));
      if (zoom === 1) pan = { x: 0, y: 0 };
      return;
    }
    if (mode === "webtoon") return;
    if (zoom > 1) {
      pan = { x: pan.x - e.deltaX, y: pan.y - e.deltaY };
      return;
    }
    if (Math.abs(e.deltaY) < 24) return;
    e.preventDefault();
    if (wheelLock) return;
    wheelLock = true;
    setTimeout(() => (wheelLock = false), 220);
    if (e.deltaY > 0) next();
    else prev();
  }
  let wheelLock = false;

  function keys(e: KeyboardEvent) {
    if ((e.target as HTMLElement)?.closest("input, select, textarea")) return;
    const k = e.key;
    if (k === "Escape") {
      if (panel) panel = "";
      else if (fullscreen) void toggleFullscreen();
      else onclose();
    } else if (k === "ArrowRight") right();
    else if (k === "ArrowLeft") left();
    else if (k === "ArrowDown" || k === "PageDown" || k === " ") {
      if (mode === "webtoon" && k === "ArrowDown") return;
      e.preventDefault();
      next();
    } else if (k === "ArrowUp" || k === "PageUp") {
      if (mode === "webtoon" && k === "ArrowUp") return;
      e.preventDefault();
      prev();
    } else if (k === "Home") jump(0);
    else if (k === "End") jump(pages.length - 1);
    else if (k === "f") void toggleFullscreen();
    else if (k === "+" || k === "=") zoom = Math.min(6, zoom * 1.2);
    else if (k === "-") zoom = Math.max(1, zoom / 1.2);
    else if (k === "0") {
      zoom = 1;
      pan = { x: 0, y: 0 };
    } else if (k === "d") {
      prefs.comicSpread = prefs.comicSpread === "double" ? "single" : "double";
      savePrefs();
    } else if (k === "m") {
      const order: Mode[] = ["rtl", "ltr", "vertical", "webtoon"];
      setMode(order[(order.indexOf(mode) + 1) % order.length]);
    } else if (k === "[") onprevchapter?.();
    else if (k === "]") onnextchapter?.();
    else if (k === "h") ui = !ui;
  }

  async function toggleFullscreen() {
    fullscreen = !fullscreen;
    try {
      await getCurrentWindow().setFullscreen(fullscreen);
    } catch {
    }
  }

  async function useAsCover() {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.src = pages[page];
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    c.getContext("2d")?.drawImage(img, 0, 0);
    onsetcover?.(c.toDataURL("image/jpeg", 0.9));
  }

  const resize = () => (viewport = { w: window.innerWidth, h: window.innerHeight });
  let clockTimer: ReturnType<typeof setInterval>;
  onMount(() => {
    window.addEventListener("resize", resize);
    const upd = () => (clock = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
    upd();
    clockTimer = setInterval(upd, 30_000);
    stage?.focus();
    if (mode === "webtoon") void tick().then(() => jump(page));
    report();
  });
  onDestroy(() => {
    resetReader();
    window.removeEventListener("resize", resize);
    clearInterval(clockTimer);
    io?.disconnect();
    for (const u of Object.values(cropped)) if (u) URL.revokeObjectURL(u);
    if (fullscreen) void getCurrentWindow().setFullscreen(false).catch(() => undefined);
  });

  let pct = $derived(pages.length ? Math.round(((page + 1) / pages.length) * 100) : 0);
</script>

<svelte:window onkeydown={keys} />

<div class="cr" style:--bg={prefs.comicBackground} class:hide-ui={!ui}>
  {#if ui}
    <header class="top">
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" onclick={onclose} aria-label={$t("read.close") as string}><ReadIcon name="back" size={18} /></button>
      <span class="tt">{title}</span>
      <span class="grow"></span>
      {#if hasPrev}<button data-agent-actions="click" use:agentActions={["click"]} class="ib" onclick={onprevchapter} title={$t("read.comic.prev_chapter") as string} aria-label={$t("read.comic.prev_chapter") as string}><ReadIcon name="left" size={18} /></button>{/if}
      {#if hasNext}<button data-agent-actions="click" use:agentActions={["click"]} class="ib" onclick={onnextchapter} title={$t("read.comic.next_chapter") as string} aria-label={$t("read.comic.next_chapter") as string}><ReadIcon name="right" size={18} /></button>{/if}
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={showTranslated && shown.some((i) => translated[i])} onclick={() => (shown.some((i) => translated[i]) ? (showTranslated = !showTranslated) : translateShown())} title={$t("read.comic.translate") as string} aria-label={$t("read.comic.translate") as string} disabled={translating}><ReadIcon name="translate" size={17} spin={false} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={panel === "pages"} onclick={() => (panel = panel === "pages" ? "" : "pages")} title={$t("read.comic.pages") as string} aria-label={$t("read.comic.pages") as string}><ReadIcon name="grid" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" class:on={panel === "settings"} onclick={() => (panel = panel === "settings" ? "" : "settings")} title={$t("read.reader.settings") as string} aria-label={$t("read.reader.settings") as string}><ReadIcon name="settings" size={17} /></button>
      <button data-agent-actions="click" use:agentActions={["click"]} class="ib" onclick={toggleFullscreen} title={$t("read.reader.fullscreen") as string} aria-label={$t("read.reader.fullscreen") as string}><ReadIcon name={fullscreen ? "minimize" : "maximize"} size={17} /></button>
    </header>
  {/if}

  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
  <div
    class="stage"
    bind:this={stage}
    tabindex="0"
    role="application"
    aria-label={title}
    onclick={tap}
    onwheel={wheel}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
  >
    {#if mode === "webtoon"}
      <div class="strip" bind:this={strip} style:gap="{prefs.comicGap}px">
        {#each pages as p, i (i)}
          <img
            data-p={i}
            src={srcOf(i)}
            alt=""
            loading={Math.abs(i - page) < 6 ? "eager" : "lazy"}
            decoding="async"
            draggable="false"
            style:filter
            style:width={prefs.comicFit === "original" ? "auto" : `min(100%, ${Math.round(Math.min(viewport.w, 900) * zoom)}px)`}
            onload={(e) => measure(i, e.currentTarget as HTMLImageElement)}
          />
        {/each}
        <div class="end">
          {#if hasNext}
            <button class="btn btn-primary" onclick={(e) => { e.stopPropagation(); onnextchapter?.(); }}>{$t("read.comic.next_chapter")}</button>
          {:else}
            <p>{$t("read.comic.last_page")}</p>
          {/if}
        </div>
      </div>
    {:else if atEnd}
      <div class="trans">
        <p class="r-muted">{$t("read.comic.finished")}</p>
        <h2>{title}</h2>
        {#if hasNext}
          <button class="btn btn-primary" onclick={(e) => { e.stopPropagation(); onnextchapter?.(); }}><ReadIcon name="right" size={15} /> {$t("read.comic.next_chapter")}</button>
        {:else}
          <p>{$t("read.comic.no_more")}</p>
        {/if}
        <button class="btn" onclick={(e) => { e.stopPropagation(); atEnd = false; }}>{$t("read.comic.back_to_pages")}</button>
      </div>
    {:else}
      <div
        class="spread fit-{prefs.comicFit}"
        class:rtl
        class:vert={mode === "vertical"}
        style:transform="translate({pan.x}px, {pan.y}px) scale({zoom})"
      >
        {#each shown as i (i)}
          <img
            src={srcOf(i)}
            alt=""
            draggable="false"
            decoding="async"
            class:half={shown.length === 2}
            style:filter
            onload={(e) => measure(i, e.currentTarget as HTMLImageElement)}
          />
        {/each}
      </div>
    {/if}
  </div>

  {#if ui}
    <footer class="bottom">
      <span class="pg">{page + 1}{shown.length === 2 ? `–${shown[1] + 1}` : ""} / {pages.length}</span>
      <input
        class="slider"
        class:rtl
        type="range"
        min="0"
        max={Math.max(0, pages.length - 1)}
        value={page}
        oninput={(e) => jump(Number((e.target as HTMLInputElement).value))}
        aria-label={$t("read.comic.page_slider") as string}
      />
      <span class="pg">{pct}%</span>
      {#if prefs.showClock}<span class="pg">{clock}</span>{/if}
    </footer>
  {/if}

  {#if panel === "settings"}
    <aside class="panel" aria-label={$t("read.reader.settings") as string}>
      <h3>{$t("read.comic.mode")}</h3>
      <div class="seg">
        {#each ["rtl", "ltr", "vertical", "webtoon"] as m (m)}
          <button class:on={mode === m} onclick={() => setMode(m as Mode)}>{$t(`read.comic.${m}`)}</button>
        {/each}
      </div>
      {#if mode !== "webtoon" && mode !== "vertical"}
        <h3>{$t("read.comic.spread")}</h3>
        <div class="seg">
          {#each ["single", "double", "auto"] as s (s)}
            <button class:on={prefs.comicSpread === s} onclick={() => { prefs.comicSpread = s as typeof prefs.comicSpread; savePrefs(); }}>{$t(`read.comic.spread_${s}`)}</button>
          {/each}
        </div>
        <label class="ck"><input type="checkbox" bind:checked={prefs.comicCoverAlone} onchange={savePrefs} /> {$t("read.comic.cover_alone")}</label>
      {/if}
      <h3>{$t("read.comic.fit")}</h3>
      <div class="seg">
        {#each ["contain", "width", "height", "original"] as f (f)}
          <button class:on={prefs.comicFit === f} onclick={() => { prefs.comicFit = f as typeof prefs.comicFit; savePrefs(); }}>{$t(`read.comic.fit_${f}`)}</button>
        {/each}
      </div>
      <h3>{$t("read.comic.filter")}</h3>
      <div class="seg">
        {#each ["none", "grayscale", "sepia", "invert"] as f (f)}
          <button class:on={prefs.comicFilter === f} onclick={() => { prefs.comicFilter = f as typeof prefs.comicFilter; savePrefs(); }}>{$t(`read.comic.filter_${f}`)}</button>
        {/each}
      </div>
      <label class="row">{$t("read.reader.brightness")} <input type="range" min="40" max="130" bind:value={prefs.brightness} onchange={savePrefs} /></label>
      <label class="row">{$t("read.comic.background")}
        <span class="sw">
          {#each ["#111111", "#000000", "#2b2b2b", "#808080", "#f2f2f2", "#ffffff"] as c (c)}
            <button style:background={c} class:on={prefs.comicBackground === c} onclick={() => { prefs.comicBackground = c; savePrefs(); }} aria-label={c}></button>
          {/each}
        </span>
      </label>
      {#if mode === "webtoon"}
        <label class="row">{$t("read.comic.gap")} <input type="range" min="0" max="48" bind:value={prefs.comicGap} onchange={savePrefs} /></label>
      {/if}
      <label class="ck"><input type="checkbox" bind:checked={prefs.comicCrop} onchange={savePrefs} /> {$t("read.comic.crop")}</label>
      <label class="ck"><input type="checkbox" bind:checked={prefs.comicAutoNext} onchange={savePrefs} /> {$t("read.comic.auto_next")}</label>
      <label class="row">{$t("read.comic.preload")} <input type="number" min="0" max="12" bind:value={prefs.comicPreload} onchange={savePrefs} class="input num" /></label>
      <h3>{$t("read.comic.tap_zones")}</h3>
      <div class="seg">
        {#each ["sides", "kindle", "edge", "off"] as z (z)}
          <button class:on={prefs.tapZones === z} onclick={() => { prefs.tapZones = z as typeof prefs.tapZones; savePrefs(); }}>{$t(`read.comic.zones_${z}`)}</button>
        {/each}
      </div>
      <label class="ck"><input type="checkbox" bind:checked={prefs.showClock} onchange={savePrefs} /> {$t("read.reader.show_clock")}</label>
      <h3>{$t("read.comic.translate")}</h3>
      <label class="row">{$t("read.comic.translator_url")} <input class="input" bind:value={prefs.translatorUrl} onchange={savePrefs} /></label>
      <label class="row">{$t("read.comic.translator_lang")} <input class="input num" bind:value={prefs.translatorLang} onchange={savePrefs} /></label>
      <p class="keys r-muted">{$t("read.comic.translate_hint")}</p>
      {#if bookId && onsetcover}
        <button class="btn" onclick={useAsCover}><ReadIcon name="image" size={14} /> {$t("read.reader.use_as_cover")}</button>
      {/if}
      <p class="keys r-muted">{$t("read.comic.keys_hint")}</p>
    </aside>
  {:else if panel === "pages"}
    <aside class="panel thumbs" aria-label={$t("read.comic.pages") as string}>
      {#each pages as p, i (i)}
        <button class:on={shown.includes(i)} onclick={() => { jump(i); panel = ""; }}>
          <img src={p} alt="" loading="lazy" decoding="async" />
          <span>{i + 1}</span>
        </button>
      {/each}
    </aside>
  {/if}
</div>

<style>
  .cr {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg);
    color: #eee;
    user-select: none;
  }
  .top,
  .bottom {
    position: absolute;
    left: 0;
    right: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    background: rgb(0 0 0 / 0.72);
    backdrop-filter: blur(10px);
    color: #eee;
  }
  .top {
    top: 0;
  }
  .bottom {
    bottom: 0;
  }
  .tt {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 50vw;
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
  }
  .ib:hover,
  .ib.on {
    background: rgb(255 255 255 / 0.14);
  }
  .ib:focus-visible {
    outline: 2px solid var(--accent);
  }
  .stage {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    outline: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: default;
  }
  .spread {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    transform-origin: center;
    transition: transform 0.08s;
  }
  .spread.rtl {
    flex-direction: row-reverse;
  }
  .spread img {
    display: block;
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .spread img.half {
    max-width: 50%;
  }
  .fit-width img {
    width: 100%;
    max-height: none;
    height: auto;
  }
  .fit-width img.half {
    width: 50%;
  }
  .fit-width {
    align-items: flex-start;
    overflow: visible;
  }
  .fit-height img {
    height: 100%;
    max-width: none;
    width: auto;
  }
  .fit-original img {
    max-width: none;
    max-height: none;
  }
  .strip {
    height: 100%;
    width: 100%;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .strip img {
    display: block;
    max-width: 100%;
    height: auto;
  }
  .end {
    display: grid;
    place-items: center;
    padding: 60px 0 120px;
    color: #aaa;
  }
  .trans {
    display: grid;
    justify-items: center;
    gap: 12px;
    text-align: center;
    color: #ddd;
  }
  .trans h2 {
    margin: 0;
  }
  .pg {
    font-variant-numeric: tabular-nums;
    font-size: var(--text-sm);
    opacity: 0.85;
    white-space: nowrap;
  }
  .slider {
    flex: 1;
    accent-color: var(--accent);
  }
  .slider.rtl {
    direction: rtl;
  }
  .panel {
    position: absolute;
    top: 52px;
    right: 10px;
    bottom: 56px;
    z-index: 6;
    width: 300px;
    overflow-y: auto;
    padding: 12px 14px;
    border-radius: 12px;
    background: rgb(22 22 24 / 0.96);
    color: #eee;
    box-shadow: 0 8px 30px rgb(0 0 0 / 0.5);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .panel h3 {
    margin: 6px 0 0;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #999;
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .seg button {
    all: unset;
    flex: 1;
    text-align: center;
    padding: 6px 8px;
    border-radius: 7px;
    background: rgb(255 255 255 / 0.07);
    font-size: 12px;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent);
    color: #fff;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 13px;
  }
  .ck {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 13px;
  }
  .num {
    width: 64px;
    height: 26px;
  }
  .sw {
    display: flex;
    gap: 4px;
  }
  .sw button {
    all: unset;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid rgb(255 255 255 / 0.25);
    cursor: pointer;
  }
  .sw button.on {
    border-color: var(--accent);
  }
  .keys {
    font-size: 11px;
    line-height: 1.5;
  }
  .thumbs {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    align-content: start;
  }
  .thumbs button {
    all: unset;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    cursor: pointer;
    font-size: 11px;
    border-radius: 6px;
    padding: 3px;
  }
  .thumbs button.on {
    background: var(--accent);
  }
  .thumbs img {
    width: 100%;
    aspect-ratio: 2 / 3;
    object-fit: cover;
    border-radius: 4px;
    background: #333;
  }
</style>
