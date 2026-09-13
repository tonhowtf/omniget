<script lang="ts">
  import { onMount, tick } from "svelte";
  import { goto } from "$app/navigation";
  import { page as routePage } from "$app/stores";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";

  type SearchHit = {
    block_id: number;
    block_uuid: string;
    page_id: number;
    page_name: string;
    snippet: string;
    rank: number;
    updated_at: number;
  };

  type PageSummary = {
    id: number;
    name: string;
    title: string | null;
    journal_day: number | null;
    block_count: number;
    updated_at: number;
  };

  let query = $state("");
  let blockHits = $state<SearchHit[]>([]);
  let pageHits = $state<PageSummary[]>([]);
  let searching = $state(false);
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let inputRef: HTMLInputElement | null = null;
  let debounceTimer: number | null = null;

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function runSearch() {
    if (debounceTimer) clearTimeout(debounceTimer);
    const q = query.trim();
    if (!q) {
      blockHits = [];
      pageHits = [];
      searching = false;
      return;
    }
    searching = true;
    debounceTimer = window.setTimeout(async () => {
      try {
        const [blocks, pages] = await Promise.all([
          pluginInvoke<SearchHit[]>("study", "study:notes:search:blocks", {
            query: q,
            limit: 40,
          }),
          pluginInvoke<PageSummary[]>("study", "study:notes:search:pages", {
            query: q,
            limit: 20,
          }),
        ]);
        blockHits = blocks;
        pageHits = pages;
      } catch (e) {
        showToast("err", e instanceof Error ? e.message : String(e));
      } finally {
        searching = false;
      }
    }, 200);
  }

  $effect(() => {
    void query;
    runSearch();
  });

  function gotoPage(name: string) {
    goto(`/study/notes?page=${encodeURIComponent(name)}`);
  }

  async function rebuildIndex() {
    try {
      const r = await pluginInvoke<{ indexed: number }>(
        "study",
        "study:notes:search:rebuild",
      );
      showToast("ok", $t("study.notes.index_rebuilt", { n: r.indexed }));
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  function fmtDate(secs: number): string {
    if (!secs) return "—";
    return new Date(secs * 1000).toLocaleDateString();
  }

  function highlightSnippet(s: string): string {
    return s.replace(/<<(.*?)>>/g, "<mark>$1</mark>");
  }

  onMount(async () => {
    const initial = $routePage.url.searchParams.get("q");
    if (initial) {
      query = initial;
    }
    await tick();
    inputRef?.focus();
  });
</script>

<section class="search-page">
  <header class="head">
    <a href="/study/notes" class="back">← Notas</a>
    <h1>Buscar em notas</h1>
    <p class="hint">
      Busca FTS5 (full-text) sobre conteúdo dos blocos. Suporta operadores
      <code>palavra*</code>, <code>"frase exata"</code>,
      <code>palavra1 OR palavra2</code>.
    </p>
  </header>

  <div class="search-bar">
    <input
      type="search"
      placeholder="Digite para buscar… (acentos ignorados)"
      bind:this={inputRef}
      bind:value={query}
    />
    {#if searching}
      <span class="searching">…</span>
    {:else if query}
      <button class="btn ghost sm" onclick={() => (query = "")}>×</button>
    {/if}
    <button class="btn ghost sm" onclick={rebuildIndex} title={$t("study.notes.rebuild_fts")}>
      ⟳
    </button>
  </div>

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  {#if !query}
    <div class="empty-state">
      <h2>Comece a digitar</h2>
      <p>
        Pesquise por blocos e páginas. Use <code>*</code> para wildcards e
        <code>"frase"</code> para busca exata.
      </p>
    </div>
  {:else}
    <div class="results">
      {#if pageHits.length > 0}
        <section>
          <h2>
            Páginas
            <span class="count">{pageHits.length}</span>
          </h2>
          <ul>
            {#each pageHits as p (p.id)}
              <li>
                <button class="row page" onclick={() => gotoPage(p.name)}>
                  <strong>{p.title ?? p.name}</strong>
                  <span class="meta">
                    {p.name} · {p.block_count} blocos · {fmtDate(p.updated_at)}
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if blockHits.length > 0}
        <section>
          <h2>
            Blocos
            <span class="count">{blockHits.length}</span>
          </h2>
          <ul>
            {#each blockHits as h (h.block_id)}
              <li>
                <button class="row block" onclick={() => gotoPage(h.page_name)}>
                  <span class="snippet">{@html highlightSnippet(h.snippet)}</span>
                  <span class="meta">
                    em <code>{h.page_name}</code> · {fmtDate(h.updated_at)}
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if !searching && pageHits.length === 0 && blockHits.length === 0}
        <div class="empty-state">
          <p>Nenhum resultado para "<strong>{query}</strong>".</p>
        </div>
      {/if}
    </div>
  {/if}
</section>

<style>
  .search-page {
    max-width: 880px;
    margin-inline: auto;
    padding: 24px 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .back {
    color: var(--tertiary);
    font-size: 12px;
    text-decoration: none;
    align-self: flex-start;
  }
  .back:hover {
    color: var(--accent);
  }
  .head h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
  }
  .hint {
    color: var(--tertiary);
    font-size: 13px;
    margin: 0;
  }
  .hint code {
    padding: 1px 5px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: 4px;
    font-size: 12px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .search-bar {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
  }
  .search-bar input {
    flex: 1;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 16px;
    padding: 6px 8px;
  }
  .search-bar input:focus {
    outline: none;
  }
  .searching {
    color: var(--tertiary);
    padding: 0 8px;
  }

  .results {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .results section h2 {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--tertiary);
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .count {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    border-radius: 999px;
    padding: 1px 8px;
    font-size: 10px;
    text-transform: none;
  }
  .results ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px 14px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--surface);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    font: inherit;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .row:hover {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 5%, var(--surface));
  }
  .row strong {
    font-size: 14px;
  }
  .row .snippet {
    font-size: 13px;
    line-height: 1.55;
  }
  .row .snippet :global(mark) {
    background: color-mix(in oklab, var(--accent) 28%, transparent);
    color: var(--text);
    padding: 0 2px;
    border-radius: 2px;
    font-weight: 600;
  }
  .row .meta {
    font-size: 11px;
    color: var(--tertiary);
  }
  .row .meta code {
    color: var(--accent);
    font-family: var(--font-mono, ui-monospace, monospace);
  }

  .empty-state {
    text-align: center;
    padding: 60px 20px;
    color: var(--tertiary);
  }
  .empty-state h2 {
    color: var(--text);
    margin: 0 0 8px;
    font-size: 18px;
  }
  .empty-state code {
    padding: 1px 5px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: 4px;
    font-family: var(--font-mono, ui-monospace, monospace);
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
    background: color-mix(
      in oklab,
      var(--error, var(--accent)) 18%,
      var(--surface)
    );
  }

  .btn {
    padding: 4px 10px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 14px;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--tertiary);
  }
  .btn.ghost:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    color: var(--text);
  }
  .btn.sm {
    padding: 2px 8px;
    font-size: 13px;
  }
</style>
