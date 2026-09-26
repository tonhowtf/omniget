<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { t } from "$lib/i18n";

  type BookItem = {
    id: string;
    title: string;
    author: string | null;
    subjects?: string[];
    languages?: string[];
    cover_url?: string | null;
    download_count?: number;
    downloadable?: boolean;
    internet_archive_id?: string | null;
  };

  type SearchResult = {
    items: BookItem[];
    page: number;
    count?: number;
    has_next?: boolean;
  };

  type SourceInfo = {
    id: string;
    name: string;
    description?: string;
    kind?: string;
    downloadable?: boolean;
    per_item_downloadable?: boolean;
  };

  let sources = $state<SourceInfo[]>([]);
  let sourceId = $state("gutendex");
  const currentSource = $derived(sources.find((s) => s.id === sourceId));
  const canDownload = $derived(currentSource?.downloadable !== false);
  let query = $state("");
  let page = $state(1);
  let items = $state<BookItem[]>([]);
  let hasNext = $state(false);
  let count = $state<number | null>(null);
  let loading = $state(false);
  let errorMsg = $state("");
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  let downloadingId = $state<string | null>(null);
  let toast = $state("");

  async function loadSources() {
    try {
      const list = await pluginInvoke<SourceInfo[]>(
        "study",
        "study:read:book_source:list",
      );
      sources = Array.isArray(list) ? list : [];
      if (sources.length > 0 && !sources.some((s) => s.id === sourceId)) {
        sourceId = sources[0].id;
      }
    } catch (e) {
      console.error(e);
    }
  }

  async function search() {
    loading = true;
    errorMsg = "";
    try {
      const res = await pluginInvoke<SearchResult>(
        "study",
        "study:read:book_source:search",
        { sourceId, query: query.trim() || undefined, page },
      );
      items = res?.items ?? [];
      hasNext = Boolean(res?.has_next);
      count = typeof res?.count === "number" ? res.count : null;
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
      items = [];
    } finally {
      loading = false;
    }
  }

  function onSearchInput() {
    if (searchTimer != null) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      page = 1;
      void search();
    }, 350);
  }

  function prevPage() {
    if (page > 1) {
      page -= 1;
      void search();
    }
  }

  function nextPage() {
    if (hasNext) {
      page += 1;
      void search();
    }
  }

  async function download(b: BookItem) {
    if (downloadingId) return;
    downloadingId = b.id;
    toast = "";
    try {
      const res = await pluginInvoke<{
        book_id: number;
        format: string;
        already_existed: boolean;
      }>("study", "study:read:book_source:download", {
        sourceId,
        bookRef: b.id,
      });
      if (res?.already_existed) {
        toast = $t("study.read.book_already_in_library");
      } else {
        toast = $t("study.read.book_download_ok", { format: res?.format ?? "?" });
      }
      setTimeout(() => (toast = ""), 3500);
    } catch (e) {
      toast = "ERR: " + (e instanceof Error ? e.message : String(e));
    } finally {
      downloadingId = null;
    }
  }

  onMount(async () => {
    await loadSources();
    await search();
  });
</script>

<section class="discover">
  <header class="head">
    <div class="row-primary">
      <button type="button" class="back-btn" onclick={() => goto("/study/read")}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M15 18l-6-6 6-6"></path>
        </svg>
        <span>{$t("study.read.back_to_library")}</span>
      </button>
      <h1>{$t("study.read.book_discover_title")}</h1>
    </div>
    <p class="subtitle">{$t("study.read.book_discover_subtitle")}</p>
    <div class="row-secondary">
      <label class="src-label">
        <span>{$t("study.read.discover_source")}</span>
        <select bind:value={sourceId} onchange={() => { page = 1; void search(); }}>
          {#each sources as s (s.id)}
            <option value={s.id}>{s.name}</option>
          {/each}
        </select>
      </label>
      <input
        type="search"
        class="search"
        placeholder={$t("study.read.book_search_placeholder")}
        bind:value={query}
        oninput={onSearchInput}
      />
    </div>
    {#if currentSource?.description}
      <p class="source-hint muted small">{currentSource.description}</p>
    {/if}
    <div class="advanced">
      <a href="/study/read/books/annas-settings" class="advanced-link">
        {$t("study.read.annas_manage_link")}
      </a>
    </div>
    {#if toast}
      <p class="toast small">{toast}</p>
    {/if}
    {#if errorMsg}
      <p class="error small">{errorMsg}</p>
    {/if}
  </header>

  {#if loading}
    <p class="muted">{$t("study.common.loading")}</p>
  {:else if items.length === 0}
    <p class="muted">{$t("study.read.discover_empty")}</p>
  {:else}
    {#if count != null}
      <p class="muted small">{$t("study.read.book_results_count", { count: count })}</p>
    {/if}
    <div class="grid">
      {#each items as b (b.id)}
        <article class="card">
          <div class="cover" style={b.cover_url ? `background-image: url('${b.cover_url}');` : ""}>
            {#if !b.cover_url}
              <svg viewBox="0 0 64 64" width="44" height="56" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M14 4h28l10 10v42a4 4 0 0 1-4 4H14a4 4 0 0 1-4-4V8a4 4 0 0 1 4-4z"/>
                <path d="M42 4v10h10"/>
              </svg>
            {/if}
          </div>
          <div class="meta">
            <div class="title">{b.title}</div>
            {#if b.author}
              <div class="author">{b.author}</div>
            {/if}
            <div class="sub">
              {#if b.languages && b.languages.length > 0}
                <span class="mono">{b.languages.join(", ")}</span>
                <span>·</span>
              {/if}
              {#if typeof b.download_count === "number"}
                <span class="mono">{b.download_count} dl</span>
              {/if}
            </div>
            {#if b.subjects && b.subjects.length > 0}
              <div class="subjects">
                {#each b.subjects.slice(0, 3) as subj (subj)}
                  <span class="subj">{subj}</span>
                {/each}
              </div>
            {/if}
            {#if !canDownload}
              <span class="muted small">{$t("study.read.book_metadata_only")}</span>
            {:else if currentSource?.per_item_downloadable && b.downloadable === false}
              <span class="borrow-badge" title={$t("study.read.book_borrow_only_hint")}>
                🔒 {$t("study.read.book_borrow_only")}
              </span>
            {:else}
              <button
                type="button"
                class="cta"
                onclick={() => download(b)}
                disabled={downloadingId === b.id}
              >
                {#if currentSource?.per_item_downloadable && b.downloadable === true}
                  🔓
                {/if}
                {downloadingId === b.id
                  ? $t("study.read.book_downloading")
                  : $t("study.read.book_download_btn")}
              </button>
            {/if}
          </div>
        </article>
      {/each}
    </div>
    <nav class="pagination">
      <button type="button" class="ghost-btn" onclick={prevPage} disabled={page <= 1 || loading}>
        ← {$t("study.read.prev_page")}
      </button>
      <span class="mono small muted">{$t("study.read.book_page_label", { current: page })}</span>
      <button type="button" class="ghost-btn" onclick={nextPage} disabled={!hasNext || loading}>
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
  .advanced {
    display: flex;
    justify-content: flex-end;
    margin-top: -4px;
  }
  .borrow-badge {
    display: inline-block;
    padding: 6px 10px;
    margin-top: 4px;
    background: color-mix(in oklab, var(--warning) 15%, transparent);
    color: var(--tertiary);
    border: 1px solid color-mix(in oklab, var(--warning) 35%, transparent);
    border-radius: var(--border-radius);
    font-size: 11px;
    font-weight: 500;
  }
  .advanced-link {
    font-size: 11px;
    color: var(--tertiary);
    text-decoration: none;
  }
  .advanced-link:hover {
    color: var(--accent);
    text-decoration: underline;
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
  .toast {
    padding: 8px 12px;
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--secondary);
    border: 1px solid var(--accent);
    border-radius: var(--border-radius);
  }
  .error { color: var(--error); }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: calc(var(--padding) * 1.2);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 0;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
    padding: 0;
  }
  .cover {
    aspect-ratio: 2 / 3;
    background: var(--input-bg);
    background-size: cover;
    background-position: center;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--tertiary);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px 12px;
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
  .subjects {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 4px 0;
  }
  .subj {
    padding: 1px 6px;
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--secondary);
    border-radius: 999px;
    font-size: 10px;
  }
  .cta {
    padding: 6px 12px;
    margin-top: 4px;
    background: var(--cta, var(--accent));
    color: var(--on-cta, white);
    border: 1px solid transparent;
    border-radius: var(--border-radius);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
  }
  .cta:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .pagination {
    display: flex;
    justify-content: center;
    align-items: center;
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
