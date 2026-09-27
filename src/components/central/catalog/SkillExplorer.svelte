<script lang="ts">
  /**
   * Multi-file viewer for folder items (skill, mod, plugin, template,
   * sandbox): filterable file tree, and for the open file preview / code /
   * slides (Markdown), tree (JSON) or plain code. Binary files show their
   * size and, for images, the picture.
   */
  import { t } from "$lib/i18n";
  import { buildTree, formatBytes, type ItemFiles, type TreeNode } from "$lib/central/catalog";
  import CodeView from "./CodeView.svelte";
  import JsonView from "./JsonView.svelte";
  import MarkdownView from "./MarkdownView.svelte";
  import SlidesView from "./SlidesView.svelte";

  let { files, title = "" }: { files: ItemFiles; title?: string } = $props();

  let filter = $state("");
  let selected = $state("");
  let mode = $state<"preview" | "code" | "slides">("preview");
  let collapsed = $state<Set<string>>(new Set());

  let paths = $derived(
    Object.entries(files.files).map(([path, f]) => ({ path, size: f.size })),
  );
  let visible = $derived(
    filter.trim() ? paths.filter((p) => p.path.toLowerCase().includes(filter.trim().toLowerCase())) : paths,
  );
  let tree = $derived(buildTree(visible));

  $effect(() => {
    if (!selected || !files.files[selected]) {
      selected = files.files[files.entry] ? files.entry : (paths[0]?.path ?? "");
    }
  });

  let file = $derived(selected ? files.files[selected] : null);
  let ext = $derived(selected.split(".").pop()?.toLowerCase() ?? "");
  let isMd = $derived(ext === "md" || ext === "mdx" || ext === "markdown");
  let isJson = $derived(ext === "json");
  let isImage = $derived(["png", "jpg", "jpeg", "gif", "webp", "svg"].includes(ext));
  let mdMode = $derived<"preview" | "code">(mode === "code" ? "code" : "preview");

  function toggle(p: string) {
    const next = new Set(collapsed);
    if (next.has(p)) next.delete(p);
    else next.add(p);
    collapsed = next;
  }

  function flat(nodes: TreeNode[], depth = 0): { node: TreeNode; depth: number }[] {
    const out: { node: TreeNode; depth: number }[] = [];
    for (const n of nodes) {
      out.push({ node: n, depth });
      if (n.dir && !collapsed.has(n.path)) out.push(...flat(n.children, depth + 1));
    }
    return out;
  }
  let rows = $derived(flat(tree));

  function onTreeKey(e: KeyboardEvent) {
    const leafs = rows.filter((r) => !r.node.dir).map((r) => r.node.path);
    const i = leafs.indexOf(selected);
    if (e.key === "ArrowDown" && i < leafs.length - 1) {
      e.preventDefault();
      selected = leafs[i + 1];
    } else if (e.key === "ArrowUp" && i > 0) {
      e.preventDefault();
      selected = leafs[i - 1];
    }
  }

  function mime(): string {
    return ext === "svg" ? "image/svg+xml" : ext === "jpg" ? "image/jpeg" : `image/${ext}`;
  }
</script>

<div class="explorer">
  <aside class="side">
    <input
      class="filter"
      bind:value={filter}
      placeholder={$t("llm.central.catalog.explorer.filter")}
      aria-label={$t("llm.central.catalog.explorer.filter")}
    />
    <p class="sum tabular">
      {$t("llm.central.catalog.explorer.files", { count: String(paths.length) })} · {formatBytes(files.total_bytes)}
    </p>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <ul class="tree" role="tree" tabindex="0" onkeydown={onTreeKey} aria-label={$t("llm.central.catalog.explorer.tree")}>
      {#each rows as { node, depth } (node.path + (node.dir ? "/" : ""))}
        <li role="treeitem" aria-selected={!node.dir && node.path === selected} aria-expanded={node.dir ? !collapsed.has(node.path) : undefined}>
          <button
            type="button"
            class:dir={node.dir}
            class:on={!node.dir && node.path === selected}
            style:padding-left="{8 + depth * 14}px"
            tabindex="-1"
            onclick={() => (node.dir ? toggle(node.path) : (selected = node.path))}
          >
            <span class="ico" aria-hidden="true">{node.dir ? (collapsed.has(node.path) ? "▸" : "▾") : "·"}</span>
            <span class="nm">{node.name}</span>
            {#if node.path === files.entry}<span class="entry">{$t("llm.central.catalog.explorer.main")}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  </aside>

  <section class="main">
    <header class="fh">
      <span class="path mono" title={selected}>{selected}</span>
      {#if isMd}
        <div class="seg" role="tablist" aria-label={$t("llm.central.catalog.view.mode")}>
          <button type="button" role="tab" aria-selected={mode === "preview"} onclick={() => (mode = "preview")}>{$t("llm.central.catalog.view.preview")}</button>
          <button type="button" role="tab" aria-selected={mode === "code"} onclick={() => (mode = "code")}>{$t("llm.central.catalog.view.code")}</button>
          <button type="button" role="tab" aria-selected={mode === "slides"} onclick={() => (mode = "slides")}>{$t("llm.central.catalog.view.slides")}</button>
        </div>
      {/if}
    </header>
    {#if !file}
      <p class="empty">{$t("llm.central.catalog.explorer.empty")}</p>
    {:else if file.encoding === "base64"}
      {#if isImage}
        <img class="img" src={`data:${mime()};base64,${file.content}`} alt={selected} />
      {:else}
        <p class="empty">{$t("llm.central.catalog.explorer.binary", { size: formatBytes(file.size) })}</p>
      {/if}
    {:else if isMd && mode === "slides"}
      {#key selected}<SlidesView text={file.content} {title} />{/key}
    {:else if isMd}
      {#key selected}<MarkdownView text={file.content} mode={mdMode} showModes={false} />{/key}
    {:else if isJson}
      {#key selected}<JsonView text={file.content} />{/key}
    {:else}
      <CodeView text={file.content} />
    {/if}
  </section>
</div>

<style>
  .explorer {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    gap: var(--space-3);
    min-height: 320px;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .filter {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .sum {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .tree {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 64vh;
    overflow: auto;
    border-radius: var(--radius-sm);
  }
  .tree:focus-visible {
    outline: var(--focus-ring);
  }
  .tree button {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    height: 24px;
    border: none;
    border-radius: var(--radius-xs);
    background: none;
    color: var(--text);
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }
  .tree button:hover {
    background: var(--fill-1);
  }
  .tree button.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }
  .tree button.dir {
    color: var(--text-muted);
  }
  .ico {
    width: 12px;
    color: var(--text-faint);
    flex-shrink: 0;
  }
  .nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .entry {
    margin-left: auto;
    padding: 0 4px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    font-size: var(--text-caption);
    color: var(--text-muted);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .fh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .path {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    flex-shrink: 0;
  }
  .seg button {
    height: 24px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .seg button[aria-selected="true"] {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .empty {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .img {
    max-width: 100%;
    max-height: 60vh;
    object-fit: contain;
    border-radius: var(--radius-md);
    background: var(--surface-mut);
  }
  @media (max-width: 760px) {
    .explorer {
      grid-template-columns: 1fr;
    }
  }
</style>
