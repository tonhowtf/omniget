<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { bookById, coverUrl, library, patchBookLocal, removeBooksLocal } from "$lib/read/library-store.svelte";
  import { back, confirm, fmtBytes, fmtDate, fmtDuration, go, openBook, tr, ui } from "$lib/read/ui-store.svelte";
  import type { Annotation, Book, MetaCandidate } from "$lib/read/types";
  import Cover from "../Cover.svelte";
  import ReadIcon from "../ReadIcon.svelte";

  let { id }: { id: number } = $props();

  let lib = $derived(library());
  let book = $derived.by(() => {
    void lib.version;
    return bookById(id);
  });
  let notes = $state<Annotation[]>([]);
  let editing = $state(false);
  let form = $state<Record<string, any>>({});
  let writeFile = $state(true);
  let candidates = $state<MetaCandidate[]>([]);
  let looking = $state(false);
  let saving = $state(false);

  $effect(() => {
    void id;
    void invoke<Annotation[]>("read_annotations", { bookId: id }).then((a) => (notes = a)).catch(() => (notes = []));
  });

  function startEdit(b: Book) {
    form = {
      title: b.title,
      authors: b.authors.join(", "),
      series: b.series,
      series_index: b.series_index ?? "",
      language: b.language,
      publisher: b.publisher,
      published: b.published,
      isbn: b.isbn,
      description: b.description,
      subjects: b.subjects.join(", "),
      tags: b.tags.join(", "),
    };
    editing = true;
  }

  const list = (s: string) =>
    String(s ?? "")
      .split(",")
      .map((x) => x.trim())
      .filter(Boolean);

  async function saveEdit() {
    if (!book) return;
    saving = true;
    try {
      const row = await invoke<Book>("read_update_book", {
        id: book.id,
        patch: {
          title: form.title,
          authors: list(form.authors),
          series: form.series,
          series_index: form.series_index === "" ? null : Number(form.series_index),
          language: form.language,
          publisher: form.publisher,
          published: form.published,
          isbn: form.isbn,
          description: form.description,
          subjects: list(form.subjects),
          tags: list(form.tags),
          write_file: writeFile && (book.format === "epub" || book.format === "kepub"),
        },
      });
      patchBookLocal(row);
      editing = false;
      showToast("success", tr("read.book.saved"));
    } catch (e) {
      showToast("error", String(e));
    } finally {
      saving = false;
    }
  }

  async function patch(p: Record<string, unknown>) {
    if (!book) return;
    try {
      patchBookLocal(await invoke<Book>("read_update_book", { id: book.id, patch: p }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function lookup() {
    if (!book) return;
    looking = true;
    candidates = [];
    try {
      candidates = await invoke<MetaCandidate[]>("read_meta_lookup", {
        title: editing ? form.title : book.title,
        author: (editing ? list(form.authors)[0] : book.authors[0]) ?? "",
        isbn: editing ? form.isbn : book.isbn,
      });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      looking = false;
    }
  }

  async function applyCandidate(c: MetaCandidate) {
    if (!book) return;
    if (!editing) startEdit(book);
    form.title = c.title || form.title;
    if (c.authors.length) form.authors = c.authors.join(", ");
    if (c.description) form.description = c.description.replace(/<[^>]+>/g, "");
    if (c.publisher) form.publisher = c.publisher;
    if (c.published) form.published = c.published;
    if (c.isbn) form.isbn = c.isbn;
    if (c.language) form.language = c.language;
    if (c.subjects.length) form.subjects = c.subjects.join(", ");
    if (c.cover) {
      try {
        patchBookLocal(await invoke<Book>("read_set_cover", { id: book.id, url: c.cover }));
      } catch {
      }
    }
    candidates = [];
  }

  async function pickCover() {
    if (!book) return;
    const f = await open({ multiple: false, filters: [{ name: "Image", extensions: ["jpg", "jpeg", "png", "webp"] }] });
    if (typeof f !== "string") return;
    try {
      patchBookLocal(await invoke<Book>("read_set_cover", { id: book.id, path: f }));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function exportNotes(format: string) {
    if (!book) return;
    const ext = format === "md" ? "md" : format;
    const path = await save({ defaultPath: `${book.title} - notes.${ext}` });
    if (!path) return;
    try {
      await invoke("read_annotations_export", { bookId: book.id, format, path });
      showToast("success", tr("read.notes.exported"));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function delNote(a: Annotation) {
    await invoke("read_annotation_delete", { id: a.id });
    notes = notes.filter((n) => n.id !== a.id);
  }

  let pct = $derived(book ? Math.round(book.progress * 100) : 0);
  let seriesBooks = $derived(
    book?.series ? lib.books.filter((b) => b.series === book!.series && b.id !== book!.id).sort((a, b) => (a.series_index ?? 0) - (b.series_index ?? 0)) : [],
  );
</script>

{#if !book}
  <div class="r-empty"><p>{$t("read.book.missing")}</p><button class="btn" onclick={back}>{$t("read.back")}</button></div>
{:else}
  {@const b = book}
  <div class="bv">
    <div class="side">
      <button class="cover" onclick={() => openBook(b)} aria-label={$t("read.book.read") as string}>
        <Cover src={coverUrl(b.cover, false)} title={b.title} radius={8} />
      </button>
      <button class="btn btn-primary wide" onclick={() => openBook(b)}>
        <ReadIcon name="book-open" size={15} />
        {b.progress > 0 && b.status !== "finished" ? $t("read.book.continue", { pct }) : $t("read.book.read")}
      </button>
      {#if pct > 0}<div class="r-bar"><i style:width="{pct}%"></i></div>{/if}
      <div class="stars" role="radiogroup" aria-label={$t("read.book.rating") as string}>
        {#each [1, 2, 3, 4, 5] as n (n)}
          <button role="radio" aria-checked={b.rating === n} onclick={() => patch({ rating: b.rating === n ? 0 : n })}>
            <ReadIcon name={b.rating >= n ? "star-fill" : "star"} size={18} />
          </button>
        {/each}
      </div>
      <select class="input" value={b.status} onchange={(e) => patch({ status: (e.target as HTMLSelectElement).value })} aria-label={$t("read.book.status") as string}>
        {#each ["unread", "want", "reading", "finished", "abandoned"] as s (s)}<option value={s}>{$t(`read.status.${s}`)}</option>{/each}
      </select>
      <div class="acts">
        <button class="r-icon-btn" class:on={b.favorite} onclick={() => patch({ favorite: !b.favorite })} title={$t("read.book.favorite") as string} aria-label={$t("read.book.favorite") as string}><ReadIcon name={b.favorite ? "heart-fill" : "heart"} size={17} /></button>
        <button class="r-icon-btn" onclick={() => (ui.shelfPicker = [b.id])} title={$t("read.shelf.add_to") as string} aria-label={$t("read.shelf.add_to") as string}><ReadIcon name="shelf" size={17} /></button>
        <button class="r-icon-btn" onclick={() => go({ kind: "tools", tab: "convert", ids: [b.id] })} title={$t("read.tools.convert") as string} aria-label={$t("read.tools.convert") as string}><ReadIcon name="convert" size={17} /></button>
        <button class="r-icon-btn" onclick={() => go({ kind: "tools", tab: "send", ids: [b.id] })} title={$t("read.tools.send") as string} aria-label={$t("read.tools.send") as string}><ReadIcon name="send" size={17} /></button>
        <button class="r-icon-btn" onclick={() => invoke("reveal_file", { path: b.path })} title={$t("read.book.reveal") as string} aria-label={$t("read.book.reveal") as string}><ReadIcon name="folder" size={17} /></button>
        {#if b.source === "webnovel" && b.source_url}
          <button class="r-icon-btn" onclick={() => invoke("read_webnovel", { url: b.source_url, tool: "fanficfare", updateBook: b.id }).then(() => showToast("success", tr("read.book.updated", { count: 1 }))).catch((e) => showToast("error", String(e)))} title={$t("read.book.update_story") as string} aria-label={$t("read.book.update_story") as string}><ReadIcon name="refresh" size={17} /></button>
        {/if}
        <button class="r-icon-btn" onclick={pickCover} title={$t("read.book.change_cover") as string} aria-label={$t("read.book.change_cover") as string}><ReadIcon name="image" size={17} /></button>
        <button
          class="r-icon-btn"
          onclick={() =>
            confirm(tr("read.book.remove", { count: 1 }), tr("read.book.remove_body"), tr("read.remove"), async () => {
              await invoke("read_remove_books", { ids: [b.id], deleteFiles: false });
              removeBooksLocal([b.id]);
              back();
            })}
          title={$t("read.remove") as string}><ReadIcon name="trash" size={17} /></button
        >
      </div>
    </div>

    <div class="main">
      {#if !editing}
        <div class="r-h" style="margin-top:0">
          <h1>{b.title}</h1>
          <span class="grow"></span>
          <button class="r-chip" onclick={() => startEdit(b)}><ReadIcon name="edit" size={13} /> {$t("read.book.edit")}</button>
          <button class="r-chip" onclick={lookup} disabled={looking}><ReadIcon name="sparkles" size={13} spin={looking} /> {$t("read.book.fetch_meta")}</button>
        </div>
        <p class="au">{b.authors.join(", ") || $t("read.book.unknown_author")}</p>
        {#if b.series}
          <button class="r-chip" onclick={() => go({ kind: "library" })}>{b.series}{b.series_index != null ? ` #${b.series_index}` : ""}</button>
        {/if}
        {#if b.description}<p class="desc">{b.description.replace(/<[^>]+>/g, "")}</p>{/if}
        <dl class="facts">
          {#if b.publisher}<dt>{$t("read.meta.publisher")}</dt><dd>{b.publisher}</dd>{/if}
          {#if b.published}<dt>{$t("read.meta.published")}</dt><dd>{b.published}</dd>{/if}
          {#if b.language}<dt>{$t("read.meta.language")}</dt><dd>{b.language}</dd>{/if}
          {#if b.isbn}<dt>ISBN</dt><dd>{b.isbn}</dd>{/if}
          {#if b.pages}<dt>{$t("read.meta.pages")}</dt><dd>{b.pages}</dd>{/if}
          <dt>{$t("read.library.format")}</dt><dd><span class="r-badge">{b.format}</span> · {fmtBytes(b.size)}</dd>
          {#if b.read_ms}<dt>{$t("read.meta.time_read")}</dt><dd>{fmtDuration(b.read_ms)}</dd>{/if}
          {#if b.started_at}<dt>{$t("read.meta.started")}</dt><dd>{fmtDate(b.started_at)}</dd>{/if}
          {#if b.finished_at}<dt>{$t("read.meta.finished")}</dt><dd>{fmtDate(b.finished_at)}</dd>{/if}
          <dt>{$t("read.meta.added")}</dt><dd>{fmtDate(b.added_at)}</dd>
          {#if b.source_url}<dt>{$t("read.meta.source")}</dt><dd class="path">{b.source_url}</dd>{/if}
          <dt>{$t("read.meta.file")}</dt><dd class="path">{b.path}</dd>
        </dl>
        {#if b.subjects.length || b.tags.length}
          <div class="r-line tags">
            {#each b.tags as tg (tg)}<span class="r-chip on">#{tg}</span>{/each}
            {#each b.subjects.slice(0, 16) as s (s)}<span class="r-chip">{s}</span>{/each}
          </div>
        {/if}
      {:else}
        <form
          class="r-panel"
          onsubmit={(e) => {
            e.preventDefault();
            void saveEdit();
          }}
        >
          <div class="r-fields">
            {#each ["title", "authors", "series", "series_index", "language", "publisher", "published", "isbn", "subjects", "tags"] as k (k)}
              <label class="r-field">{$t(`read.meta.${k}`)}<input class="input" bind:value={form[k]} /></label>
            {/each}
          </div>
          <label class="r-field" style="margin-top:12px">{$t("read.meta.description")}<textarea class="input" rows="6" bind:value={form.description}></textarea></label>
          {#if b.format === "epub" || b.format === "kepub"}
            <label class="r-line wf"><input type="checkbox" bind:checked={writeFile} /> {$t("read.book.write_file")}</label>
          {/if}
          <div class="dialog-actions">
            <button type="button" class="btn" onclick={lookup} disabled={looking}><ReadIcon name="sparkles" size={14} /> {$t("read.book.fetch_meta")}</button>
            <span style="flex:1"></span>
            <button type="button" class="btn" onclick={() => (editing = false)}>{$t("read.cancel")}</button>
            <button type="submit" class="btn btn-primary" disabled={saving}>{$t("read.save")}</button>
          </div>
        </form>
      {/if}

      {#if candidates.length}
        <h2 class="r-sub">{$t("read.book.candidates")}</h2>
        <ul class="r-list">
          {#each candidates as c, i (i)}
            <li class="r-li cand">
              <div class="cc"><Cover src={c.cover ?? ""} title={c.title} radius={3} /></div>
              <div class="ci">
                <b>{c.title}</b>
                <span class="r-muted">{c.authors.join(", ")} · {c.published} · {c.publisher} <span class="r-badge">{c.source}</span></span>
                <span class="r-muted clamp">{c.description.replace(/<[^>]+>/g, "")}</span>
              </div>
              <button class="btn" onclick={() => applyCandidate(c)}>{$t("read.book.use")}</button>
            </li>
          {/each}
        </ul>
      {/if}

      {#if seriesBooks.length}
        <h2 class="r-sub">{$t("read.book.more_in_series")}</h2>
        <div class="r-row" style="--card-w:110px">
          {#each seriesBooks as s (s.id)}
            <button class="mini" onclick={() => go({ kind: "book", id: s.id })}>
              <Cover src={coverUrl(s.cover)} title={s.title} />
              <span>#{s.series_index ?? "?"}</span>
            </button>
          {/each}
        </div>
      {/if}

      <h2 class="r-sub">
        {$t("read.notes.title")} <span class="r-muted">{notes.length}</span>
        {#if notes.length}
          <span style="flex:1"></span>
          {#each ["md", "html", "json", "csv", "txt"] as f (f)}<button class="r-chip" onclick={() => exportNotes(f)}>{f.toUpperCase()}</button>{/each}
        {/if}
      </h2>
      {#if !notes.length}
        <p class="r-muted">{$t("read.notes.none")}</p>
      {:else}
        <ul class="r-list">
          {#each notes as a (a.id)}
            <li class="note" style:--c={a.color}>
              <button class="nt" onclick={() => openBook(b, a.locator)}>
                {#if a.kind === "bookmark"}<ReadIcon name="bookmark-fill" size={14} />{/if}
                {#if a.text}<q>{a.text}</q>{/if}
                {#if a.note}<p>{a.note}</p>{/if}
                <span class="r-muted">{a.chapter}{a.progress != null ? ` · ${Math.round(a.progress * 100)}%` : ""} · {fmtDate(a.created_at)}</span>
              </button>
              <button class="r-icon-btn" onclick={() => delNote(a)} aria-label={$t("read.delete") as string}><ReadIcon name="trash" size={14} /></button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>
{/if}

<style>
  .bv {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    gap: var(--space-6);
    padding-top: var(--space-3);
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .cover {
    all: unset;
    cursor: pointer;
    border-radius: 8px;
  }
  .cover:focus-visible {
    outline: var(--focus-ring);
  }
  .wide {
    justify-content: center;
  }
  .stars {
    display: flex;
    justify-content: center;
    gap: 2px;
    color: #f2b300;
  }
  .stars button {
    all: unset;
    cursor: pointer;
    padding: 2px;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 2px;
  }
  .au {
    margin: 0 0 var(--space-2);
    font-size: var(--text-lg);
    color: var(--text-dim);
  }
  .desc {
    white-space: pre-line;
    line-height: 1.55;
    max-width: 72ch;
    color: var(--text-dim);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px var(--space-4);
    font-size: var(--text-sm);
    margin: var(--space-3) 0;
  }
  .facts dt {
    color: var(--text-muted);
  }
  .facts dd {
    margin: 0;
  }
  .path {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    word-break: break-all;
  }
  .tags {
    margin-top: var(--space-2);
  }
  .wf {
    margin-top: var(--space-2);
    font-size: var(--text-sm);
  }
  .cand {
    align-items: flex-start;
  }
  .cc {
    width: 46px;
    flex-shrink: 0;
  }
  .ci {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .mini {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: 4px;
    cursor: pointer;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: 8px 10px;
    border-left: 3px solid var(--c, #e5c100);
    margin-bottom: 6px;
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
    background: var(--fill-1);
  }
  .nt {
    all: unset;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
    cursor: pointer;
    min-width: 0;
  }
  .nt q {
    font-family: Georgia, serif;
    line-height: 1.5;
  }
  .nt p {
    margin: 0;
    font-size: var(--text-sm);
  }
  @media (max-width: 760px) {
    .bv {
      grid-template-columns: 1fr;
    }
    .side {
      max-width: 240px;
    }
  }
</style>
