<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import PageHero from "$lib/study-components/PageHero.svelte";

  type TagTreeNode = {
    name: string;
    full_name: string;
    level: number;
    collapsed: boolean;
    used_count: number;
    children: TagTreeNode[];
  };

  let tree = $state<TagTreeNode[]>([]);
  let loading = $state(true);
  let error = $state("");
  let toast = $state<{ kind: "ok" | "err"; msg: string } | null>(null);

  let filter = $state("");
  let totalTags = $state(0);
  let unusedCount = $derived.by(() => {
    let count = 0;
    function walk(nodes: TagTreeNode[]) {
      for (const n of nodes) {
        if (n.used_count === 0) count++;
        walk(n.children);
      }
    }
    walk(tree);
    return count;
  });

  let renameTarget = $state<TagTreeNode | null>(null);
  let renameNewValue = $state("");
  let renameBusy = $state(false);

  let reparentTarget = $state<TagTreeNode | null>(null);
  let reparentNewParent = $state("");
  let reparentBusy = $state(false);

  let cleanupBusy = $state(false);

  function showToast(kind: "ok" | "err", msg: string) {
    toast = { kind, msg };
    setTimeout(() => (toast = null), 2400);
  }

  async function load() {
    loading = true;
    error = "";
    try {
      await pluginInvoke("study", "study:anki:storage:open");
      const nodes = await pluginInvoke<TagTreeNode[]>(
        "study",
        "study:anki:tags:tree",
      );
      tree = nodes ?? [];
      totalTags = countAll(tree);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function countAll(nodes: TagTreeNode[]): number {
    let n = 0;
    for (const node of nodes) {
      n += 1 + countAll(node.children);
    }
    return n;
  }

  async function toggleCollapse(node: TagTreeNode) {
    const next = !node.collapsed;
    try {
      await pluginInvoke("study", "study:anki:tags:set_collapsed", {
        tag: node.full_name,
        collapsed: next,
      });
      node.collapsed = next;
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    }
  }

  function askRename(node: TagTreeNode) {
    renameTarget = node;
    renameNewValue = node.full_name;
  }

  async function confirmRename() {
    if (!renameTarget) return;
    const newName = renameNewValue.trim();
    if (!newName || newName === renameTarget.full_name) {
      renameTarget = null;
      return;
    }
    renameBusy = true;
    try {
      const r = await pluginInvoke<{ updated: number }>(
        "study",
        "study:anki:tags:rename",
        { old: renameTarget.full_name, new: newName },
      );
      showToast(
        "ok",
        r.updated === 0
          ? "Tag renomeada"
          : r.updated === 1
            ? "Tag renomeada · 1 nota atualizada"
            : `Tag renomeada · ${r.updated} notas atualizadas`,
      );
      renameTarget = null;
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      renameBusy = false;
    }
  }

  function askReparent(node: TagTreeNode) {
    reparentTarget = node;
    const sep = node.full_name.lastIndexOf("::");
    reparentNewParent = sep === -1 ? "" : node.full_name.slice(0, sep);
  }

  async function confirmReparent() {
    if (!reparentTarget) return;
    reparentBusy = true;
    try {
      const newParent = reparentNewParent.trim();
      const r = await pluginInvoke<{ updated: number }>(
        "study",
        "study:anki:tags:reparent",
        {
          tag: reparentTarget.full_name,
          newParent: newParent === "" ? null : newParent,
        },
      );
      showToast(
        "ok",
        r.updated === 0
          ? "Tag movida"
          : r.updated === 1
            ? "Tag movida · 1 nota atualizada"
            : `Tag movida · ${r.updated} notas atualizadas`,
      );
      reparentTarget = null;
      await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      reparentBusy = false;
    }
  }

  async function clearUnused() {
    cleanupBusy = true;
    try {
      const r = await pluginInvoke<{ removed: number }>(
        "study",
        "study:anki:tags:clear_unused",
      );
      showToast(
        "ok",
        r.removed === 0
          ? $t("study.anki.tags.no_unused")
          : r.removed === 1
            ? "1 tag removida"
            : $t("study.anki.tags.removed_many", { n: r.removed }),
      );
      if (r.removed > 0) await load();
    } catch (e) {
      showToast("err", e instanceof Error ? e.message : String(e));
    } finally {
      cleanupBusy = false;
    }
  }

  function matchesFilter(node: TagTreeNode, q: string): boolean {
    if (!q) return true;
    if (node.full_name.toLowerCase().includes(q)) return true;
    return node.children.some((c) => matchesFilter(c, q));
  }

  let visibleTree = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return tree;
    function prune(nodes: TagTreeNode[]): TagTreeNode[] {
      const out: TagTreeNode[] = [];
      for (const n of nodes) {
        if (matchesFilter(n, q)) {
          out.push({ ...n, children: prune(n.children) });
        }
      }
      return out;
    }
    return prune(tree);
  });

  onMount(load);
</script>

<section class="study-page">
  <PageHero
    title="Tags"
    subtitle={totalTags === 0
      ? $t("study.anki.tags.subtitle")
      : totalTags === 1
        ? $t("study.anki.tags.count_one_unused", { total: 1, unused: 1 })
        : $t("study.anki.tags.count_many_unused", { total: totalTags, unused: unusedCount })}
  />

  {#if toast}
    <div class="toast" class:err={toast.kind === "err"} role="status">
      {toast.msg}
    </div>
  {/if}

  <div class="toolbar">
    <input
      class="filter"
      type="search"
      placeholder={$t("study.anki.tags.filter_placeholder")}
      bind:value={filter}
    />
    <button
      class="btn ghost sm"
      onclick={clearUnused}
      disabled={cleanupBusy || unusedCount === 0}
    >
      {cleanupBusy ? $t("study.anki.tags.cleaning") : $t("study.anki.tags.clean_unused")}
    </button>
  </div>

  {#if loading}
    <div class="state">{$t("study.anki.tags.loading")}</div>
  {:else if error}
    <div class="state err">{error}</div>
    <button class="btn ghost" onclick={load}>Tentar de novo</button>
  {:else if tree.length === 0}
    <div class="empty">
      <p>{$t("study.anki.tags.no_tags")}</p>
      <p class="hint">{$t("study.anki.tags.no_tags_hint")}</p>
    </div>
  {:else if visibleTree.length === 0}
    <div class="empty">
      <p>{$t("study.anki.tags.no_match", { filter })}</p>
    </div>
  {:else}
    <ul class="tree">
      {#each visibleTree as node (node.full_name)}
        {@render renderNode(node)}
      {/each}
    </ul>
  {/if}
</section>

{#snippet renderNode(node: TagTreeNode)}
  <li class="tag-row" style:--depth={node.level}>
    <div class="tag-line">
      {#if node.children.length > 0}
        <button
          type="button"
          class="caret"
          aria-expanded={!node.collapsed}
          aria-label={node.collapsed ? "Expandir" : "Recolher"}
          onclick={() => toggleCollapse(node)}
        >
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d={node.collapsed ? "M9 6l6 6-6 6" : "M6 9l6 6 6-6"} />
          </svg>
        </button>
      {:else}
        <span class="caret-spacer" aria-hidden="true"></span>
      {/if}

      <span class="tag-name" class:unused={node.used_count === 0}>
        {node.name}
      </span>

      <span class="tag-count" class:unused={node.used_count === 0}>
        {node.used_count}
      </span>

      <div class="tag-actions">
        <button
          type="button"
          class="btn ghost xs"
          onclick={() => askRename(node)}
        >
          Renomear
        </button>
        <button
          type="button"
          class="btn ghost xs"
          onclick={() => askReparent(node)}
        >
          Mover
        </button>
      </div>
    </div>

    {#if node.children.length > 0 && !node.collapsed}
      <ul class="tree">
        {#each node.children as child (child.full_name)}
          {@render renderNode(child)}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

{#if renameTarget}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={() => (renameTarget = null)}
    onkeydown={(e) => { if (e.key === "Escape") renameTarget = null; }}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="rename-title"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); renameTarget = null; } }}
    >
      <h3 id="rename-title">Renomear tag</h3>
      <p class="modal-hint">
        Use <code>::</code> para hierarquia (ex: <code>livro::cap1</code>).
      </p>
      <input
        class="modal-input"
        type="text"
        bind:value={renameNewValue}
        placeholder={renameTarget.full_name}
        onkeydown={(e) => { if (e.key === "Enter") confirmRename(); }}
      />
      <footer class="modal-foot">
        <button
          type="button"
          class="btn ghost"
          onclick={() => (renameTarget = null)}
          disabled={renameBusy}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn primary"
          onclick={confirmRename}
          disabled={renameBusy || !renameNewValue.trim() || renameNewValue.trim() === renameTarget.full_name}
        >
          {renameBusy ? "Renomeando…" : "Renomear"}
        </button>
      </footer>
    </div>
  </div>
{/if}

{#if reparentTarget}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={() => (reparentTarget = null)}
    onkeydown={(e) => { if (e.key === "Escape") reparentTarget = null; }}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="reparent-title"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); reparentTarget = null; } }}
    >
      <h3 id="reparent-title">Mover <code>{reparentTarget.full_name}</code></h3>
      <p class="modal-hint">
        Novo parent (deixe vazio para mover pra raiz). Use <code>::</code> para
        encadear níveis.
      </p>
      <input
        class="modal-input"
        type="text"
        bind:value={reparentNewParent}
        placeholder="(raiz)"
        onkeydown={(e) => { if (e.key === "Enter") confirmReparent(); }}
      />
      <footer class="modal-foot">
        <button
          type="button"
          class="btn ghost"
          onclick={() => (reparentTarget = null)}
          disabled={reparentBusy}
        >
          Cancelar
        </button>
        <button
          type="button"
          class="btn primary"
          onclick={confirmReparent}
          disabled={reparentBusy}
        >
          {reparentBusy ? "Movendo…" : "Mover"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .study-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.25);
    width: 100%;
    max-width: 820px;
    margin-inline: auto;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .filter {
    flex: 1;
    padding: 8px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .filter:focus {
    outline: none;
    border-color: var(--accent);
  }

  .state {
    padding: calc(var(--padding) * 2);
    text-align: center;
    color: var(--secondary);
    font-size: 14px;
  }
  .state.err {
    color: var(--error, var(--accent));
  }

  .empty {
    padding: 32px 16px;
    text-align: center;
    color: var(--secondary);
    font-size: 13px;
  }
  .empty .hint {
    margin: 4px 0 0;
    color: var(--tertiary);
    font-size: 12px;
  }

  .tree {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .tag-row {
    --indent: calc(var(--depth, 0) * 18px);
    display: flex;
    flex-direction: column;
  }
  .tag-line {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px calc(8px + var(--indent));
    border-radius: var(--border-radius);
    transition: background 100ms ease;
  }
  .tag-line:hover {
    background: color-mix(in oklab, var(--accent) 5%, transparent);
  }
  .tag-line:hover .tag-actions {
    opacity: 1;
  }

  .caret,
  .caret-spacer {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .caret {
    border: 0;
    background: transparent;
    color: var(--tertiary);
    cursor: pointer;
    border-radius: 4px;
  }
  .caret:hover {
    color: var(--text);
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }

  .tag-name {
    flex: 1;
    min-width: 0;
    color: var(--text);
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tag-name.unused {
    color: var(--tertiary);
  }

  .tag-count {
    flex-shrink: 0;
    padding: 0 8px;
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
  }
  .tag-count.unused {
    opacity: 0.5;
  }

  .tag-actions {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity 100ms ease;
  }

  .btn {
    padding: 6px 12px;
    border-radius: var(--border-radius);
    border: 1px solid transparent;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--on-accent, var(--on-cta, white));
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--accent-hover, var(--accent));
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--input-border);
    color: var(--text);
  }
  .btn.ghost:hover:not(:disabled) {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .btn.sm {
    padding: 5px 10px;
    font-size: 12px;
  }
  .btn.xs {
    padding: 3px 8px;
    font-size: 11px;
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
    box-shadow: 0 8px 24px color-mix(in oklab, black 20%, transparent);
    z-index: 100;
  }
  .toast.err {
    background: color-mix(in oklab, var(--error, var(--accent)) 18%, var(--surface));
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: var(--dialog-backdrop, rgba(0, 0, 0, 0.55));
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    padding: 16px;
  }
  .modal {
    background: var(--popup-bg, var(--surface));
    border: none;
    border-radius: var(--border-radius);
    padding: 20px;
    max-width: 460px;
    width: 100%;
    box-shadow: 0 20px 50px color-mix(in oklab, black 30%, transparent);
  }
  .modal h3 {
    margin: 0 0 8px;
    font-size: 15px;
    font-weight: 600;
  }
  .modal-hint {
    margin: 0 0 12px;
    color: var(--tertiary);
    font-size: 12px;
  }
  .modal-input {
    width: 100%;
    padding: 8px 12px;
    border: none;
    border-radius: var(--border-radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    font-family: var(--font-mono, ui-monospace, monospace);
    margin-bottom: 16px;
  }
  .modal-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    border-top: none;
    padding-top: 12px;
  }

  code {
    padding: 1px 6px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border-radius: 4px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px;
    color: var(--text);
  }
</style>
