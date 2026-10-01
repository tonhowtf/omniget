<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { openResolved } from "$lib/video/player.svelte";
  import { goRoot, ui } from "$lib/video/ui.svelte";
  import type { EpgProgramme, IptvChannel, SourceInfo } from "$lib/video/types";
  import Empty from "./parts/Empty.svelte";
  import Head from "./parts/Head.svelte";
  import Skel from "./parts/Skel.svelte";
  import Thumb from "./parts/Thumb.svelte";
  import VIcon from "./parts/VIcon.svelte";
  import { errText, fmtHour, lsGet, lsSet } from "./parts/util";

  const FAV = "\u0000fav";
  const PAGE = 200;
  const GUIDE_ROWS = 80;
  const PX_PER_MIN = 4;
  const GUIDE_HOURS = 6;

  type Last = { id: number; name: string; logo: string | null };

  let { group: groupProp = undefined }: { group?: string | null } = $props();

  let sources = $state<SourceInfo[] | null>(null);
  let groups = $state<{ name: string; count: number }[]>([]);
  let channels = $state<IptvChannel[] | null>(null);
  let error = $state<string | null>(null);
  let group = $state<string | null>(null);
  let q = $state("");
  let qLive = $state("");
  let tab = $state<"channels" | "guide">(lsGet<"channels" | "guide">("omniget.video.live.tab", "channels"));
  let limit = $state(PAGE);
  let last = $state<Last | null>(lsGet<Last | null>("omniget.video.live.last", null));
  let now = $state(Date.now());
  let starting = $state<number | null>(null);
  let refreshing = $state(false);

  let epg = $state<Record<number, EpgProgramme[]>>({});
  let epgLoading = $state(false);
  let guideEl: HTMLDivElement | undefined = $state();

  let iptv = $derived((sources ?? []).filter((s) => s.kind === "iptv"));
  let live = $derived(iptv.filter((s) => s.enabled));

  async function loadSources() {
    try {
      sources = await api.sources();
      if (live.length) groups = await api.iptvGroups();
      error = null;
    } catch (e) {
      error = errText(e);
      sources ??= [];
    }
  }

  let reqId = 0;
  async function loadChannels() {
    const id = ++reqId;
    const g = group;
    try {
      const list = await api.iptvChannels({
        group: g && g !== FAV ? g : null,
        search: qLive.trim() || null,
        favorites: g === FAV,
      });
      if (id !== reqId) return;
      channels = list;
      limit = PAGE;
    } catch (e) {
      if (id === reqId) {
        error = errText(e);
        channels = [];
      }
    }
  }

  onMount(() => {
    group = groupProp ?? (ui.view.name === "live" ? (ui.view.group ?? null) : null);
    void loadSources();
    const tick = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(tick);
  });

  $effect(() => {
    const v = q;
    const h = setTimeout(() => (qLive = v), 250);
    return () => clearTimeout(h);
  });

  $effect(() => {
    void group;
    void qLive;
    if (sources && live.length) void loadChannels();
  });

  $effect(() => lsSet("omniget.video.live.tab", tab));

  async function play(ch: { id: number; name: string; logo: string | null }) {
    starting = ch.id;
    try {
      const info = await api.iptvPlay(ch.id);
      last = { id: ch.id, name: ch.name, logo: ch.logo };
      lsSet("omniget.video.live.last", last);
      await openResolved(info);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      starting = null;
    }
  }

  async function toggleFav(ch: IptvChannel) {
    const next = !ch.favorite;
    ch.favorite = next;
    try {
      await api.iptvFavorite(ch.id, next);
      if (!next && group === FAV && channels) channels = channels.filter((c) => c.id !== ch.id);
    } catch (e) {
      ch.favorite = !next;
      showToast("error", errText(e));
    }
  }

  async function refresh() {
    refreshing = true;
    try {
      await Promise.all(live.map((s) => api.iptvRefresh(s.id)));
      await loadSources();
      await loadChannels();
      if (tab === "guide") void loadEpg();
      showToast("success", $t("video.live.refreshed") as string);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      refreshing = false;
    }
  }

  function share(p: EpgProgramme | null): number {
    if (!p || p.stop <= p.start) return 0;
    return Math.min(1, Math.max(0, (now - p.start) / (p.stop - p.start)));
  }

  let winStart = $derived(Math.floor((now - 30 * 60_000) / (30 * 60_000)) * 30 * 60_000);
  let winEnd = $derived(winStart + GUIDE_HOURS * 3_600_000);
  let guideRows = $derived((channels ?? []).slice(0, GUIDE_ROWS));
  let slots = $derived(Array.from({ length: GUIDE_HOURS * 2 }, (_, i) => winStart + i * 30 * 60_000));
  let nowX = $derived(((now - winStart) / 60_000) * PX_PER_MIN);

  async function loadEpg() {
    const ids = guideRows.map((c) => c.id);
    if (!ids.length) {
      epg = {};
      return;
    }
    epgLoading = true;
    try {
      const list = await api.iptvEpg(ids, winStart, winEnd);
      const map: Record<number, EpgProgramme[]> = {};
      for (const p of list) (map[p.channel] ??= []).push(p);
      for (const k in map) map[k].sort((a, b) => a.start - b.start);
      epg = map;
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      epgLoading = false;
    }
  }

  $effect(() => {
    if (tab !== "guide") return;
    void guideRows;
    void winStart;
    void loadEpg();
  });

  let scrolled = false;
  $effect(() => {
    if (tab !== "guide" || !guideEl) {
      scrolled = false;
      return;
    }
    if (!scrolled) {
      guideEl.scrollLeft = Math.max(0, nowX - 120);
      scrolled = true;
    }
  });

  function blockStyle(p: EpgProgramme): string {
    const s = Math.max(p.start, winStart);
    const e = Math.min(p.stop, winEnd);
    const left = ((s - winStart) / 60_000) * PX_PER_MIN;
    const width = Math.max(4, ((e - s) / 60_000) * PX_PER_MIN - 2);
    return `left:${left}px;width:${width}px`;
  }
</script>

<div class="page">
  <Head title={$t("video.live.title") as string}>
    {#snippet actions()}
      {#if live.length}
        {#if last}
          <button class="btn btn-sm btn-tinted" onclick={() => void play(last!)} disabled={starting === last.id}>
            <VIcon name="play" size={12} /> {$t("video.live.last_channel", { name: last.name })}
          </button>
        {/if}
        <button class="btn btn-sm btn-ghost" onclick={() => void refresh()} disabled={refreshing}>
          <VIcon name="refresh" size={13} spin={refreshing} /> {$t("video.live.refresh")}
        </button>
      {/if}
    {/snippet}
  </Head>

  {#if error}
    <div class="notice notice-danger"><VIcon name="alert" size={16} /><div class="notice-body">{error}</div></div>
  {/if}

  {#if !sources}
    <Skel kind="list" count={10} />
  {:else if !iptv.length}
    <Empty icon="live" title={$t("video.live.empty_title") as string} body={$t("video.live.empty_body") as string}>
      <button class="btn btn-primary" onclick={() => goRoot({ name: "sources" })}><VIcon name="plus" size={14} /> {$t("video.live.add_list")}</button>
    </Empty>
  {:else if !live.length}
    <Empty icon="live" title={$t("video.live.all_off_title") as string} body={$t("video.live.all_off_body") as string}>
      <button class="btn" onclick={() => goRoot({ name: "sources" })}>{$t("video.live.open_sources")}</button>
    </Empty>
  {:else}
    {#each live.filter((s) => s.status === "down" || s.status === "slow") as s (s.id)}
      <div class="notice" class:notice-danger={s.status === "down"} class:notice-warning={s.status === "slow"}>
        <VIcon name="alert" size={16} />
        <div class="notice-body">
          <span class="notice-title">{s.status === "down" ? $t("video.sources.status_down_title", { name: s.name }) : $t("video.sources.status_slow_title", { name: s.name })}</span>
          {#if s.last_error}<span class="notice-text">{s.last_error}</span>{/if}
        </div>
      </div>
    {/each}

    <div class="split">
      <nav class="groups" aria-label={$t("video.live.groups") as string}>
        <button class="g" class:sel={group === null} onclick={() => (group = null)}><VIcon name="tv" size={14} /><span>{$t("video.live.all_channels")}</span></button>
        <button class="g" class:sel={group === FAV} onclick={() => (group = FAV)}><VIcon name="star" size={14} /><span>{$t("video.live.favorites")}</span></button>
        <div class="gsep"></div>
        {#each groups as g (g.name)}
          <button class="g" class:sel={group === g.name} onclick={() => (group = g.name)} title={g.name}>
            <span class="gn">{g.name || $t("video.live.no_group")}</span><small>{g.count}</small>
          </button>
        {/each}
      </nav>

      <section class="main">
        <div class="bar">
          <input class="input input-search" type="search" placeholder={$t("video.live.search") as string} bind:value={q} />
          <div class="segmented" role="tablist" aria-label={$t("video.live.view") as string}>
            <button class="segmented-btn" role="tab" aria-selected={tab === "channels"} onclick={() => (tab = "channels")}><VIcon name="list" size={13} /> {$t("video.live.channels")}</button>
            <button class="segmented-btn" role="tab" aria-selected={tab === "guide"} onclick={() => (tab = "guide")}><VIcon name="clock" size={13} /> {$t("video.live.guide")}</button>
          </div>
        </div>

        {#if !channels}
          <Skel kind="list" count={10} />
        {:else if !channels.length}
          <Empty compact icon="live" title={(group === FAV ? $t("video.live.no_favorites") : $t("video.common.no_match")) as string} body={group === FAV ? ($t("video.live.no_favorites_body") as string) : ""} />
        {:else if tab === "channels"}
          <ul class="chs" role="list">
            {#each channels.slice(0, limit) as ch (ch.id)}
              {@const pct = share(ch.now)}
              <li class="ch" class:cur={last?.id === ch.id}>
                <button class="ch-open" onclick={() => void play(ch)} disabled={starting === ch.id}>
                  <span class="logo"><Thumb src={ch.logo} seed={ch.name} ratio="1" icon="tv" fit="contain" radius="var(--radius-sm)" /></span>
                  <span class="ch-tx">
                    <b>{ch.name}</b>
                    {#if ch.now}
                      <span class="now">
                        <span class="live-dot" aria-hidden="true"></span>
                        <span class="nt">{ch.now.title}</span>
                        <small>{fmtHour(ch.now.start)}–{fmtHour(ch.now.stop)}</small>
                      </span>
                      <span class="progress thin"><span class="progress-fill" style:width="{pct * 100}%"></span></span>
                    {:else}
                      <small class="dim">{$t("video.live.no_guide")}</small>
                    {/if}
                    {#if ch.next}
                      <small class="next">{$t("video.live.next", { time: fmtHour(ch.next.start), title: ch.next.title })}</small>
                    {/if}
                  </span>
                  {#if starting === ch.id}<span class="spinner"></span>{/if}
                </button>
                <button class="fav" class:on={ch.favorite} onclick={() => void toggleFav(ch)} aria-pressed={ch.favorite} aria-label={(ch.favorite ? $t("video.live.unfavorite") : $t("video.live.favorite")) as string}>
                  <VIcon name={ch.favorite ? "star-fill" : "star"} size={15} />
                </button>
              </li>
            {/each}
          </ul>
          {#if channels.length > limit}
            <button class="btn more" onclick={() => (limit += PAGE)}>{$t("video.live.show_more", { count: channels.length - limit })}</button>
          {/if}
        {:else}
          {#if channels.length > GUIDE_ROWS}
            <p class="hint">{$t("video.live.guide_limited", { count: GUIDE_ROWS })}</p>
          {/if}
          <div class="guide" bind:this={guideEl} style:--px="{PX_PER_MIN}px" aria-busy={epgLoading}>
            <div class="gtable" style:width="calc(var(--col) + {GUIDE_HOURS * 60 * PX_PER_MIN}px)">
              <div class="ghead">
                <div class="gcorner">{$t("video.live.channel")}</div>
                <div class="gslots">
                  {#each slots as s (s)}
                    <div class="gslot" style:width="{30 * PX_PER_MIN}px">{fmtHour(s)}</div>
                  {/each}
                  {#if nowX >= 0}
                    <div class="gnow-tag" style:left="{nowX}px">{$t("video.live.now")} {fmtHour(now)}</div>
                  {/if}
                </div>
              </div>
              {#each guideRows as ch (ch.id)}
                <div class="grow">
                  <button class="gch" onclick={() => void play(ch)} title={ch.name}>
                    <span class="glogo"><Thumb src={ch.logo} seed={ch.name} ratio="1" icon="tv" fit="contain" radius="6px" /></span>
                    <span class="gchn">{ch.name}</span>
                  </button>
                  <div class="glane">
                    {#each epg[ch.id] ?? [] as p (p.start)}
                      {@const onAir = p.start <= now && p.stop > now}
                      <button
                        class="gprog"
                        class:air={onAir}
                        class:past={p.stop <= now}
                        style={blockStyle(p)}
                        title={`${p.title} · ${fmtHour(p.start)}–${fmtHour(p.stop)}${p.desc ? `\n${p.desc}` : ""}`}
                        onclick={() => (onAir ? void play(ch) : undefined)}
                        disabled={!onAir}
                      >
                        <b>{p.title}</b>
                        <small>{fmtHour(p.start)}</small>
                      </button>
                    {:else}
                      {#if !epgLoading}<span class="gnone">{$t("video.live.no_guide")}</span>{/if}
                    {/each}
                  </div>
                </div>
              {/each}
              {#if nowX >= 0}<div class="gnow" style:left="calc(var(--col) + {nowX}px)" aria-hidden="true"></div>{/if}
            </div>
          </div>
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
    min-width: 0;
  }
  .split {
    display: grid;
    grid-template-columns: 220px 1fr;
    gap: var(--space-4);
    align-items: start;
    min-width: 0;
  }
  @media (max-width: 760px) {
    .split {
      grid-template-columns: 1fr;
    }
    .groups {
      position: static !important;
      max-height: 180px !important;
    }
  }
  .groups {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
    max-height: calc(100vh - 160px);
    overflow-y: auto;
    padding: var(--space-1);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .g {
    all: unset;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 28px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .g:hover {
    background: var(--fill-1);
  }
  .g.sel {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 600;
  }
  .g:focus-visible {
    outline: var(--focus-ring);
  }
  .g span,
  .gn {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .g small {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .gsep {
    height: var(--hairline);
    margin: 4px 6px;
    background: var(--separator);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .bar .input-search {
    flex: 1;
    min-width: 180px;
    max-width: 360px;
  }
  .chs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ch {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    border-radius: var(--radius-md);
  }
  .ch:hover {
    background: var(--fill-1);
  }
  .ch.cur {
    background: var(--accent-soft);
  }
  .ch-open {
    all: unset;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 6px 8px;
    cursor: pointer;
  }
  .ch-open:focus-visible {
    outline: var(--focus-ring);
    border-radius: var(--radius-md);
  }
  .logo {
    width: 44px;
    flex-shrink: 0;
  }
  .logo :global(.th) {
    background: var(--fill-1);
  }
  .ch-tx {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ch-tx b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .now {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    min-width: 0;
  }
  .nt {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .now small,
  .next,
  .dim {
    color: var(--text-dim);
    font-size: var(--text-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .live-dot {
    width: 6px;
    height: 6px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--danger);
  }
  .thin {
    height: 3px;
    max-width: 280px;
  }
  .fav {
    all: unset;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    margin-right: 6px;
    border-radius: 50%;
    color: var(--text-dim);
    cursor: pointer;
  }
  .fav:hover {
    background: var(--fill-2);
  }
  .fav.on {
    color: var(--warning);
  }
  .fav:focus-visible {
    outline: var(--focus-ring);
  }
  .more {
    align-self: center;
  }
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .guide {
    --col: 180px;
    position: relative;
    overflow: auto;
    max-height: calc(100vh - 220px);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .gtable {
    position: relative;
  }
  .ghead {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    height: 34px;
    background: var(--surface);
    border-bottom: var(--hairline) solid var(--separator);
  }
  .gcorner {
    position: sticky;
    left: 0;
    z-index: 4;
    width: var(--col);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 var(--space-3);
    font-size: var(--text-xs);
    font-weight: 650;
    color: var(--text-dim);
    background: var(--surface);
    border-right: var(--hairline) solid var(--separator);
  }
  .gslots {
    position: relative;
    display: flex;
  }
  .gslot {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding-left: 6px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    border-left: var(--hairline) solid var(--separator);
    font-variant-numeric: tabular-nums;
  }
  .gnow-tag {
    position: absolute;
    top: 6px;
    transform: translateX(-50%);
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    background: var(--danger);
    color: white;
    font-size: var(--text-xs);
    font-weight: 650;
    white-space: nowrap;
  }
  .grow {
    display: flex;
    height: 52px;
    border-bottom: var(--hairline) solid var(--separator);
  }
  .gch {
    all: unset;
    position: sticky;
    left: 0;
    z-index: 2;
    box-sizing: border-box;
    width: var(--col);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-2);
    background: var(--surface);
    border-right: var(--hairline) solid var(--separator);
    cursor: pointer;
  }
  .gch:hover {
    background: var(--surface-hi);
  }
  .gch:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }
  .glogo {
    width: 30px;
    flex-shrink: 0;
  }
  .gchn {
    font-size: var(--text-sm);
    font-weight: 550;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .glane {
    position: relative;
    flex: 1;
  }
  .gprog {
    all: unset;
    box-sizing: border-box;
    position: absolute;
    top: 5px;
    bottom: 5px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 0 8px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
    cursor: default;
  }
  .gprog.air {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
    cursor: pointer;
  }
  .gprog.air:hover {
    background: color-mix(in srgb, var(--accent) 24%, transparent);
  }
  .gprog.past {
    opacity: 0.5;
  }
  .gprog:focus-visible {
    outline: var(--focus-ring);
  }
  .gprog b {
    font-size: var(--text-sm);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .gprog small {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .gnone {
    position: absolute;
    left: 10px;
    top: 50%;
    transform: translateY(-50%);
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .gnow {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: 1;
    width: 2px;
    background: var(--danger);
    pointer-events: none;
  }
</style>
