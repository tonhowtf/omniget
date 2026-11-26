<script lang="ts">
  import { onMount, tick } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { t } from "$lib/i18n";
  import * as api from "$lib/icons/api";
  import { buildSprite, buildSvg } from "$lib/icons/svg";
  import { svgToPngBase64 } from "$lib/icons/png";
  import { errorKey } from "$lib/icons/errors";
  import { createSaver } from "$lib/icons/saver";
  import { RECOMMENDED, iconifyId, orderRecommended, relatedQuery, splitId } from "$lib/icons/sets";
  import type { IconData, IconSet, IconsError, Library, SavedIcon } from "$lib/icons/types";
  import NavIcon from "#components/shell/NavIcon.svelte";
  import IconGrid from "#components/icons/IconGrid.svelte";
  import IconDetail, { type CopyKind, type DetailOpts } from "#components/icons/IconDetail.svelte";
  import SetFilter, { type SetScope } from "#components/icons/SetFilter.svelte";
  import LibraryView from "#components/icons/LibraryView.svelte";
  import Glyph from "#components/icons/Glyph.svelte";

  type Phase = "idle" | "loading" | "results" | "empty" | "error";
  type Tab = "search" | "library";

  const SUGGESTIONS = ["arrow", "home", "user", "settings", "download", "chart", "heart", "github"];
  const BATCH = 150;

  let sets = $state<IconSet[]>([]);
  let setsState = $state<"loading" | "ready" | "offline" | "error">("loading");
  let setsError = $state<IconsError | null>(null);
  let setsFetchedAt = $state(0);
  let setByPrefix = $derived(new Map(sets.map((s) => [s.prefix, s])));

  let tab = $state<Tab>("search");
  let query = $state("");
  let scope = $state<SetScope>({ mode: "recommended" });
  let phase = $state<Phase>("idle");
  let results = $state<string[]>([]);
  let capped = $state(false);
  let more = $state<string[]>([]);
  let nextStart = $state(0);
  let trimmed = $state(0);
  let loadingMore = $state(false);
  let moreError = $state<IconsError | null>(null);
  let resultsOffline = $state(false);
  let searchError = $state<IconsError | null>(null);
  let searchedFor = $state("");
  let active = $state(0);
  let searchSeq = 0;
  let debounce: ReturnType<typeof setTimeout> | null = null;

  const bodies = new SvelteMap<string, IconData | null>();
  const inflight = new Set<string>();
  const queued = new Map<string, Set<string>>();
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  let bodiesOffline = $state(false);

  let library = $state<Library>({ version: 1, favorites: [], collections: [] });
  let libraryState = $state<"loading" | "ready" | "error">("loading");
  let favIds = $derived(new Set(library.favorites.map((s) => iconifyId(s.prefix, s.icon.name))));
  let savedCount = $derived(library.favorites.length + library.collections.reduce((n, c) => n + c.icons.length, 0));

  let selected = $state<{ prefix: string; name: string } | null>(null);
  let selectedId = $derived(selected ? iconifyId(selected.prefix, selected.name) : null);
  let related = $state<string[]>([]);
  let relatedState = $state<"loading" | "ready" | "offline">("ready");
  let relatedSeq = 0;
  let detailOpts = $state<DetailOpts>({ useColor: false, color: "#ff9f0a", size: 24, strokeWidth: null, rotate: 0, hFlip: false, vFlip: false });

  let busySave = $state<"svg" | "png" | null>(null);
  let exporting = $state<{ target: string; mode: "folder" | "sprite" } | null>(null);
  let notice = $state<{ kind: "error" | "success"; text: string; path?: string } | null>(null);
  let live = $state("");
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  let grid = $state<{ focus: () => void } | null>(null);
  let detail = $state<{ focus: () => void } | null>(null);
  let opener: HTMLElement | null = null;
  let cacheBytes = $state<number | null>(null);
  let clearingCache = $state(false);
  let searchInput = $state<HTMLInputElement | null>(null);

  let prefixes = $derived.by(() => {
    if (scope.mode === "all") return [] as string[];
    if (scope.mode === "custom") return scope.prefixes;
    const rec = orderRecommended(sets).map((s) => s.prefix);
    return rec.length ? rec : [...RECOMMENDED];
  });
  let scopeLabel = $derived(
    scope.mode === "all"
      ? ($t("icons.scope_all") as string)
      : scope.mode === "recommended"
        ? ($t("icons.scope_recommended") as string)
        : scope.prefixes.map((p) => setByPrefix.get(p)?.name ?? p).join(", "),
  );
  let fmt = $derived(new Intl.NumberFormat());
  let offlineSince = $derived(setsFetchedAt ? new Date(setsFetchedAt * 1000).toLocaleString() : "");
  let showOffline = $derived(setsState === "offline" || (phase === "results" && resultsOffline) || bodiesOffline);

  function errorText(e: IconsError | null): string {
    if (!e) return "";
    const { key, params } = errorKey(e);
    return $t(key, params) as string;
  }

  function isPalette(prefix: string): boolean {
    return setByPrefix.get(prefix)?.palette ?? false;
  }

  function announce(text: string) {
    live = "";
    void tick().then(() => (live = text));
  }

  function showToast(text: string) {
    toast = text;
    announce(text);
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 2400);
  }

  async function loadSets(refresh = false) {
    setsState = "loading";
    setsError = null;
    try {
      const res = await api.collections(refresh);
      sets = res.sets;
      setsFetchedAt = res.fetched_at;
      setsState = res.offline ? "offline" : "ready";
    } catch (e) {
      setsError = api.errorOf(e);
      setsState = "error";
    }
  }

  async function loadLibrary() {
    try {
      library = await api.library();
      libraryState = "ready";
      for (const s of [...library.favorites, ...library.collections.flatMap((c) => c.icons)]) {
        const id = iconifyId(s.prefix, s.icon.name);
        if (!bodies.get(id)) bodies.set(id, s.icon);
      }
    } catch {
      libraryState = "error";
    }
  }

  function need(ids: string[]) {
    let added = false;
    for (const id of ids) {
      if (bodies.has(id) || inflight.has(id)) continue;
      const parsed = splitId(id);
      if (!parsed) continue;
      const set = queued.get(parsed.prefix) ?? new Set<string>();
      if (!set.has(parsed.name)) {
        set.add(parsed.name);
        added = true;
      }
      queued.set(parsed.prefix, set);
    }
    if (added && !flushTimer) flushTimer = setTimeout(flush, 30);
  }

  async function flush() {
    flushTimer = null;
    const work = [...queued.entries()];
    queued.clear();
    await Promise.all(
      work.flatMap(([prefix, names]) => {
        const list = [...names];
        const chunks: string[][] = [];
        for (let i = 0; i < list.length; i += BATCH) chunks.push(list.slice(i, i + BATCH));
        return chunks.map(async (chunk) => {
          const ids = chunk.map((n) => iconifyId(prefix, n));
          ids.forEach((id) => inflight.add(id));
          try {
            const res = await api.getIcons(prefix, chunk);
            for (const icon of res.icons) bodies.set(iconifyId(prefix, icon.name), icon);
            for (const name of res.missing) bodies.set(iconifyId(prefix, name), null);
            if (res.offline) bodiesOffline = true;
          } catch (e) {
            if (api.errorOf(e).kind === "offline") bodiesOffline = true;
            for (const id of ids) if (!bodies.has(id)) bodies.set(id, null);
          } finally {
            ids.forEach((id) => inflight.delete(id));
          }
        });
      }),
    );
  }

  function retryBodies() {
    bodiesOffline = false;
    for (const [id, value] of [...bodies.entries()]) if (value === null) bodies.delete(id);
    if (phase === "results") {
      const snapshot = results;
      results = [];
      void tick().then(() => (results = snapshot));
    }
  }

  async function runSearch() {
    if (debounce) {
      clearTimeout(debounce);
      debounce = null;
    }
    const q = query.trim();
    const seq = ++searchSeq;
    if (!q) {
      phase = "idle";
      results = [];
      searchError = null;
      return;
    }
    phase = results.length && phase === "results" ? "results" : "loading";
    searching = true;
    searchError = null;
    try {
      const res = await api.search(q, prefixes);
      if (seq !== searchSeq) return;
      results = res.icons;
      capped = res.capped;
      more = res.more;
      nextStart = res.next_start;
      trimmed = res.trimmed;
      moreError = null;
      resultsOffline = res.offline;
      searchedFor = q;
      active = 0;
      phase = res.icons.length ? "results" : "empty";
    } catch (e) {
      if (seq !== searchSeq) return;
      searchError = api.errorOf(e);
      searchedFor = q;
      results = [];
      phase = "error";
    } finally {
      if (seq === searchSeq) searching = false;
    }
    if (seq === searchSeq) {
      announce(
        phase === "results"
          ? ($t("icons.results_count", { count: results.length }) as string)
          : phase === "empty"
            ? ($t("icons.no_results_title", { query: q }) as string)
            : ($t("icons.search_failed") as string),
      );
    }
  }

  let searching = $state(false);

  async function loadMore() {
    if (loadingMore || !capped) return;
    const seq = searchSeq;
    const q = searchedFor;
    const from = scope.mode === "all" ? [] : more;
    loadingMore = true;
    moreError = null;
    try {
      const res = await api.search(q, from, nextStart);
      if (seq !== searchSeq) return;
      const have = new Set(results);
      const fresh = res.icons.filter((id) => !have.has(id));
      results = [...results, ...fresh];
      capped = res.capped && fresh.length > 0;
      more = res.more;
      nextStart = res.next_start;
      if (res.offline) resultsOffline = true;
      announce($t("icons.loaded_more", { count: fresh.length, total: results.length }) as string);
    } catch (e) {
      if (seq !== searchSeq) return;
      moreError = api.errorOf(e);
    } finally {
      if (seq === searchSeq) loadingMore = false;
    }
  }

  function scheduleSearch() {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => void runSearch(), 320);
  }

  function setScope(next: SetScope) {
    scope = next;
    if (query.trim()) void runSearch();
  }

  function suggest(word: string) {
    query = word;
    tab = "search";
    void runSearch();
  }

  async function open(id: string) {
    const parsed = splitId(id);
    if (!parsed) return;
    const from = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!from?.closest(".ic-detail")) opener = from;
    selected = parsed;
    need([id]);
    void loadRelated(parsed.prefix, parsed.name);
    announce($t("icons.detail_opened", { name: parsed.name }) as string);
    await tick();
    if (!from?.closest(".ic-detail")) detail?.focus();
  }

  function openSaved(saved: SavedIcon) {
    const id = iconifyId(saved.prefix, saved.icon.name);
    if (!bodies.get(id)) bodies.set(id, saved.icon);
    void open(id);
  }

  async function loadRelated(prefix: string, name: string) {
    const seq = ++relatedSeq;
    relatedState = "loading";
    related = [];
    try {
      const res = await api.search(relatedQuery(name), [prefix]);
      if (seq !== relatedSeq) return;
      const self = iconifyId(prefix, name);
      related = res.icons.filter((i) => i !== self).slice(0, 48);
      relatedState = "ready";
    } catch {
      if (seq !== relatedSeq) return;
      relatedState = "offline";
    }
  }

  function closeDetail() {
    const back = opener;
    opener = null;
    selected = null;
    void tick().then(() => {
      if (back?.isConnected) back.focus();
      else if (tab === "search" && phase === "results") grid?.focus();
      else searchInput?.focus();
    });
  }

  function onPageKey(event: KeyboardEvent) {
    if (event.key === "Escape" && selected && !event.defaultPrevented) {
      const target = event.target as HTMLElement | null;
      if (target?.tagName === "INPUT" && (target as HTMLInputElement).type !== "range" && target.closest(".ic-detail")) return;
      event.preventDefault();
      closeDetail();
    }
  }

  function onWindowKey(event: KeyboardEvent) {
    if (event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT" || target.isContentEditable)) return;
    event.preventDefault();
    tab = "search";
    void tick().then(() => searchInput?.focus());
  }

  function savedFrom(prefix: string, icon: IconData): SavedIcon {
    const set = setByPrefix.get(prefix);
    return {
      prefix,
      icon: $state.snapshot(icon) as IconData,
      set_name: set?.name ?? null,
      license: set?.license ?? null,
      license_spdx: set?.license_spdx ?? null,
      added_at: Math.floor(Date.now() / 1000),
    };
  }

  const saver = createSaver<Library>({
    write: (value) => api.saveLibrary(value),
    applied: (value) => (library = value),
    failed: (e) => {
      notice = { kind: "error", text: $t("icons.library_save_failed", { error: errorText(api.errorOf(e)) }) as string };
      void loadLibrary();
    },
  });

  async function persist(next: Library, success?: string) {
    library = next;
    const ok = await saver.save($state.snapshot(next) as Library);
    if (ok && success) showToast(success);
  }

  function same(a: SavedIcon, prefix: string, name: string) {
    return a.prefix === prefix && a.icon.name === name;
  }

  function toggleFavorite() {
    if (!selected) return;
    const { prefix, name } = selected;
    const icon = bodies.get(iconifyId(prefix, name));
    if (!icon) return;
    const snap = $state.snapshot(library) as Library;
    const has = snap.favorites.some((f) => same(f, prefix, name));
    snap.favorites = has ? snap.favorites.filter((f) => !same(f, prefix, name)) : [savedFrom(prefix, icon), ...snap.favorites];
    void persist(snap, has ? ($t("icons.unfavorited", { name }) as string) : ($t("icons.favorited", { name }) as string));
  }

  function newId(): string {
    return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `c-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }

  function addTo(collectionId: string) {
    if (!selected) return;
    const { prefix, name } = selected;
    const icon = bodies.get(iconifyId(prefix, name));
    if (!icon) return;
    const snap = $state.snapshot(library) as Library;
    const c = snap.collections.find((x) => x.id === collectionId);
    if (!c || c.icons.some((s) => same(s, prefix, name))) return;
    c.icons.push(savedFrom(prefix, icon));
    void persist(snap, $t("icons.added_to", { name, collection: c.name }) as string);
  }

  function createCollection(name: string, withSelected: boolean) {
    const snap = $state.snapshot(library) as Library;
    const icons: SavedIcon[] = [];
    if (withSelected && selected) {
      const icon = bodies.get(iconifyId(selected.prefix, selected.name));
      if (icon) icons.push(savedFrom(selected.prefix, icon));
    }
    snap.collections.push({ id: newId(), name, created_at: Math.floor(Date.now() / 1000), icons });
    void persist(snap, $t("icons.collection_created", { name }) as string);
  }

  function renameCollection(id: string, name: string) {
    const snap = $state.snapshot(library) as Library;
    const c = snap.collections.find((x) => x.id === id);
    if (!c) return;
    c.name = name;
    void persist(snap);
  }

  function deleteCollection(id: string) {
    const snap = $state.snapshot(library) as Library;
    const c = snap.collections.find((x) => x.id === id);
    snap.collections = snap.collections.filter((x) => x.id !== id);
    void persist(snap, c ? ($t("icons.collection_deleted", { name: c.name }) as string) : undefined);
  }

  function removeFrom(target: string, saved: SavedIcon) {
    const snap = $state.snapshot(library) as Library;
    if (target === "favorites") snap.favorites = snap.favorites.filter((f) => !same(f, saved.prefix, saved.icon.name));
    else {
      const c = snap.collections.find((x) => x.id === target);
      if (!c) return;
      c.icons = c.icons.filter((f) => !same(f, saved.prefix, saved.icon.name));
    }
    void persist(snap, $t("icons.removed", { name: saved.icon.name }) as string);
  }

  async function exportGroup(target: string, mode: "folder" | "sprite") {
    const group = target === "favorites" ? { name: $t("icons.favorites") as string, icons: library.favorites } : library.collections.find((c) => c.id === target);
    if (!group || group.icons.length === 0 || exporting) return;
    exporting = { target, mode };
    notice = null;
    try {
      const icons = $state.snapshot(group.icons) as SavedIcon[];
      const files = icons.map((s) => ({ name: s.icon.name, svg: buildSvg(s.icon) }));
      const sprite = mode === "sprite" ? buildSprite(icons.map((s) => ({ prefix: s.prefix, icon: s.icon }))) : undefined;
      const out = await api.exportCollection(mode, group.name, files, sprite);
      if (out) {
        notice = {
          kind: "success",
          text: (mode === "sprite" ? $t("icons.exported_sprite", { count: out.count, path: out.path }) : $t("icons.exported_folder", { count: out.count, path: out.path })) as string,
          path: out.path,
        };
        announce(notice.text);
      }
    } catch (e) {
      notice = { kind: "error", text: $t("icons.export_failed", { error: errorText(api.errorOf(e)) }) as string };
    } finally {
      exporting = null;
    }
  }

  const COPY_LABEL: Record<CopyKind, string> = {
    svg: "icons.copied_svg",
    svelte: "icons.copied_svelte",
    react: "icons.copied_react",
    css: "icons.copied_css",
    id: "icons.copied_id",
    install: "icons.copied_install",
  };

  async function copy(kind: CopyKind, text: string) {
    if (!text) return;
    try {
      await writeText(text);
      showToast($t(COPY_LABEL[kind]) as string);
    } catch (e) {
      notice = { kind: "error", text: $t("icons.copy_failed", { error: errorText(api.errorOf(e)) }) as string };
    }
  }

  async function save(kind: "svg" | "png", svg: string, size: { width: number; height: number }) {
    if (!selected || busySave) return;
    busySave = kind;
    notice = null;
    const fileName = selected.name;
    try {
      const out = kind === "svg" ? await api.saveSvg(fileName, svg) : await api.savePng(fileName, await svgToPngBase64(svg, size.width, size.height));
      if (out) {
        notice = { kind: "success", text: $t("icons.saved_file", { path: out.path }) as string, path: out.path };
        announce(notice.text);
      }
    } catch (e) {
      notice = { kind: "error", text: $t("icons.save_failed", { error: errorText(api.errorOf(e)) }) as string };
    } finally {
      busySave = null;
    }
  }

  async function reveal() {
    const path = notice?.path;
    if (!path) return;
    try {
      await api.reveal(path);
    } catch (e) {
      notice = { kind: "error", text: $t("icons.reveal_failed", { path, error: errorText(api.errorOf(e)) }) as string };
    }
  }

  async function loadCacheInfo() {
    try {
      cacheBytes = (await api.cacheInfo()).bytes;
    } catch {
      cacheBytes = null;
    }
  }

  async function clearCache() {
    if (clearingCache) return;
    clearingCache = true;
    try {
      await api.clearCache();
      cacheBytes = 0;
      bodiesOffline = false;
      showToast($t("icons.cache_cleared") as string);
    } catch (e) {
      notice = { kind: "error", text: $t("icons.cache_clear_failed", { error: errorText(api.errorOf(e)) }) as string };
    } finally {
      clearingCache = false;
    }
  }

  let cacheSize = $derived(
    cacheBytes === null ? "" : new Intl.NumberFormat(undefined, { style: "unit", unit: cacheBytes >= 1_000_000 ? "megabyte" : "kilobyte", maximumFractionDigits: 1 }).format(cacheBytes >= 1_000_000 ? cacheBytes / 1_000_000 : cacheBytes / 1000),
  );

  $effect(() => {
    if (tab === "library") void loadCacheInfo();
  });

  function errorTitle(e: IconsError | null): string {
    if (!e) return "";
    if (e.kind === "offline") return $t("icons.offline_title") as string;
    if (e.kind === "invalid") return $t("icons.invalid_title") as string;
    return $t("icons.server_error_title") as string;
  }

  function errorBody(e: IconsError | null): string {
    if (!e) return "";
    if (e.kind === "offline") return $t("icons.offline_body") as string;
    return errorText(e);
  }

  onMount(() => {
    void loadSets();
    void loadLibrary();
    window.addEventListener("keydown", onWindowKey);
    return () => {
      window.removeEventListener("keydown", onWindowKey);
      if (debounce) clearTimeout(debounce);
      if (flushTimer) clearTimeout(flushTimer);
      if (toastTimer) clearTimeout(toastTimer);
    };
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section class="ic" class:with-detail={selected !== null} aria-busy={phase === "loading"} onkeydown={onPageKey} aria-labelledby="ic-title">
  <header class="ic-head">
    <NavIcon icon="icons" size={56} />
    <div class="ic-head-copy">
      <h1 id="ic-title">{$t("icons.title")}</h1>
      <p class="ic-subtitle">{$t("icons.subtitle")}</p>
    </div>
  </header>

  <p class="ic-live" role="status" aria-live="polite">{live}</p>

  <div class="ic-tabs" role="tablist" aria-label={$t("icons.tabs_label") as string}>
    <button type="button" role="tab" id="ic-tab-search" aria-controls="ic-panel-search" aria-selected={tab === "search"} class:on={tab === "search"} onclick={() => (tab = "search")}>
      <Glyph name="search" size={13} />
      {$t("icons.tab_search")}
    </button>
    <button type="button" role="tab" id="ic-tab-library" aria-controls="ic-panel-library" aria-selected={tab === "library"} class:on={tab === "library"} onclick={() => (tab = "library")}>
      <Glyph name="star" size={13} />
      {$t("icons.tab_library")}
      {#if savedCount > 0}<span class="ic-tab-count">{savedCount}</span>{/if}
    </button>
  </div>

  {#if notice}
    <div class="ic-notice" class:success={notice.kind === "success"} role={notice.kind === "error" ? "alert" : "status"}>
      <Glyph name={notice.kind === "error" ? "warning" : "check"} size={15} />
      <p>{notice.text}</p>
      {#if notice.path}
        <button type="button" class="btn btn-secondary btn-sm" onclick={reveal}>{$t("icons.show_in_folder")}</button>
      {/if}
      <button type="button" class="btn btn-icon btn-ghost btn-sm" aria-label={$t("icons.dismiss") as string} onclick={() => (notice = null)}><Glyph name="close" size={12} /></button>
    </div>
  {/if}

  {#if showOffline}
    <div class="ic-offline" role="status">
      <Glyph name="offline" size={15} />
      <p>
        {setsState === "offline" && offlineSince ? $t("icons.offline_cached_since", { date: offlineSince }) : $t("icons.offline_partial")}
      </p>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => { retryBodies(); void loadSets(true); if (query.trim()) void runSearch(); }}>
        <Glyph name="refresh" size={12} />
        {$t("icons.try_again")}
      </button>
    </div>
  {/if}

  <div class="ic-body">
    <div class="ic-main">
      {#if tab === "search"}
        <div id="ic-panel-search" role="tabpanel" aria-labelledby="ic-tab-search" class="ic-panel">
          <form
            class="ic-search"
            role="search"
            onsubmit={(e) => {
              e.preventDefault();
              void runSearch();
            }}
          >
            <Glyph name="search" size={15} />
            <label class="ic-sr" for="ic-q">{$t("icons.search_label")}</label>
            <input
              id="ic-q"
              bind:this={searchInput}
              type="search"
              bind:value={query}
              oninput={scheduleSearch}
              onkeydown={(e) => {
                if (e.key === "ArrowDown" && phase === "results") {
                  e.preventDefault();
                  grid?.focus();
                }
              }}
              placeholder={$t("icons.search_placeholder") as string}
              autocomplete="off"
              spellcheck="false"
            />
            {#if searching}<span class="spinner ic-spin" aria-hidden="true"></span>{/if}
            <kbd class="ic-kbd" aria-hidden="true">/</kbd>
          </form>

          {#if setsState === "error"}
            <div class="ic-inline-error" role="alert">
              <Glyph name="warning" size={14} />
              <p>{$t("icons.sets_failed", { error: errorText(setsError) })}</p>
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => loadSets(true)}>{$t("icons.reload_sets")}</button>
            </div>
          {/if}

          <SetFilter {sets} {scope} loading={setsState === "loading"} onchange={setScope} />

          {#if phase === "idle"}
            <div class="ic-empty">
              <h2>{$t("icons.empty_title")}</h2>
              <p>{$t("icons.empty_body", { count: sets.length ? fmt.format(sets.length) : ($t("icons.sets_fallback_count") as string) })}</p>
              <div class="ic-suggest" role="group" aria-label={$t("icons.suggestions") as string}>
                {#each SUGGESTIONS as word (word)}
                  <button type="button" class="btn btn-secondary btn-sm" onclick={() => suggest(word)}>{word}</button>
                {/each}
              </div>
              <p class="ic-empty-note">{$t("icons.empty_note")}</p>
            </div>
          {:else if phase === "loading"}
            <div class="ic-skeleton" aria-hidden="true">
              {#each Array(24) as _, i (i)}<span></span>{/each}
            </div>
            <p class="ic-sr">{$t("icons.searching", { query: query.trim() })}</p>
          {:else if phase === "error"}
            <div class="ic-empty ic-error" role="alert">
              <Glyph name={searchError?.kind === "offline" ? "offline" : "warning"} size={28} />
              <h2>{errorTitle(searchError)}</h2>
              <p>{errorBody(searchError)}</p>
              <div class="ic-suggest">
                <button type="button" class="btn btn-primary btn-sm" onclick={() => runSearch()}>{$t("icons.try_again")}</button>
                {#if searchError?.kind === "offline"}
                  <button type="button" class="btn btn-secondary btn-sm" onclick={() => (tab = "library")}>{$t("icons.open_library")}</button>
                {/if}
              </div>
            </div>
          {:else if phase === "empty"}
            <div class="ic-empty">
              <h2>{$t("icons.no_results_title", { query: searchedFor })}</h2>
              <p>{$t("icons.no_results_body", { scope: scopeLabel })}</p>
              {#if scope.mode !== "all"}
                <div class="ic-suggest">
                  <button type="button" class="btn btn-primary btn-sm" onclick={() => setScope({ mode: "all" })}>{$t("icons.search_all_sets")}</button>
                </div>
              {/if}
            </div>
          {:else}
            <div class="ic-results-head">
              <p>
                {$t("icons.results_for", { count: fmt.format(results.length), query: searchedFor })}
                <span class="ic-results-scope">· {scopeLabel}</span>
              </p>
              <p class="ic-results-keys">{$t("icons.keys_hint")}</p>
            </div>
            <div class="ic-results" class:stale={searching}>
              <IconGrid
                bind:this={grid}
                ids={results}
                {bodies}
                favorites={favIds}
                selected={selectedId}
                bind:active
                label={$t("icons.results_label", { query: searchedFor }) as string}
                height={selected ? 520 : 460}
                {isPalette}
                onopen={(id) => open(id)}
                onneed={need}
              />
            </div>
            {#if trimmed > 0}
              <p class="ic-hint"><Glyph name="info" size={12} /> {$t("icons.trimmed_hint", { count: trimmed, max: prefixes.length - trimmed })}</p>
            {/if}
            {#if capped}
              <div class="ic-more">
                <p class="ic-hint"><Glyph name="info" size={12} /> {$t("icons.capped_hint")}</p>
                <button type="button" class="btn btn-secondary btn-sm" disabled={loadingMore} onclick={loadMore}>
                  {#if loadingMore}<span class="spinner ic-spin" aria-hidden="true"></span>{$t("icons.loading_more")}{:else}{$t("icons.load_more")}{/if}
                </button>
              </div>
              {#if moreError}
                <p class="ic-hint ic-hint-error" role="alert"><Glyph name="warning" size={12} /> {$t("icons.load_more_failed", { error: errorText(moreError) })}</p>
              {/if}
            {/if}
          {/if}
        </div>
      {:else}
        <div id="ic-panel-library" role="tabpanel" aria-labelledby="ic-tab-library" class="ic-panel">
          {#if libraryState === "loading"}
            <div class="ic-empty"><span class="spinner" aria-hidden="true"></span><p>{$t("icons.library_loading")}</p></div>
          {:else if libraryState === "error"}
            <div class="ic-empty ic-error" role="alert">
              <Glyph name="warning" size={28} />
              <h2>{$t("icons.library_failed_title")}</h2>
              <p>{$t("icons.library_failed_body")}</p>
              <button type="button" class="btn btn-primary btn-sm" onclick={loadLibrary}>{$t("icons.try_again")}</button>
            </div>
          {:else}
            <p class="ic-hint">{$t("icons.library_hint")}</p>
            <LibraryView
              {library}
              selected={selectedId}
              {exporting}
              {isPalette}
              onopen={openSaved}
              onremove={removeFrom}
              oncreate={(name) => createCollection(name, false)}
              onrename={renameCollection}
              ondelete={deleteCollection}
              onexport={exportGroup}
              onsearch={() => {
                tab = "search";
                void tick().then(() => searchInput?.focus());
              }}
            />
            <div class="ic-cache">
              <Glyph name="folder" size={13} />
              <p>
                {cacheBytes === null ? $t("icons.cache_unknown") : $t("icons.cache_size", { size: cacheSize })}
                <span class="ic-cache-note">{$t("icons.cache_hint")}</span>
              </p>
              <button type="button" class="btn btn-ghost btn-sm" disabled={clearingCache || cacheBytes === 0} title={cacheBytes === 0 ? ($t("icons.cache_empty") as string) : undefined} onclick={clearCache}>
                {#if clearingCache}<span class="spinner ic-spin" aria-hidden="true"></span>{:else}<Glyph name="trash" size={12} />{/if}
                {$t("icons.cache_clear")}
              </button>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    {#if selected}
      {@const icon = bodies.get(selectedId ?? "")}
      <div class="ic-side">
        <IconDetail
          bind:this={detail}
          prefix={selected.prefix}
          name={selected.name}
          {icon}
          set={setByPrefix.get(selected.prefix) ?? null}
          favorite={favIds.has(selectedId ?? "")}
          collections={library.collections}
          {related}
          {relatedState}
          {bodies}
          favorites={favIds}
          bind:opts={detailOpts}
          busy={busySave}
          {isPalette}
          onclose={closeDetail}
          onfavorite={toggleFavorite}
          onaddto={addTo}
          oncreate={(name) => createCollection(name, true)}
          oncopy={copy}
          onsave={save}
          onopen={(id) => open(id)}
          onneed={need}
        />
      </div>
    {/if}
  </div>

  {#if toast}
    <div class="ic-toast" aria-hidden="true"><Glyph name="check" size={13} />{toast}</div>
  {/if}
</section>

<style>
  .ic {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .ic-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .ic-head-copy {
    min-width: 0;
  }

  .ic-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .ic-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .ic-live,
  .ic-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  .ic-tabs {
    display: inline-flex;
    align-self: flex-start;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }

  .ic-tabs button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    height: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 550;
    cursor: pointer;
  }

  .ic-tabs button.on {
    background: var(--surface-hi);
    box-shadow: var(--elev-1);
    color: var(--text);
  }

  .ic-tabs button:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .ic-tab-count {
    padding: 0 5px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    line-height: 16px;
  }

  .ic-notice,
  .ic-offline,
  .ic-inline-error {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ic-notice p,
  .ic-offline p,
  .ic-inline-error p {
    flex: 1;
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .ic-notice :global(.ic-glyph),
  .ic-inline-error :global(.ic-glyph) {
    color: var(--warning);
  }

  .ic-notice.success :global(.ic-glyph) {
    color: var(--success);
  }

  .ic-offline :global(.ic-glyph) {
    color: var(--text-dim);
  }

  .ic-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-4);
    align-items: start;
  }

  .ic.with-detail .ic-body {
    grid-template-columns: minmax(0, 1fr) 360px;
  }

  .ic-side {
    position: sticky;
    top: var(--space-3);
    max-height: calc(100vh - 2 * var(--space-5));
    overflow-y: auto;
    border-radius: var(--radius-lg);
  }

  .ic-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .ic-search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h-lg);
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    background: var(--input-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    color: var(--text-dim);
  }

  .ic-search:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }

  .ic-search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-md);
    outline: none;
  }

  .ic-kbd {
    padding: 0 6px;
    border-radius: var(--radius-xs);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .ic-spin {
    width: 14px;
    height: 14px;
  }

  .ic-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-7) var(--space-5);
    border-radius: var(--radius-xl);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    text-align: center;
  }

  .ic-empty h2 {
    margin: 0;
    color: var(--text);
    font-size: var(--text-lg);
    font-weight: 650;
  }

  .ic-empty p {
    margin: 0;
    max-width: 460px;
    color: var(--text-dim);
    font-size: var(--text-base);
    overflow-wrap: anywhere;
  }

  .ic-error > :global(.ic-glyph) {
    color: var(--warning);
  }

  .ic-empty-note {
    font-size: var(--text-sm) !important;
  }

  .ic-suggest {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
  }

  .ic-skeleton {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(76px, 1fr));
    gap: 6px;
  }

  .ic-skeleton span {
    height: 76px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    animation: ic-pulse 1.2s var(--ease-in-out) infinite alternate;
  }

  @keyframes ic-pulse {
    from {
      opacity: 0.45;
    }
    to {
      opacity: 1;
    }
  }

  .ic-results-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .ic-results-head p {
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: 550;
  }

  .ic-results-scope {
    color: var(--text-dim);
    font-weight: 400;
  }

  .ic-results-head .ic-results-keys {
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 400;
  }

  .ic-results {
    padding: var(--space-2);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    transition: opacity var(--duration-fast) var(--ease-out);
  }

  .ic-results.stale {
    opacity: 0.6;
  }

  .ic-hint {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .ic-hint-error {
    color: var(--error);
  }

  .ic-more {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .ic-cache {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .ic-cache p {
    flex: 1;
    min-width: 0;
    margin: 0;
  }

  .ic-cache-note {
    display: block;
    color: var(--tertiary);
  }

  .ic-toast {
    position: fixed;
    bottom: var(--space-6);
    left: 50%;
    z-index: 10;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-full);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-2);
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: 550;
    transform: translateX(-50%);
  }

  .ic-toast :global(.ic-glyph) {
    color: var(--success);
  }

  @media (max-width: 860px) {
    .ic.with-detail .ic-body {
      grid-template-columns: minmax(0, 1fr);
    }

    .ic-side {
      position: static;
      max-height: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ic-skeleton span {
      animation: none;
    }

    .ic-results {
      transition: none;
    }
  }
</style>
