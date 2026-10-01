<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openPlayer } from "$lib/video/player.svelte";
  import { ui } from "$lib/video/ui.svelte";
  import type { MediaItem, Root } from "$lib/video/types";
  import ContextMenu, { type MenuItem } from "$components/music/ContextMenu.svelte";
  import { addFolder, itemMenu, markWatched } from "./parts/actions";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import Skel from "./parts/Skel.svelte";
  import Thumb from "./parts/Thumb.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { baseName, errText, fmtDur, onLibraryChanged, ratio, ready } from "./parts/util";

  let { path = undefined }: { path?: string | null } = $props();

  type Node = { name: string; path: string; count: number; children: Node[] };

  let roots = $state<Root[] | null>(null);
  let byRoot = $state<Record<string, MediaItem[]>>({});
  let error = $state<string | null>(null);
  let selected = $state<string | null>(null);
  let expanded = $state<Record<string, boolean>>({});
  let confirmRemove = $state<string | null>(null);
  let cm = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  const norm = (p: string) => p.replace(/[\\/]+$/, "");

  async function load() {
    try {
      await ready();
      const s = await api.status();
      const lists = await Promise.all(s.roots.map((r) => api.items({ root: r.path, limit: 100000 }).catch(() => [] as MediaItem[])));
      const map: Record<string, MediaItem[]> = {};
      s.roots.forEach((r, i) => (map[r.path] = lists[i]));
      byRoot = map;
      roots = s.roots;
      error = null;
      if (!selected && s.roots.length) {
        selected = s.roots[0].path;
        expanded[s.roots[0].path] = true;
      }
    } catch (e) {
      error = errText(e);
      roots ??= [];
    }
  }

  onMount(() => {
    const start = path ?? (ui.view.name === "folders" ? ui.view.path : null);
    if (start) selected = start;
    void load();
    return onLibraryChanged(() => void load());
  });

  function buildTree(root: Root): Node {
    const top: Node = { name: baseName(root.path), path: norm(root.path), count: 0, children: [] };
    const sep = root.path.includes("\\") && !root.path.includes("/") ? "\\" : "/";
    for (const it of byRoot[root.path] ?? []) {
      top.count++;
      const rel = norm(it.folder).slice(norm(root.path).length).split(/[\\/]/).filter(Boolean);
      let node = top;
      for (const seg of rel) {
        let child = node.children.find((c) => c.name === seg);
        if (!child) {
          child = { name: seg, path: `${node.path}${sep}${seg}`, count: 0, children: [] };
          node.children.push(child);
        }
        child.count++;
        node = child;
      }
    }
    const coll = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
    const sortRec = (n: Node) => {
      n.children.sort((a, b) => coll.compare(a.name, b.name));
      n.children.forEach(sortRec);
    };
    sortRec(top);
    return top;
  }

  let trees = $derived((roots ?? []).map((r) => ({ root: r, node: buildTree(r) })));
  let selRoot = $derived((roots ?? []).find((r) => selected && (norm(selected) === norm(r.path) || norm(selected).startsWith(norm(r.path)))) ?? null);
  let here = $derived.by(() => {
    if (!selected || !selRoot) return [];
    const coll = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
    return (byRoot[selRoot.path] ?? []).filter((x) => norm(x.folder) === norm(selected!)).sort((a, b) => coll.compare(baseName(a.path), baseName(b.path)));
  });
  let subfolders = $derived.by(() => {
    if (!selected) return [];
    const find = (n: Node): Node | null => (n.path === norm(selected!) ? n : n.children.reduce<Node | null>((acc, c) => acc ?? find(c), null));
    for (const tr of trees) {
      const n = find(tr.node);
      if (n) return n.children;
    }
    return [];
  });

  function pick(p: string) {
    selected = p;
    expanded[p] = true;
  }

  async function rescan() {
    try {
      await api.scan();
      showToast("info", $t("video.folders.rescan_started") as string);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  async function removeRoot(r: Root) {
    try {
      await api.rootsRemove(r.path);
      confirmRemove = null;
      if (selRoot?.path === r.path) selected = null;
      showToast("success", $t("video.folders.removed", { name: baseName(r.path) }) as string);
      void load();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function menu(e: MouseEvent, it: MediaItem) {
    e.preventDefault();
    cm = { x: e.clientX, y: e.clientY, items: itemMenu(it, () => void load()) };
  }

  function playFolder() {
    const first = here.find((x) => !x.progress?.watched) ?? here[0];
    if (first) void openPlayer({ kind: "local", id: first.id });
  }
</script>

{#snippet tree(n: Node, depth: number, offline: boolean)}
  {@const open = !!expanded[n.path]}
  <div class="tn" role="treeitem" aria-expanded={n.children.length ? open : undefined} aria-selected={selected != null && norm(selected) === n.path}>
    <div class="tr" class:sel={selected != null && norm(selected) === n.path} style:padding-left="{6 + depth * 14}px">
      {#if n.children.length}
        <button class="tw" class:open onclick={() => (expanded[n.path] = !open)} aria-label={(open ? $t("video.folders.collapse") : $t("video.folders.expand")) as string}>
          <VIcon name="chevron-right" size={12} />
        </button>
      {:else}
        <span class="tw"></span>
      {/if}
      <button class="tl" onclick={() => pick(n.path)}>
        <VIcon name={offline && depth === 0 ? "folder-off" : "folder"} size={15} />
        <span class="tln">{n.name}</span>
        <span class="tlc">{n.count}</span>
      </button>
    </div>
    {#if open && n.children.length}
      <div role="group">
        {#each n.children as c (c.path)}
          {@render tree(c, depth + 1, offline)}
        {/each}
      </div>
    {/if}
  </div>
{/snippet}

<div class="page">
  <Head title={$t("video.folders.title") as string}>
    {#snippet actions()}
      <button class="btn btn-sm" onclick={() => void addFolder("library").then((ok) => { if (ok) void load(); })}><VIcon name="plus" size={13} /> {$t("video.home.add_folder")}</button>
      <button class="btn btn-sm" onclick={() => void addFolder("course").then((ok) => { if (ok) void load(); })}><VIcon name="plus" size={13} /> {$t("video.home.import_course")}</button>
      {#if roots?.length}
        <button class="btn btn-sm btn-ghost" onclick={() => void rescan()}><VIcon name="refresh" size={13} /> {$t("video.folders.rescan")}</button>
      {/if}
    {/snippet}
  </Head>

  {#if error}
    <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
  {/if}

  {#if !roots}
    <Skel kind="list" count={8} />
  {:else if !roots.length}
    <Empty icon="folders" title={$t("video.folders.empty_title") as string} body={$t("video.folders.empty_body") as string}>
      <button class="btn btn-primary" onclick={() => void addFolder("library").then((ok) => { if (ok) void load(); })}><VIcon name="folder" size={14} /> {$t("video.home.add_folder")}</button>
    </Empty>
  {:else}
    <div class="split">
      <div class="side" role="tree" aria-label={$t("video.folders.title") as string}>
        {#each trees as tr (tr.root.path)}
          <div class="rootblock">
            {@render tree(tr.node, 0, !tr.root.online)}
            <div class="rmeta">
              {#if !tr.root.online}<span class="tag tag-warning">{$t("video.common.offline")}</span>{/if}
              {#if tr.root.kind === "course"}<span class="tag tag-info">{$t("video.folders.kind_course")}</span>{/if}
            </div>
          </div>
        {/each}
      </div>

      <section class="main">
        {#if selRoot && selected}
          <div class="mh">
            <div class="mt">
              <h2>{baseName(selected)}</h2>
              <small title={selected}>{selected}</small>
            </div>
            {#if here.length}
              <button class="btn btn-sm btn-primary" onclick={playFolder}><VIcon name="play" size={12} /> {$t("video.folders.play_folder")}</button>
              <button class="btn btn-sm" onclick={() => void markWatched(here.map((x) => x.key), true, () => void load())}><VIcon name="check" size={12} /> {$t("video.folders.mark_all_watched")}</button>
            {/if}
          </div>

          {#if !selRoot.online}
            <div class="notice notice-warning">
              <VIcon name="folder-off" size={16} />
              <div class="notice-body">{$t("video.folders.offline_notice")}</div>
            </div>
          {/if}

          {#if subfolders.length}
            <div class="subs">
              {#each subfolders as s (s.path)}
                <button class="sub" onclick={() => pick(s.path)}>
                  <VIcon name="folder" size={16} />
                  <span>{s.name}</span>
                  <small>{s.count}</small>
                </button>
              {/each}
            </div>
          {/if}

          {#if here.length}
            <ul class="files" role="list">
              {#each here as it (it.id)}
                {@const r = ratio(it.progress)}
                <li>
                  <button class="file" class:seen={it.progress?.watched} class:off={it.offline} onclick={() => void openPlayer({ kind: "local", id: it.id })} oncontextmenu={(e) => menu(e, it)}>
                    <span class="fth">
                      <Thumb src={it.thumb} seed={it.title} radius="var(--radius-sm)" />
                      {#if r != null}<span class="fbar"><span style:width="{r * 100}%"></span></span>{/if}
                    </span>
                    <span class="ftx">
                      <b>{baseName(it.path)}</b>
                      <small>
                        {fmtDur(it.duration_ms)}
                        {#if it.height}· {it.height}p{/if}
                        {#if it.offline}· {$t("video.common.offline")}{/if}
                      </small>
                    </span>
                    {#if it.progress?.watched}<span class="fseen" title={$t("video.common.watched") as string}><VIcon name="check-circle" size={16} /></span>{/if}
                  </button>
                </li>
              {/each}
            </ul>
          {:else if !subfolders.length}
            <Empty compact icon="folders" title={$t("video.folders.no_videos") as string} />
          {/if}

          {#if norm(selected) === norm(selRoot.path)}
            <div class="danger-zone">
              {#if confirmRemove === selRoot.path}
                <div class="notice notice-warning confirm">
                  <VIcon name="info" size={16} />
                  <div class="notice-body">
                    <span class="notice-title">{$t("video.folders.remove_confirm_title", { name: baseName(selRoot.path) })}</span>
                    <span class="notice-text">{$t("video.folders.remove_confirm_body")}</span>
                    <div class="cf-acts">
                      <button class="btn btn-sm btn-destructive" onclick={() => void removeRoot(selRoot!)}>{$t("video.folders.remove_from_library")}</button>
                      <button class="btn btn-sm" onclick={() => (confirmRemove = null)}>{$t("video.common.cancel")}</button>
                    </div>
                  </div>
                </div>
              {:else}
                <button class="btn btn-sm btn-ghost" onclick={() => (confirmRemove = selRoot!.path)}><VIcon name="close" size={12} /> {$t("video.folders.remove_from_library")}</button>
                <small class="hint">{$t("video.folders.remove_hint")}</small>
              {/if}
            </div>
          {/if}
        {:else}
          <Empty compact icon="folders" title={$t("video.folders.pick") as string} />
        {/if}
      </section>
    </div>
  {/if}
</div>

{#if cm}<ContextMenu x={cm.x} y={cm.y} items={cm.items} onclose={() => (cm = null)} />{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
  }
  .split {
    display: grid;
    grid-template-columns: minmax(200px, 280px) 1fr;
    gap: var(--space-5);
    align-items: start;
  }
  @media (max-width: 760px) {
    .split {
      grid-template-columns: 1fr;
    }
  }
  .side {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-2);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    max-height: calc(100vh - 160px);
    overflow: auto;
  }
  .rmeta {
    display: flex;
    gap: var(--space-1);
    padding-left: 26px;
  }
  .rmeta:empty {
    display: none;
  }
  .tr {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius-sm);
  }
  .tr:hover {
    background: var(--fill-1);
  }
  .tr.sel {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .tw {
    all: unset;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    border-radius: 4px;
    color: var(--text-dim);
    cursor: pointer;
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .tw.open {
    transform: rotate(90deg);
  }
  .tl {
    all: unset;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding-right: 6px;
    cursor: pointer;
    font-size: var(--text-sm);
  }
  .tl:focus-visible,
  .tw:focus-visible {
    outline: var(--focus-ring);
  }
  .tln {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tlc {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }
  .mh {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .mt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .mt h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .mt small {
    color: var(--text-dim);
    font-size: var(--text-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .subs {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-2);
  }
  .sub {
    all: unset;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    cursor: pointer;
    min-width: 0;
  }
  .sub:hover {
    background: var(--fill-2);
  }
  .sub:focus-visible {
    outline: var(--focus-ring);
  }
  .sub span {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
  }
  .sub small {
    color: var(--text-dim);
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .file {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 6px;
    border-radius: var(--radius-md);
    cursor: pointer;
  }
  .file:hover {
    background: var(--fill-1);
  }
  .file:focus-visible {
    outline: var(--focus-ring);
  }
  .file.off {
    opacity: 0.55;
  }
  .fth {
    position: relative;
    width: 112px;
    flex-shrink: 0;
  }
  .fbar {
    position: absolute;
    left: 4px;
    right: 4px;
    bottom: 4px;
    height: 3px;
    border-radius: var(--radius-full);
    background: rgba(0, 0, 0, 0.45);
    overflow: hidden;
  }
  .fbar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .ftx {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .ftx b {
    font-weight: 550;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .seen .ftx b {
    color: var(--text-dim);
  }
  .ftx small {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .fseen {
    color: var(--success);
  }
  .danger-zone {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    padding-top: var(--space-3);
    border-top: var(--hairline) solid var(--separator);
  }
  .hint {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .confirm {
    width: 100%;
  }
  .cf-acts {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
</style>
