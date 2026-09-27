<script lang="ts">
  // Cartão "N arquivos alterados +A −D" de um turno: árvore de pastas na
  // ordem do diff, +/- por pasta e por arquivo, clique abre o diff no arquivo.
  // Recebe `files` prontos ou carrega sozinho com `path` + `thread` + `turn`.
  import { t } from "$lib/i18n";
  import { compactCount, vcsDiffTurn, vcsError, type ChangeSummary, type FileStatus } from "$lib/central/vcs";

  type Item = { path: string; status: FileStatus | string; additions: number; deletions: number };

  let {
    files = null,
    path = null,
    thread = null,
    turn = null,
    title = null,
    onopenfile,
    onopendiff,
  }: {
    files?: Item[] | ChangeSummary[] | null;
    /** Pasta da thread (worktree ou projeto), para carregar sozinho. */
    path?: string | null;
    thread?: string | null;
    turn?: number | null;
    title?: string | null;
    onopenfile?: (path: string) => void;
    onopendiff?: () => void;
  } = $props();

  let loaded = $state<Item[] | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  $effect(() => {
    if (files || !path || !thread || turn == null || turn < 1) return;
    const [p, th, tu] = [path, thread, turn];
    loading = true;
    error = null;
    vcsDiffTurn(p, th, tu, { stat_only: true })
      .then((d) => (loaded = d.files))
      .catch((e) => (error = vcsError(e).message))
      .finally(() => (loading = false));
  });

  const list: Item[] = $derived((files as Item[] | null) ?? loaded ?? []);

  type Dir = { kind: "dir"; name: string; key: string; add: number; del: number; children: Node[]; order: number };
  type Leaf = { kind: "file"; name: string; item: Item; order: number };
  type Node = Dir | Leaf;

  const tree = $derived.by(() => {
    const root: Dir = { kind: "dir", name: "", key: "", add: 0, del: 0, children: [], order: 0 };
    list.forEach((item, order) => {
      const parts = item.path.split("/");
      let dir = root;
      let key = "";
      for (const part of parts.slice(0, -1)) {
        key = key ? `${key}/${part}` : part;
        let next = dir.children.find((c): c is Dir => c.kind === "dir" && c.name === part);
        if (!next) {
          next = { kind: "dir", name: part, key, add: 0, del: 0, children: [], order };
          dir.children.push(next);
        }
        dir = next;
      }
      dir.children.push({ kind: "file", name: parts[parts.length - 1], item, order });
    });
    // Totais e pastas de filho único achatadas ("src/core/vcs").
    const finish = (d: Dir): Dir => {
      d.children = d.children.map((c) => (c.kind === "dir" ? finish(c) : c));
      d.add = 0;
      d.del = 0;
      for (const c of d.children) {
        d.add += c.kind === "dir" ? c.add : c.item.additions;
        d.del += c.kind === "dir" ? c.del : c.item.deletions;
      }
      if (d.name && d.children.length === 1 && d.children[0].kind === "dir") {
        const only = d.children[0];
        return { ...only, name: `${d.name}/${only.name}` };
      }
      return d;
    };
    return finish(root);
  });

  const totals = $derived(
    list.reduce((a, f) => ({ add: a.add + f.additions, del: a.del + f.deletions }), { add: 0, del: 0 }),
  );

  let closed = $state(new Set<string>());
  const allOpen = $derived(closed.size === 0);

  function toggle(key: string) {
    const next = new Set(closed);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    closed = next;
  }

  function toggleAll() {
    if (!allOpen) {
      closed = new Set();
      return;
    }
    const keys: string[] = [];
    const walk = (d: Dir) => {
      for (const c of d.children)
        if (c.kind === "dir") {
          keys.push(c.key);
          walk(c);
        }
    };
    walk(tree);
    closed = new Set(keys);
  }

  const letter: Record<string, string> = { added: "A", deleted: "D", modified: "M", renamed: "R", binary: "B" };
</script>

{#snippet nodes(children: Node[], depth: number)}
  {#each children as n (n.kind === "dir" ? "d:" + n.key : "f:" + n.item.path)}
    {#if n.kind === "dir"}
      <button class="r dir" style:padding-left="{8 + depth * 14}px" onclick={() => toggle(n.key)}>
        <span class="chev" class:shut={closed.has(n.key)}>▾</span>
        <span class="ico">▤</span>
        <span class="name">{n.name}</span>
        <span class="add">+{compactCount(n.add)}</span><span class="del">−{compactCount(n.del)}</span>
      </button>
      {#if !closed.has(n.key)}
        {@render nodes(n.children, depth + 1)}
      {/if}
    {:else}
      <button
        class="r file"
        style:padding-left="{22 + depth * 14}px"
        title={n.item.path}
        onclick={() => onopenfile?.(n.item.path)}
      >
        <span class="st st-{n.item.status}">{letter[n.item.status] ?? "M"}</span>
        <span class="name">{n.name}</span>
        <span class="add">+{compactCount(n.item.additions)}</span><span class="del">−{compactCount(n.item.deletions)}</span>
      </button>
    {/if}
  {/each}
{/snippet}

<section class="card">
  <header>
    <span class="title">
      {title ?? $t("llm.central.vcs.changed.title", { count: list.length })}
      <span class="add">+{compactCount(totals.add)}</span>
      <span class="del">−{compactCount(totals.del)}</span>
    </span>
    {#if list.some((f) => f.path.includes("/"))}
      <button class="ghost" onclick={toggleAll}>
        {allOpen ? $t("llm.central.vcs.changed.collapse") : $t("llm.central.vcs.changed.expand")}
      </button>
    {/if}
    {#if onopendiff}
      <button class="ghost strong" onclick={() => onopendiff?.()}>{$t("llm.central.vcs.changed.open_diff")}</button>
    {/if}
  </header>
  {#if loading && !list.length}
    <div class="muted">{$t("llm.central.vcs.loading")}</div>
  {:else if error}
    <div class="muted err">{error}</div>
  {:else if !list.length}
    <div class="muted">{$t("llm.central.vcs.changed.none")}</div>
  {:else}
    <div class="tree">{@render nodes(tree.children, 0)}</div>
  {/if}
</section>

<style>
  .card {
    border: 1px solid var(--separator);
    border-radius: var(--radius-md, 10px);
    background: var(--surface);
    overflow: hidden;
    font-size: 12px;
    container-type: inline-size;
  }
  header {
    position: sticky;
    top: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--surface-hi, var(--surface));
    border-bottom: 1px solid var(--separator);
  }
  .title {
    flex: 1;
    display: inline-flex;
    gap: 6px;
    font-weight: 600;
    color: var(--text);
  }
  .ghost {
    background: transparent;
    border: 0;
    color: var(--text-muted);
    font-size: 11px;
    padding: 3px 6px;
    border-radius: 6px;
    cursor: pointer;
  }
  .ghost:hover {
    background: var(--fill-2, var(--surface-mut));
  }
  .ghost.strong {
    color: var(--accent);
  }
  @container (max-width: 22rem) {
    .ghost.strong {
      font-size: 0;
    }
    .ghost.strong::before {
      content: "↗";
      font-size: 12px;
    }
  }
  .tree {
    max-height: 320px;
    overflow: auto;
    padding: 4px 0;
  }
  .r {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    padding: 2px 10px;
    height: 24px;
    cursor: pointer;
    text-align: left;
    font-size: 12px;
  }
  .r:hover {
    background: var(--fill-1, var(--surface-mut));
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dir .name {
    color: var(--text-muted);
  }
  .chev {
    color: var(--text-muted);
    display: inline-block;
    transition: transform 0.12s;
  }
  .chev.shut {
    transform: rotate(-90deg);
  }
  .ico {
    color: var(--text-muted);
    font-size: 11px;
  }
  .st {
    font-family: var(--font-mono, monospace);
    font-size: 10px;
    font-weight: 700;
    width: 14px;
    text-align: center;
    color: var(--orange, #ff9500);
  }
  .st-added {
    color: var(--green, #34c759);
  }
  .st-deleted {
    color: var(--red, #ff3b30);
  }
  .add {
    color: var(--green, #34c759);
    font-variant-numeric: tabular-nums;
  }
  .del {
    color: var(--red, #ff3b30);
    font-variant-numeric: tabular-nums;
  }
  .muted {
    padding: 10px 12px;
    color: var(--text-muted);
  }
  .err {
    color: var(--error, var(--red));
  }
</style>
