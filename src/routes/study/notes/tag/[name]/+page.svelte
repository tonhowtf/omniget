<script lang="ts">
  import { page as routePage } from "$app/stores";
  import { t } from "$lib/i18n";
  import {
    notesPagesListByTag,
    notesPagesGetByName,
    notesPagesEnsure,
    notesPagesLinkedRefs,
    notesTagsList,
    notesBlocksPageTree,
    type PageSummary,
    type TagSummary,
    type NotePage,
    type LinkedRef,
    type BlockNode,
  } from "$lib/notes-bridge";

  const tagName = $derived($routePage.params.name ?? "");

  let pages = $state<PageSummary[]>([]);
  let tagInfo = $state<TagSummary | null>(null);
  let descriptionPage = $state<NotePage | null>(null);
  let descriptionBlocks = $state<BlockNode[]>([]);
  let blocksTagged = $state<LinkedRef[]>([]);
  let loading = $state(true);
  let error = $state("");
  let creatingDescription = $state(false);

  function fmtDay(secs: number): string {
    if (!secs) return "—";
    const d = new Date(secs * 1000);
    return d.toLocaleDateString();
  }

  function snippetOf(content: string, max = 120): string {
    const trimmed = content.trim().replace(/\s+/g, " ");
    return trimmed.length > max ? trimmed.slice(0, max - 1) + "…" : trimmed;
  }

  function flattenFirst(nodes: BlockNode[], limit: number): BlockNode[] {
    const out: BlockNode[] = [];
    function walk(list: BlockNode[]) {
      for (const n of list) {
        if (out.length >= limit) return;
        out.push(n);
        if (n.children?.length) walk(n.children);
      }
    }
    walk(nodes);
    return out;
  }

  async function load() {
    loading = true;
    error = "";
    descriptionPage = null;
    descriptionBlocks = [];
    blocksTagged = [];
    try {
      const [byTag, all] = await Promise.all([
        notesPagesListByTag(tagName),
        notesTagsList(200),
      ]);
      pages = byTag;
      tagInfo = all.find((t) => t.name === tagName) ?? null;

      const desc = await notesPagesGetByName(tagName);
      if (desc) {
        descriptionPage = desc;
        const tree = await notesBlocksPageTree(desc.id);
        descriptionBlocks = tree;
        try {
          const refs = await notesPagesLinkedRefs(desc.id);
          blocksTagged = refs;
        } catch (e) {
          console.error("linked refs failed", e);
        }
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function createDescription() {
    if (!tagName) return;
    creatingDescription = true;
    try {
      await notesPagesEnsure({ name: tagName, title: tagName });
      window.location.href = `/study/notes?page=${encodeURIComponent(tagName)}`;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      creatingDescription = false;
    }
  }

  $effect(() => {
    if (tagName) void load();
  });
</script>

<div class="tag-shell" data-surface="notes">
  <header class="head">
    <a href="/study/notes" class="back">← Notas</a>
    <h1 class="page-title"><span class="hash">#</span>{tagName}</h1>
    {#if tagInfo}
      <span class="meta">
        {tagInfo.ref_count} ref · {tagInfo.block_count} bloco{tagInfo.block_count === 1 ? "" : "s"}
      </span>
    {/if}
  </header>

  {#if loading}
    <p class="state">Carregando…</p>
  {:else if error}
    <p class="state err">{error}</p>
  {:else}
    {#if descriptionPage}
      <section class="description">
        <header class="section-head">
          <h2>Sobre esta tag</h2>
          <a
            class="btn ghost sm"
            href={`/study/notes?page=${encodeURIComponent(descriptionPage.name)}`}
          >Editar →</a>
        </header>
        {#if descriptionBlocks.length === 0}
          <p class="muted">Página de descrição existe mas está vazia.</p>
        {:else}
          <ul class="snippet-list">
            {#each flattenFirst(descriptionBlocks, 5) as n (n.id)}
              <li>{snippetOf(n.content, 200)}</li>
            {/each}
          </ul>
        {/if}
      </section>
    {:else}
      <section class="description description-empty">
        <h2>Sobre esta tag</h2>
        <p class="muted">
          Não há página descritiva para <code>#{tagName}</code>. Crie uma para
          documentar o conceito da tag.
        </p>
        <button
          type="button"
          class="btn primary sm"
          onclick={createDescription}
          disabled={creatingDescription}
        >
          {creatingDescription ? $t("study.notes.creating") : $t("study.notes.create_tagged_page", { tag: tagName })}
        </button>
      </section>
    {/if}

    {#if pages.length > 0}
      <section class="pages">
        <h2>Páginas com esta tag</h2>
        <ul>
          {#each pages as p (p.id)}
            <li>
              <a class="page-row" href={`/study/notes?page=${encodeURIComponent(p.name)}`}>
                <span class="title">{p.title ?? p.name}</span>
                <span class="path">{p.name}</span>
                <span class="meta-line">
                  {p.block_count} bloco{p.block_count === 1 ? "" : "s"} · atualizado {fmtDay(p.updated_at)}
                </span>
              </a>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if blocksTagged.length > 0}
      <section class="blocks">
        <h2>Blocos referenciando esta tag ({blocksTagged.length})</h2>
        <ul>
          {#each blocksTagged.slice(0, 50) as r (r.block_id)}
            <li>
              <a class="block-row" href={`/study/notes?page=${encodeURIComponent(r.page_name)}`}>
                <span class="block-page">{r.page_name}</span>
                <span class="block-snippet">{r.snippet}</span>
              </a>
            </li>
          {/each}
        </ul>
        {#if blocksTagged.length > 50}
          <p class="muted">Mostrando 50 de {blocksTagged.length}.</p>
        {/if}
      </section>
    {/if}

    {#if pages.length === 0 && blocksTagged.length === 0}
      <p class="state">Sem páginas ou blocos com tag <code>#{tagName}</code>.</p>
    {/if}
  {/if}
</div>

<style>
  .tag-shell {
    max-width: 820px;
    margin: 0 auto;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    flex-wrap: wrap;
  }
  .back {
    color: var(--tertiary);
    font-size: 12px;
    text-decoration: none;
  }
  .back:hover {
    color: var(--accent);
  }
  .page-title {
    margin: 0;
    font-size: 24px;
    font-weight: 600;
  }
  .hash {
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .meta {
    color: var(--tertiary);
    font-size: 11px;
  }
  .state {
    padding: 24px;
    text-align: center;
    color: var(--secondary);
  }
  .state.err {
    color: var(--error, var(--accent));
  }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .description,
  .pages,
  .blocks {
    background: color-mix(in oklab, var(--input-border) 14%, transparent);
    border: none;
    border-radius: var(--border-radius);
    padding: 16px;
  }
  .description-empty {
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .muted {
    color: var(--tertiary);
    font-size: 13px;
    margin: 0;
  }
  .snippet-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .snippet-list li {
    font-size: 13px;
    line-height: 1.5;
    padding: 6px 10px;
    border-left: 2px solid var(--accent);
    background: color-mix(in oklab, var(--accent) 6%, transparent);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    color: var(--secondary);
  }
  .pages ul,
  .blocks ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .page-row,
  .block-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--surface);
    color: var(--text);
    text-decoration: none;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .page-row:hover,
  .block-row:hover {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 5%, transparent);
  }
  .title {
    font-size: 14px;
    font-weight: 500;
  }
  .path,
  .block-page {
    font-size: 11px;
    color: var(--accent);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .meta-line {
    font-size: 11px;
    color: var(--tertiary);
  }
  .block-snippet {
    font-size: 12px;
    color: var(--secondary);
    line-height: 1.4;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  code {
    font-family: var(--font-mono, ui-monospace, monospace);
    background: color-mix(in oklab, var(--input-border) 30%, transparent);
    padding: 1px 5px;
    border-radius: 3px;
  }
  .btn {
    padding: 6px 12px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    text-decoration: none;
    display: inline-block;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.sm {
    padding: 4px 10px;
    font-size: 11px;
  }
</style>
