<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { proxiedImage } from "$lib/read/library-store.svelte";
  import { go, ui, tr } from "$lib/read/ui-store.svelte";
  import type { Hit, Page, SeriesFull, SourceInfo } from "$lib/read/types";
  import Cover from "../Cover.svelte";
  import ReadIcon from "../ReadIcon.svelte";
  import { agentActions } from "$lib/native-agent/actions";

  let { source = "", q = "" }: { source?: string; q?: string } = $props();

  const LKEY = "omniget.read.browse.v1";
  function loadState() {
    try {
      return { source: "gutenberg", langs: [] as string[], nsfw: false, ...JSON.parse(localStorage.getItem(LKEY) ?? "{}") };
    } catch {
      return { source: "gutenberg", langs: [] as string[], nsfw: false };
    }
  }
  const saved = loadState();

  let sources = $state<SourceInfo[]>([]);
  let current = $state<string>(untrack(() => source) || saved.source);
  let mode = $state<"popular" | "latest" | "search">(untrack(() => q) ? "search" : "popular");
  let query = $state(untrack(() => q));
  let langs = $state<string[]>(saved.langs);
  let nsfw = $state<boolean>(saved.nsfw);
  let hits = $state<Hit[]>([]);
  let next = $state<string | null>(null);
  let page = $state(1);
  let loading = $state(false);
  let error = $state("");
  let global = $state<[string, Page][] | null>(null);
  let folderStack = $state<{ title: string; url: string }[]>([]);
  let opening = $state<string | null>(null);

  $effect(() => {
    try {
      localStorage.setItem(LKEY, JSON.stringify({ source: current, langs, nsfw }));
    } catch {
    }
  });

  onMount(async () => {
    sources = await invoke<SourceInfo[]>("read_sources").catch(() => []);
    if (!sources.some((s) => s.id === current)) current = sources[0]?.id ?? "gutenberg";
    void run(true);
  });

  let info = $derived(sources.find((s) => s.id === current));
  let books = $derived(sources.filter((s) => s.kind === "book"));
  let manga = $derived(sources.filter((s) => s.kind === "manga"));
  let activeKind = $derived<"book" | "manga">(info?.kind === "manga" ? "manga" : "book");
  let visibleSources = $derived(activeKind === "manga" ? manga : books);

  function switchKind(kind: "book" | "manga") {
    const list = kind === "manga" ? manga : books;
    if (list.length && activeKind !== kind) {
      pick(list[0].id);
    }
  }

  async function jumpFolder(idx: number) {
    const target = folderStack[idx];
    if (!target) return;
    loading = true;
    try {
      const p = await invoke<Page>("read_opds_open", { source: current, url: target.url });
      folderStack = folderStack.slice(0, idx + 1);
      hits = p.hits;
      next = p.next;
    } catch (e) {
      showToast("error", String(e));
    } finally {
      loading = false;
    }
  }

  const LANGS = ["en", "pt", "pt-br", "es", "fr", "de", "it", "ja", "ko", "zh", "ru", "id", "vi", "th"];

  async function run(reset: boolean) {
    if (loading) return;
    loading = true;
    error = "";
    global = null;
    if (reset) {
      hits = [];
      next = null;
      page = 1;
      folderStack = [];
    }
    try {
      const m = query.trim() ? "search" : mode === "search" ? "popular" : mode;
      const p = await invoke<Page>("read_browse", {
        req: {
          source: current,
          query: query.trim(),
          mode: m,
          cursor: reset ? null : next && isNaN(Number(next)) ? next : null,
          page,
          lang: langs[0] ?? "",
          langs,
          nsfw,
          filters: {},
        },
      });
      hits = reset ? p.hits : [...hits, ...p.hits];
      next = p.next;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function more() {
    page += 1;
    await run(false);
  }

  async function searchAll() {
    if (!query.trim()) return;
    loading = true;
    error = "";
    try {
      global = await invoke<[string, Page][]>("read_global_search", { query: query.trim(), kind: info?.kind ?? "book", langs });
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function pick(id: string) {
    current = id;
    mode = "popular";
    void run(true);
  }

  async function open(h: Hit) {
    if (h.kind === "folder") {
      loading = true;
      try {
        const p = await invoke<Page>("read_opds_open", { source: h.source, url: h.key });
        folderStack = [...folderStack, { title: h.title, url: h.key }];
        hits = p.hits;
        next = p.next;
      } catch (e) {
        showToast("error", String(e));
      } finally {
        loading = false;
      }
      return;
    }
    if (h.kind === "series") {
      opening = h.key;
      try {
        const full = await invoke<SeriesFull>("read_series_open", { source: h.source, key: h.key, langs: langs.length ? langs : null });
        go({ kind: "series", id: full.series.id });
      } catch (e) {
        showToast("error", String(e));
      } finally {
        opening = null;
      }
      return;
    }
    go({ kind: "hit", hit: h });
  }

  function coverOf(h: Hit) {
    return h.source.startsWith("decl:") ? proxiedImage(h.cover, h.url || "") : (h.cover ?? "");
  }

  function srcName(id: string) {
    return sources.find((s) => s.id === id)?.name ?? id;
  }
</script>

<div class="r-h">
  <h1>{$t("read.nav.browse")}</h1>
  <span class="grow"></span>
  <button class="r-chip" onclick={() => go({ kind: "settings", tab: "sources" })}><ReadIcon name="plus" size={13} /> {$t("read.browse.add_source")}</button>
</div>

<div class="srcs">
  <div class="mac-segmented">
    <button type="button" class="mac-segmented-btn" class:active={activeKind === "book"} onclick={() => switchKind("book")}>{$t("read.browse.books")}</button>
    <button type="button" class="mac-segmented-btn" class:active={activeKind === "manga"} onclick={() => switchKind("manga")}>{$t("read.browse.manga")}</button>
  </div>
  {#each visibleSources as s (s.id)}
    <button class="r-chip" data-agent-actions="click" use:agentActions={["click"]} class:on={current === s.id} onclick={() => pick(s.id)}>
      {#if s.icon}<img src={s.icon} alt="" width="14" height="14" />{/if}{s.name}{#if s.lang && s.lang !== "all"} <em>{s.lang}</em>{/if}
    </button>
  {/each}
</div>

<form
  class="bar"
  onsubmit={(e) => {
    e.preventDefault();
    mode = "search";
    void run(true);
  }}
>
  <div class="mac-segmented">
    <button type="button" class="mac-segmented-btn" class:active={mode === "popular" && !query} onclick={() => { query = ""; mode = "popular"; void run(true); }}>{$t("read.browse.popular")}</button>
    {#if info?.latest}
      <button type="button" class="mac-segmented-btn" class:active={mode === "latest" && !query} onclick={() => { query = ""; mode = "latest"; void run(true); }}>{$t("read.browse.latest")}</button>
    {/if}
  </div>
  <input class="input q" data-agent-actions="fill" use:agentActions={["fill"]} bind:value={query} placeholder={$t("read.browse.search_in", { source: info?.name ?? "" }) as string} />
  <button class="btn btn-primary" data-agent-actions="click" use:agentActions={["click"]} type="submit" disabled={loading}>{$t("read.search.go")}</button>
  <button class="btn" type="button" onclick={searchAll} disabled={loading || !query.trim()} title={$t("read.browse.search_all_hint") as string} aria-label={$t("read.browse.search_all_hint") as string}>{$t("read.browse.search_all")}</button>
</form>

<details class="flt">
  <summary>{$t("read.browse.languages")}{langs.length ? `: ${langs.join(", ")}` : ""}</summary>
  <div class="r-line">
    {#each LANGS as l (l)}
      <button class="r-chip" class:on={langs.includes(l)} onclick={() => { langs = langs.includes(l) ? langs.filter((x) => x !== l) : [...langs, l]; void run(true); }}>{l}</button>
    {/each}
    {#if info?.kind === "manga"}
      <label class="r-chip"><input type="checkbox" bind:checked={nsfw} onchange={() => void run(true)} /> {$t("read.browse.nsfw")}</label>
    {/if}
  </div>
</details>

{#if info?.id === "mangaplus"}<p class="r-muted note"><ReadIcon name="info" size={13} /> {$t("read.series.stream_only")}</p>{/if}
{#if info?.id === "standardebooks" && !query}<p class="r-muted note"><ReadIcon name="info" size={13} /> {$t("read.browse.se_note")}</p>{/if}

{#if folderStack.length}
  <div class="r-line crumbs">
    <button class="r-chip" onclick={() => run(true)}>{srcName(current)}</button>
    {#each folderStack as f, i (i)}<ReadIcon name="right" size={12} /><button class="r-chip" onclick={() => jumpFolder(i)}>{f.title}</button>{/each}
  </div>
{/if}

{#if error}<p class="err">{error}</p>{/if}

{#if global}
  {#each global as [sid, p] (sid)}
    {#if p.hits.length}
      <h2 class="r-sub">{srcName(sid)} <span class="r-muted">{p.total ?? p.hits.length}</span></h2>
      <div class="r-row">
        {#each p.hits.slice(0, 16) as h (h.source + h.key)}
          <button class="hit" data-agent-actions="click" use:agentActions={["click"]} onclick={() => open(h)}>
            <Cover src={coverOf(h)} title={h.title} ratio={h.kind === "series" ? 1.45 : 1.5} />
            <b>{h.title}</b><span>{h.authors.slice(0, 2).join(", ")}</span>
          </button>
        {/each}
      </div>
    {/if}
  {/each}
{:else}
  <div class="r-grid">
    {#each hits as h (h.source + h.key)}
      <button class="hit" data-agent-actions="click" use:agentActions={["click"]} onclick={() => open(h)} disabled={opening === h.key}>
        {#if h.kind === "folder"}
          <div class="folder"><ReadIcon name="folder" size={30} /></div>
        {:else}
          <Cover src={coverOf(h)} title={h.title} ratio={h.kind === "series" ? 1.45 : 1.5} />
        {/if}
        <b>{h.title}</b>
        <span>{h.authors.slice(0, 2).join(", ")}{h.year ? ` · ${h.year}` : ""}</span>
        {#if h.downloads.length}<span class="fmts">{[...new Set(h.downloads.map((d) => d.format))].join(" · ")}</span>{/if}
        {#if opening === h.key}<ReadIcon name="refresh" size={14} spin />{/if}
      </button>
    {/each}
  </div>
  {#if loading}
    <div class="r-empty"><ReadIcon name="refresh" size={22} spin /></div>
  {:else if next}
    <div class="r-empty"><button class="btn" onclick={more}>{$t("read.browse.more")}</button></div>
  {:else if !hits.length && !error}
    <div class="r-empty"><p>{$t("read.browse.no_results")}</p></div>
  {/if}
{/if}

<style>
  .srcs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
  }
  .srcs img {
    border-radius: 3px;
  }
  .srcs em {
    font-style: normal;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-3) 0 var(--space-2);
  }
  .q {
    flex: 1;
    min-width: 200px;
  }
  .flt summary {
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
    margin-bottom: 6px;
  }
  label.r-chip input {
    margin: 0;
  }
  .note {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .crumbs {
    margin: var(--space-2) 0;
    font-size: var(--text-sm);
  }
  .err {
    color: var(--danger);
  }
  .hit {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    cursor: pointer;
  }
  .hit:focus-visible {
    outline: var(--focus-ring);
    border-radius: 6px;
  }
  .hit b {
    font-size: var(--text-sm);
    line-height: 1.25;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .hit span {
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fmts {
    text-transform: uppercase;
  }
  .folder {
    display: grid;
    place-items: center;
    aspect-ratio: 1 / 1.5;
    border-radius: 6px;
    background: var(--fill-1);
    color: var(--text-muted);
  }
</style>
