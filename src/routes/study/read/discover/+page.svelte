<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";

  type BrowseItem = {
    id: string;
    title: string;
    description: string | null;
    status: string | null;
    cover_url: string | null;
    author: string | null;
    year: number | null;
  };

  type BrowseResult = {
    items: BrowseItem[];
    page: number;
    limit: number;
    total: number;
  };

  let sourceId = $state("__global__");

  type AvailableSource = {
    id: string;
    name: string;
    base_url: string;
    kind: string;
    always_on?: boolean;
  };

  let apiSources = $state<AvailableSource[]>([
    { id: "mangadex", name: "MangaDex", base_url: "https://api.mangadex.org", kind: "official", always_on: true },
    { id: "comick", name: "ComicK", base_url: "https://api.comick.fun", kind: "official", always_on: true },
  ]);

  type GlobalSourceResult = {
    source_id: string;
    source_name: string;
    kind: string;
    ok: boolean;
    items: BrowseItem[];
    elapsed_ms: number;
    error?: string;
  };

  type GlobalResult = {
    query: string | null;
    page: number;
    sources_total: number;
    sources_alive: number;
    items_total: number;
    results: GlobalSourceResult[];
  };

  let globalResult = $state<GlobalResult | null>(null);

  type BrowserStatus = {
    chrome_path: string | null;
    chrome_found: boolean;
    browser_alive: boolean;
  };
  let browserStatus = $state<BrowserStatus | null>(null);
  let browserPrewarming = $state(false);

  async function loadBrowserStatus() {
    try {
      browserStatus = await pluginInvoke<BrowserStatus>("study", "study:read:browser:status");
    } catch (e) {
      console.error("browser status failed", e);
    }
  }

  async function prewarmBrowser() {
    if (browserPrewarming || !browserStatus?.chrome_found) return;
    if (browserStatus.browser_alive) return;
    browserPrewarming = true;
    try {
      await pluginInvoke("study", "study:read:browser:test", { url: "about:blank" });
      await loadBrowserStatus();
    } catch (e) {
      console.error("prewarm failed", e);
    } finally {
      browserPrewarming = false;
    }
  }

  async function toggleBrowserAlive() {
    if (!browserStatus?.chrome_found) return;
    if (browserStatus.browser_alive) {
      try {
        await pluginInvoke("study", "study:read:browser:close");
        await loadBrowserStatus();
      } catch (e) {
        console.error("browser close failed", e);
      }
    } else {
      void prewarmBrowser();
    }
  }

  function browserPillState(): string {
    if (!browserStatus) return "idle";
    if (!browserStatus.chrome_found) return "not_found";
    if (browserPrewarming) return "launching";
    if (browserStatus.browser_alive) return "ready";
    return "off";
  }

  async function loadSources() {
    try {
      const apis = await pluginInvoke<AvailableSource[]>("study", "study:read:source:list_apis");
      if (Array.isArray(apis) && apis.length > 0) apiSources = apis;
    } catch (e) {
      console.error("list_apis failed", e);
    }
  }

  async function track(item: BrowseItem & { __sourceId?: string }) {
    const usedSource = item.__sourceId ?? (sourceId === "__global__" ? "mangadex" : sourceId);
    if (trackingId) return;
    trackingId = item.id;
    trackedToast = "";
    try {
      const res = await pluginInvoke<{ series_id: number; chapters_inserted: number; chapters_total: number }>(
        "study",
        "study:read:source:track",
        { sourceId: usedSource, seriesRef: item.id },
      );
      trackedToast = $t("study.read.tracked_ok", {
        count: res?.chapters_inserted ?? 0,
        total: res?.chapters_total ?? 0,
      });
      setTimeout(() => (trackedToast = ""), 3500);
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      trackingId = null;
    }
  }
  let query = $state("");
  let page = $state(0);
  let items = $state<BrowseItem[]>([]);
  let total = $state(0);
  let loading = $state(false);
  let errorMsg = $state("");
  let trackingId = $state<string | null>(null);
  let trackedToast = $state("");
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  async function load() {
    loading = true;
    errorMsg = "";
    if (sourceId === "__global__") {
      try {
        const res = await pluginInvoke<GlobalResult>("study", "study:read:source:global_browse_apis", {
          query: query.trim() || undefined,
          page,
          timeoutMs: 8000,
        });
        globalResult = res;
        items = (res?.results ?? []).flatMap((r) => r.items ?? []);
        total = res?.items_total ?? 0;
      } catch (e) {
        errorMsg = e instanceof Error ? e.message : String(e);
        globalResult = null;
        items = [];
        total = 0;
      } finally {
        loading = false;
      }
      return;
    }
    globalResult = null;
    try {
      const res = await pluginInvoke<BrowseResult>("study", "study:read:source:browse", {
        sourceId,
        query: query.trim() || undefined,
        page,
      });
      items = res?.items ?? [];
      total = res?.total ?? 0;
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function onSearchInput() {
    if (searchTimer != null) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      page = 0;
      void load();
    }, 350);
  }

  function goSeries(item: BrowseItem & { __sourceId?: string }) {
    const usedSource = item.__sourceId ?? (sourceId === "__global__" ? "mangadex" : sourceId);
    void (async () => {
      try {
        const res = await pluginInvoke<{ series_id: number }>(
          "study",
          "study:read:source:track",
          { sourceId: usedSource, seriesRef: item.id },
        );
        if (res?.series_id) {
          goto(`/study/read/manga/${res.series_id}`);
        }
      } catch (e) {
        errorMsg = e instanceof Error ? e.message : String(e);
      }
    })();
  }

  function prevPage() {
    if (page > 0) {
      page -= 1;
      void load();
    }
  }

  function nextPage() {
    if ((page + 1) * 24 < total) {
      page += 1;
      void load();
    }
  }

  onMount(async () => {
    await loadSources();
    await load();
    void loadBrowserStatus();
  });
</script>

<section class="discover">
  <header class="head">
    <div class="row-primary">
      <button type="button" class="back-btn" onclick={() => goto("/study/read/manga")}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M15 18l-6-6 6-6"></path>
        </svg>
        <span>{$t("study.read.back_to_manga")}</span>
      </button>
      <h1>{$t("study.read.discover_title")}</h1>
      {#if browserStatus}
        <button
          type="button"
          class="browser-pill"
          class:state-ready={browserPillState() === "ready"}
          class:state-launching={browserPillState() === "launching"}
          class:state-off={browserPillState() === "off"}
          class:state-not-found={browserPillState() === "not_found"}
          onclick={toggleBrowserAlive}
          disabled={!browserStatus.chrome_found}
          title={browserStatus.chrome_found
            ? (browserStatus.chrome_path ?? "")
            : "Chrome/Edge not detected"}
        >
          {#if browserPillState() === "ready"}
            🟢 Browser
          {:else if browserPillState() === "launching"}
            🟠 Launching…
          {:else if browserPillState() === "not_found"}
            ⚠ No browser
          {:else}
            ⚫ Browser off
          {/if}
        </button>
      {/if}
    </div>
    <p class="subtitle">{$t("study.read.discover_subtitle")}</p>
    <div class="row-secondary">
      <label class="src-label">
        <span>{$t("study.read.discover_source")}</span>
        <select bind:value={sourceId} onchange={() => { page = 0; void load(); }}>
          <option value="__global__">{$t("study.read.discover_source_all")}</option>
          {#each apiSources as s (s.id)}
            <option value={s.id}>{s.name}</option>
          {/each}
        </select>
      </label>
      <input
        type="search"
        class="search"
        placeholder={$t("study.read.discover_search_placeholder")}
        bind:value={query}
        oninput={onSearchInput}
      />
    </div>
    {#if trackedToast}
      <p class="muted small success">{trackedToast}</p>
    {/if}
    {#if errorMsg}
      <div class="error-toast">
        <span class="error small">{errorMsg}</span>
      </div>
    {/if}
  </header>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if globalResult}
    <div class="global-summary muted small">
      {$t("study.read.discover_global_summary", {
        alive: globalResult.sources_alive,
        total: globalResult.sources_total,
        items: globalResult.items_total,
      })}
    </div>
    {#each globalResult.results.filter((r) => r.ok && r.items.length > 0) as r (r.source_id)}
      <section class="source-group">
        <header class="source-head">
          <span class="src-name">{r.source_name}</span>
          <span class="muted small">· {r.items.length} · {r.elapsed_ms}ms</span>
        </header>
        <div class="grid">
          {#each r.items as it (r.source_id + ':' + it.id)}
            <article class="card">
              <button type="button" class="card-cover" onclick={() => goSeries({ ...it, __sourceId: r.source_id })}>
                <div class="thumb" style={it.cover_url ? `background-image: url('${it.cover_url}');` : ""}>
                  {#if !it.cover_url}
                    <svg viewBox="0 0 64 64" width="44" height="56" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
                      <rect x="8" y="6" width="48" height="52" rx="2"></rect>
                    </svg>
                  {/if}
                </div>
              </button>
              <div class="meta">
                <div class="title">{it.title}</div>
                {#if it.author}<div class="author">{it.author}</div>{/if}
                <button type="button" class="cta small" onclick={() => track({ ...it, __sourceId: r.source_id })} disabled={trackingId === it.id}>
                  {trackingId === it.id ? $t("study.read.tracking") : $t("study.read.track_btn")}
                </button>
              </div>
            </article>
          {/each}
        </div>
      </section>
    {/each}
    {#if globalResult.results.some((r) => !r.ok)}
      <details class="failed-sources">
        <summary class="muted small">{$t("study.read.discover_global_failed", { count: globalResult.results.filter((r) => !r.ok).length })}</summary>
        <ul>
          {#each globalResult.results.filter((r) => !r.ok) as r (r.source_id)}
            <li class="small muted">
              <strong>{r.source_name}</strong> — {r.error ?? "no items"} ({r.elapsed_ms}ms)
            </li>
          {/each}
        </ul>
      </details>
    {/if}
    {#if globalResult.items_total === 0}
      <p class="muted">{$t("study.read.discover_empty")}</p>
    {/if}
  {:else if items.length === 0}
    <p class="muted">{$t("study.read.discover_empty")}</p>
  {:else}
    <div class="grid">
      {#each items as it (it.id)}
        <article class="card">
          <button type="button" class="card-cover" onclick={() => goSeries(it)}>
            <div class="thumb" style={it.cover_url ? `background-image: url('${it.cover_url}');` : ""}>
              {#if !it.cover_url}
                <svg viewBox="0 0 64 64" width="44" height="56" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
                  <rect x="8" y="6" width="48" height="52" rx="2"></rect>
                </svg>
              {/if}
            </div>
          </button>
          <div class="meta">
            <div class="title">{it.title}</div>
            {#if it.author}
              <div class="author">{it.author}</div>
            {/if}
            <div class="sub">
              {#if it.year}
                <span class="mono">{it.year}</span>
                <span>·</span>
              {/if}
              {#if it.status}
                <span class="status">{it.status}</span>
              {/if}
            </div>
            {#if it.description}
              <p class="description">{it.description}</p>
            {/if}
            <button
              type="button"
              class="cta small"
              onclick={() => track(it)}
              disabled={trackingId === it.id}
            >
              {trackingId === it.id
                ? $t("study.read.tracking")
                : $t("study.read.track_btn")}
            </button>
          </div>
        </article>
      {/each}
    </div>

    <nav class="pagination">
      <button type="button" class="ghost-btn" onclick={prevPage} disabled={page === 0 || loading}>
        ← {$t("study.read.prev_page")}
      </button>
      <span class="mono small muted">{page + 1} / {Math.max(1, Math.ceil(total / 24))}</span>
      <button type="button" class="ghost-btn" onclick={nextPage} disabled={(page + 1) * 24 >= total || loading}>
        {$t("study.read.next_page")} →
      </button>
    </nav>
  {/if}
</section>

<style>
  .discover {
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
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 500;
    letter-spacing: -0.5px;
    color: var(--secondary);
    flex: 1;
  }
  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
    cursor: pointer;
    font-family: inherit;
  }
  .back-btn:hover {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .subtitle {
    margin: 0;
    font-size: 14px;
    color: var(--tertiary);
  }
  .row-secondary {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  .src-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--tertiary);
  }
  .src-label select {
    padding: 6px 10px;
    background: var(--input-bg);
    color: var(--secondary);
    border: none;
    border-radius: var(--border-radius);
    font-family: inherit;
    font-size: 12px;
  }
  .search {
    flex: 1;
    min-width: 240px;
    padding: 6px 12px;
    background: var(--input-bg);
    color: var(--secondary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 13px;
    font-family: inherit;
  }
  .muted { color: var(--tertiary); }
  .small { font-size: 11px; }
  .success { color: var(--success); }
  .error { color: var(--error); }
  .error-toast {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: color-mix(in oklab, var(--error) 12%, transparent);
    border: 1px solid color-mix(in oklab, var(--error) 35%, transparent);
    border-radius: var(--border-radius);
  }
  .browser-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 500;
    border: none;
    background: var(--input-bg);
    color: var(--tertiary);
    cursor: pointer;
    font-family: inherit;
    transition: all 150ms ease;
  }
  .browser-pill:disabled {
    cursor: not-allowed;
    opacity: 0.7;
  }
  .browser-pill:hover:not(:disabled) {
    background: var(--sidebar-highlight);
    color: var(--secondary);
  }
  .browser-pill.state-ready {
    color: color-mix(in oklab, var(--success) 80%, var(--secondary));
    border-color: color-mix(in oklab, var(--success) 35%, var(--input-border));
  }
  .browser-pill.state-launching {
    color: color-mix(in oklab, var(--warning) 90%, var(--secondary));
    border-color: color-mix(in oklab, var(--warning) 50%, var(--input-border));
    background: linear-gradient(
      90deg,
      var(--input-bg) 0%,
      color-mix(in oklab, var(--warning) 12%, var(--input-bg)) 50%,
      var(--input-bg) 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s linear infinite;
  }
  .browser-pill.state-not-found {
    color: color-mix(in oklab, var(--error) 80%, var(--secondary));
    border-color: color-mix(in oklab, var(--error) 35%, var(--input-border));
  }
  .browser-pill.state-off {
    opacity: 0.7;
  }
  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }
  @media (prefers-reduced-motion: reduce) {
    .browser-pill.state-launching {
      animation: none;
    }
  }
  .global-summary {
    padding: 8px 0;
  }
  .source-group {
    margin-bottom: calc(var(--padding) * 2);
    animation: fade-in 220ms ease-out both;
  }
  @keyframes fade-in {
    from { opacity: 0; transform: translateY(4px); }
    to { opacity: 1; transform: translateY(0); }
  }
  @media (prefers-reduced-motion: reduce) {
    .source-group { animation: none; }
  }
  .source-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding-bottom: 8px;
    border-bottom: none;
    margin-bottom: var(--padding);
  }
  .src-name {
    font-size: 14px;
    font-weight: 500;
    color: var(--secondary);
  }
  .failed-sources {
    margin-top: var(--padding);
    padding: 8px 12px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
  }
  .failed-sources summary {
    cursor: pointer;
  }
  .failed-sources ul {
    margin: 8px 0 0;
    padding-left: 16px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: calc(var(--padding) * 1.2);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
    padding: 0;
  }
  .card-cover {
    display: block;
    padding: 0;
    background: transparent;
    border: 0;
    cursor: pointer;
  }
  .thumb {
    aspect-ratio: 2 / 3;
    background: var(--input-bg);
    background-size: cover;
    background-position: center;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tertiary);
    transition: opacity 120ms ease;
  }
  .thumb:hover {
    opacity: 0.9;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 12px 12px;
  }
  .title {
    font-size: 13px;
    font-weight: 500;
    color: var(--secondary);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .author {
    font-size: 11px;
    color: var(--tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    display: flex;
    gap: 4px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--tertiary);
  }
  .description {
    margin: 2px 0 6px;
    font-size: 11px;
    color: var(--tertiary);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 1.35;
  }
  .cta {
    padding: 6px 12px;
    background: var(--cta, var(--accent));
    color: var(--on-cta, white);
    border: 1px solid transparent;
    border-radius: var(--border-radius);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
  }
  .cta.small {
    margin-top: 2px;
  }
  .cta:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    padding: var(--padding) 0;
  }
  .ghost-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    background: transparent;
    color: var(--tertiary);
    border: none;
    border-radius: var(--border-radius);
    font-size: 12px;
    cursor: pointer;
    font-family: inherit;
  }
  .ghost-btn:hover:not(:disabled) {
    color: var(--secondary);
    background: var(--sidebar-highlight);
  }
  .ghost-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .mono {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-variant-numeric: tabular-nums;
  }
</style>
