<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openUrl } from "@tauri-apps/plugin-shell";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { library, seriesCover } from "$lib/read/library-store.svelte";
  import { chapterLabel, confirm, fmtDate, go, openChapter, openMenu, relTime, tr } from "$lib/read/ui-store.svelte";
  import type { Chapter, Hit, Page, SeriesFull } from "$lib/read/types";
  import Cover from "../Cover.svelte";
  import { agentActions } from "$lib/native-agent/actions";
  import ReadIcon from "../ReadIcon.svelte";
  import Modal from "$components/music/Modal.svelte";

  let { id }: { id: number } = $props();

  let data = $state<SeriesFull | null>(null);
  let loading = $state(false);
  let refreshing = $state(false);
  let asc = $state(false);
  let show = $state<"all" | "unread" | "downloaded" | "bookmarked">("all");
  let lang = $state("");
  let scan = $state("");
  let sel = $state<number[]>([]);
  let migrate = $state<{ q: string; source: string; results: Hit[]; busy: boolean } | null>(null);
  type TrackHit = { id: number; title: string; cover: string | null; url: string; chapters: number | null; status: string };
  let track = $state<{ service: string; q: string; results: TrackHit[]; busy: boolean } | null>(null);

  async function searchTrack() {
    if (!track) return;
    track.busy = true;
    try {
      track.results = await invoke<TrackHit[]>("read_tracker_search", { service: track.service, query: track.q });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      if (track) track.busy = false;
    }
  }

  async function linkTrack(service: string, hit: TrackHit | null) {
    try {
      await invoke("read_tracker_link", { seriesId: id, service, hit });
      track = null;
      await load();
    } catch (e) {
      showToast("error", String(e));
    }
  }
  let lib = $derived(library());

  async function load() {
    loading = !data;
    try {
      data = await invoke<SeriesFull>("read_series_get", { id });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void id;
    void lib.version;
    void lib.series;
    void load();
  });

  async function refresh() {
    refreshing = true;
    try {
      const n = await invoke<number>("read_series_refresh", { id });
      if (n) showToast("success", tr("read.updates.found", { count: n }));
      await load();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      refreshing = false;
    }
  }

  let langs = $derived([...new Set((data?.chapters ?? []).map((c) => c.lang).filter(Boolean))]);
  let scanlators = $derived([...new Set((data?.chapters ?? []).map((c) => c.scanlator).filter(Boolean))]);
  let chapters = $derived.by(() => {
    let l = [...(data?.chapters ?? [])];
    if (lang) l = l.filter((c) => c.lang === lang);
    if (scan) l = l.filter((c) => c.scanlator === scan);
    if (show === "unread") l = l.filter((c) => !c.read);
    if (show === "downloaded") l = l.filter((c) => c.book_id);
    if (show === "bookmarked") l = l.filter((c) => c.bookmarked);
    return asc ? l : l.reverse();
  });
  let next = $derived.by(() => {
    const all = (data?.chapters ?? []).filter((c) => !lang || c.lang === lang);
    return all.find((c) => !c.read) ?? all[all.length - 1];
  });
  let streamOnly = $derived(data?.series.source === "mangaplus");

  async function follow() {
    if (!data) return;
    await invoke("read_series_follow", { id, follow: !data.series.in_library });
    await load();
  }

  async function mark(ids: number[], read: boolean) {
    await invoke("read_chapters_mark", { ids, read });
    sel = [];
    await load();
  }

  async function download(ids: number[]) {
    try {
      const n = await invoke<number>("read_chapters_download", { ids });
      showToast("success", tr("read.series.queued", { count: n }));
      sel = [];
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function toggle(c: Chapter, e: MouseEvent) {
    if (e.shiftKey && sel.length) {
      const idx = chapters.findIndex((x) => x.id === sel[sel.length - 1]);
      const j = chapters.findIndex((x) => x.id === c.id);
      const [a, b] = idx < j ? [idx, j] : [j, idx];
      sel = [...new Set([...sel, ...chapters.slice(a, b + 1).map((x) => x.id)])];
    } else {
      sel = sel.includes(c.id) ? sel.filter((x) => x !== c.id) : [...sel, c.id];
    }
  }

  function chMenu(c: Chapter, e: MouseEvent) {
    openMenu(e, [
      { label: tr("read.book.read"), icon: "play", action: () => openChapter(c) },
      { label: c.read ? tr("read.series.mark_unread") : tr("read.series.mark_read"), icon: "check", action: () => mark([c.id], !c.read) },
      { label: tr("read.series.mark_previous"), icon: "check", action: async () => { await invoke("read_chapters_mark_previous", { id: c.id }); await load(); } },
      { label: c.bookmarked ? tr("read.series.unbookmark") : tr("read.series.bookmark"), icon: "star", action: async () => { await invoke("read_chapter_bookmark", { id: c.id, on: !c.bookmarked }); await load(); } },
      { divider: true },
      c.book_id
        ? { label: tr("read.series.delete_download"), icon: "trash", action: async () => { await invoke("read_chapters_delete_downloads", { ids: [c.id] }); await load(); } }
        : { label: tr("read.series.download"), icon: "download", disabled: streamOnly || !!c.external_url, action: () => download([c.id]) },
    ]);
  }

  async function setReader(mode: string) {
    if (!data) return;
    await invoke("read_series_update", { id, patch: { reader: { ...data.series.reader, mode } } });
    await load();
  }

  async function setAuto(on: boolean) {
    await invoke("read_series_update", { id, patch: { auto_download: on } });
    await load();
  }

  async function saveNotes(v: string) {
    await invoke("read_series_update", { id, patch: { notes: v } });
  }

  async function mergeVolume() {
    if (!data) return;
    const ids = (sel.length ? sel : data.chapters.filter((c) => c.book_id).map((c) => c.id)).filter((x) => data!.chapters.find((c) => c.id === x)?.book_id);
    if (!ids.length) return showToast("error", tr("read.series.nothing_downloaded"));
    const first = data.chapters.find((c) => c.id === ids[0]);
    const last = data.chapters.find((c) => c.id === ids[ids.length - 1]);
    const title = `${data.series.title} ${first?.number ?? ""}–${last?.number ?? ""}`.trim();
    try {
      await invoke("read_merge_chapters", { chapterIds: ids, to: "epub", title, opts: null });
      showToast("success", tr("read.series.merged"));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function searchMigrate() {
    if (!migrate) return;
    migrate.busy = true;
    try {
      const p = await invoke<Page>("read_browse", { req: { source: migrate.source, query: migrate.q, mode: "search", page: 1 } });
      migrate.results = p.hits;
    } catch (e) {
      showToast("error", String(e));
    } finally {
      if (migrate) migrate.busy = false;
    }
  }

  async function doMigrate(h: Hit) {
    try {
      const to = await invoke<number>("read_series_migrate", { fromId: id, source: h.source, key: h.key });
      migrate = null;
      showToast("success", tr("read.series.migrated"));
      go({ kind: "series", id: to });
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

{#if loading}
  <div class="r-empty"><ReadIcon name="refresh" size={22} spin /></div>
{:else if data}
  {@const s = data.series}
  <div class="head">
    <div class="cv"><Cover src={seriesCover(s)} title={s.title} ratio={1.45} radius={8} /></div>
    <div class="info">
      <h1>{s.title}</h1>
      <p class="r-muted">{s.authors.join(", ")}{s.status ? ` · ${s.status}` : ""} · <span class="r-badge">{s.source.replace(/^decl:\d+/, "custom")}</span></p>
      <div class="r-line">
        {#if next}
          <button data-agent-actions="click" use:agentActions={["click"]} class="btn btn-primary" onclick={() => openChapter(next!)}>
            <ReadIcon name={next.external_url ? "external" : "book-open"} size={15} />
            {next.read ? $t("read.series.reread") : next.last_page ? $t("read.series.continue", { ch: chapterLabel(next) }) : $t("read.series.start", { ch: chapterLabel(next) })}
          </button>
        {/if}
        <button data-agent-actions="click" use:agentActions={["click"]} class="btn" onclick={follow}>
          <ReadIcon name={s.in_library ? "heart-fill" : "heart"} size={15} /> {s.in_library ? $t("read.series.following") : $t("read.series.follow")}
        </button>
        <button data-agent-actions="click" use:agentActions={["click"]} class="btn" onclick={refresh} disabled={refreshing}><ReadIcon name="refresh" size={15} spin={refreshing} /> {$t("read.series.refresh")}</button>
        {#if s.url}<button class="r-icon-btn" onclick={() => openUrl(s.url)} title={$t("read.series.open_site") as string} aria-label={$t("read.series.open_site") as string}><ReadIcon name="external" size={16} /></button>{/if}
        <button class="r-icon-btn" onclick={() => (migrate = { q: s.title, source: "mangadex", results: [], busy: false })} title={$t("read.series.migrate") as string} aria-label={$t("read.series.migrate") as string}><ReadIcon name="migrate" size={16} /></button>
      </div>
      {#if s.genres.length}<div class="r-line gen">{#each s.genres.slice(0, 14) as g (g)}<span class="r-chip">{g}</span>{/each}</div>{/if}
      {#if s.description}<p class="desc">{s.description}</p>{/if}
      <div class="r-line opts">
        <label class="r-field inl">{$t("read.series.reader_mode")}
          <select class="input" value={(s.reader.mode as string) ?? ""} onchange={(e) => setReader((e.target as HTMLSelectElement).value)}>
            <option value="">{$t("read.series.mode_default")}</option>
            <option value="rtl">{$t("read.comic.rtl")}</option>
            <option value="ltr">{$t("read.comic.ltr")}</option>
            <option value="vertical">{$t("read.comic.vertical")}</option>
            <option value="webtoon">{$t("read.comic.webtoon")}</option>
          </select>
        </label>
        {#if !streamOnly}
          <label class="r-line"><input type="checkbox" checked={s.auto_download} onchange={(e) => setAuto((e.target as HTMLInputElement).checked)} /> {$t("read.series.auto_download")}</label>
        {:else}
          <span class="r-muted"><ReadIcon name="info" size={13} /> {$t("read.series.stream_only")}</span>
        {/if}
      </div>
      <div class="r-line trk">
        <span class="r-muted">{$t("read.track.title")}</span>
        {#each ["anilist", "mangaupdates"] as sv (sv)}
          {@const linked = (s.tracking as any)?.[sv]}
          {#if linked}
            <span class="r-chip on" title={linked.url}>{sv === "anilist" ? "AniList" : "MangaUpdates"}: {linked.title}
              <button class="x" onclick={() => linkTrack(sv, null)} aria-label={$t("read.remove") as string}>×</button></span>
          {:else}
            <button class="r-chip" onclick={() => { track = { service: sv, q: s.title, results: [], busy: false }; void searchTrack(); }}>+ {sv === "anilist" ? "AniList" : "MangaUpdates"}</button>
          {/if}
        {/each}
      </div>
      <details class="notes">
        <summary>{$t("read.series.notes")}</summary>
        <textarea class="input" rows="3" value={s.notes} onchange={(e) => saveNotes((e.target as HTMLTextAreaElement).value)}></textarea>
      </details>
    </div>
  </div>

  <div class="r-h">
    <h2>{$t("read.series.chapters", { count: data.chapters.length })}</h2>
    <span class="r-muted">{$t("read.series.unread_count", { count: s.unread })}</span>
    <span class="grow"></span>
    <button class="r-icon-btn" onclick={() => (asc = !asc)} title={$t("read.library.reverse") as string} aria-label={$t("read.library.reverse") as string}><ReadIcon name="sort" size={15} /></button>
  </div>
  <div class="r-toolbar">
    {#each ["all", "unread", "downloaded", "bookmarked"] as f (f)}
      <button class="r-chip" class:on={show === f} onclick={() => (show = f as typeof show)}>{$t(`read.series.show_${f}`)}</button>
    {/each}
    {#if langs.length > 1}
      <select class="input sel" bind:value={lang}>
        <option value="">{$t("read.series.all_langs")}</option>
        {#each langs as l (l)}<option value={l}>{l}</option>{/each}
      </select>
    {/if}
    {#if scanlators.length > 1}
      <select class="input sel" bind:value={scan}>
        <option value="">{$t("read.series.all_groups")}</option>
        {#each scanlators as g (g)}<option value={g}>{g}</option>{/each}
      </select>
    {/if}
  </div>
  {#if sel.length}
    <div class="selbar">
      <b>{$t("read.library.selected", { count: sel.length })}</b>
      <button class="r-chip" onclick={() => mark(sel, true)}>{$t("read.series.mark_read")}</button>
      <button class="r-chip" onclick={() => mark(sel, false)}>{$t("read.series.mark_unread")}</button>
      {#if !streamOnly}<button class="r-chip" onclick={() => download(sel)}>{$t("read.series.download")}</button>{/if}
      <button class="r-chip" onclick={mergeVolume}>{$t("read.series.merge")}</button>
      <button class="r-chip" onclick={() => (sel = chapters.map((c) => c.id))}>{$t("read.library.select_all")}</button>
      <button class="r-chip" onclick={() => (sel = [])}>{$t("read.cancel")}</button>
    </div>
  {:else if !streamOnly && data.chapters.some((c) => !c.book_id && !c.read)}
    <div class="r-toolbar">
      <button class="r-chip" data-agent-actions="click" use:agentActions={["click"]} onclick={() => download(data!.chapters.filter((c) => !c.book_id && !c.read && !c.external_url).map((c) => c.id))}><ReadIcon name="download" size={13} /> {$t("read.series.download_unread")}</button>
      <button class="r-chip" onclick={() => download(data!.chapters.filter((c) => !c.book_id && !c.external_url).map((c) => c.id))}><ReadIcon name="download" size={13} /> {$t("read.series.download_all")}</button>
      {#if data.chapters.some((c) => c.book_id)}<button class="r-chip" onclick={mergeVolume}><ReadIcon name="layers" size={13} /> {$t("read.series.merge")}</button>{/if}
    </div>
  {/if}

  <ul class="r-list chs">
    {#each chapters as c (c.id)}
      <li class="ch" class:read={c.read} class:sel={sel.includes(c.id)}>
        <input type="checkbox" checked={sel.includes(c.id)} onclick={(e) => toggle(c, e)} aria-label={chapterLabel(c)} />
        <button data-agent-actions="click" use:agentActions={["click"]} class="cb" onclick={() => (c.external_url ? openUrl(c.external_url) : openChapter(c))} oncontextmenu={(e) => chMenu(c, e)}>
          <span class="nm">{chapterLabel(c)}{#if c.bookmarked} <ReadIcon name="star-fill" size={11} />{/if}</span>
          <span class="mt r-muted">
            {#if c.scanlator}{c.scanlator} · {/if}{c.lang ? `${c.lang} · ` : ""}{c.published ? relTime(c.published) : fmtDate(c.fetched_at)}
            {#if !c.read && c.last_page > 0} · {$t("read.series.page", { page: c.last_page + 1 })}{/if}
          </span>
        </button>
        {#if c.external_url}<span class="r-badge">{$t("read.series.external")}</span>{/if}
        {#if c.book_id}<ReadIcon name="check" size={14} class="dl" />{/if}
        <button class="r-icon-btn" onclick={(e) => chMenu(c, e)} aria-label={$t("read.more") as string}><ReadIcon name="more" size={15} /></button>
      </li>
    {/each}
  </ul>
{/if}

{#if track}
  <Modal title={$t("read.track.link") as string} width={520} onclose={() => (track = null)}>
    <form class="r-line" onsubmit={(e) => { e.preventDefault(); void searchTrack(); }}>
      <input class="input" style="flex:1" bind:value={track.q} />
      <button class="btn btn-primary" type="submit" disabled={track.busy}>{$t("read.search.go")}</button>
    </form>
    <p class="r-muted">{$t("read.track.hint")}</p>
    <ul class="r-list mig">
      {#each track.results as h (h.id)}
        <li class="r-li">
          <div style="width:36px"><Cover src={h.cover ?? ""} title={h.title} radius={3} /></div>
          <span style="flex:1">{h.title} <span class="r-muted">{h.chapters ? `· ${h.chapters} ch.` : ""} {h.status}</span></span>
          <button class="btn" onclick={() => linkTrack(track!.service, h)}>{$t("read.track.use")}</button>
        </li>
      {/each}
    </ul>
  </Modal>
{/if}

{#if migrate}
  <Modal title={$t("read.series.migrate") as string} width={560} onclose={() => (migrate = null)}>
    <form
      class="r-line"
      onsubmit={(e) => {
        e.preventDefault();
        void searchMigrate();
      }}
    >
      <select class="input" bind:value={migrate.source}>
        {#each lib.series.length ? ["mangadex", "mangaplus"] : ["mangadex"] as src (src)}<option value={src}>{src}</option>{/each}
      </select>
      <input class="input" style="flex:1" bind:value={migrate.q} />
      <button class="btn btn-primary" type="submit" disabled={migrate.busy}>{$t("read.search.go")}</button>
    </form>
    <p class="r-muted">{$t("read.series.migrate_hint")}</p>
    <ul class="r-list mig">
      {#each migrate.results as h (h.key)}
        <li class="r-li">
          <div style="width:40px"><Cover src={h.cover ?? ""} title={h.title} radius={3} /></div>
          <span style="flex:1">{h.title}</span>
          <button class="btn" onclick={() => doMigrate(h)}>{$t("read.series.migrate_here")}</button>
        </li>
      {/each}
    </ul>
  </Modal>
{/if}

<style>
  .head {
    display: grid;
    grid-template-columns: 180px minmax(0, 1fr);
    gap: var(--space-5);
    margin-top: var(--space-3);
  }
  .info h1 {
    margin: 0 0 4px;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
  }
  .info .r-line {
    margin-top: var(--space-2);
  }
  .gen .r-chip {
    cursor: default;
    height: 24px;
    font-size: var(--text-xs);
  }
  .desc {
    white-space: pre-line;
    line-height: 1.5;
    max-width: 80ch;
    color: var(--text-dim);
    max-height: 9em;
    overflow-y: auto;
  }
  .opts {
    font-size: var(--text-sm);
  }
  .opts .r-field.inl {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  .opts .inl select {
    height: 28px;
    padding: 0 8px;
  }
  .trk {
    font-size: var(--text-sm);
  }
  .trk .x {
    all: unset;
    cursor: pointer;
    margin-left: 4px;
  }
  .notes summary {
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
    margin-top: var(--space-2);
  }
  .notes textarea {
    width: 100%;
    margin-top: 6px;
  }
  .sel {
    height: 28px;
    padding: 0 8px;
    font-size: var(--text-sm);
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
  .ch {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px 6px;
    border-radius: var(--radius-sm);
  }
  .ch:hover {
    background: var(--fill-1);
  }
  .ch.sel {
    background: var(--accent-soft);
  }
  .ch.read .nm {
    color: var(--text-muted);
  }
  .cb {
    all: unset;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    padding: 4px 0;
    cursor: pointer;
  }
  .nm {
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .mt {
    font-size: var(--text-xs);
  }
  .ch :global(.dl) {
    color: var(--success, #2f9e52);
  }
  .mig {
    max-height: 360px;
    overflow-y: auto;
  }
  @media (max-width: 720px) {
    .head {
      grid-template-columns: 110px 1fr;
    }
  }
</style>
