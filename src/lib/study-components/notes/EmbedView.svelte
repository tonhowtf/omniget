<script lang="ts">
  import { onMount } from "svelte";
  import {
    notesEmbedResolve,
    type EmbedTarget,
    type ResolvedEmbed,
    type BlockNode,
  } from "$lib/notes-bridge";

  type Props = {
    target: EmbedTarget;
    onNavigate?: (pageName: string) => void;
  };

  let { target, onNavigate }: Props = $props();

  type EmbedState =
    | { kind: "loading" }
    | { kind: "error"; message: string }
    | { kind: "resolved"; data: ResolvedEmbed };

  let eState = $state<EmbedState>({ kind: "loading" });

  function snippetOf(content: string, max = 200): string {
    const trimmed = content.trim().replace(/\s+/g, " ");
    return trimmed.length > max ? trimmed.slice(0, max - 1) + "…" : trimmed;
  }

  function flattenBlocks(nodes: BlockNode[], limit: number): BlockNode[] {
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

  async function resolve() {
    eState = { kind: "loading" };
    try {
      const data = await notesEmbedResolve(target);
      eState = { kind: "resolved", data };
    } catch (e) {
      eState = {
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      };
    }
  }

  function navigateToPage(name: string) {
    if (onNavigate) onNavigate(name);
    else window.location.href = `/study/notes?page=${encodeURIComponent(name)}`;
  }

  function targetLabel(): string {
    if (target.kind === "page") return `[[${target.name}]]`;
    return `((${target.uuid.slice(0, 14)}…))`;
  }

  onMount(() => {
    void resolve();
  });
</script>

<aside class="embed-view" data-target-kind={target.kind}>
  <header class="embed-head">
    <span class="embed-icon">⤴</span>
    <span class="embed-target">{targetLabel()}</span>
    <button
      type="button"
      class="refresh-btn"
      onclick={() => void resolve()}
      title="Recarregar embed"
    >↻</button>
  </header>

  {#if eState.kind === "loading"}
    <p class="embed-state">resolvendo…</p>
  {:else if eState.kind === "error"}
    <p class="embed-state err">erro: {eState.message}</p>
  {:else if eState.kind === "resolved"}
    {@const data = eState.data}
    {#if data.kind === "missing"}
      <div class="embed-warning">
        <svg class="warning-icon" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 3l10 18H2z M12 10v5 M12 18v.5" />
        </svg>
        <span>
          {data.target.kind === "page"
            ? `Página "${data.target.name}" não encontrada`
            : "Bloco não encontrado"}
        </span>
      </div>
    {:else if data.kind === "cycle"}
      <div class="embed-warning cycle">
        <svg class="warning-icon" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 3l10 18H2z M12 10v5 M12 18v.5" />
        </svg>
        <span>Embed cíclico bloqueado</span>
      </div>
    {:else if data.kind === "block"}
      <div class="embed-block">
        <p class="embed-content">{snippetOf(data.node.content, 240)}</p>
        {#if data.truncated}
          <p class="embed-note">… (subtree maior, truncado)</p>
        {/if}
      </div>
    {:else if data.kind === "page"}
      <div class="embed-page">
        <button
          type="button"
          class="embed-page-title"
          onclick={() => navigateToPage(data.page.name)}
        >
          <svg class="page-icon" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z M14 3v5h5" />
          </svg>
          {data.page.title ?? data.page.name}
        </button>
        <ul class="embed-block-list">
          {#each flattenBlocks(data.nodes, 8) as n (n.id)}
            <li>{snippetOf(n.content, 160)}</li>
          {/each}
        </ul>
        {#if data.truncated}
          <p class="embed-note">… (mais blocos truncados)</p>
        {/if}
      </div>
    {/if}
  {/if}
</aside>

<style>
  .embed-view {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: color-mix(in oklab, var(--input-border) 14%, transparent);
    border-left: 3px solid var(--accent);
    border-radius: 0 var(--border-radius) var(--border-radius) 0;
    user-select: none;
    font-size: 12px;
  }
  .embed-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .embed-icon {
    color: var(--accent);
    font-size: 13px;
  }
  .embed-target {
    flex: 1 1 auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .refresh-btn {
    border: 0;
    background: transparent;
    color: var(--tertiary);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .refresh-btn:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .embed-state {
    margin: 0;
    color: var(--tertiary);
    font-size: 11px;
  }
  .embed-state.err {
    color: var(--error, var(--accent));
  }
  .embed-warning {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: color-mix(in oklab, var(--warning) 10%, transparent);
    border: 1px solid color-mix(in oklab, var(--warning) 30%, transparent);
    border-radius: var(--border-radius);
    color: var(--warning);
    font-size: 11px;
  }
  .embed-warning.cycle {
    background: color-mix(in oklab, var(--error) 10%, transparent);
    border-color: color-mix(in oklab, var(--error) 30%, transparent);
    color: var(--error);
  }
  .warning-icon {
    flex-shrink: 0;
  }
  .page-icon {
    flex-shrink: 0;
    margin-right: 4px;
    vertical-align: -2px;
  }
  .embed-block {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .embed-content {
    margin: 0;
    color: var(--text);
    font-size: 13px;
    line-height: 1.5;
  }
  .embed-page {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .embed-page-title {
    align-self: flex-start;
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .embed-page-title:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }
  .embed-block-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .embed-block-list li {
    font-size: 12px;
    color: var(--secondary);
    line-height: 1.4;
    padding: 4px 8px;
    background: var(--surface);
    border-radius: var(--border-radius);
  }
  .embed-note {
    margin: 0;
    color: var(--tertiary);
    font-size: 10px;
    font-style: italic;
  }
</style>
