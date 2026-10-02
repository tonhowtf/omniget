<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { library, loadLibrary, upsertBooks } from "$lib/read/library-store.svelte";
  import { back, go, ui, type View } from "$lib/read/ui-store.svelte";
  import { agentActions } from "$lib/native-agent/actions";
  import type { Book } from "$lib/read/types";
  import ContextMenu from "$components/music/ContextMenu.svelte";
  import Modal from "$components/music/Modal.svelte";
  import ReadIcon from "$components/read/ReadIcon.svelte";
  import ShelfPicker from "$components/read/ShelfPicker.svelte";
  import ReaderHost from "$components/read/ReaderHost.svelte";
  import HomeView from "$components/read/views/HomeView.svelte";
  import LibraryView from "$components/read/views/LibraryView.svelte";
  import BookView from "$components/read/views/BookView.svelte";
  import MangaView from "$components/read/views/MangaView.svelte";
  import SeriesView from "$components/read/views/SeriesView.svelte";
  import UpdatesView from "$components/read/views/UpdatesView.svelte";
  import BrowseView from "$components/read/views/BrowseView.svelte";
  import HitView from "$components/read/views/HitView.svelte";
  import AnnotationsView from "$components/read/views/AnnotationsView.svelte";
  import DownloadsView from "$components/read/views/DownloadsView.svelte";
  import StatsView from "$components/read/views/StatsView.svelte";
  import ToolsView from "$components/read/views/ToolsView.svelte";
  import SettingsView from "$components/read/views/SettingsView.svelte";

  let enabled = $derived(getSettings()?.read?.enabled ?? true);
  let lib = $derived(library());
  let dropping = $state(false);
  let searchEl: HTMLInputElement | undefined = $state();
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  const NAV: { kind: View["kind"]; icon: string }[] = [
    { kind: "home", icon: "home" },
    { kind: "library", icon: "library" },
    { kind: "manga", icon: "manga" },
    { kind: "updates", icon: "bell" },
    { kind: "browse", icon: "compass" },
  ];
  const NAV2: { kind: View["kind"]; icon: string }[] = [
    { kind: "annotations", icon: "highlighter" },
    { kind: "downloads", icon: "download" },
    { kind: "stats", icon: "chart" },
    { kind: "tools", icon: "convert" },
    { kind: "settings", icon: "settings" },
  ];

  function isActive(kind: string) {
    const v = ui.view.kind;
    if (kind === "library") return v === "library" || v === "book";
    if (kind === "manga") return v === "manga" || v === "series";
    if (kind === "browse") return v === "browse" || v === "hit";
    return v === kind;
  }

  let unreadTotal = $derived(lib.series.reduce((a, s) => a + s.unread, 0));

  function onSearch(e: Event) {
    const q = (e.target as HTMLInputElement).value;
    ui.search = q;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      if (q.trim() && ui.view.kind !== "library" && ui.view.kind !== "browse") go({ kind: "library" });
    }, 250);
  }

  function keys(e: KeyboardEvent) {
    if (ui.reader) return;
    const el = e.target as HTMLElement;
    if (el?.closest("input, textarea, select, [contenteditable]")) return;
    if (e.key === "/") {
      e.preventDefault();
      searchEl?.focus();
    } else if (e.key === "Backspace" || (e.altKey && e.key === "ArrowLeft")) {
      back();
    }
  }

  onMount(() => {
    void loadLibrary();
    const q = page.url.searchParams;
    const book = Number(q.get("book"));
    if (book) ui.reader = { kind: "book", id: book, at: q.get("loc") ?? undefined };
    const view = q.get("view");
    if (view) go({ kind: view } as View);

    let un: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent(async (ev) => {
        const p = ev.payload;
        if (p.type === "over" || p.type === "enter") dropping = true;
        else if (p.type === "leave") dropping = false;
        else if (p.type === "drop") {
          dropping = false;
          if (!p.paths.length) return;
          try {
            const rows = await invoke<Book[]>("read_import_paths", { paths: p.paths });
            upsertBooks(rows);
            showToast("success", $t("read.import.added", { count: rows.length }) as string);
          } catch (e) {
            showToast("error", String(e));
          }
        }
      })
      .then((u) => (un = u));
    return () => un?.();
  });
</script>

<svelte:window onkeydown={keys} />

{#if !enabled}
  <div class="off">
    <ReadIcon name="book-open" size={32} />
    <h2>{$t("read.disabled_title")}</h2>
    <p>{$t("superpowers.disabled_hint")}</p>
    <a class="btn btn-primary" href="/settings?tab=integrations&section=read">{$t("superpowers.open_settings")}</a>
  </div>
{:else}
  <div class="read">
    <nav class="r-nav" aria-label={$t("read.nav_label") as string}>
      <div class="brand">
        <span class="brand-tile"><ReadIcon name="book-open" size={17} /></span>
        <span>{$t("read.title")}</span>
      </div>
      <ul>
        {#each NAV as n (n.kind)}
          <li>
            <button class="ni" data-agent-actions="click" use:agentActions={["click"]} class:on={isActive(n.kind)} onclick={() => go({ kind: n.kind } as View)} aria-current={isActive(n.kind)}>
              <ReadIcon name={n.icon} size={16} /> <span>{$t(`read.nav.${n.kind}`)}</span>
              {#if n.kind === "manga" && unreadTotal}<em class="count">{unreadTotal > 999 ? "999+" : unreadTotal}</em>{/if}
            </button>
          </li>
        {/each}
      </ul>
      {#if lib.shelves.length}
        <div class="sec">{$t("read.nav.shelves")}</div>
        <ul class="shelves">
          {#each lib.shelves as s (s.id)}
            <li>
              <button class="ni sh" class:on={ui.view.kind === "library" && ui.view.shelf === s.id} onclick={() => go({ kind: "library", shelf: s.id })}>
                <ReadIcon name={s.smart ? "wand" : "shelf"} size={14} /> <span>{s.name}</span><em class="count dim">{s.book_ids.length}</em>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <div class="spacer"></div>
      {/if}
      <ul class="nav2">
        {#each NAV2 as n (n.kind)}
          <li>
            <button class="ni" data-agent-actions="click" use:agentActions={["click"]} class:on={isActive(n.kind)} onclick={() => go({ kind: n.kind } as View)} aria-current={isActive(n.kind)}>
              <ReadIcon name={n.icon} size={16} /> <span>{$t(`read.nav.${n.kind}`)}</span>
              {#if n.kind === "downloads" && lib.queue.some((q) => q.state === "running" || q.state === "queued")}<em class="dot"></em>{/if}
            </button>
          </li>
        {/each}
      </ul>
      {#if lib.scan || lib.updating}
        <div class="status" role="status">
          {#if lib.scan}<span><ReadIcon name="refresh" size={12} spin /> {$t("read.scan.short", { done: lib.scan.done, total: lib.scan.total })}</span>{/if}
          {#if lib.updating}<span><ReadIcon name="bell" size={12} /> {$t("read.updates.checking", { done: lib.updating.done, total: lib.updating.total })}</span>{/if}
        </div>
      {/if}
    </nav>

    <section class="r-main">
      <div class="topbar">
        <button class="nb" onclick={back} disabled={!ui.stack.length} aria-label={$t("read.back") as string}><ReadIcon name="back" size={16} /></button>
        <label class="search">
          <ReadIcon name="search" size={14} />
          <input bind:this={searchEl} data-agent-actions="fill" use:agentActions={["fill"]} type="search" value={ui.search} oninput={onSearch} placeholder={$t("read.search.placeholder") as string} aria-label={$t("read.search.placeholder") as string} />
          <kbd>/</kbd>
        </label>
      </div>
      <div class="view" data-agent-actions="scroll" use:agentActions={["scroll"]}>
        {#if lib.loading && !lib.loaded}
          <div class="loading"><ReadIcon name="refresh" size={22} spin /></div>
        {:else if lib.error}
          <p class="err">{lib.error}</p>
        {:else}
          {@const v = ui.view}
          {#if v.kind === "home"}<HomeView />
          {:else if v.kind === "library"}<LibraryView shelf={v.shelf ?? null} filter={v.filter ?? ""} />
          {:else if v.kind === "book"}<BookView id={v.id} />
          {:else if v.kind === "manga"}<MangaView />
          {:else if v.kind === "series"}<SeriesView id={v.id} />
          {:else if v.kind === "updates"}<UpdatesView />
          {:else if v.kind === "browse"}<BrowseView source={v.source ?? ""} q={v.q ?? ""} />
          {:else if v.kind === "hit"}<HitView hit={v.hit} />
          {:else if v.kind === "annotations"}<AnnotationsView />
          {:else if v.kind === "downloads"}<DownloadsView />
          {:else if v.kind === "stats"}<StatsView />
          {:else if v.kind === "tools"}<ToolsView tab={v.tab ?? "convert"} ids={v.ids ?? []} />
          {:else if v.kind === "settings"}<SettingsView tab={v.tab ?? "library"} />
          {/if}
        {/if}
      </div>
    </section>

    {#if dropping}
      <div class="dropzone"><ReadIcon name="download" size={28} /> {$t("read.drop_hint")}</div>
    {/if}

    {#if ui.reader}
      <ReaderHost target={ui.reader} onclose={() => (ui.reader = null)} />
    {/if}
  </div>
{/if}

{#if ui.menu}
  <ContextMenu x={ui.menu.x} y={ui.menu.y} items={ui.menu.items} onclose={() => (ui.menu = null)} />
{/if}

{#if ui.shelfPicker}
  <ShelfPicker ids={ui.shelfPicker} onclose={() => (ui.shelfPicker = null)} />
{/if}

{#if ui.confirm}
  {@const c = ui.confirm}
  <Modal title={c.title} width={420} onclose={() => (ui.confirm = null)}>
    <p class="dialog-body">{c.body}</p>
    <div class="dialog-actions">
      <button class="btn" onclick={() => (ui.confirm = null)}>{$t("read.cancel")}</button>
      <button
        class="btn"
        class:btn-destructive={c.danger}
        class:btn-primary={!c.danger}
        onclick={() => {
          ui.confirm = null;
          c.run();
        }}>{c.ok}</button
      >
    </div>
  </Modal>
{/if}

<style>
  .off {
    display: grid;
    place-items: center;
    align-content: center;
    gap: var(--space-3);
    height: 100%;
    text-align: center;
    color: var(--text-dim);
  }
  .read {
    position: relative;
    display: grid;
    grid-template-columns: 212px minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }
  .r-nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3) var(--space-2);
    border-right: var(--hairline) solid var(--content-border);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px var(--space-3);
    font-family: var(--font-display);
    font-weight: 750;
    font-size: var(--text-lg);
  }
  .brand-tile {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 8px;
    color: #fff;
    background: linear-gradient(160deg, #ffb86b, #e0712a);
  }
  .r-nav ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .ni {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-sm);
    font-size: var(--text-base);
    color: var(--text-dim);
    cursor: pointer;
  }
  .ni span {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ni:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .ni.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-weight: 600;
  }
  .ni:focus-visible,
  .nb:focus-visible {
    outline: var(--focus-ring);
  }
  .count {
    font-style: normal;
    font-size: var(--text-caption);
    font-weight: 700;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--accent);
    color: #fff;
  }
  .count.dim {
    background: none;
    color: var(--text-muted);
    font-weight: 500;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }
  .sec {
    margin: var(--space-3) 0 2px;
    padding: 0 10px;
    font-size: var(--text-xs);
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .shelves {
    flex: 1;
    min-height: 40px;
  }
  .ni.sh {
    height: 28px;
    font-size: var(--text-sm);
  }
  .spacer {
    flex: 1;
  }
  .nav2 {
    border-top: var(--hairline) solid var(--content-border);
    padding-top: var(--space-2) !important;
    margin-top: var(--space-2) !important;
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: var(--space-2);
    padding: 8px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .status span {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .r-main {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-2);
  }
  .nb {
    all: unset;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: var(--fill-1);
    cursor: pointer;
  }
  .nb:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 1 460px;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-muted);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .search:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .search input {
    all: unset;
    flex: 1;
    min-width: 0;
    font-size: var(--text-base);
    color: var(--text);
  }
  .search kbd {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    padding: 0 5px;
    border-radius: 4px;
    background: var(--fill-2);
  }
  .view {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-2) var(--space-5) var(--space-6);
  }
  .loading {
    display: grid;
    place-items: center;
    height: 60%;
    color: var(--text-muted);
  }
  .err {
    color: var(--danger);
  }
  .dropzone {
    position: absolute;
    inset: 12px;
    z-index: 30;
    display: grid;
    place-items: center;
    align-content: center;
    gap: 10px;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--bg) 85%, transparent);
    font-weight: 600;
    color: var(--accent-hi);
    pointer-events: none;
  }

  .read :global(.r-h) {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-3) 0 var(--space-3);
  }
  .read :global(.r-h h1) {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  .read :global(.r-h h2) {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 700;
  }
  .read :global(.r-h .grow) {
    flex: 1;
  }
  .read :global(.r-sub) {
    margin: var(--space-5) 0 var(--space-2);
    font-size: var(--text-lg);
    font-weight: 700;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .read :global(.r-muted) {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .read :global(.r-grid) {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--card-w, 138px), 1fr));
    gap: var(--space-4) var(--space-3);
  }
  .read :global(.r-row) {
    display: flex;
    gap: var(--space-3);
    overflow-x: auto;
    padding-bottom: var(--space-2);
    scrollbar-width: thin;
  }
  .read :global(.r-row > *) {
    flex: 0 0 var(--card-w, 138px);
  }
  .read :global(.r-toolbar) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }
  .read :global(.r-chip) {
    all: unset;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 11px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    font-size: var(--text-sm);
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .read :global(.r-chip:hover) {
    background: var(--fill-2);
    color: var(--text);
  }
  .read :global(.r-chip.on) {
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-weight: 600;
  }
  .read :global(.r-chip:focus-visible) {
    outline: var(--focus-ring);
  }
  .read :global(.r-icon-btn) {
    all: unset;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: var(--radius-sm);
    color: var(--text-dim);
    cursor: pointer;
  }
  .read :global(.r-icon-btn:hover) {
    background: var(--fill-2);
    color: var(--text);
  }
  .read :global(.r-icon-btn.on) {
    color: var(--accent-hi);
  }
  .read :global(.r-icon-btn:focus-visible) {
    outline: var(--focus-ring);
  }
  .read :global(.r-panel) {
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .read :global(.r-panel + .r-panel) {
    margin-top: var(--space-3);
  }
  .read :global(.r-field) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .read :global(.r-fields) {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-3);
  }
  .read :global(.r-line) {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .read :global(.r-list) {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .read :global(.r-li) {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 44px;
    padding: 6px 8px;
    border-radius: var(--radius-sm);
  }
  .read :global(.r-li:hover) {
    background: var(--fill-1);
  }
  .read :global(.r-badge) {
    display: inline-block;
    padding: 1px 7px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 650;
    background: var(--fill-2);
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .read :global(.r-empty) {
    display: grid;
    justify-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-4);
    text-align: center;
    color: var(--text-dim);
  }
  .read :global(.r-empty h2) {
    margin: 0;
    color: var(--text);
  }
  .read :global(.r-bar) {
    height: 4px;
    border-radius: 2px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .read :global(.r-bar > i) {
    display: block;
    height: 100%;
    background: var(--accent);
  }
</style>
