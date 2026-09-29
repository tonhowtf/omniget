<script lang="ts">
  // Add-ons that speak the open Stremio protocol: catalogs of the installed
  // add-ons, a meta page with episodes, and the list of streams. Only direct
  // http links and YouTube play; the rest is shown greyed out with the reason.
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openResolved } from "$lib/video/player.svelte";
  import { go, goRoot, ui } from "$lib/video/ui.svelte";
  import type { AddonCatalog, AddonInfo, AddonMeta, AddonStream } from "$lib/video/types";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import MediaCard from "./parts/MediaCard.svelte";
  import Shelf from "./parts/Shelf.svelte";
  import Skel from "./parts/Skel.svelte";
  import Thumb from "./parts/Thumb.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText } from "./parts/util";

  const MAX_SHELVES = 10;

  let {
    addon: addonProp = undefined,
    type: typeProp = undefined,
    id: idProp = undefined,
  }: { addon?: number | null; type?: string | null; id?: string | null } = $props();

  let v = $derived(ui.view.name === "addons" ? ui.view : null);
  let onlyAddon = $derived(addonProp ?? v?.addon ?? null);
  let metaType = $derived(typeProp ?? v?.type ?? null);
  let metaId = $derived(idProp ?? v?.id ?? null);

  let addons = $state<AddonInfo[] | null>(null);
  let error = $state<string | null>(null);
  let rows = $state<Record<string, AddonMeta[] | null | string>>({});
  let q = $state("");
  let qLive = $state("");
  let results = $state<{ cat: AddonCatalog; metas: AddonMeta[] }[] | null>(null);

  let meta = $state<AddonMeta | null>(null);
  let season = $state<number | null>(null);
  let videoId = $state<string | null>(null);
  let streams = $state<AddonStream[] | null>(null);
  let starting = $state<string | null>(null);

  $effect(() => {
    api
      .addons()
      .then((a) => (addons = a))
      .catch((e) => {
        error = errText(e);
        addons = [];
      });
  });

  let enabled = $derived((addons ?? []).filter((a) => a.enabled && (onlyAddon == null || a.id === onlyAddon)));
  let browseCats = $derived(
    enabled
      .flatMap((a) => a.catalogs.map((c) => ({ ...c, addon: a.id, addonName: a.name })))
      .filter((c) => !c.extra.some((x) => x.required))
      .slice(0, MAX_SHELVES),
  );
  let searchCats = $derived(enabled.flatMap((a) => a.catalogs.map((c) => ({ ...c, addon: a.id }))).filter((c) => c.extra.some((x) => x.name === "search")));

  const rowKey = (c: AddonCatalog) => `${c.addon}:${c.type}:${c.id}`;

  // load each browse shelf once
  $effect(() => {
    if (metaId) return;
    const cats = browseCats;
    untrack(() => {
      for (const c of cats) {
        const k = rowKey(c);
        if (k in rows) continue;
        rows[k] = null;
        api
          .addonCatalog(c.addon, c.type, c.id, [], 0)
          .then((m) => (rows[k] = m))
          .catch((e) => (rows[k] = errText(e)));
      }
    });
  });

  $effect(() => {
    const val = q;
    const h = setTimeout(() => (qLive = val.trim()), 400);
    return () => clearTimeout(h);
  });

  let searchReq = 0;
  $effect(() => {
    const needle = qLive;
    const cats = searchCats;
    if (!needle) {
      results = null;
      return;
    }
    const id = ++searchReq;
    results = null;
    Promise.all(
      cats.map((c) =>
        api
          .addonCatalog(c.addon, c.type, c.id, [["search", needle]], 0)
          .then((metas) => ({ cat: c as AddonCatalog, metas }))
          .catch(() => ({ cat: c as AddonCatalog, metas: [] as AddonMeta[] })),
      ),
    ).then((r) => {
      if (id === searchReq) results = r.filter((x) => x.metas.length);
    });
  });

  // meta page
  $effect(() => {
    const ty = metaType;
    const id = metaId;
    meta = null;
    season = null;
    videoId = null;
    streams = null;
    if (!ty || !id) return;
    api
      .addonMeta(ty, id)
      .then((m) => {
        if (metaId !== id) return;
        meta = m;
        if (m.videos.length) {
          const seasons = [...new Set(m.videos.map((x) => x.season ?? 0))].sort((a, b) => a - b);
          season = seasons.find((s) => s > 0) ?? seasons[0] ?? null;
        } else {
          loadStreams(ty, id);
        }
      })
      .catch((e) => (error = errText(e)));
  });

  let reqStreams = 0;
  function loadStreams(ty: string, id: string) {
    const r = ++reqStreams;
    videoId = id;
    streams = null;
    api
      .addonStreams(ty, id)
      .then((s) => {
        if (r === reqStreams) streams = s;
      })
      .catch((e) => {
        if (r === reqStreams) {
          streams = [];
          error = errText(e);
        }
      });
  }

  let seasonList = $derived(meta ? [...new Set(meta.videos.map((x) => x.season ?? 0))].sort((a, b) => a - b) : []);
  let seasonVideos = $derived(meta ? meta.videos.filter((x) => (x.season ?? 0) === season).sort((a, b) => (a.episode ?? 0) - (b.episode ?? 0)) : []);
  let playable = $derived((streams ?? []).filter((s) => s.playable));
  let external = $derived((streams ?? []).filter((s) => !s.playable && s.kind === "external"));
  let refused = $derived((streams ?? []).filter((s) => !s.playable && s.kind !== "external"));

  async function play(s: AddonStream) {
    if (!meta || !videoId) return;
    const k = `${s.addon}:${s.index}`;
    starting = k;
    const vid = meta.videos.find((x) => x.id === videoId);
    const title = vid ? `${meta.name} · ${vid.title}` : meta.name;
    try {
      const info = await api.addonPlay(s, meta.type, videoId, title);
      await openResolved(info);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      starting = null;
    }
  }

  async function openOutside(url: string) {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(url);
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  function openMeta(m: AddonMeta) {
    go({ name: "addons", addon: onlyAddon, type: m.type, id: m.id });
  }

  function seasonName(n: number): string {
    return n === 0 ? ($t("video.show.specials") as string) : ($t("video.show.season", { number: n }) as string);
  }
</script>

{#snippet metaCard(m: AddonMeta)}
  <MediaCard title={m.name} subtitle={m.release_info ?? ""} src={m.poster} seed={m.name} ratio="2 / 3" onopen={() => openMeta(m)} />
{/snippet}

{#snippet streamRow(s: AddonStream, state: "play" | "external" | "refused")}
  <li class="st" class:dim={state === "refused"}>
    <span class="st-ic" class:yt={s.kind === "youtube"}>
      <VIcon name={state === "refused" ? "ban" : s.kind === "youtube" ? "youtube" : s.kind === "external" ? "external" : "link"} size={16} />
    </span>
    <span class="st-tx">
      <b>{s.name || s.addon_name}</b>
      {#if s.title}<small>{s.title}</small>{/if}
      <small class="st-src">{s.addon_name}</small>
      {#if state === "refused"}<small class="why">{$t("video.addons.refused_reason")}</small>{/if}
      {#if state === "external"}<small class="why">{$t("video.addons.external_reason")}</small>{/if}
    </span>
    {#if state === "play"}
      <button class="btn btn-sm btn-primary" onclick={() => void play(s)} disabled={starting != null}>
        {#if starting === `${s.addon}:${s.index}`}<span class="spinner"></span>{:else}<VIcon name="play" size={12} />{/if}
        {$t("video.common.play")}
      </button>
    {:else if state === "external" && s.url}
      <button class="btn btn-sm" onclick={() => void openOutside(s.url)}><VIcon name="external" size={12} /> {$t("video.addons.open_browser")}</button>
    {/if}
  </li>
{/snippet}

<div class="page">
  {#if metaId && metaType}
    {#if !meta}
      {#if error}
        <Head title="" showBack />
        <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
      {:else}
        <div class="skeleton" style="height: 240px; border-radius: var(--radius-xl)"></div>
        <Skel kind="list" count={5} />
      {/if}
    {:else}
      <div class="hero" style:background-image={meta.background ? `url("${meta.background}")` : "none"}>
        <div class="shade"></div>
        <div class="hero-in">
          <div class="poster"><Thumb src={meta.poster} seed={meta.name} ratio="2 / 3" radius="var(--radius-lg)" /></div>
          <div class="hinfo">
            <Head title={meta.name} showBack subtitle={meta.release_info ?? ""} />
            {#if meta.genres.length}<div class="tags">{#each meta.genres as g (g)}<span class="tag">{g}</span>{/each}</div>{/if}
            {#if meta.description}<p class="desc">{meta.description}</p>{/if}
          </div>
        </div>
      </div>

      {#if error}
        <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
      {/if}

      <div class="body" class:has-eps={meta.videos.length > 0}>
        {#if meta.videos.length}
          <section class="eps-col">
            {#if seasonList.length > 1}
              <div class="segmented seasons" role="tablist" aria-label={$t("video.show.seasons") as string}>
                {#each seasonList as s (s)}
                  <button class="segmented-btn" role="tab" aria-selected={season === s} onclick={() => (season = s)}>{seasonName(s)}</button>
                {/each}
              </div>
            {/if}
            <ol class="eps" role="list">
              {#each seasonVideos as vid (vid.id)}
                <li>
                  <button class="ep" class:sel={videoId === vid.id} onclick={() => loadStreams(meta!.type, vid.id)}>
                    <span class="ep-th"><Thumb src={vid.thumbnail} seed={meta.name} radius="var(--radius-sm)" /></span>
                    <span class="ep-tx">
                      <b>{vid.episode != null ? `${vid.episode}. ` : ""}{vid.title}</b>
                      {#if vid.released}<small>{new Date(vid.released).toLocaleDateString()}</small>{/if}
                    </span>
                    <VIcon name="chevron-right" size={14} />
                  </button>
                </li>
              {/each}
            </ol>
          </section>
        {/if}

        <section class="streams">
          <h2>{$t("video.addons.streams")}</h2>
          {#if !videoId}
            <p class="hint">{$t("video.addons.pick_episode")}</p>
          {:else if !streams}
            <Skel kind="list" count={4} />
          {:else if !streams.length}
            <Empty compact icon="addons" title={$t("video.addons.no_streams") as string} body={$t("video.addons.no_streams_body") as string} />
          {:else}
            {#if !playable.length}
              <div class="notice notice-info"><VIcon name="info" size={16} /><div class="notice-body">{$t("video.addons.none_playable")}</div></div>
            {/if}
            <ul class="sts" role="list">
              {#each playable as s (`${s.addon}:${s.index}`)}{@render streamRow(s, "play")}{/each}
              {#each external as s (`${s.addon}:${s.index}`)}{@render streamRow(s, "external")}{/each}
            </ul>
            {#if refused.length}
              <details class="refused">
                <summary>{$t("video.addons.refused_count", { count: refused.length })}</summary>
                <ul class="sts" role="list">
                  {#each refused as s (`${s.addon}:${s.index}`)}{@render streamRow(s, "refused")}{/each}
                </ul>
              </details>
            {/if}
          {/if}
        </section>
      </div>
    {/if}
  {:else}
    <Head title={$t("video.addons.title") as string} showBack={onlyAddon != null} subtitle={onlyAddon != null ? (enabled[0]?.name ?? "") : ""}>
      {#snippet actions()}
        <button class="btn btn-sm btn-ghost" onclick={() => goRoot({ name: "sources" })}><VIcon name="settings" size={13} /> {$t("video.addons.manage")}</button>
      {/snippet}
    </Head>

    {#if error}
      <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
    {/if}

    {#if !addons}
      <Skel kind="row" count={6} min={150} ratio="2 / 3" />
    {:else if !enabled.length}
      <Empty icon="addons" title={$t("video.addons.empty_title") as string} body={$t("video.addons.empty_body") as string}>
        <button class="btn btn-primary" onclick={() => goRoot({ name: "sources" })}><VIcon name="plus" size={14} /> {$t("video.addons.install")}</button>
      </Empty>
    {:else}
      {#if searchCats.length}
        <input class="input input-search input-lg search" type="search" placeholder={$t("video.addons.search") as string} bind:value={q} />
      {/if}
      {#if qLive}
        {#if !results}
          <Skel count={8} ratio="2 / 3" min={150} />
        {:else if !results.length}
          <Empty compact icon="addons" title={$t("video.common.no_match") as string} />
        {:else}
          {#each results as r (rowKey(r.cat))}
            <Shelf title={r.cat.name} width={160}>
              {#each r.metas as m (m.id)}{@render metaCard(m)}{/each}
            </Shelf>
          {/each}
        {/if}
      {:else if !browseCats.length}
        <Empty compact icon="addons" title={$t("video.addons.no_catalogs") as string} body={$t("video.addons.no_catalogs_body") as string} />
      {:else}
        {#each browseCats as c (rowKey(c))}
          {@const row = rows[rowKey(c)]}
          {#if row === null || row === undefined}
            <section class="sk"><h2>{c.name}</h2><Skel kind="row" count={7} min={160} ratio="2 / 3" /></section>
          {:else if typeof row === "string"}
            <section class="sk">
              <h2>{c.name}</h2>
              <p class="hint err">{$t("video.addons.catalog_failed", { name: c.addonName })} {row}</p>
            </section>
          {:else if row.length}
            <Shelf title={`${c.name} · ${c.addonName}`} width={160}>
              {#each row as m (m.id)}{@render metaCard(m)}{/each}
            </Shelf>
          {/if}
        {/each}
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
  .search {
    max-width: 480px;
  }
  .sk {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .hint {
    margin: 0;
    color: var(--text-dim);
  }
  .err {
    color: var(--danger);
  }
  .hero {
    position: relative;
    border-radius: var(--radius-xl);
    background: var(--fill-1) center / cover;
    overflow: hidden;
  }
  .shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(90deg, var(--bg) 20%, color-mix(in srgb, var(--bg) 70%, transparent) 60%, color-mix(in srgb, var(--bg) 30%, transparent));
  }
  .hero-in {
    position: relative;
    display: flex;
    gap: var(--space-5);
    padding: var(--space-5);
  }
  .poster {
    width: 160px;
    flex-shrink: 0;
    box-shadow: var(--elev-3);
    border-radius: var(--radius-lg);
  }
  .hinfo {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    max-width: 720px;
  }
  .tags {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .desc {
    margin: 0;
    line-height: 1.55;
    color: var(--text-muted);
    display: -webkit-box;
    -webkit-line-clamp: 6;
    line-clamp: 6;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .body {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-5);
  }
  .body.has-eps {
    grid-template-columns: minmax(280px, 1fr) minmax(320px, 1fr);
    align-items: start;
  }
  @media (max-width: 860px) {
    .body.has-eps {
      grid-template-columns: 1fr;
    }
    .hero-in {
      flex-direction: column;
    }
  }
  .eps-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
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
  .ep.sel {
    background: var(--accent-soft);
  }
  .ep:focus-visible {
    outline: var(--focus-ring);
  }
  .ep-th {
    width: 104px;
    flex-shrink: 0;
  }
  .ep-tx {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .ep-tx b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ep-tx small {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .streams {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .sts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .st {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .st.dim {
    opacity: 0.6;
  }
  .st-ic {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text-muted);
  }
  .st-ic.yt {
    color: var(--red);
  }
  .st-tx {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .st-tx b {
    font-weight: 600;
    white-space: pre-line;
  }
  .st-tx small {
    color: var(--text-dim);
    font-size: var(--text-sm);
    white-space: pre-line;
  }
  .st-src {
    font-size: var(--text-xs) !important;
  }
  .why {
    color: var(--text-muted) !important;
    font-style: italic;
  }
  .refused summary {
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--text-sm);
    margin-bottom: var(--space-2);
  }
</style>
