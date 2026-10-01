<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { ChannelSort, ChannelTabName } from "$lib/tube/api";
  import { bestAvatar, thumbUrl } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { ChannelPage } from "$lib/tube/types";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import TIcon from "../../TIcon.svelte";
  import ChipBar from "../../cards/ChipBar.svelte";
  import Grid from "../../cards/Grid.svelte";
  import ItemMenu from "../../cards/ItemMenu.svelte";
  import Sentinel from "../../cards/Sentinel.svelte";
  import Shelf from "../../cards/Shelf.svelte";
  import SkeletonGrid from "../../cards/SkeletonGrid.svelte";
  import State from "../../cards/State.svelte";
  import SubscribeButton from "../../cards/SubscribeButton.svelte";
  import Tabs from "../../cards/Tabs.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import { mergePages } from "../../cards/pager.svelte";
  import { errText } from "../../cards/util";

  let { route }: { route: Extract<Route, { name: "channel" }> } = $props();

  const KNOWN: ChannelTabName[] = ["home", "videos", "shorts", "streams", "playlists", "podcasts", "releases"];
  const SORTS: ChannelSort[] = ["latest", "popular", "oldest"];

  let data = $state<ChannelPage | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let tabLoading = $state(false);
  let sort = $state<ChannelSort>("latest");
  let loadingMore = $state(false);
  let moreError = $state<string | null>(null);
  let aboutOpen = $state(false);
  let menuOpen = $state(false);
  let menuBtn = $state<HTMLButtonElement>();
  let gen = 0;

  let tab = $derived<ChannelTabName>(route.tab ?? "home");
  let prefs = $derived(tubePrefs());

  async function load(keepHeader: boolean) {
    const g = ++gen;
    if (keepHeader) tabLoading = true;
    else {
      loading = true;
      data = null;
    }
    error = null;
    moreError = null;
    try {
      const res = await api.channel(route.id, tab, tab === "videos" || tab === "streams" || tab === "shorts" ? sort : null);
      if (g !== gen) return;
      data = res;
    } catch (e) {
      if (g === gen) error = errText(e);
    } finally {
      if (g === gen) {
        loading = false;
        tabLoading = false;
      }
    }
  }

  let lastId = "";
  $effect(() => {
    const id = route.id;
    void tab;
    void sort;
    const same = id === lastId && untrack(() => !!data);
    lastId = id;
    void load(same);
  });

  async function more() {
    const token = data?.page.continuation;
    if (!token || loadingMore || !data) return;
    loadingMore = true;
    moreError = null;
    const g = gen;
    try {
      const next = await api.continuation("channel", token);
      if (g === gen && data) data = { ...data, page: mergePages(data.page, next) };
    } catch (e) {
      if (g === gen) moreError = errText(e);
    } finally {
      loadingMore = false;
    }
  }

  let ch = $derived(data?.channel);
  let tabs = $derived(
    (data?.tabs ?? [])
      .filter((x) => KNOWN.includes(x.id as ChannelTabName))
      .filter((x) => !(prefs.hide_shorts && x.id === "shorts"))
      .map((x) => ({ id: x.id, label: x.title })),
  );
  let banner = $derived(data ? thumbUrl(data.banner, 1284) : "");
  let isGrid = $derived(tab === "videos" || tab === "streams" || tab === "shorts" || tab === "playlists" || tab === "podcasts" || tab === "releases");
  let flat = $derived(isGrid ? (data?.page.shelves ?? []).flatMap((s) => s.items) : []);

  function selectTab(id: string) {
    sort = "latest";
    go({ name: "channel", id: route.id, tab: id as ChannelTabName }, { replace: true });
  }
</script>

<div class="channel" data-agent-readiness={loading || tabLoading ? "loading" : error ? "error" : "ready"} data-agent-surface="omnitube-channel">
  {#if loading && !data}
    <div class="inner">
      <div class="yt-skel banner-sk"></div>
      <div class="head">
        <div class="yt-skel av-sk"></div>
        <div class="head-text">
          <div class="yt-skel" style="height: 36px; width: 40%"></div>
          <div class="yt-skel" style="height: 16px; width: 55%"></div>
          <div class="yt-skel" style="height: 16px; width: 35%"></div>
          <div class="yt-skel" style="height: 36px; width: 120px; border-radius: 18px"></div>
        </div>
      </div>
      <SkeletonGrid count={8} showAvatar={false} />
    </div>
  {:else if error && !data}
    <State icon="subscriptions" {error} onretry={() => load(false)} />
  {:else if data && ch}
    <div class="inner">
      {#if banner}
        <div class="banner"><img src={banner} alt="" decoding="async" referrerpolicy="no-referrer" /></div>
      {/if}
      <header class="head">
        {#if ch.avatar}
          <img class="yt-avatar av" src={bestAvatar(ch.avatar, 160)} width="160" height="160" alt="" referrerpolicy="no-referrer" />
        {/if}
        <div class="head-text">
          <h1>
            {ch.name}
            {#if ch.verified}<TIcon name="verified" size={22} label={$t("tube.card.verified") as string} />{/if}
          </h1>
          <div class="facts">
            {#if ch.handle}<b>{ch.handle}</b>{/if}
            {#if ch.subscribers_text && !prefs.hide_metrics}<span>• {ch.subscribers_text}</span>{/if}
            {#if ch.video_count_text}<span>• {ch.video_count_text}</span>{/if}
          </div>
          {#if ch.description}
            <button class="about" class:open={aboutOpen} onclick={() => (aboutOpen = !aboutOpen)} aria-expanded={aboutOpen}>
              <span class:yt-clamp-1={!aboutOpen}>{ch.description}</span>
              <TIcon name={aboutOpen ? "chevron-up" : "chevron-right"} size={18} />
            </button>
          {/if}
          <div class="cta">
            <SubscribeButton channel={{ id: ch.id, name: ch.name, avatar: ch.avatar }} subscribed={ch.subscribed} />
            <button bind:this={menuBtn} class="yt-btn yt-btn--icon tonal" aria-label={$t("tube.card.actions") as string} onclick={() => (menuOpen = true)}>
              <TIcon name="more-horiz" />
            </button>
          </div>
        </div>
      </header>

      {#if tabs.length}
        <Tabs {tabs} active={tab} onselect={selectTab} label={$t("tube.channel.tabs") as string} />
      {/if}

      <div class="content" class:dim={tabLoading}>
        {#if (tab === "videos" || tab === "streams" || tab === "shorts") && flat.length}
          <ChipBar
            chips={SORTS.map((s) => ({ key: s, label: $t(`tube.channel.sort_${s}`) as string, selected: s === sort }))}
            onselect={(k) => (sort = k as ChannelSort)}
            label={$t("tube.channel.sort") as string}
          />
        {/if}
        {#if error}
          <State icon="subscriptions" {error} onretry={() => load(true)} compact />
        {:else if isGrid}
          {#if flat.length}
            <Grid items={flat} shorts={tab === "shorts"} showAvatar={false} feedbackActions={false} />
          {:else if !tabLoading}
            <State icon="library" title={$t("tube.channel.empty_tab") as string} compact />
          {/if}
        {:else}
          <div class="shelves">
            {#each data.page.shelves as s, i (s.id)}
              <Shelf shelf={s} first={i === 0} showAvatar={false} feedbackActions={false} />
            {/each}
          </div>
          {#if !data.page.shelves.some((s) => s.items.length)}
            <State icon="library" title={$t("tube.channel.empty_tab") as string} compact />
          {/if}
        {/if}
        {#if data.page.continuation}
          <Sentinel onmore={more} loading={loadingMore} error={moreError} />
        {/if}
      </div>
    </div>
    {#if menuOpen}
      <ItemMenu item={ch} anchor={menuBtn ?? null} feedbackActions={false} onclose={() => (menuOpen = false)} />
    {/if}
  {/if}
  <UndoPill />
</div>

<style>
  .channel {
    padding: 0 var(--yt-grid-gutter) 24px;
  }
  .inner {
    max-width: 1284px;
    margin: 0 auto;
  }
  .banner,
  .banner-sk {
    margin-top: 16px;
    aspect-ratio: 6.2 / 1;
    border-radius: 16px;
    overflow: hidden;
    background: var(--yt-skeleton);
  }
  .banner img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 16px 0 8px;
  }
  .av,
  .av-sk {
    flex: none;
    width: 160px;
    height: 160px;
    border-radius: 50%;
  }
  .head-text {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  h1 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font: var(--yt-type-display);
    color: var(--yt-text);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 0 6px;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
  }
  .facts b {
    color: var(--yt-text);
    font-weight: 500;
  }
  .about {
    display: flex;
    align-items: flex-start;
    gap: 4px;
    max-width: 640px;
    padding: 0;
    border: 0;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .about span {
    min-width: 0;
    white-space: pre-line;
  }
  .about:hover {
    color: var(--yt-text);
  }
  .about:focus-visible {
    outline: 2px solid var(--yt-link);
    border-radius: 4px;
  }
  .cta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .tonal {
    background: var(--yt-tonal);
  }
  .content {
    padding-top: 8px;
    transition: opacity var(--yt-dur-base);
  }
  .content > :global(.chipbar) {
    margin-bottom: 8px;
  }
  .content.dim {
    opacity: 0.5;
    pointer-events: none;
  }
  .shelves {
    display: flex;
    flex-direction: column;
    gap: 32px;
    padding-top: 24px;
  }
  @container omnitube (max-width: 720px) {
    .channel {
      padding: 0 16px 24px;
    }
    .head {
      align-items: flex-start;
    }
    .av,
    .av-sk {
      width: 72px;
      height: 72px;
    }
    h1 {
      font: var(--yt-type-headline-m);
    }
  }
</style>
