<script lang="ts" module>
  // State that survives going to a detail page and back (same session).
  const memo: {
    tab: string;
    query: string;
    categories: string[];
    sources: string[];
    licenses: string[];
    origins: string[];
    marketplace: string | null;
    hideCollisions: boolean;
    worksIn: string[];
    sort: "relevance" | "name" | "stars" | "updated";
    offset: number;
    collection: string | null;
    mcpMode: "catalog" | "registry";
  } = {
    tab: "all",
    query: "",
    categories: [],
    sources: [],
    licenses: [],
    origins: [],
    marketplace: null,
    hideCollisions: false,
    worksIn: [],
    sort: "relevance",
    offset: 0,
    collection: null,
    mcpMode: "catalog",
  };
</script>

<script lang="ts">
  /**
   * Catálogo (`/llm/catalog`): 3.4k components from the pinned mirror, user
   * sources and plugin marketplaces, plus the official MCP Registry live.
   * Tabs by kind, search, facets with counts, "works in" chips of the tools
   * detected on this machine, sort, grid/list, pages of 48, collections in
   * the sidebar and the stack drawer (layout).
   */
  import { onMount, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import CatalogCard from "$components/central/catalog/CatalogCard.svelte";
  import CollectionsSidebar from "$components/central/catalog/CollectionsSidebar.svelte";
  import FacetGroup from "$components/central/catalog/FacetGroup.svelte";
  import SourcesDialog from "$components/central/catalog/SourcesDialog.svelte";
  import KindIcon from "$components/central/catalog/KindIcon.svelte";
  import {
    EXTRA_KINDS,
    KIND_TABS,
    agentkitDetect,
    agentkitTargets,
    catalogFacets,
    catalogItem,
    catalogMeta,
    catalogSearch,
    collections as coll,
    compatFor,
    errText,
    itemHref,
    kindsWorkingIn,
    loadKindMatrix,
    marketplacesList,
    registrySearch,
    sourceLabel,
    toHit,
    type CatalogMeta,
    type Collection,
    type Compat,
    type DetectedTarget,
    type Facets,
    type Filters,
    type MarketplaceInfo,
    type SearchHit,
    type TargetAdapter,
  } from "$lib/central/catalog";
  import { stack } from "$lib/central/stack-store.svelte";

  const PAGE = 48;
  const VIEW_KEY = "omniget.central.catalog.view";

  let tab = $state(memo.tab);
  let query = $state(memo.query);
  let debounced = $state(memo.query);
  let categories = $state<string[]>(memo.categories);
  let sources = $state<string[]>(memo.sources);
  let licenses = $state<string[]>(memo.licenses);
  let origins = $state<string[]>(memo.origins);
  let marketplace = $state<string | null>(memo.marketplace);
  let hideCollisions = $state(memo.hideCollisions);
  let worksIn = $state<string[]>(memo.worksIn);
  let sort = $state(memo.sort);
  let offset = $state(memo.offset);
  let collection = $state<string | null>(memo.collection);
  let mcpMode = $state(memo.mcpMode);
  let view = $state<"grid" | "list">("grid");

  let meta = $state<CatalogMeta | null>(null);
  let targets = $state<TargetAdapter[]>([]);
  let detected = $state<DetectedTarget[]>([]);
  let markets = $state<MarketplaceInfo[]>([]);
  let collectionsList = $state<Collection[]>([]);
  let hits = $state<SearchHit[]>([]);
  let total = $state(0);
  let facets = $state<Facets | null>(null);
  let compat = $state<Record<string, Record<string, Compat>>>({});
  let loading = $state(false);
  let error = $state("");
  let showSources = $state(false);
  let showFacets = $state(true);
  let searchInput = $state<HTMLInputElement | null>(null);
  let reloadTick = $state(0);
  let matrixTick = $state(0);

  // registry (live)
  let regHits = $state<SearchHit[]>([]);
  let regCursor = $state<string | null>(null);
  let regFrom = $state("");
  let regLoading = $state(false);

  // collection view
  let collHits = $state<SearchHit[]>([]);
  let collMissing = $state<string[]>([]);

  let installedTools = $derived(detected.filter((d) => d.installed));
  let toolList = $derived(targets.map((x) => ({ id: x.id, name: x.name })));
  let detectedIds = $derived(installedTools.map((d) => d.id));
  let allKinds = $derived(meta?.counts ? Object.keys(meta.counts) : [...KIND_TABS.filter((k) => k !== "all"), ...EXTRA_KINDS]);
  let tabs = $derived([
    ...KIND_TABS,
    ...EXTRA_KINDS.filter((k) => (meta?.counts?.[k] ?? 0) > 0),
  ]);
  let kindCount = $derived(Object.fromEntries((facets?.kind ?? []).map((f) => [f.value, f.count])));
  let facetTotal = $derived((facets?.kind ?? []).reduce((a, f) => a + f.count, 0));
  let registryMode = $derived(tab === "mcp" && mcpMode === "registry");
  let selectedCollection = $derived(collectionsList.find((c) => c.id === collection) ?? null);

  let filters = $derived.by<Filters>(() => {
    void matrixTick;
    let kinds: string[] = tab === "all" ? [] : [tab];
    if (worksIn.length && targets.length) {
      const ok = kindsWorkingIn(worksIn, targets, allKinds);
      kinds = tab === "all" ? ok : ok.includes(tab) ? [tab] : ["__none__"];
      if (!kinds.length) kinds = ["__none__"];
    }
    return {
      kinds,
      categories,
      sources,
      licenses,
      origin_tools: origins,
      marketplace,
      hide_collisions: hideCollisions,
    };
  });
  let pages = $derived(Math.max(1, Math.ceil(total / PAGE)));
  let page = $derived(Math.floor(offset / PAGE) + 1);

  // keep the memo in sync
  $effect(() => {
    Object.assign(memo, {
      tab,
      query,
      categories,
      sources,
      licenses,
      origins,
      marketplace,
      hideCollisions,
      worksIn,
      sort,
      offset,
      collection,
      mcpMode,
    });
  });

  // debounce the query
  $effect(() => {
    const q = query;
    const h = setTimeout(() => {
      if (debounced !== q) {
        debounced = q;
        offset = 0;
      }
    }, 220);
    return () => clearTimeout(h);
  });

  // search + facets
  let seq = 0;
  $effect(() => {
    const f = filters;
    const q = debounced;
    const s = sort;
    const o = offset;
    void reloadTick;
    if (registryMode || collection) return;
    const my = ++seq;
    loading = true;
    error = "";
    Promise.all([catalogSearch({ query: q, filters: f, sort: s, offset: o, limit: PAGE }), catalogFacets(f, q)])
      .then(async ([r, fc]) => {
        if (my !== seq) return;
        hits = r.items;
        total = r.total;
        facets = fc;
        loading = false;
        const tg = untrack(() => targets);
        if (tg.length) {
          const c = await compatFor(r.items, tg);
          if (my === seq) compat = { ...untrack(() => compat), ...c };
        }
      })
      .catch((e) => {
        if (my !== seq) return;
        error = errText(e);
        loading = false;
      });
  });

  // compat once the targets arrive
  $effect(() => {
    const tg = targets;
    void matrixTick;
    const list = untrack(() => hits);
    if (tg.length && list.length) void compatFor(list, tg).then((c) => (compat = { ...untrack(() => compat), ...c }));
  });

  // registry live search
  let regSeq = 0;
  async function registry(more = false) {
    const my = ++regSeq;
    regLoading = true;
    error = "";
    try {
      const r = await registrySearch(debounced, more ? regCursor : null, 30);
      if (my !== regSeq) return;
      const next = r.items.map(toHit);
      regHits = more ? [...regHits, ...next] : next;
      regCursor = r.next_cursor ?? null;
      regFrom = r.from;
      if (targets.length) compat = { ...compat, ...(await compatFor(next, targets)) };
    } catch (e) {
      if (my === regSeq) error = errText(e);
    } finally {
      if (my === regSeq) regLoading = false;
    }
  }

  $effect(() => {
    void debounced;
    if (registryMode) untrack(() => registry(false));
  });

  // collection view
  $effect(() => {
    const c = selectedCollection;
    if (!c) return;
    let dead = false;
    Promise.allSettled(c.items.map((i) => catalogItem(i.item_id))).then(async (rs) => {
      if (dead) return;
      const ok: SearchHit[] = [];
      const missing: string[] = [];
      rs.forEach((r, i) => (r.status === "fulfilled" ? ok.push(toHit(r.value)) : missing.push(c.items[i].item_id)));
      collHits = ok;
      collMissing = missing;
      if (targets.length) compat = { ...compat, ...(await compatFor(ok, targets)) };
    });
    return () => {
      dead = true;
    };
  });

  async function loadCollections() {
    try {
      collectionsList = await coll.list();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  $effect(() => {
    void stack.rev;
    untrack(loadCollections);
  });

  onMount(() => {
    try {
      const v = localStorage.getItem(VIEW_KEY);
      if (v === "list" || v === "grid") view = v;
    } catch {
      /* default */
    }
    catalogMeta().then((m) => (meta = m)).catch(() => {});
    agentkitTargets().then((x) => (targets = x)).catch(() => {});
    loadKindMatrix().then(() => matrixTick++);
    agentkitDetect(null).then((d) => (detected = d)).catch(() => {});
    marketplacesList().then((m) => (markets = [...m].sort((a, b) => (b.stars ?? 0) - (a.stars ?? 0)))).catch(() => {});
  });

  function setView(v: "grid" | "list") {
    view = v;
    try {
      localStorage.setItem(VIEW_KEY, v);
    } catch {
      /* not remembered */
    }
  }

  function setTab(k: string) {
    tab = k;
    offset = 0;
    categories = [];
    collection = null;
    if (k !== "plugin") marketplace = null;
  }

  function resetFilters() {
    categories = [];
    sources = [];
    licenses = [];
    origins = [];
    marketplace = null;
    hideCollisions = false;
    worksIn = [];
    offset = 0;
  }

  let activeFilters = $derived(
    categories.length + sources.length + licenses.length + origins.length + worksIn.length + (marketplace ? 1 : 0) + (hideCollisions ? 1 : 0),
  );

  function toggleWorks(id: string) {
    worksIn = worksIn.includes(id) ? worksIn.filter((x) => x !== id) : [...worksIn, id];
    offset = 0;
  }

  function toggleStack(h: SearchHit) {
    stack.toggle({ id: h.id, kind: h.kind, name: h.name, category: h.category, description: h.description });
  }

  function addAllVisible() {
    const list = collection ? collHits : registryMode ? regHits : hits;
    const n = stack.addMany(list.map((h) => ({ id: h.id, kind: h.kind, name: h.name, category: h.category, description: h.description })));
    showToast("success", $t("llm.central.catalog.stack.added_n", { count: String(n) }));
  }

  function onkey(e: KeyboardEvent) {
    const el = e.target as HTMLElement;
    const typing = el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
    if ((e.key === "/" && !typing) || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k")) {
      e.preventDefault();
      searchInput?.focus();
      searchInput?.select();
    }
  }

  // collection item actions
  async function collAction(fn: () => Promise<unknown>) {
    try {
      await fn();
      await loadCollections();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function loadCollectionIntoStack(c: Collection, open: boolean) {
    const n = stack.addMany(collHits.map((h) => ({ id: h.id, kind: h.kind, name: h.name, category: h.category, description: h.description })));
    if (!stack.name) stack.set("name", c.name);
    if (c.targets.length) stack.setTargets(c.targets, false);
    if (c.scope === "project" || c.scope === "global" || c.scope === "local") stack.set("scope", c.scope);
    showToast("success", $t("llm.central.catalog.stack.added_n", { count: String(n) }));
    if (open) stack.open = true;
  }

  const licenseLabel = (v: string) => (v === "unknown" ? $t("llm.central.catalog.meta.no_license") : v);
  const toolLabel = (v: string) => targets.find((x) => x.id === v)?.name ?? v;
</script>

<svelte:window onkeydown={onkey} />

<div class="catalog">
  <aside class="side" class:hidden={!showFacets} aria-label={$t("llm.central.catalog.filters")}>
    <CollectionsSidebar list={collectionsList} bind:selected={collection} onchanged={loadCollections} />

    {#if !collection && !registryMode}
      <div class="facets">
        <div class="facet-head">
          <h3>{$t("llm.central.catalog.filters")}</h3>
          {#if activeFilters}<button type="button" class="link" onclick={resetFilters}>{$t("llm.central.catalog.clear_all")} ({activeFilters})</button>{/if}
        </div>
        <label class="toggle">
          <input type="checkbox" bind:checked={hideCollisions} onchange={() => (offset = 0)} />
          {$t("llm.central.catalog.hide_collisions")}
        </label>
        <FacetGroup title={$t("llm.central.catalog.facet.category")} values={facets?.category ?? []} selected={categories} onchange={(v) => { categories = v; offset = 0; }} />
        <FacetGroup title={$t("llm.central.catalog.facet.source")} values={facets?.source ?? []} selected={sources} label={sourceLabel} onchange={(v) => { sources = v; offset = 0; }} />
        <FacetGroup title={$t("llm.central.catalog.facet.license")} values={facets?.license ?? []} selected={licenses} label={licenseLabel} onchange={(v) => { licenses = v; offset = 0; }} />
        <FacetGroup title={$t("llm.central.catalog.facet.origin")} values={facets?.origin_tool ?? []} selected={origins} label={toolLabel} onchange={(v) => { origins = v; offset = 0; }} />
      </div>
    {/if}

    <button type="button" class="sources-btn" onclick={() => (showSources = true)}>
      <span class="plus" aria-hidden="true">+</span> {$t("llm.central.catalog.sources.open")}
    </button>
    {#if meta?.source}
      <p class="attrib">{meta.source.attribution}</p>
    {/if}
  </aside>

  <main class="main">
    <div class="toolbar">
      <div class="search">
        <span class="mag" aria-hidden="true"></span>
        <input
          bind:this={searchInput}
          bind:value={query}
          type="search"
          placeholder={registryMode ? $t("llm.central.catalog.search_registry") : $t("llm.central.catalog.search", { count: String(meta?.total ?? "") })}
          aria-label={$t("llm.central.catalog.search_label")}
        />
        <kbd>/</kbd>
      </div>
      <select bind:value={sort} onchange={() => (offset = 0)} aria-label={$t("llm.central.catalog.sort.label")} disabled={registryMode || !!collection}>
        <option value="relevance">{$t("llm.central.catalog.sort.relevance")}</option>
        <option value="name">{$t("llm.central.catalog.sort.name")}</option>
        <option value="stars">{$t("llm.central.catalog.sort.stars")}</option>
        <option value="updated">{$t("llm.central.catalog.sort.updated")}</option>
      </select>
      <div class="seg" role="radiogroup" aria-label={$t("llm.central.catalog.view.layout")}>
        <button type="button" role="radio" aria-checked={view === "grid"} onclick={() => setView("grid")} title={$t("llm.central.catalog.view.grid")} aria-label={$t("llm.central.catalog.view.grid")}>▦</button>
        <button type="button" role="radio" aria-checked={view === "list"} onclick={() => setView("list")} title={$t("llm.central.catalog.view.list")} aria-label={$t("llm.central.catalog.view.list")}>☰</button>
      </div>
      <button type="button" class="ghost" onclick={() => (showFacets = !showFacets)} aria-pressed={showFacets}>{$t("llm.central.catalog.filters")}</button>
    </div>

    {#if !collection}
      <nav class="tabs" aria-label={$t("llm.central.catalog.kinds")}>
        {#each tabs as k (k)}
          <button type="button" class="tab" aria-current={tab === k ? "page" : undefined} onclick={() => setTab(k)}>
            {$t(`llm.central.catalog.tab.${k}`)}
            <span class="n tabular">{k === "all" ? facetTotal || "" : (kindCount[k] ?? "")}</span>
          </button>
        {/each}
      </nav>

      {#if installedTools.length}
        <div class="works" role="group" aria-label={$t("llm.central.catalog.works_in")}>
          <span class="lbl">{$t("llm.central.catalog.works_in")}</span>
          {#each installedTools as d (d.id)}
            <button type="button" class="chip" class:on={worksIn.includes(d.id)} aria-pressed={worksIn.includes(d.id)} onclick={() => toggleWorks(d.id)}>
              {d.name}{#if d.version}<span class="ver">{d.version}</span>{/if}
            </button>
          {/each}
          <span class="legend" aria-hidden="true">
            <i class="lg native"></i>{$t("llm.central.agentkit.compat.native")}
            <i class="lg converted"></i>{$t("llm.central.agentkit.compat.converted")}
            <i class="lg degraded"></i>{$t("llm.central.agentkit.compat.degraded")}
            <i class="lg unsupported"></i>{$t("llm.central.agentkit.compat.unsupported")}
          </span>
        </div>
      {/if}

      {#if tab === "mcp"}
        <div class="subbar">
          <div class="seg" role="radiogroup" aria-label={$t("llm.central.catalog.mcp_source")}>
            <button type="button" role="radio" aria-checked={mcpMode === "catalog"} onclick={() => (mcpMode = "catalog")}>{$t("llm.central.catalog.mcp_catalog")}</button>
            <button type="button" role="radio" aria-checked={mcpMode === "registry"} onclick={() => (mcpMode = "registry")}>{$t("llm.central.catalog.mcp_registry")}</button>
          </div>
          {#if registryMode && regFrom}<span class="muted small">{$t(`llm.central.catalog.registry_from.${regFrom}`)}</span>{/if}
        </div>
      {/if}

      {#if tab === "plugin"}
        <div class="subbar">
          <label class="muted small" for="mk">{$t("llm.central.catalog.marketplace")}</label>
          <select id="mk" value={marketplace ?? ""} onchange={(e) => { marketplace = (e.currentTarget as HTMLSelectElement).value || null; offset = 0; }}>
            <option value="">{$t("llm.central.catalog.marketplace_all")}</option>
            {#each markets as m (m.repo)}<option value={m.repo}>{m.name || m.repo} ({m.plugins})</option>{/each}
          </select>
        </div>
      {:else if marketplace}
        <div class="subbar">
          <span class="chip on">{marketplace} <button type="button" class="x" onclick={() => (marketplace = null)} aria-label={$t("llm.central.catalog.clear")}>✕</button></span>
        </div>
      {/if}
    {/if}

    <div class="results" aria-busy={loading || regLoading}>
      {#if collection && selectedCollection}
        {@const sc = selectedCollection}
        <header class="coll-head">
          <div>
            <h2>{sc.name}</h2>
            <p class="muted small">
              {$t("llm.central.catalog.collections.count", { count: String(sc.items.length) })}
              {#if sc.targets.length} · {sc.targets.map(toolLabel).join(", ")}{/if}
              {#if sc.scope} · {$t(`llm.central.catalog.scope.${sc.scope}`)}{/if}
            </p>
          </div>
          <div class="coll-acts">
            <button type="button" class="btn" onclick={() => (collection = null)}>{$t("llm.central.catalog.collections.back")}</button>
            <button type="button" class="btn" onclick={() => loadCollectionIntoStack(sc, false)} disabled={!collHits.length}>{$t("llm.central.catalog.collections.to_stack")}</button>
            <button type="button" class="btn primary" onclick={() => loadCollectionIntoStack(sc, true)} disabled={!collHits.length}>{$t("llm.central.catalog.collections.install")}</button>
          </div>
        </header>
        {#if collMissing.length}
          <p class="warn small">{$t("llm.central.catalog.collections.missing", { count: String(collMissing.length) })}: {collMissing.join(", ")}</p>
        {/if}
        {#if !sc.items.length}
          <p class="empty">{$t("llm.central.catalog.collections.empty_items")}</p>
        {/if}
        <ul class="coll-items">
          {#each collHits as h, i (h.id)}
            <li>
              <CatalogCard hit={h} compat={compat[h.id]} tools={toolList} detected={detectedIds} view="list" inStack={stack.has(h.id)} ontoggle={toggleStack} />
              <div class="item-acts">
                <button type="button" onclick={() => collAction(() => coll.move(sc.id, h.id, i - 1))} disabled={i === 0} aria-label={$t("llm.central.catalog.move_up")}>↑</button>
                <button type="button" onclick={() => collAction(() => coll.move(sc.id, h.id, i + 1))} disabled={i === collHits.length - 1} aria-label={$t("llm.central.catalog.move_down")}>↓</button>
                <select
                  aria-label={$t("llm.central.catalog.collections.move_to")}
                  onchange={(e) => {
                    const to = (e.currentTarget as HTMLSelectElement).value;
                    (e.currentTarget as HTMLSelectElement).value = "";
                    if (to) void collAction(async () => { await coll.add(to, h.id); await coll.remove(sc.id, h.id); });
                  }}
                >
                  <option value="">{$t("llm.central.catalog.collections.move_to")}</option>
                  {#each collectionsList.filter((c) => c.id !== sc.id) as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
                </select>
                <button type="button" onclick={() => collAction(() => coll.remove(sc.id, h.id))} aria-label={$t("llm.central.catalog.collections.remove_item")}>✕</button>
              </div>
            </li>
          {/each}
        </ul>
      {:else}
        <div class="count-row">
          <span class="muted small" aria-live="polite">
            {#if registryMode}
              {$t("llm.central.catalog.results", { count: String(regHits.length) })}
            {:else}
              {$t("llm.central.catalog.results", { count: String(total) })}
            {/if}
          </span>
          {#if (registryMode ? regHits : hits).length}
            <button type="button" class="link" onclick={addAllVisible}>{$t("llm.central.catalog.stack.add_visible")}</button>
          {/if}
        </div>

        {#if error}
          <p class="err" role="alert">{error}</p>
        {/if}

        {#if registryMode}
          <div class="grid {view}">
            {#each regHits as h (h.id)}
              <CatalogCard hit={h} compat={compat[h.id]} tools={toolList} detected={detectedIds} {view} inStack={stack.has(h.id)} ontoggle={toggleStack} />
            {/each}
          </div>
          {#if regCursor}
            <div class="pager"><button type="button" class="btn" onclick={() => registry(true)} disabled={regLoading}>{$t("llm.central.catalog.load_more")}</button></div>
          {/if}
          {#if regLoading && !regHits.length}<p class="muted">{$t("llm.central.catalog.loading")}</p>{/if}
        {:else}
          {#if !loading && !hits.length && !error}
            <div class="empty-state">
              <KindIcon kind={tab} size={48} />
              <p>{$t("llm.central.catalog.no_results")}</p>
              {#if activeFilters}<button type="button" class="btn" onclick={resetFilters}>{$t("llm.central.catalog.clear_all")}</button>{/if}
            </div>
          {/if}
          <div class="grid {view}" class:dim={loading}>
            {#each hits as h (h.id)}
              <CatalogCard hit={h} compat={compat[h.id]} tools={toolList} detected={detectedIds} {view} inStack={stack.has(h.id)} ontoggle={toggleStack} onopen={(x) => goto(itemHref(x.id))} />
            {/each}
          </div>
          {#if pages > 1}
            <nav class="pager" aria-label={$t("llm.central.catalog.pages")}>
              <button type="button" class="btn" onclick={() => (offset = 0)} disabled={page === 1} aria-label={$t("llm.central.catalog.first_page")}>«</button>
              <button type="button" class="btn" onclick={() => (offset = Math.max(0, offset - PAGE))} disabled={page === 1} aria-label={$t("llm.central.catalog.prev_page")}>‹</button>
              <span class="tabular">{$t("llm.central.catalog.page_of", { page: String(page), pages: String(pages) })}</span>
              <button type="button" class="btn" onclick={() => (offset = offset + PAGE)} disabled={page >= pages} aria-label={$t("llm.central.catalog.next_page")}>›</button>
              <button type="button" class="btn" onclick={() => (offset = (pages - 1) * PAGE)} disabled={page >= pages} aria-label={$t("llm.central.catalog.last_page")}>»</button>
            </nav>
          {/if}
        {/if}
      {/if}
    </div>
  </main>
</div>

{#if showSources}
  <SourcesDialog
    {meta}
    onclose={() => (showSources = false)}
    onchanged={() => {
      reloadTick++;
      catalogMeta().then((m) => (meta = m)).catch(() => {});
      marketplacesList().then((m) => (markets = [...m].sort((a, b) => (b.stars ?? 0) - (a.stars ?? 0)))).catch(() => {});
    }}
    onbrowse={(repo) => {
      showSources = false;
      setTab("plugin");
      marketplace = repo;
    }}
  />
{/if}

<style>
  .catalog {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-3);
    border-right: 1px solid var(--separator);
    overflow: auto;
  }
  .side.hidden {
    display: none;
  }
  .catalog:has(.side.hidden) {
    grid-template-columns: minmax(0, 1fr);
  }
  .facets {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .facet-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .facet-head h3 {
    margin: 0;
    font-size: var(--text-md);
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .toggle input {
    accent-color: var(--accent);
  }
  .sources-btn {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h-lg);
    padding: 0 var(--space-3);
    border: 1px dashed var(--border-hi);
    border-radius: var(--radius-md);
    background: none;
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .sources-btn:hover {
    background: var(--fill-1);
  }
  .plus {
    font-weight: 700;
    color: var(--accent-hi);
  }
  .attrib {
    margin: 0;
    font-size: var(--text-caption);
    line-height: 1.4;
    color: var(--text-faint);
  }
  .main {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4) var(--space-2);
    flex-wrap: wrap;
  }
  .search {
    position: relative;
    flex: 1;
    min-width: 220px;
    display: flex;
    align-items: center;
  }
  .mag {
    position: absolute;
    left: 10px;
    width: 14px;
    height: 14px;
    background: var(--text-muted);
    -webkit-mask: url(/icons/magnifying-glass.svg) center / contain no-repeat;
    mask: url(/icons/magnifying-glass.svg) center / contain no-repeat;
  }
  .search input {
    width: 100%;
    height: var(--control-h-lg);
    padding: 0 36px 0 32px;
    border: none;
    border-radius: var(--radius-md);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-base);
  }
  .search input:focus-visible {
    outline: var(--focus-ring);
  }
  kbd {
    position: absolute;
    right: 10px;
    padding: 0 5px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-family: var(--font-mono);
  }
  select {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .seg {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .seg button {
    min-width: 30px;
    height: 24px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .seg button[aria-checked="true"] {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .ghost {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .ghost[aria-pressed="true"] {
    color: var(--accent-hi);
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 0 var(--space-4);
    overflow-x: auto;
    scrollbar-width: none;
    border-bottom: 1px solid var(--separator);
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border: none;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
  }
  .tab[aria-current="page"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tab:focus-visible {
    outline: var(--focus-ring);
  }
  .tab .n {
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .works,
  .subbar {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    padding: var(--space-2) var(--space-4) 0;
  }
  .works .lbl {
    margin-right: 4px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .chip .ver {
    font-size: var(--text-caption);
    opacity: 0.6;
  }
  .chip .x {
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
  }
  .legend {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .lg {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-left: 6px;
    border-radius: 2px;
  }
  .lg.native {
    background: var(--success);
  }
  .lg.converted {
    background: var(--blue);
  }
  .lg.degraded {
    background: var(--warning);
  }
  .lg.unsupported {
    box-shadow: inset 0 0 0 1px var(--border-hi);
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-3) var(--space-4) calc(var(--space-9) + var(--space-4));
  }
  .count-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-2);
  }
  .grid.grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-3);
  }
  .grid.list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grid.dim {
    opacity: 0.6;
    transition: opacity var(--duration-base);
  }
  .pager {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-4);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .btn {
    height: var(--control-h);
    min-width: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .btn.primary {
    background: var(--cta);
    color: var(--on-cta);
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent-hi);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--text-sm);
    margin: 0;
  }
  .err {
    color: var(--error);
    font-size: var(--text-sm);
  }
  .warn {
    color: var(--warning);
  }
  .empty,
  .empty-state p {
    color: var(--text-muted);
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) 0;
  }
  .coll-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
    flex-wrap: wrap;
  }
  .coll-head h2 {
    margin: 0;
    font-size: var(--text-xl);
    letter-spacing: var(--track-tight);
  }
  .coll-acts {
    display: flex;
    gap: var(--space-2);
  }
  .coll-items {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .coll-items li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-2);
  }
  .item-acts {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .item-acts button {
    width: 26px;
    height: 26px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    cursor: pointer;
  }
  .item-acts button:disabled {
    opacity: 0.3;
  }
  @media (max-width: 860px) {
    .catalog {
      grid-template-columns: minmax(0, 1fr);
    }
    .side {
      display: none;
    }
    .legend {
      display: none;
    }
  }
</style>
