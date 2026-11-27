<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { bookById, library, removeBooksLocal } from "$lib/read/library-store.svelte";
  import { confirm, go, openBook, openMenu, setStatus, toggleFavorite, tr, ui, fmtBytes } from "$lib/read/ui-store.svelte";
  import { isComic, type Book } from "$lib/read/types";
  import type { MenuItem } from "#components/music/ContextMenu.svelte";
  import BookCard from "../BookCard.svelte";
  import Cover from "../Cover.svelte";
  import ReadIcon from "../ReadIcon.svelte";
  import { coverUrl } from "$lib/read/library-store.svelte";

  let { shelf = null, filter = "" }: { shelf?: number | null; filter?: string } = $props();

  const KEY = "omniget.read.library.v1";
  type Opts = { status: string; kind: string; sort: string; desc: boolean; layout: "grid" | "list"; group: boolean; chapters: boolean };
  function loadOpts(): Opts {
    try {
      return { status: "all", kind: "all", sort: "title", desc: false, layout: "grid", group: false, chapters: false, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
    } catch {
      return { status: "all", kind: "all", sort: "title", desc: false, layout: "grid", group: false, chapters: false };
    }
  }
  let o = $state<Opts>(loadOpts());
  $effect(() => {
    try {
      localStorage.setItem(KEY, JSON.stringify($state.snapshot(o)));
    } catch {
    }
  });
  $effect(() => {
    if (filter) o.status = filter;
  });

  let lib = $derived(library());
  let webUrl = $state<string | null>(null);
  let saving = $state(false);
  let webKind = $state<"article" | "fanficfare" | "lncrawl">("article");

  async function saveWeb() {
    if (!webUrl?.trim()) return;
    saving = true;
    try {
      if (webKind !== "article") {
        const ids = await invoke<number[]>("read_webnovel", { url: webUrl.trim(), tool: webKind, updateBook: null });
        webUrl = null;
        showToast("success", tr("read.import.added", { count: ids.length }));
        return;
      }
      const b = await invoke<Book>("read_save_url", { url: webUrl.trim() });
      webUrl = null;
      showToast("success", tr("read.library.web_saved", { title: b.title }));
      openBook(b);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      saving = false;
    }
  }
  let ftsIds = $state<number[] | null>(null);
  let ftsTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const q = ui.search.trim();
    if (ftsTimer) clearTimeout(ftsTimer);
    if (!q) {
      ftsIds = null;
      return;
    }
    ftsTimer = setTimeout(async () => {
      ftsIds = await invoke<number[]>("read_search", { query: q }).catch(() => []);
    }, 180);
  });

  let shelfRow = $derived(shelf != null ? lib.shelves.find((s) => s.id === shelf) : null);

  let books = $derived.by(() => {
    void lib.version;
    let list: Book[];
    if (ftsIds) {
      list = ftsIds.map((id) => bookById(id)).filter((b): b is Book => !!b);
    } else {
      list = [...lib.books];
    }
    if (shelfRow) {
      const s = new Set(shelfRow.book_ids);
      list = list.filter((b) => s.has(b.id));
    }
    if (!o.chapters && !shelfRow) list = list.filter((b) => !b.chapter_id);
    if (o.status === "favorites") list = list.filter((b) => b.favorite);
    else if (o.status !== "all") list = list.filter((b) => b.status === o.status);
    if (o.kind === "books") list = list.filter((b) => !isComic(b) && b.format !== "pdf");
    else if (o.kind === "comics") list = list.filter((b) => isComic(b));
    else if (o.kind === "pdf") list = list.filter((b) => b.format === "pdf");
    if (!ftsIds) {
      const cmp: Record<string, (a: Book, b: Book) => number> = {
        title: (a, b) => a.sort_title.localeCompare(b.sort_title),
        author: (a, b) => (a.authors[0] ?? "").localeCompare(b.authors[0] ?? "") || a.sort_title.localeCompare(b.sort_title),
        added: (a, b) => b.added_at - a.added_at,
        read: (a, b) => (b.last_read_at ?? 0) - (a.last_read_at ?? 0),
        progress: (a, b) => b.progress - a.progress,
        rating: (a, b) => b.rating - a.rating,
        series: (a, b) => a.series.localeCompare(b.series) || (a.series_index ?? 0) - (b.series_index ?? 0),
        size: (a, b) => b.size - a.size,
      };
      list.sort(cmp[o.sort] ?? cmp.title);
      if (o.desc) list.reverse();
      if (shelfRow && !shelfRow.smart && o.sort === "title") {
        const pos = new Map(shelfRow.book_ids.map((id, i) => [id, i]));
        list.sort((a, b) => (pos.get(a.id) ?? 0) - (pos.get(b.id) ?? 0));
      }
    }
    return list;
  });

  let groups = $derived.by(() => {
    if (!o.group) return null;
    const m = new Map<string, Book[]>();
    for (const b of books) {
      const k = b.series || "";
      m.set(k, [...(m.get(k) ?? []), b]);
    }
    return [...m.entries()].sort((a, b) => (a[0] === "" ? 1 : b[0] === "" ? -1 : a[0].localeCompare(b[0])));
  });

  function toggleSel(id: number) {
    ui.selection = ui.selection.includes(id) ? ui.selection.filter((x) => x !== id) : [...ui.selection, id];
  }

  function menuFor(b: Book, e: MouseEvent) {
    const ids = ui.selection.includes(b.id) ? ui.selection : [b.id];
    const items: MenuItem[] = [
      { label: tr("read.book.read"), icon: "play", action: () => openBook(b) },
      { label: tr("read.book.details"), icon: "note", action: () => go({ kind: "book", id: b.id }) },
      { divider: true },
      { label: b.favorite ? tr("read.book.unfavorite") : tr("read.book.favorite"), icon: "heart", action: () => toggleFavorite(b) },
      {
        label: tr("read.book.status"),
        icon: "tag",
        sub: ["unread", "want", "reading", "finished", "abandoned"].map((s) => ({
          label: tr(`read.status.${s}`),
          checked: b.status === s,
          action: () => setStatus(ids, s),
        })),
      },
      { label: tr("read.shelf.add_to"), icon: "plus", action: () => (ui.shelfPicker = ids) },
    ];
    if (shelfRow && !shelfRow.smart) {
      items.push({
        label: tr("read.shelf.remove_from", { name: shelfRow.name }),
        icon: "close",
        action: () => invoke("read_shelf_remove", { shelfId: shelfRow!.id, bookIds: ids }),
      });
    }
    items.push(
      { divider: true },
      { label: tr("read.tools.convert"), icon: "convert", action: () => go({ kind: "tools", tab: "convert", ids }) },
      { label: tr("read.tools.send"), icon: "upload", action: () => go({ kind: "tools", tab: "send", ids }) },
      { label: tr("read.book.reveal"), icon: "folder", action: () => invoke("reveal_file", { path: b.path }) },
      { divider: true },
      {
        label: tr("read.book.remove", { count: ids.length }),
        icon: "trash",
        danger: true,
        action: () =>
          confirm(tr("read.book.remove", { count: ids.length }), tr("read.book.remove_body"), tr("read.remove"), async () => {
            await invoke("read_remove_books", { ids, deleteFiles: false });
            removeBooksLocal(ids);
          }),
      },
      {
        label: tr("read.book.delete_files", { count: ids.length }),
        icon: "trash",
        danger: true,
        action: () =>
          confirm(tr("read.book.delete_files", { count: ids.length }), tr("read.book.delete_body"), tr("read.delete"), async () => {
            await invoke("read_remove_books", { ids, deleteFiles: true });
            removeBooksLocal(ids);
          }),
      },
    );
    openMenu(e, items);
  }

  async function bulk(status: string) {
    await setStatus(ui.selection, status);
    showToast("success", tr("read.book.updated", { count: ui.selection.length }));
  }
</script>

<div class="r-h">
  <h1>{shelfRow ? shelfRow.name : $t("read.nav.library")}</h1>
  <span class="r-muted">{books.length}</span>
  <span class="grow"></span>
  {#if shelfRow}
    <button class="r-chip" onclick={() => go({ kind: "settings", tab: "shelves" })}><ReadIcon name="edit" size={13} /> {$t("read.shelf.edit")}</button>
  {/if}
  <button class="r-chip" onclick={() => (webUrl = "")}><ReadIcon name="globe" size={13} /> {$t("read.library.save_web")}</button>
  <button class="r-icon-btn" class:on={o.layout === "grid"} onclick={() => (o.layout = "grid")} aria-label={$t("read.library.grid") as string}><ReadIcon name="grid" size={16} /></button>
  <button class="r-icon-btn" class:on={o.layout === "list"} onclick={() => (o.layout = "list")} aria-label={$t("read.library.list") as string}><ReadIcon name="list" size={16} /></button>
</div>

<div class="r-toolbar">
  {#each ["all", "reading", "unread", "want", "finished", "abandoned", "favorites"] as s (s)}
    <button class="r-chip" class:on={o.status === s} onclick={() => (o.status = s)}>{$t(`read.status.${s}`)}</button>
  {/each}
</div>
<div class="r-toolbar">
  {#each ["all", "books", "comics", "pdf"] as k (k)}
    <button class="r-chip" class:on={o.kind === k} onclick={() => (o.kind = k)}>{$t(`read.library.kind_${k}`)}</button>
  {/each}
  <span class="grow" style="flex:1"></span>
  <label class="r-chip"><input type="checkbox" bind:checked={o.group} /> {$t("read.library.group_series")}</label>
  <label class="r-chip"><input type="checkbox" bind:checked={o.chapters} /> {$t("read.library.show_chapters")}</label>
  <select class="input sel" bind:value={o.sort} aria-label={$t("read.library.sort") as string}>
    {#each ["title", "author", "series", "added", "read", "progress", "rating", "size"] as s (s)}
      <option value={s}>{$t(`read.library.sort_${s}`)}</option>
    {/each}
  </select>
  <button class="r-icon-btn" onclick={() => (o.desc = !o.desc)} aria-label={$t("read.library.reverse") as string}><ReadIcon name="sort" size={15} /></button>
</div>

{#if webUrl !== null}
  <form class="r-panel web" onsubmit={(e) => { e.preventDefault(); void saveWeb(); }}>
    <select class="input" bind:value={webKind} style="width:auto">
      <option value="article">{$t("read.library.web_article")}</option>
      <option value="fanficfare">{$t("read.library.web_fanfic")}</option>
      <option value="lncrawl">{$t("read.library.web_novel")}</option>
    </select>
    <input class="input" style="flex:1" bind:value={webUrl} placeholder="https://…" />
    <button class="btn btn-primary" type="submit" disabled={saving}>{saving ? "…" : $t("read.library.save_web_go")}</button>
    <button class="btn" type="button" onclick={() => (webUrl = null)}>{$t("read.cancel")}</button>
  </form>
{/if}

{#if ui.selection.length}
  <div class="selbar">
    <b>{$t("read.library.selected", { count: ui.selection.length })}</b>
    <button class="r-chip" onclick={() => bulk("finished")}>{$t("read.status.finished")}</button>
    <button class="r-chip" onclick={() => bulk("unread")}>{$t("read.status.unread")}</button>
    <button class="r-chip" onclick={() => (ui.shelfPicker = [...ui.selection])}>{$t("read.shelf.add_to")}</button>
    <button class="r-chip" onclick={() => go({ kind: "tools", tab: "convert", ids: [...ui.selection] })}>{$t("read.tools.convert")}</button>
    <button class="r-chip" onclick={() => go({ kind: "tools", tab: "send", ids: [...ui.selection] })}>{$t("read.tools.send")}</button>
    <button class="r-chip" onclick={() => (ui.selection = books.map((b) => b.id))}>{$t("read.library.select_all")}</button>
    <button class="r-chip" onclick={() => (ui.selection = [])}>{$t("read.cancel")}</button>
  </div>
{/if}

{#if !books.length}
  <div class="r-empty">
    <ReadIcon name="library" size={36} />
    <p>{ui.search ? $t("read.library.no_match") : $t("read.library.empty")}</p>
  </div>
{:else if o.layout === "list"}
  <table class="tbl">
    <thead>
      <tr>
        <th></th><th>{$t("read.library.sort_title")}</th><th>{$t("read.library.sort_author")}</th><th>{$t("read.library.sort_series")}</th>
        <th>{$t("read.library.format")}</th><th>{$t("read.library.sort_progress")}</th><th>{$t("read.library.sort_size")}</th>
      </tr>
    </thead>
    <tbody>
      {#each books as b (b.id)}
        <tr
          class:sel={ui.selection.includes(b.id)}
          onclick={(e) => (e.metaKey || e.ctrlKey || e.shiftKey ? toggleSel(b.id) : go({ kind: "book", id: b.id }))}
          ondblclick={() => openBook(b)}
          oncontextmenu={(e) => menuFor(b, e)}
        >
          <td class="c"><Cover src={coverUrl(b.cover)} title={b.title} radius={3} /></td>
          <td class="ti">{b.title}{#if b.favorite} <span class="heart"><ReadIcon name="heart-fill" size={11} /></span>{/if}</td>
          <td>{b.authors.join(", ")}</td>
          <td>{b.series}{b.series_index != null ? ` #${b.series_index}` : ""}</td>
          <td><span class="r-badge">{b.format}</span></td>
          <td>{b.status === "finished" ? "✓" : `${Math.round(b.progress * 100)}%`}</td>
          <td class="r-muted">{fmtBytes(b.size)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{:else if groups}
  {#each groups as [name, list] (name)}
    <h2 class="r-sub">{name || $t("read.library.no_series")} <span class="r-muted">{list.length}</span></h2>
    <div class="r-grid">
      {#each list as b (b.id)}
        <BookCard book={b} selected={ui.selection.includes(b.id)} onopen={() => go({ kind: "book", id: b.id })} onread={() => openBook(b)} onmenu={(e) => menuFor(b, e)} onselect={() => toggleSel(b.id)} />
      {/each}
    </div>
  {/each}
{:else}
  <div class="r-grid">
    {#each books as b (b.id)}
      <BookCard book={b} selected={ui.selection.includes(b.id)} onopen={() => go({ kind: "book", id: b.id })} onread={() => openBook(b)} onmenu={(e) => menuFor(b, e)} onselect={() => toggleSel(b.id)} />
    {/each}
  </div>
{/if}

<style>
  .sel {
    height: 28px;
    padding: 0 8px;
    font-size: var(--text-sm);
  }
  label.r-chip input {
    margin: 0;
  }
  .web {
    display: flex;
    gap: 8px;
    margin-bottom: var(--space-3);
  }
  .selbar {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
    padding: 8px 12px;
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.15);
  }
  .tbl {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }
  .tbl th {
    text-align: left;
    font-weight: 600;
    color: var(--text-muted);
    padding: 6px 8px;
    border-bottom: var(--hairline) solid var(--content-border);
  }
  .tbl td {
    padding: 4px 8px;
    border-bottom: var(--hairline) solid color-mix(in srgb, var(--content-border) 50%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
  }
  .tbl tr {
    cursor: pointer;
  }
  .tbl tbody tr:hover {
    background: var(--fill-1);
  }
  .tbl tr.sel {
    background: var(--accent-soft);
  }
  .tbl .c {
    width: 28px;
  }
  .ti {
    font-weight: 600;
  }
  .heart {
    display: inline-block;
    color: #ff5a7a;
    vertical-align: middle;
  }
</style>
