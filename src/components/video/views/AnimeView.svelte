<script lang="ts">
  // AniList: search, the user's list by status, and a detail page with the
  // episodes where progress is marked. Skipping openings (AniSkip) lives in
  // the player.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { back, go, goRoot, ui } from "$lib/video/ui.svelte";
  import type { AniMedia, AniViewer } from "$lib/video/types";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import MediaCard from "./parts/MediaCard.svelte";
  import Skel from "./parts/Skel.svelte";
  import Thumb from "./parts/Thumb.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, lsGet, lsSet, stripHtml } from "./parts/util";

  const STATUSES = ["CURRENT", "PLANNING", "COMPLETED", "PAUSED", "DROPPED", "REPEATING"] as const;

  let { id: idProp = undefined }: { id?: number | null } = $props();
  let detailId = $derived(idProp ?? (ui.view.name === "anime" ? (ui.view.id ?? null) : null));

  let viewer = $state<AniViewer | null | undefined>(undefined);
  let tab = $state<"list" | "search">("list");
  let status = $state<string>(lsGet<string>("omniget.video.anime.status", "CURRENT"));
  let list = $state<AniMedia[] | null>(null);
  let q = $state("");
  let results = $state<AniMedia[] | null>(null);
  let searching = $state(false);
  let error = $state<string | null>(null);

  let media = $state<AniMedia | null>(null);
  let saving = $state<number | null>(null);

  onMount(async () => {
    try {
      viewer = await api.anilistViewer();
    } catch (e) {
      viewer = null;
      error = errText(e);
    }
    if (!viewer) tab = "search";
  });

  async function loadList(s: string) {
    list = null;
    try {
      list = await api.anilistList(s);
      error = null;
    } catch (e) {
      error = errText(e);
      list = [];
    }
  }

  $effect(() => {
    const s = status;
    lsSet("omniget.video.anime.status", s);
    if (viewer && tab === "list" && detailId == null) void loadList(s);
  });

  let searchReq = 0;
  $effect(() => {
    const needle = q.trim();
    if (!needle) {
      results = null;
      return;
    }
    const h = setTimeout(async () => {
      const id = ++searchReq;
      searching = true;
      try {
        const r = await api.anilistSearch(needle);
        if (id === searchReq) results = r;
      } catch (e) {
        if (id === searchReq) error = errText(e);
      } finally {
        if (id === searchReq) searching = false;
      }
    }, 350);
    return () => clearTimeout(h);
  });

  $effect(() => {
    const id = detailId;
    media = null;
    if (id == null) return;
    api
      .anilistMedia(id)
      .then((m) => {
        if (detailId === id) media = m;
      })
      .catch((e) => (error = errText(e)));
  });

  async function setProgress(ep: number) {
    if (!media) return;
    saving = ep;
    try {
      media = await api.anilistProgress(media.id, ep);
      showToast("success", $t("video.anime.progress_saved", { episode: ep }) as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      saving = null;
    }
  }

  function subtitleOf(m: AniMedia): string {
    const parts: string[] = [];
    if (m.list && m.episodes) parts.push($t("video.anime.progress_of", { done: m.list.progress, total: m.episodes }) as string);
    else if (m.list) parts.push($t("video.anime.progress_only", { done: m.list.progress }) as string);
    else if (m.format) parts.push(m.format.replace(/_/g, " "));
    if (m.season_year) parts.push(String(m.season_year));
    return parts.join(" · ");
  }

  let epCount = $derived(media ? (media.episodes ?? (media.next_airing ? media.next_airing.episode - 1 : 0)) : 0);
  let done = $derived(media?.list?.progress ?? 0);

  function airingIn(ts: number): string {
    const s = ts * 1000 - Date.now();
    if (s <= 0) return "";
    const d = Math.floor(s / 86_400_000);
    const h = Math.floor((s % 86_400_000) / 3_600_000);
    return d ? `${d}d ${h}h` : `${h}h`;
  }
</script>

{#snippet grid(items: AniMedia[])}
  <div class="grid">
    {#each items as m (m.id)}
      <MediaCard
        title={m.title}
        subtitle={subtitleOf(m)}
        src={m.cover}
        seed={m.title}
        ratio="2 / 3"
        icon="film"
        progress={m.list && m.episodes ? m.list.progress / m.episodes : null}
        watched={m.list?.status === "COMPLETED"}
        onopen={() => go({ name: "anime", id: m.id })}
      />
    {/each}
  </div>
{/snippet}

{#if detailId != null}
  <div class="page">
    {#if !media}
      {#if error}
        <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
      {:else}
        <div class="skeleton" style="height: 220px; border-radius: var(--radius-xl)"></div>
        <Skel kind="list" count={4} />
      {/if}
    {:else}
      <div class="hero" style:background-image={media.banner ? `url("${media.banner}")` : "none"}>
        <div class="hero-shade"></div>
        <button class="hero-back" onclick={() => (ui.stack.length ? back() : goRoot({ name: "anime" }))} aria-label={$t("video.common.back") as string}><VIcon name="back" size={16} /></button>
      </div>
      <div class="dtop">
        <div class="dcover"><Thumb src={media.cover} seed={media.title} ratio="2 / 3" radius="var(--radius-lg)" /></div>
        <div class="dinfo">
          <h1>{media.title}</h1>
          {#if media.title_native}<p class="native">{media.title_native}</p>{/if}
          <p class="facts">
            {[media.format?.replace(/_/g, " "), media.season_year, media.episodes ? $t("video.anime.episodes", { count: media.episodes }) : null, media.status?.replace(/_/g, " ").toLowerCase()].filter(Boolean).join(" · ")}
          </p>
          {#if media.genres.length}
            <div class="tags">{#each media.genres as g (g)}<span class="tag">{g}</span>{/each}</div>
          {/if}
          {#if media.next_airing}
            <p class="airing"><VIcon name="clock" size={13} /> {$t("video.anime.next_airing", { episode: media.next_airing.episode, time: airingIn(media.next_airing.airing_at) })}</p>
          {/if}
          <div class="dacts">
            {#if viewer}
              <button class="btn btn-primary" disabled={saving != null || (!!media.episodes && done >= media.episodes)} onclick={() => void setProgress(done + 1)}>
                <VIcon name="check" size={13} /> {$t("video.anime.mark_episode", { episode: done + 1 })}
              </button>
            {/if}
            <button class="btn" onclick={() => go({ name: "search", q: media!.title })}><VIcon name="search" size={13} /> {$t("video.anime.find_in_library")}</button>
          </div>
        </div>
      </div>

      {#if media.description}
        <p class="desc">{stripHtml(media.description)}</p>
      {/if}

      <section class="eps-sec">
        <h2>{$t("video.anime.episodes_title")}</h2>
        {#if !viewer}
          <div class="notice notice-info">
            <VIcon name="info" size={16} />
            <div class="notice-body">
              {$t("video.anime.connect_to_track")}
              <button class="btn btn-sm btn-tinted" onclick={() => goRoot({ name: "sources" })}>{$t("video.anime.connect")}</button>
            </div>
          </div>
        {/if}
        {#if epCount > 0}
          <div class="eps" role="group" aria-label={$t("video.anime.episodes_title") as string}>
            {#each Array.from({ length: epCount }, (_, i) => i + 1) as ep (ep)}
              <button
                class="ep"
                class:seen={ep <= done}
                aria-pressed={ep <= done}
                disabled={!viewer || saving != null}
                title={(ep <= done ? $t("video.anime.watched_up_to", { episode: ep }) : $t("video.anime.mark_up_to", { episode: ep })) as string}
                onclick={() => void setProgress(ep)}
              >
                {#if saving === ep}<span class="spinner"></span>{:else}{ep}{/if}
              </button>
            {/each}
          </div>
        {:else}
          <p class="dim">{$t("video.anime.no_episodes")}</p>
        {/if}
      </section>
    {/if}
  </div>
{:else}
  <div class="page">
    <Head title={$t("video.anime.title") as string} subtitle={viewer ? ($t("video.anime.signed_in", { name: viewer.name }) as string) : ""}>
      {#snippet actions()}
        <div class="segmented" role="tablist" aria-label={$t("video.anime.title") as string}>
          <button class="segmented-btn" role="tab" aria-selected={tab === "list"} onclick={() => (tab = "list")}>{$t("video.anime.my_list")}</button>
          <button class="segmented-btn" role="tab" aria-selected={tab === "search"} onclick={() => (tab = "search")}>{$t("video.anime.search_tab")}</button>
        </div>
      {/snippet}
    </Head>

    {#if error}
      <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
    {/if}

    {#if tab === "list"}
      {#if viewer === undefined}
        <Skel count={10} ratio="2 / 3" min={150} />
      {:else if !viewer}
        <Empty icon="anime" title={$t("video.anime.connect_title") as string} body={$t("video.anime.connect_body") as string}>
          <button class="btn btn-primary" onclick={() => goRoot({ name: "sources" })}><VIcon name="key" size={14} /> {$t("video.anime.connect")}</button>
          <button class="btn" onclick={() => (tab = "search")}><VIcon name="search" size={14} /> {$t("video.anime.search_tab")}</button>
        </Empty>
      {:else}
        <div class="segmented statuses" role="tablist" aria-label={$t("video.anime.status") as string}>
          {#each STATUSES as s (s)}
            <button class="segmented-btn" role="tab" aria-selected={status === s} onclick={() => (status = s)}>{$t(`video.anime.status_${s.toLowerCase()}`)}</button>
          {/each}
        </div>
        {#if !list}
          <Skel count={10} ratio="2 / 3" min={150} />
        {:else if !list.length}
          <Empty compact icon="anime" title={$t("video.anime.list_empty") as string} />
        {:else}
          {@render grid(list)}
        {/if}
      {/if}
    {:else}
      <input class="input input-search input-lg search" type="search" placeholder={$t("video.anime.search_placeholder") as string} bind:value={q} />
      {#if searching && !results}
        <Skel count={10} ratio="2 / 3" min={150} />
      {:else if results && !results.length}
        <Empty compact icon="anime" title={$t("video.common.no_match") as string} />
      {:else if results}
        {@render grid(results)}
      {:else}
        <Empty compact icon="anime" title={$t("video.anime.search_hint") as string} />
      {/if}
    {/if}
  </div>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
    min-width: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-5) var(--space-4);
  }
  .statuses {
    align-self: flex-start;
    max-width: 100%;
    overflow-x: auto;
  }
  .search {
    max-width: 480px;
  }
  .hero {
    position: relative;
    height: 200px;
    border-radius: var(--radius-xl);
    background: var(--fill-2) center / cover;
    overflow: hidden;
  }
  .hero-shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(180deg, rgba(0, 0, 0, 0.1), rgba(0, 0, 0, 0.55));
  }
  .hero-back {
    all: unset;
    position: absolute;
    top: 12px;
    left: 12px;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.5);
    color: white;
    cursor: pointer;
  }
  .hero-back:focus-visible {
    outline: var(--focus-ring);
  }
  .dtop {
    display: flex;
    gap: var(--space-5);
    margin-top: -110px;
    padding: 0 var(--space-5);
    position: relative;
  }
  .dcover {
    width: 170px;
    flex-shrink: 0;
    box-shadow: var(--elev-3);
    border-radius: var(--radius-lg);
  }
  .dinfo {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: 118px;
    min-width: 0;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
    letter-spacing: var(--track-tight);
  }
  .native,
  .facts {
    margin: 0;
    color: var(--text-dim);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .airing {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--accent-text);
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .dacts {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    margin-top: var(--space-1);
  }
  .desc {
    margin: 0;
    max-width: 780px;
    line-height: 1.55;
    color: var(--text-muted);
    white-space: pre-line;
  }
  .eps-sec {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .eps {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(52px, 1fr));
    gap: var(--space-2);
  }
  .ep {
    all: unset;
    display: grid;
    place-items: center;
    height: 40px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    transition: background var(--duration-fast);
  }
  .ep:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .ep.seen {
    background: var(--accent);
    color: var(--on-accent);
  }
  .ep:disabled {
    cursor: default;
  }
  .ep:focus-visible {
    outline: var(--focus-ring);
  }
  .dim {
    color: var(--text-dim);
    margin: 0;
  }
  @media (max-width: 640px) {
    .dtop {
      flex-direction: column;
      padding: 0;
      margin-top: -60px;
    }
    .dinfo {
      padding-top: 0;
    }
    .dcover {
      width: 120px;
    }
  }
</style>
