<script lang="ts">
  import { onDestroy } from "svelte";
import { t } from "$lib/i18n";
  import {
    notesPagesGetByName,
    notesBlocksPageTree,
    type NotePage,
    type BlockNode,
  } from "$lib/notes-bridge";

  type Props = {
    pageName: string;
    position: { x: number; y: number };
    onClose: () => void;
    onNavigate: (name: string) => void;
  };

  let { pageName, position, onClose, onNavigate }: Props = $props();

  let page = $state<NotePage | null>(null);
  let snippets = $state<string[]>([]);
  let loading = $state(true);
  let error = $state("");
  let popoverEl = $state<HTMLDivElement | undefined>(undefined);

  function snippetOf(content: string, max = 140): string {
    const trimmed = content.trim().replace(/\s+/g, " ");
    if (trimmed.length <= max) return trimmed;
    return trimmed.slice(0, max - 1) + "…";
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

  $effect(() => {
    let cancelled = false;
    loading = true;
    error = "";
    page = null;
    snippets = [];
    (async () => {
      try {
        const p = await notesPagesGetByName(pageName);
        if (cancelled) return;
        if (!p) {
          error = $t("study.notes.nb.page_not_found");
          loading = false;
          return;
        }
        page = p;
        const tree = await notesBlocksPageTree(p.id);
        if (cancelled) return;
        snippets = flattenFirst(tree, 3)
          .map((n) => snippetOf(n.content))
          .filter((s) => s.length > 0);
        loading = false;
      } catch (e) {
        if (cancelled) return;
        error = e instanceof Error ? e.message : String(e);
        loading = false;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  onDestroy(() => {});
</script>

<div
  bind:this={popoverEl}
  class="page-popover"
  style:left={`${position.x}px`}
  style:top={`${position.y}px`}
  role="dialog"
  aria-label={$t("study.notes.nb.preview_aria", { name: pageName })}
>
  {#if loading}
    <div class="state">carregando…</div>
  {:else if error}
    <div class="state err">{error}</div>
  {:else if page}
    <header class="head">
      <button
        type="button"
        class="title-btn"
        onclick={() => {
          onNavigate(page!.name);
          onClose();
        }}
      >
        <span class="title">{page.title ?? page.name}</span>
        <span class="page-name">{page.name}</span>
      </button>
    </header>
    {#if snippets.length === 0}
      <p class="empty">Página vazia</p>
    {:else}
      <ul class="blocks">
        {#each snippets as s, i (i)}
          <li>{s}</li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .page-popover {
    position: fixed;
    z-index: 950;
    min-width: 280px;
    max-width: 360px;
    max-height: 320px;
    overflow-y: auto;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    box-shadow: 0 12px 32px color-mix(in oklab, black 28%, transparent);
    padding: 8px;
  }
  .state {
    padding: 10px;
    color: var(--tertiary);
    font-size: 12px;
    text-align: center;
  }
  .state.err {
    color: var(--error, var(--accent));
  }
  .head {
    margin-bottom: 6px;
  }
  .title-btn {
    display: flex;
    flex-direction: column;
    width: 100%;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 4px 6px;
    border-radius: var(--border-radius);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .title-btn:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .title {
    font-size: 14px;
    font-weight: 600;
  }
  .page-name {
    font-size: 11px;
    color: var(--tertiary);
    font-family: var(--font-mono);
  }
  .blocks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .blocks li {
    font-size: 12px;
    line-height: 1.45;
    color: var(--secondary);
    padding: 4px 6px;
    border-left: 2px solid color-mix(in oklab, var(--accent) 30%, transparent);
    background: color-mix(in oklab, var(--input-border) 14%, transparent);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
  }
  .empty {
    margin: 0;
    padding: 4px 6px;
    color: var(--tertiary);
    font-size: 12px;
    font-style: italic;
  }
</style>
