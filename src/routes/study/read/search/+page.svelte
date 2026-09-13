<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";

  type SearchResult = {
    annotation_id: number;
    book_id: number;
    book_title: string | null;
    page_index: number | null;
    snippet: string;
    color: string | null;
    note: string | null;
    created_secs: number;
  };

  let query = $state("");
  let results = $state<SearchResult[]>([]);
  let searching = $state(false);
  let rebuilding = $state(false);
  let error = $state("");
  let searched = $state(false);
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function runSearch() {
    const q = query.trim();
    if (!q) {
      results = [];
      searched = false;
      return;
    }
    searching = true;
    error = "";
    try {
      const list = await pluginInvoke<SearchResult[]>(
        "study",
        "study:read:search:global",
        { query: q, limit: 100 },
      );
      results = list ?? [];
      searched = true;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      results = [];
    } finally {
      searching = false;
    }
  }

  function debouncedSearch() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(runSearch, 300);
  }

  async function rebuildIndex() {
    rebuilding = true;
    try {
      await pluginInvoke<{ indexed: number }>(
        "study",
        "study:read:search:rebuild",
      );
      showToast("ok", $t("study.read.search.index_rebuilt"));
      if (query.trim()) await runSearch();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      rebuilding = false;
    }
  }

  function openAnnotation(r: SearchResult) {
    const params = new URLSearchParams();
    if (r.page_index != null) params.set("page", String(r.page_index));
    params.set("annotation", String(r.annotation_id));
    goto(`/study/read/${r.book_id}?${params.toString()}`);
  }

  function highlight(text: string, q: string): string {
    if (!q.trim()) return text;
    const safeQ = q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const re = new RegExp(`(${safeQ})`, "gi");
    return text.replace(re, "<mark>$1</mark>");
  }

  function fmtDate(secs: number): string {
    if (!secs) return "";
    return new Date(secs * 1000).toLocaleDateString();
  }

  $effect(() => {
    void query;
    debouncedSearch();
  });
</script>

<section class="study-page">
  <PageHero
    title={$t("study.read.search.title")}
    subtitle={$t("study.read.search.subtitle")}
  />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  <div class="search-box">
    <input
      type="search"
      class="search-input"
      placeholder="Texto, palavra-chave ou frase…"
      bind:value={query}
      autofocus
    />
    <button
      type="button"
      class="btn ghost"
      onclick={rebuildIndex}
      disabled={rebuilding}
      title={$t("study.read.search.rebuild_hint")}
    >
      {rebuilding ? "Reindexando…" : "Reindexar"}
    </button>
  </div>

  {#if error}
    <div class="state err">{error}</div>
  {:else if searching}
    <div class="state">Buscando…</div>
  {:else if !searched}
    <div class="empty">
      <p>Digite uma palavra ou frase pra buscar.</p>
      <p class="hint">
        A busca cobre highlights, notas e o texto dos livros indexados.
      </p>
    </div>
  {:else if results.length === 0}
    <div class="empty">
      <p>Nada encontrado pra "{query}".</p>
      <p class="hint">
        Se você tem certeza que o texto existe, tente
        <button type="button" class="btn-link" onclick={rebuildIndex}>
          reindexar
        </button>.
      </p>
    </div>
  {:else}
    <p class="result-count">
      {results.length === 1 ? "1 resultado" : `${results.length} resultados`}
    </p>
    <ul class="result-list">
      {#each results as r (r.annotation_id)}
        <li class="result-row">
          <button
            type="button"
            class="result-link"
            onclick={() => openAnnotation(r)}
          >
            <div class="result-head">
              <span class="result-book">
                {r.book_title ?? $t("study.read.untitled")}
              </span>
              {#if r.page_index != null}
                <span class="result-page">p. {r.page_index + 1}</span>
              {/if}
              {#if r.color}
                <span
                  class="result-color"
                  style:background={r.color}
                  aria-label="Cor: {r.color}"
                ></span>
              {/if}
            </div>
            <div class="result-snippet">
              {@html highlight(r.snippet, query)}
            </div>
            {#if r.note}
              <div class="result-note">{@html highlight(r.note, query)}</div>
            {/if}
            <div class="result-meta">{fmtDate(r.created_secs)}</div>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 820px;
    margin-inline: auto;
  }

  .search-box {
    display: flex;
    gap: 8px;
  }
  .search-input {
    flex: 1;
    padding: 10px 14px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 14px;
  }
  .search-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .state, .empty {
    padding: 32px 16px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
  }
  .empty .hint {
    margin: 8px 0 0;
    color: var(--tertiary);
    font-size: 12px;
  }
  .state.err { color: var(--error, var(--accent)); }

  .result-count {
    margin: 0;
    color: var(--tertiary);
    font-size: 12px;
  }

  .result-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .result-row {
    display: flex;
  }
  .result-link {
    flex: 1;
    text-align: left;
    border: none;
    border-radius: var(--border-radius);
    background: var(--surface);
    padding: 12px 14px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font: inherit;
    color: inherit;
    transition: border-color 100ms ease, background 100ms ease;
  }
  .result-link:hover {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 5%, var(--surface));
  }

  .result-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .result-book {
    color: var(--secondary);
    font-size: 12px;
    font-weight: 500;
  }
  .result-page {
    color: var(--tertiary);
    font-size: 11px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .result-color {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1px solid color-mix(in oklab, currentColor 20%, transparent);
  }
  .result-snippet {
    color: var(--text);
    font-size: 13px;
    line-height: 1.5;
  }
  .result-snippet :global(mark) {
    background: color-mix(in oklab, var(--accent) 30%, transparent);
    color: var(--text);
    padding: 0 2px;
    border-radius: 2px;
  }
  .result-note {
    margin-top: 4px;
    padding: 4px 8px;
    border-left: 2px solid var(--accent);
    background: color-mix(in oklab, var(--accent) 5%, transparent);
    color: var(--secondary);
    font-size: 12px;
    line-height: 1.5;
  }
  .result-note :global(mark) {
    background: color-mix(in oklab, var(--accent) 30%, transparent);
    padding: 0 2px;
    border-radius: 2px;
  }
  .result-meta {
    color: var(--tertiary);
    font-size: 11px;
  }

  .btn {
    padding: 8px 14px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn-link {
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
  }

  .toast {
    position: fixed;
    bottom: 20px;
    right: 20px;
    padding: 10px 16px;
    border-radius: var(--border-radius);
    background: color-mix(in oklab, var(--accent) 16%, var(--surface));
    color: var(--text);
    font-size: 13px;
    z-index: 100;
  }
  .toast.err {
    background: color-mix(in oklab, var(--error, var(--accent)) 18%, var(--surface));
  }
</style>
