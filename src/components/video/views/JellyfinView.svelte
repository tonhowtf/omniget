<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openResolved } from "$lib/video/player.svelte";
  import { go, goRoot, ui } from "$lib/video/ui.svelte";
  import type { JfItem, JfView, SourceInfo } from "$lib/video/types";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import MediaCard from "./parts/MediaCard.svelte";
  import Shelf from "./parts/Shelf.svelte";
  import Skel from "./parts/Skel.svelte";
  import Thumb from "./parts/Thumb.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, fmtDur } from "./parts/util";

  const PAGE = 60;
  const PLAYABLE = new Set(["Movie", "Episode", "Video", "MusicVideo", "Trailer", "TvChannel", "Recording"]);

  let {
    source: sourceProp = undefined,
    parent: parentProp = undefined,
    title: titleProp = undefined,
  }: { source?: number; parent?: string | null; title?: string | null } = $props();

  let v = $derived(ui.view.name === "jellyfin" ? ui.view : null);
  let wantSource = $derived(sourceProp ?? v?.source ?? null);
  let parent = $derived(parentProp ?? v?.parent ?? null);
  let pageTitle = $derived(titleProp ?? v?.title ?? null);

  let sources = $state<SourceInfo[] | null>(null);
  let error = $state<string | null>(null);

  let views = $state<JfView[] | null>(null);
  let resume = $state<JfItem[]>([]);
  let nextUp = $state<JfItem[]>([]);

  let item = $state<JfItem | null>(null);
  let items = $state<JfItem[] | null>(null);
  let total = $state(0);
  let q = $state("");
  let qLive = $state("");
  let loadingMore = $state(false);

  let seasons = $state<JfItem[] | null>(null);
  let season = $state<string | null>(null);
  let episodes = $state<JfItem[] | null>(null);

  let starting = $state<string | null>(null);

  let jf = $derived((sources ?? []).filter((s) => s.kind === "jellyfin"));
  let src = $derived(jf.find((s) => s.id === wantSource) ?? jf.find((s) => s.enabled) ?? null);

  $effect(() => {
    api
      .sources()
      .then((s) => (sources = s))
      .catch((e) => {
        error = errText(e);
        sources = [];
      });
  });

  $effect(() => {
    const h = setTimeout(() => (qLive = q), 300);
    return () => clearTimeout(h);
  });

  $effect(() => {
    if (!src || parent) return;
    const id = src.id;
    views = null;
    Promise.all([api.jfViews(id), api.jfResume(id).catch(() => []), api.jfNextUp(id).catch(() => [])])
      .then(([vs, r, n]) => {
        views = vs;
        resume = r;
        nextUp = n;
        error = null;
      })
      .catch((e) => {
        error = errText(e);
        views = [];
      });
  });

  $effect(() => {
    if (!src || !parent) return;
    const id = src.id;
    const p = parent;
    item = null;
    items = null;
    seasons = null;
    episodes = null;
    season = null;
    q = "";
    qLive = "";
    api
      .jfItem(id, p)
      .then((it) => {
        item = it;
        if (it.type === "Series") {
          return api.jfChildren(id, p).then((ss) => {
            seasons = ss;
            const firstUnplayed = ss.find((s) => !s.played) ?? ss[0];
            season = firstUnplayed?.id ?? null;
          });
        }
        if (it.type === "Season") {
          season = it.id;
          seasons = [];
        }
      })
      .catch((e) => (error = errText(e)));
  });

  $effect(() => {
    if (!src || !season) return;
    const id = src.id;
    const s = season;
    episodes = null;
    api
      .jfChildren(id, s)
      .then((eps) => {
        if (season === s) episodes = eps;
      })
      .catch((e) => (error = errText(e)));
  });

  $effect(() => {
    if (!src || !parent || !item || item.type === "Series" || item.type === "Season") return;
    const id = src.id;
    const p = parent;
    const needle = qLive.trim() || null;
    items = null;
    api
      .jfItems(id, { parentId: p, search: needle, start: 0, limit: PAGE })
      .then((r) => {
        items = r.items;
        total = r.total;
      })
      .catch((e) => {
        error = errText(e);
        items = [];
      });
  });

  async function more() {
    if (!src || !parent || !items) return;
    loadingMore = true;
    try {
      const r = await api.jfItems(src.id, { parentId: parent, search: qLive.trim() || null, start: items.length, limit: PAGE });
      items = [...items, ...r.items];
      total = r.total;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      loadingMore = false;
    }
  }

  async function play(it: JfItem) {
    if (!src) return;
    starting = it.id;
    try {
      const info = await api.jfPlay(src.id, it.id);
      await openResolved(info);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      starting = null;
    }
  }

  function open(it: JfItem) {
    if (PLAYABLE.has(it.type)) void play(it);
    else if (src) go({ name: "jellyfin", source: src.id, parent: it.id, title: it.name });
  }

  function sub(it: JfItem): string {
    if (it.type === "Episode") {
      const code = it.season != null && it.episode != null ? `S${it.season}E${it.episode}` : "";
      return [it.series_name, code].filter(Boolean).join(" · ");
    }
    if (it.type === "Series" || it.type === "Season" || it.type === "Folder" || it.type === "BoxSet") {
      return it.child_count != null ? ($t("video.jellyfin.items_count", { count: it.child_count }) as string) : (it.year ? String(it.year) : "");
    }
    return [it.year, fmtDur(it.runtime_ms)].filter(Boolean).join(" · ");
  }

  function prog(it: JfItem): number | null {
    return it.position_ms && it.runtime_ms && !it.played ? it.position_ms / it.runtime_ms : null;
  }

  function ratioFor(it: JfItem): string {
    return it.type === "Episode" || it.type === "Video" ? "16 / 9" : "2 / 3";
  }

  let playNextEp = $derived.by(() => {
    if (!item || item.type !== "Series") return null;
    return nextUp.find((n) => n.series_name === item!.name) ?? episodes?.find((e) => !e.played) ?? episodes?.[0] ?? null;
  });
</script>

{#snippet itemGrid(list: JfItem[])}
  <div class="grid" class:wide={list.every((x) => ratioFor(x) === "16 / 9")}>
    {#each list as it (it.id)}
      <MediaCard
        title={it.type === "Episode" ? it.name : it.name}
        subtitle={sub(it)}
        src={it.image}
        seed={it.series_name ?? it.name}
        ratio={ratioFor(it)}
        progress={prog(it)}
        watched={it.played}
        onopen={() => open(it)}
        onplay={PLAYABLE.has(it.type) ? () => void play(it) : undefined}
      />
    {/each}
  </div>
{/snippet}

<div class="page">
  {#if !sources}
    <Skel count={8} />
  {:else if !src}
    <Head title={$t("video.jellyfin.title") as string} />
    <Empty icon="server" title={$t("video.jellyfin.empty_title") as string} body={$t("video.jellyfin.empty_body") as string}>
      <button class="btn btn-primary" onclick={() => goRoot({ name: "sources" })}><VIcon name="plus" size={14} /> {$t("video.jellyfin.connect")}</button>
    </Empty>
  {:else if !parent}
    <Head title={src.name} subtitle={String(src.extra?.user ?? src.url)}>
      {#snippet actions()}
        {#if jf.length > 1}
          <select class="input" value={src.id} onchange={(e) => goRoot({ name: "jellyfin", source: Number((e.currentTarget as HTMLSelectElement).value) })} aria-label={$t("video.jellyfin.server") as string}>
            {#each jf as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
          </select>
        {/if}
      {/snippet}
    </Head>
    {#if src.status === "down" || !src.enabled}
      <div class="notice notice-danger">
        <VIcon name="alert" size={16} />
        <div class="notice-body">
          <span class="notice-title">{!src.enabled ? $t("video.sources.disabled_title", { name: src.name }) : $t("video.sources.status_down_title", { name: src.name })}</span>
          {#if src.last_error}<span class="notice-text">{src.last_error}</span>{/if}
        </div>
      </div>
    {/if}
    {#if error}
      <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
    {/if}
    {#if !views}
      <Skel kind="row" count={5} min={240} />
      <Skel count={6} />
    {:else}
      {#if resume.length}
        <Shelf title={$t("video.jellyfin.continue") as string} width={260}>
          {#each resume as it (it.id)}
            <MediaCard title={it.name} subtitle={sub(it)} src={it.backdrop ?? it.image} seed={it.series_name ?? it.name} progress={prog(it)} onopen={() => void play(it)} onplay={() => void play(it)} />
          {/each}
        </Shelf>
      {/if}
      {#if nextUp.length}
        <Shelf title={$t("video.jellyfin.next_up") as string} width={260}>
          {#each nextUp as it (it.id)}
            <MediaCard title={it.name} subtitle={sub(it)} src={it.image} seed={it.series_name ?? it.name} onopen={() => void play(it)} onplay={() => void play(it)} />
          {/each}
        </Shelf>
      {/if}
      <section class="sec">
        <h2>{$t("video.jellyfin.libraries")}</h2>
        {#if views.length}
          <div class="grid wide">
            {#each views as lib (lib.id)}
              <MediaCard title={lib.name} src={lib.image} seed={lib.name} icon="folder" onopen={() => go({ name: "jellyfin", source: src!.id, parent: lib.id, title: lib.name })} />
            {/each}
          </div>
        {:else}
          <Empty compact icon="library" title={$t("video.jellyfin.no_libraries") as string} />
        {/if}
      </section>
    {/if}
  {:else}
    <Head title={item?.name ?? pageTitle ?? ""} showBack subtitle={item?.type === "Series" ? [item.year, item.child_count != null ? $t("video.jellyfin.seasons_count", { count: item.child_count }) : null].filter(Boolean).join(" · ") : ""}>
      {#snippet actions()}
        {#if playNextEp}
          <button class="btn btn-primary" onclick={() => void play(playNextEp!)} disabled={starting != null}>
            <VIcon name="play" size={13} /> {$t("video.jellyfin.play_named", { name: playNextEp.name })}
          </button>
        {/if}
      {/snippet}
    </Head>
    {#if error}
      <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
    {/if}

    {#if !item}
      <Skel count={12} ratio="2 / 3" min={160} />
    {:else if item.type === "Series" || item.type === "Season"}
      {#if item.overview}<p class="overview">{item.overview}</p>{/if}
      {#if seasons && seasons.length > 1}
        <div class="segmented segmented-lg seasons" role="tablist" aria-label={$t("video.show.seasons") as string}>
          {#each seasons as s (s.id)}
            <button class="segmented-btn" role="tab" aria-selected={season === s.id} onclick={() => (season = s.id)}>{s.name}</button>
          {/each}
        </div>
      {/if}
      {#if !episodes}
        <Skel kind="list" count={8} />
      {:else if !episodes.length}
        <Empty compact icon="series" title={$t("video.jellyfin.no_episodes") as string} />
      {:else}
        <ol class="eps" role="list">
          {#each episodes as e (e.id)}
            {@const r = prog(e)}
            <li>
              <button class="ep" class:seen={e.played} onclick={() => void play(e)} disabled={starting === e.id}>
                <span class="ep-th">
                  <Thumb src={e.image} seed={item.name} radius="var(--radius-sm)" />
                  {#if r != null}<span class="ep-bar"><span style:width="{r * 100}%"></span></span>{/if}
                </span>
                <span class="ep-no">{e.episode ?? ""}</span>
                <span class="ep-tx">
                  <b>{e.name}</b>
                  <small>{fmtDur(e.runtime_ms)}</small>
                  {#if e.overview}<small class="ov">{e.overview}</small>{/if}
                </span>
                {#if starting === e.id}<span class="spinner"></span>{:else if e.played}<span class="ok"><VIcon name="check-circle" size={16} /></span>{/if}
              </button>
            </li>
          {/each}
        </ol>
      {/if}
    {:else}
      <input class="input input-search search" type="search" placeholder={$t("video.jellyfin.search_in", { name: item.name }) as string} bind:value={q} />
      {#if !items}
        <Skel count={12} ratio="2 / 3" min={160} />
      {:else if !items.length}
        <Empty compact icon="library" title={(qLive ? $t("video.common.no_match") : $t("video.jellyfin.empty_folder")) as string} />
      {:else}
        {@render itemGrid(items)}
        {#if items.length < total}
          <button class="btn more" onclick={() => void more()} disabled={loadingMore}>
            {#if loadingMore}<span class="spinner"></span>{/if}
            {$t("video.jellyfin.load_more", { count: total - items.length })}
          </button>
        {/if}
      {/if}
    {/if}
  {/if}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
    min-width: 0;
  }
  .sec {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: var(--space-5) var(--space-4);
  }
  .grid.wide {
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  }
  .search {
    max-width: 360px;
  }
  .more {
    align-self: center;
  }
  .overview {
    margin: 0;
    max-width: 780px;
    color: var(--text-muted);
    line-height: 1.55;
  }
  .seasons {
    align-self: flex-start;
    max-width: 100%;
    overflow-x: auto;
  }
  .eps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ep {
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
  .ep:hover {
    background: var(--fill-1);
  }
  .ep:focus-visible {
    outline: var(--focus-ring);
  }
  .ep-th {
    position: relative;
    width: 144px;
    flex-shrink: 0;
  }
  .ep-bar {
    position: absolute;
    left: 4px;
    right: 4px;
    bottom: 4px;
    height: 3px;
    border-radius: var(--radius-full);
    background: rgba(0, 0, 0, 0.45);
    overflow: hidden;
  }
  .ep-bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .ep-no {
    width: 28px;
    text-align: right;
    font-weight: 600;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ep-tx {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ep-tx b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .seen .ep-tx b {
    color: var(--text-dim);
  }
  .ep-tx small {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .ov {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .ok {
    color: var(--success);
  }
</style>
