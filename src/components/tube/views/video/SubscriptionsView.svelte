<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { Item, Page, VideoItem } from "$lib/tube/types";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import TIcon from "../../TIcon.svelte";
  import Grid from "../../cards/Grid.svelte";
  import NewItemsPill from "../../cards/NewItemsPill.svelte";
  import Sentinel from "../../cards/Sentinel.svelte";
  import Shelf from "../../cards/Shelf.svelte";
  import SkeletonGrid from "../../cards/SkeletonGrid.svelte";
  import State from "../../cards/State.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import VideoRow from "../../cards/VideoRow.svelte";
  import { isVisible } from "../../cards/feedback.svelte";
  import { Pager } from "../../cards/pager.svelte";
  import { ensureSession, localSubCount, loggedIn, login as signIn, sessionReady } from "../../cards/session.svelte";
  import { dayBucket, errText, lsGet, lsSet } from "../../cards/util";
  import { showToast } from "$lib/stores/toast-store.svelte";

  let { route: _route }: { route: Extract<Route, { name: "subscriptions" }> } = $props();

  const VIEW_KEY = "omnitube.subs.view";
  let view = $state<"grid" | "list">(lsGet(VIEW_KEY, "grid"));
  let pending = $state<Page | null>(null);
  let pendingCount = $state(0);
  let refreshing = $state(false);
  let top = $state<HTMLDivElement>();

  const pager = new Pager("feed", () => api.subscriptionsFeed(false));

  function ids(p: Page | null): Set<string> {
    return new Set((p?.shelves ?? []).flatMap((s) => s.items.map((i) => ("id" in i ? i.id : ""))));
  }

  async function start() {
    await ensureSession();
    await pager.load();
    if (loggedIn()) return;
    refreshing = true;
    try {
      const fresh = await api.subscriptionsFeed(true);
      const before = ids(pager.page);
      const added = [...ids(fresh)].filter((id) => id && !before.has(id)).length;
      if (!pager.page || before.size === 0) pager.page = fresh;
      else if (added > 0) {
        pending = fresh;
        pendingCount = added;
      }
    } catch {
    } finally {
      refreshing = false;
    }
  }

  $effect(() => {
    void start();
  });

  function jump() {
    if (pending) pager.page = pending;
    pending = null;
    pendingCount = 0;
    top?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function setView(v: "grid" | "list") {
    view = v;
    lsSet(VIEW_KEY, v);
  }

  async function login() {
    try {
      await signIn();
      await pager.load();
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  let videos = $derived(pager.shelves.flatMap((s) => s.items).filter((i): i is VideoItem => i.kind === "video" && isVisible(i)));
  let dated = $derived(videos.length > 0 && videos.every((v) => v.published_ms));
  let groups = $derived.by(() => {
    if (!dated) return [];
    const out: { key: string; items: Item[] }[] = [];
    for (const v of [...videos].sort((a, b) => (b.published_ms ?? 0) - (a.published_ms ?? 0))) {
      const k = dayBucket(v.published_ms!);
      const last = out[out.length - 1];
      if (last && last.key === k) last.items.push(v);
      else out.push({ key: k, items: [v] });
    }
    return out;
  });
  let empty = $derived(!!pager.page && pager.shelves.every((s) => s.items.length === 0));
  let noSubs = $derived(sessionReady() && !loggedIn() && localSubCount() === 0);
</script>

<div class="subs" data-agent-readiness={pager.loading ? "loading" : pager.error ? "error" : "ready"} data-agent-surface="omnitube-subscriptions">
  <div bind:this={top}></div>
  <NewItemsPill count={pendingCount} onjump={jump} />
  <header>
    <h1 class="yt-shelf-title">{$t("tube.subs.latest")}</h1>
    {#if refreshing}<span class="refreshing"><TIcon name="loader" size={16} />{$t("tube.subs.checking")}</span>{/if}
    <div class="hact">
      <button class="yt-btn yt-btn--ghost link" onclick={() => go({ name: "settings", section: "subscriptions" })}>{$t("tube.subs.manage")}</button>
      <div class="yt-seg" role="group" aria-label={$t("tube.subs.layout") as string}>
        <button class="yt-btn" aria-pressed={view === "grid"} class:on={view === "grid"} onclick={() => setView("grid")} aria-label={$t("tube.subs.grid") as string}><TIcon name="grid" size={20} /></button>
        <button class="yt-btn" aria-pressed={view === "list"} class:on={view === "list"} onclick={() => setView("list")} aria-label={$t("tube.subs.list") as string}><TIcon name="list" size={20} /></button>
      </div>
    </div>
  </header>

  {#if pager.loading && !pager.page}
    <SkeletonGrid layout={view === "grid" ? "grid" : "rows"} count={12} />
  {:else if pager.error && !pager.page}
    <State icon="subscriptions" error={pager.error} onretry={() => pager.load()} />
  {:else if noSubs || empty}
    <State icon="subscriptions" title={$t(noSubs ? "tube.subs.none_title" : "tube.subs.empty_title") as string} text={$t(noSubs ? "tube.subs.none_text" : "tube.subs.empty_text") as string}>
      <button class="yt-btn yt-btn--outline" onclick={() => go({ name: "import" })}><TIcon name="import" size={20} />{$t("tube.subs.import")}</button>
      {#if !loggedIn()}
        <button class="yt-btn" onclick={login}><TIcon name="login" size={20} />{$t("tube.common.sign_in")}</button>
      {/if}
    </State>
  {:else if dated}
    {#each groups as g (g.key)}
      <section class="day">
        <h2>{$t(`tube.subs.day_${g.key}`)}</h2>
        {#if view === "grid"}
          <Grid items={g.items} feedbackActions={false} />
        {:else}
          <div class="rows">
            {#each g.items as it (("id" in it ? it.id : ""))}
              {#if it.kind === "video"}<VideoRow item={it} size="search" feedbackActions={false} />{/if}
            {/each}
          </div>
        {/if}
      </section>
    {/each}
  {:else}
    <div class="shelves">
      {#each pager.shelves as s, i (s.id)}
        {#if view === "list" && s.kind === "grid"}
          <Shelf shelf={{ ...s, kind: "list" }} first={i === 0} feedbackActions={false} />
        {:else}
          <Shelf shelf={s} first={i === 0} feedbackActions={false} />
        {/if}
      {/each}
    </div>
  {/if}
  {#if pager.hasMore}
    <Sentinel onmore={() => pager.more()} loading={pager.loadingMore} error={pager.moreError} />
  {/if}
  <UndoPill />
</div>

<style>
  .subs {
    padding: 0 var(--yt-grid-gutter) 24px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 24px 0 16px;
  }
  .refreshing {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
  }
  .hact {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
  }
  .link {
    color: var(--yt-link);
  }
  .yt-seg .yt-btn {
    width: 44px;
    padding: 0;
    color: var(--yt-text-2);
  }
  .yt-seg .yt-btn.on {
    color: var(--yt-text);
    background: var(--yt-tonal-hover);
  }
  .day + .day {
    margin-top: 40px;
  }
  .day h2 {
    margin: 0 0 16px;
    font: var(--yt-type-headline-s);
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 1096px;
  }
  .shelves {
    display: flex;
    flex-direction: column;
    gap: 40px;
  }
  @container omnitube (max-width: 640px) {
    .subs {
      padding: 0 12px 24px;
    }
  }
</style>
