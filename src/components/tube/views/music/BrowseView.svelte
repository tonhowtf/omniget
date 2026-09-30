<script lang="ts">
  // Generic YT Music browse page: moods & genres, a mood's playlists, charts,
  // new releases, "More" of any shelf.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { back, canGoBack, type Route } from "$lib/tube/ui.svelte";
  import type { Page } from "$lib/tube/types";
  import MusicShelf from "../../music/MusicShelf.svelte";
  import TIcon from "../../TIcon.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";
  import { nearEnd } from "./shared.svelte";

  let { route }: { route: Extract<Route, { name: "m_browse" }> } = $props();

  let page = $state<Page | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let more = $state(false);
  let seq = 0;

  let title = $derived(page?.header?.title || route.title || "");
  // A page that is only one shelf reads better as a grid than as one carousel.
  let single = $derived(!!page && page.shelves.length === 1);

  async function load() {
    const my = ++seq;
    loading = true;
    error = null;
    try {
      const res = await api.musicBrowse(route.id, route.params ?? null);
      if (my === seq) page = res;
    } catch (e) {
      if (my === seq) error = String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function loadMore() {
    if (!page?.continuation || more) return;
    more = true;
    const token = page.continuation;
    try {
      const res = await api.continuation("music_browse", token);
      if (page && page.continuation === token) {
        if (single && res.shelves.length === 1 && !res.shelves[0].title) page.shelves[0].items.push(...res.shelves[0].items);
        else page.shelves.push(...res.shelves);
        page.continuation = res.continuation ?? null;
      }
    } catch {
      if (page) page.continuation = null;
    } finally {
      more = false;
    }
  }

  $effect(() => {
    void route.id;
    void route.params;
    void load();
  });
</script>

<MusicPage readiness={loading ? "loading" : error ? "error" : "ready"} label={title}>
  <header class="br-head">
    {#if canGoBack()}
      <button class="br-back" onclick={() => back()} aria-label={$t("tube.common.back") as string}><TIcon name="back" size={22} /></button>
    {/if}
    {#if title}<h1>{title}</h1>{:else if loading}<span class="yt-skel br-skel"></span>{/if}
  </header>
  {#if page?.header?.subtitle}<p class="br-sub">{page.header.subtitle}</p>{/if}

  {#if loading}
    <MusicSkeleton variant="grid" />
  {:else if error}
    <MusicState icon="explore" error title={$t("tube.music.error_title") as string} text={error} onretry={load} />
  {:else if !page || !page.shelves.length}
    <MusicState icon="explore" title={$t("tube.music.browse_empty") as string} />
  {:else}
    {#each page.shelves as shelf, i (shelf.id + i)}
      <MusicShelf {shelf} layout={single && shelf.kind !== "moods" && shelf.kind !== "list" ? "grid" : undefined} />
    {/each}
    {#if page.continuation}
      <div use:nearEnd={loadMore}><MusicSkeleton variant="grid" /></div>
    {/if}
  {/if}
</MusicPage>

<style>
  .br-head { display: flex; align-items: center; gap: 12px; margin: 8px 0 32px; }
  .br-head h1 { margin: 0; font: var(--yt-type-display); color: var(--ytm-text, var(--yt-text)); }
  .br-skel { width: 280px; height: 40px; display: block; }
  .br-sub { margin: -24px 0 32px; color: var(--ytm-text-2, var(--yt-text-2)); font: var(--yt-type-meta); }
  .br-back {
    display: grid; place-items: center; width: 40px; height: 40px; border-radius: 50%;
    border: 0; background: transparent; color: var(--ytm-text, var(--yt-text)); cursor: pointer;
  }
  .br-back:hover { background: var(--ytm-chip, var(--yt-tonal)); }
  .br-back:focus-visible { outline: 2px solid var(--yt-link); }
</style>
