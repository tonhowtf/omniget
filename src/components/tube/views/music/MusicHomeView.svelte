<script lang="ts">
  // OmniTube Music home: mood chips from FEmusic_home itself, then the shelves
  // (Quick picks, mixes, albums...) with infinite continuation.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { Route } from "$lib/tube/ui.svelte";
  import type { Chip, Page } from "$lib/tube/types";
  import MusicShelf from "../../music/MusicShelf.svelte";
  import TIcon from "../../TIcon.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";
  import { loadAccount, macct, nearEnd, signIn } from "./shared.svelte";

  let { route }: { route: Extract<Route, { name: "m_home" }> } = $props();

  const BANNER_KEY = "omnitube.music.signin_banner_hidden";

  let page = $state<Page | null>(null);
  let chips = $state<Chip[]>([]);
  let params = $state<string | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let more = $state(false);
  let moreFailed = $state(false);
  let bannerHidden = $state(readBanner());
  let seq = 0;

  function readBanner(): boolean {
    try {
      return localStorage.getItem(BANNER_KEY) === "1";
    } catch {
      return false;
    }
  }

  function hideBanner() {
    bannerHidden = true;
    try {
      localStorage.setItem(BANNER_KEY, "1");
    } catch {
      /* private mode */
    }
  }

  async function load(p: string | null) {
    const my = ++seq;
    loading = true;
    error = null;
    moreFailed = false;
    try {
      const res = await api.musicHome(p);
      if (my !== seq) return;
      page = res;
      // Keep the chip bar from the unfiltered home; a filtered page may omit it.
      if (res.chips.length) chips = res.chips;
    } catch (e) {
      if (my === seq) error = String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function loadMore() {
    if (!page?.continuation || more || loading || moreFailed) return;
    more = true;
    const token = page.continuation;
    try {
      const res = await api.continuation("music_home", token);
      if (page && page.continuation === token) {
        page.shelves.push(...res.shelves);
        page.continuation = res.continuation ?? null;
      }
    } catch {
      moreFailed = true;
    } finally {
      more = false;
    }
  }

  function pick(c: Chip) {
    const next = params === (c.token ?? null) ? null : (c.token ?? null);
    params = next;
    void load(next);
  }

  async function doSignIn() {
    if (await signIn()) void load(params);
  }

  onMount(() => {
    void loadAccount();
    void load(null);
  });

  let readiness = $derived(loading ? "loading" : error ? "error" : page && page.shelves.length ? "ready" : "empty");
</script>

<MusicPage readiness={readiness as "loading" | "ready" | "error" | "empty"} label={$t("tube.music.home") as string}>
  {#if chips.length}
    <div class="mh-chips" role="toolbar" aria-label={$t("tube.music.moods") as string}>
      {#each chips as c, i (c.label + i)}
        {@const on = params !== null && params === (c.token ?? null)}
        <button class="mh-chip" class:is-on={on} aria-pressed={on} onclick={() => pick(c)}>{c.label}</button>
      {/each}
    </div>
  {/if}

  {#if macct.loaded && !macct.logged && !bannerHidden}
    <aside class="mh-banner">
      <img src="/icons/tube/tube-music-64.webp" alt="" width="48" height="48" />
      <div class="mh-banner-text">
        <strong>{$t("tube.music.signin_banner_title")}</strong>
        <span>{$t("tube.music.signin_banner_text")}</span>
      </div>
      <button class="yt-btn yt-btn--s mh-signin" onclick={doSignIn}>{$t("tube.common.sign_in")}</button>
      <button class="mh-x" onclick={hideBanner} aria-label={$t("tube.common.dismiss") as string}><TIcon name="close" size={18} /></button>
    </aside>
  {/if}

  {#if loading}
    <MusicSkeleton variant={chips.length ? "shelves" : "home"} />
  {:else if error}
    <MusicState icon="tube-music" error title={$t("tube.music.error_title") as string} text={error} onretry={() => load(params)} />
  {:else if !page || !page.shelves.length}
    <MusicState icon="mix" title={$t("tube.music.home_empty_title") as string} text={$t("tube.music.home_empty_text") as string} onretry={() => load(params)} />
  {:else}
    {#each page.shelves as shelf, i (shelf.id + i)}
      <MusicShelf {shelf} />
    {/each}
    {#if page.continuation}
      <div class="mh-sentinel" use:nearEnd={loadMore}>
        {#if moreFailed}
          <button class="yt-btn yt-btn--s" onclick={() => ((moreFailed = false), void loadMore())}>{$t("tube.common.load_more")}</button>
        {:else}
          <MusicSkeleton variant="shelves" idle={!more} />
        {/if}
      </div>
    {/if}
  {/if}
</MusicPage>

<style>
  .mh-chips {
    display: flex; gap: 12px; flex-wrap: wrap;
    margin: 0 0 32px;
  }
  .mh-chip {
    height: 32px; padding: 0 12px; border-radius: 8px;
    border: 1px solid transparent;
    font: 500 14px/20px var(--yt-font); color: var(--ytm-text, var(--yt-text));
    background: var(--ytm-chip, var(--yt-tonal)); cursor: pointer; white-space: nowrap;
    transition: background var(--yt-dur-base) var(--yt-ease-standard), color var(--yt-dur-base);
  }
  .mh-chip:hover { background: var(--ytm-chip-hover, var(--yt-tonal-hover)); }
  .mh-chip.is-on { background: var(--ytm-text, var(--yt-text)); color: var(--ytm-bg, var(--yt-bg)); }
  .mh-chip:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }

  .mh-banner {
    display: flex; align-items: center; gap: 16px;
    margin: 0 0 32px; padding: 12px 12px 12px 16px;
    border-radius: 12px;
    background: linear-gradient(90deg, color-mix(in srgb, var(--yt-accent) 16%, var(--ytm-bg-a, var(--yt-bg-raised))), var(--ytm-bg-a, var(--yt-bg-raised)) 60%);
    border: 1px solid var(--ytm-divider, var(--yt-divider));
  }
  .mh-banner img { flex: none; }
  .mh-banner-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .mh-banner-text strong { font: var(--yt-type-body-strong); font-size: 15px; }
  .mh-banner-text span { font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); }
  .mh-signin { background: var(--ytm-text, var(--yt-text)); color: var(--ytm-bg, var(--yt-bg)); }
  .mh-signin:hover { background: var(--ytm-text-2, var(--yt-text-2)); }
  .mh-x {
    display: grid; place-items: center; width: 32px; height: 32px; border-radius: 50%;
    border: 0; background: transparent; color: var(--ytm-text-2, var(--yt-text-2)); cursor: pointer;
  }
  .mh-x:hover { background: var(--ytm-chip, var(--yt-tonal)); }
  .mh-x:focus-visible { outline: 2px solid var(--yt-link); }
  .mh-sentinel { min-height: 40px; display: flex; flex-direction: column; align-items: stretch; }
  .mh-sentinel > :global(.yt-btn) { align-self: center; }
  @container omnitube (max-width: 560px) {
    .mh-banner { flex-wrap: wrap; }
  }
</style>
