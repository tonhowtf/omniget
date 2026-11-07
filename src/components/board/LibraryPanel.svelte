<script lang="ts">
  import { t, locale } from "$lib/i18n";
  import * as api from "$lib/board/api";
  import { ago, formatSize, visibleBoards } from "$lib/board/library";
  import { checkName } from "$lib/board/names";
  import type { BoardEntry, SortMode } from "$lib/board/types";
  import BoardIcon from "./BoardIcon.svelte";

  let {
    boards,
    folder,
    isDefault,
    activePath,
    loading,
    loadError,
    thumbRevision,
    onopen,
    oncreate,
    onimport,
    onopenfile,
    onrename,
    onduplicate,
    ondelete,
    onchoosefolder,
    onresetfolder,
    onreload,
  }: {
    boards: BoardEntry[];
    folder: string;
    isDefault: boolean;
    activePath: string | null;
    loading: boolean;
    loadError: string | null;
    thumbRevision: Record<string, number>;
    onopen: (entry: BoardEntry) => void;
    oncreate: () => void;
    onimport: () => void;
    onopenfile: () => void;
    onrename: (entry: BoardEntry, name: string) => Promise<string | null>;
    onduplicate: (entry: BoardEntry) => void;
    ondelete: (entry: BoardEntry) => void;
    onchoosefolder: () => void;
    onresetfolder: () => void;
    onreload: () => void;
  } = $props();

  let query = $state("");
  let sort = $state<SortMode>("recent");
  let menuFor = $state<string | null>(null);
  let renaming = $state<string | null>(null);
  let draft = $state("");
  let renameError = $state<string | null>(null);
  let renameBusy = $state(false);
  let thumbs = $state<Record<string, { rev: number; url: string | null }>>({});
  let now = $state(Date.now());

  let shown = $derived(visibleBoards(boards, query, sort));
  let names = $derived(boards.map((b) => b.name));

  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 60_000);
    return () => clearInterval(id);
  });

  $effect(() => {
    for (const b of shown) {
      if (!b.has_thumb) continue;
      const rev = (thumbRevision[b.path] ?? 0) + b.modified_ms;
      const cached = thumbs[b.path];
      if (cached && cached.rev === rev) continue;
      thumbs[b.path] = { rev, url: cached?.url ?? null };
      void api
        .thumbnail(b.path)
        .then((url) => {
          if (thumbs[b.path]?.rev === rev) thumbs[b.path] = { rev, url };
        })
        .catch(() => {});
    }
  });

  function when(ms: number): string {
    const a = ago(ms, now);
    if (a.unit === "now") return $t("board.ago_now") as string;
    if (a.unit === "minute") return $t("board.ago_minutes", { count: a.value }) as string;
    if (a.unit === "hour") return $t("board.ago_hours", { count: a.value }) as string;
    if (a.unit === "day") return $t("board.ago_days", { count: a.value }) as string;
    return new Date(a.value).toLocaleDateString($locale || undefined, { day: "numeric", month: "short", year: "numeric" });
  }

  function startRename(entry: BoardEntry) {
    menuFor = null;
    renaming = entry.path;
    draft = entry.name;
    renameError = null;
  }

  function nameMessage(error: string): string {
    return $t(`board.name_${error}`) as string;
  }

  async function commitRename(entry: BoardEntry) {
    if (renameBusy) return;
    const check = checkName(draft, names, entry.name);
    if (!check.ok) {
      renameError = nameMessage(check.error);
      return;
    }
    if (check.name === entry.name) {
      renaming = null;
      return;
    }
    renameBusy = true;
    const failure = await onrename(entry, check.name);
    renameBusy = false;
    if (failure) renameError = failure;
    else renaming = null;
  }

  function renameKey(event: KeyboardEvent, entry: BoardEntry) {
    if (event.key === "Enter") {
      event.preventDefault();
      void commitRename(entry);
    } else if (event.key === "Escape") {
      event.preventDefault();
      renaming = null;
    }
  }

  function closeMenu(refocus: boolean) {
    const path = menuFor;
    menuFor = null;
    if (!refocus || !path) return;
    queueMicrotask(() => {
      const trigger = [...document.querySelectorAll<HTMLButtonElement>(".lp-more")].find((b) => b.dataset.path === path);
      trigger?.focus();
    });
  }

  function menuKey(event: KeyboardEvent) {
    const menu = event.currentTarget as HTMLElement;
    const items = [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    let next: number | null = null;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeMenu(true);
      return;
    }
    if (event.key === "Tab") {
      closeMenu(false);
      return;
    }
    if (event.key === "ArrowDown") next = at < 0 ? 0 : (at + 1) % items.length;
    else if (event.key === "ArrowUp") next = at < 0 ? items.length - 1 : (at - 1 + items.length) % items.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = items.length - 1;
    if (next === null) return;
    event.preventDefault();
    items[next]?.focus();
  }

  function autofocus(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  function focusFirst(node: HTMLElement) {
    node.querySelector<HTMLElement>("button")?.focus();
  }

  function act(fn: () => void) {
    closeMenu(true);
    fn();
  }
</script>

<svelte:window onclick={() => (menuFor = null)} />

<aside class="lp" aria-label={$t("board.library")}>
  <div class="lp-actions">
    <button type="button" class="btn btn-primary lp-new" onclick={oncreate}>
      <BoardIcon name="plus" size={14} />
      {$t("board.new_board")}
    </button>
    <button type="button" class="btn btn-secondary btn-icon" title={$t("board.open_file")} aria-label={$t("board.open_file")} onclick={onopenfile}>
      <BoardIcon name="file" size={15} />
    </button>
    <button type="button" class="btn btn-secondary btn-icon" title={$t("board.import")} aria-label={$t("board.import")} onclick={onimport}>
      <BoardIcon name="import" size={15} />
    </button>
  </div>

  <div class="lp-filter">
    <label class="lp-search">
      <BoardIcon name="search" size={14} />
      <input type="search" bind:value={query} placeholder={$t("board.search")} aria-label={$t("board.search")} />
    </label>
    <div class="lp-sort" role="radiogroup" aria-label={$t("board.sort")}>
      <button type="button" role="radio" aria-checked={sort === "recent"} class:on={sort === "recent"} onclick={() => (sort = "recent")}>{$t("board.sort_recent")}</button>
      <button type="button" role="radio" aria-checked={sort === "name"} class:on={sort === "name"} onclick={() => (sort = "name")}>{$t("board.sort_name")}</button>
    </div>
  </div>

  <div class="lp-list-wrap">
    {#if loading && boards.length === 0}
      <div class="lp-state" aria-busy="true">
        <span class="spinner" aria-hidden="true"></span>
        <p>{$t("board.loading_library")}</p>
      </div>
    {:else if loadError}
      <div class="lp-state lp-state-error" role="alert">
        <BoardIcon name="warning" size={18} />
        <p>{loadError}</p>
        <button type="button" class="btn btn-secondary btn-sm" onclick={onreload}>{$t("board.reload_library")}</button>
      </div>
    {:else if boards.length === 0}
      <div class="lp-state">
        <p class="lp-state-title">{$t("board.library_empty_title")}</p>
        <p>{$t("board.library_empty_body")}</p>
      </div>
    {:else if shown.length === 0}
      <div class="lp-state">
        <p>{$t("board.no_results", { query })}</p>
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (query = "")}>{$t("board.clear_search")}</button>
      </div>
    {:else}
      <ul class="lp-list">
        {#each shown as entry (entry.path)}
          {@const thumb = thumbs[entry.path]?.url}
          <li class="lp-item" class:active={entry.path === activePath}>
            {#if renaming === entry.path}
              <div class="lp-rename">
                <input
                  use:autofocus
                  bind:value={draft}
                  aria-label={$t("board.rename_label")}
                  aria-invalid={renameError ? "true" : undefined}
                  aria-describedby={renameError ? "lp-rename-error" : undefined}
                  disabled={renameBusy}
                  onkeydown={(e) => renameKey(e, entry)}
                  onblur={() => void commitRename(entry)}
                />
                {#if renameError}
                  <p id="lp-rename-error" class="lp-rename-error" role="alert">{renameError}</p>
                {/if}
              </div>
            {:else}
              <button type="button" class="lp-open" aria-current={entry.path === activePath ? "true" : undefined} onclick={() => onopen(entry)}>
                <span class="lp-thumb" aria-hidden="true">
                  {#if thumb}
                    <img src={thumb} alt="" loading="lazy" />
                  {:else}
                    <BoardIcon name="board" size={18} />
                  {/if}
                </span>
                <span class="lp-meta">
                  <span class="lp-name" title={entry.name}>{entry.name}</span>
                  <span class="lp-sub">{when(entry.modified_ms)} · {formatSize(entry.size)}</span>
                </span>
              </button>
              <div class="lp-more-wrap">
                <button
                  type="button"
                  class="btn btn-ghost btn-icon btn-sm lp-more"
                  aria-label={$t("board.actions_for", { name: entry.name })}
                  aria-haspopup="menu"
                  aria-expanded={menuFor === entry.path}
                  data-path={entry.path}
                  onclick={(e) => {
                    e.stopPropagation();
                    menuFor = menuFor === entry.path ? null : entry.path;
                  }}
                >
                  <BoardIcon name="more" size={14} />
                </button>
                {#if menuFor === entry.path}
                  <div class="lp-menu" role="menu" tabindex="-1" use:focusFirst onkeydown={menuKey} onclick={(e) => e.stopPropagation()}>
                    <button type="button" role="menuitem" tabindex="-1" onclick={() => startRename(entry)}><BoardIcon name="edit" size={14} />{$t("board.rename")}</button>
                    <button type="button" role="menuitem" tabindex="-1" onclick={() => act(() => onduplicate(entry))}><BoardIcon name="copy" size={14} />{$t("board.duplicate")}</button>
                    <button type="button" role="menuitem" tabindex="-1" onclick={() => act(() => void api.reveal(entry.path).catch(() => {}))}><BoardIcon name="reveal" size={14} />{$t("board.show_in_folder")}</button>
                    <button type="button" role="menuitem" tabindex="-1" class="lp-danger" onclick={() => act(() => ondelete(entry))}><BoardIcon name="trash" size={14} />{$t("board.move_to_trash")}</button>
                  </div>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <footer class="lp-folder">
    <span class="lp-folder-label">{isDefault ? $t("board.folder_default") : $t("board.folder_custom")}</span>
    <button type="button" class="lp-folder-path" title={folder} onclick={() => void api.reveal(folder).catch(() => {})}>
      <BoardIcon name="folder" size={13} />
      <span class="lp-folder-text"><bdi dir="ltr">{folder}</bdi></span>
    </button>
    <div class="lp-folder-actions">
      <button type="button" class="btn btn-ghost btn-sm" onclick={onchoosefolder}>{$t("board.change_folder")}</button>
      {#if !isDefault}
        <button type="button" class="btn btn-ghost btn-sm" onclick={onresetfolder}>{$t("board.use_default_folder")}</button>
      {/if}
    </div>
  </footer>
</aside>

<style>
  .lp {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 280px;
    flex-shrink: 0;
    min-height: 0;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .lp-actions {
    display: flex;
    gap: var(--space-1);
  }

  .lp-new {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
  }

  .lp-filter {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .lp-search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h);
    padding: 0 var(--space-2);
    border-radius: var(--radius-md);
    background: var(--pane-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    color: var(--text-muted);
  }

  .lp-search:focus-within {
    outline: 2px solid var(--focus-ring);
    outline-offset: 1px;
  }

  .lp-search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--text-sm);
    outline: none;
  }

  .lp-sort {
    display: flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--pane-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .lp-sort button {
    flex: 1;
    height: 22px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .lp-sort button.on {
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .lp-sort button:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .lp-list-wrap {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0 calc(var(--space-1) * -1);
    padding: 0 var(--space-1);
  }

  .lp-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .lp-item {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: var(--radius-md);
  }

  .lp-item:hover {
    background: color-mix(in srgb, var(--secondary) 5%, transparent);
  }

  .lp-item.active {
    background: var(--accent-soft);
  }

  .lp-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1);
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: inherit;
    text-align: start;
    cursor: pointer;
  }

  .lp-open:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: -2px;
  }

  .lp-thumb {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 40px;
    flex-shrink: 0;
    overflow: hidden;
    border-radius: var(--radius-sm);
    background: var(--pane-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-muted);
  }

  .lp-thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .lp-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .lp-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text);
  }

  .lp-sub {
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }

  .lp-more-wrap {
    position: relative;
    flex-shrink: 0;
    padding-inline-end: var(--space-1);
  }

  .lp-more {
    opacity: 0;
  }

  .lp-item:hover .lp-more,
  .lp-item.active .lp-more,
  .lp-more:focus-visible,
  .lp-more[aria-expanded="true"] {
    opacity: 1;
  }

  .lp-menu {
    position: absolute;
    top: calc(100% + 2px);
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    min-width: 180px;
    padding: var(--space-1);
    border-radius: var(--radius-md);
    background: var(--popup-bg);
    box-shadow:
      inset 0 0 0 var(--hairline) var(--content-border),
      0 10px 28px color-mix(in srgb, var(--shadow-ink) calc(var(--elev-alpha-3) * 100%), transparent);
  }

  .lp-menu button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--text-sm);
    text-align: start;
    cursor: pointer;
  }

  .lp-menu button:hover,
  .lp-menu button:focus-visible {
    background: var(--accent-soft);
    outline: none;
  }

  .lp-menu .lp-danger {
    color: var(--error);
  }

  .lp-rename {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-1);
  }

  .lp-rename input {
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--pane-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--input-border);
    color: var(--text);
    font-size: var(--text-base);
  }

  .lp-rename input:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .lp-rename-error {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--error);
  }

  .lp-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-3);
    text-align: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .lp-state p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .lp-state-title {
    color: var(--text);
    font-weight: 600;
  }

  .lp-state-error {
    color: var(--text);
  }

  .lp-state-error :global(.bi) {
    color: var(--error);
  }

  .lp-folder {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-top: var(--space-2);
    border-top: var(--hairline) solid var(--content-border);
  }

  .lp-folder-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .lp-folder-path {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    padding: 2px 0;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-family: var(--font-mono);
    cursor: pointer;
    text-align: start;
  }

  .lp-folder-path span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
  }

  .lp-folder-path:hover {
    color: var(--text);
  }

  .lp-folder-path:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .lp-folder-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-inline-start: calc(var(--space-2) * -1);
  }

  @media (prefers-reduced-motion: reduce) {
    .lp-more {
      transition: none;
    }
  }
</style>
