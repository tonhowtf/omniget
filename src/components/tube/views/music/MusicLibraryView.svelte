<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { MusicLibraryKind } from "$lib/tube/api";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import type { Item, Page, PlaylistItem } from "$lib/tube/types";
  import SongRow from "../../music/SongRow.svelte";
  import SquareCard from "../../music/SquareCard.svelte";
  import TIcon from "../../TIcon.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";
  import { itemKey, loadAccount, macct, nearEnd, signIn, songFromItem } from "./shared.svelte";

  let { route }: { route: Extract<Route, { name: "m_library" }> } = $props();

  const TABS: { id: MusicLibraryKind; key: string; icon: string }[] = [
    { id: "playlists", key: "tube.music.lib_playlists", icon: "library" },
    { id: "songs", key: "tube.music.lib_songs", icon: "liked" },
    { id: "albums", key: "tube.music.lib_albums", icon: "mix" },
    { id: "artists", key: "tube.music.lib_artists", icon: "radio" },
    { id: "subscriptions", key: "tube.music.lib_subscriptions", icon: "subscriptions" },
    { id: "history", key: "tube.music.lib_history", icon: "history" },
  ];

  let tab = $derived<MusicLibraryKind>(route.tab && TABS.some((x) => x.id === route.tab) ? route.tab : "playlists");
  let page = $state<Page | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let more = $state(false);
  let seq = 0;

  let LIKED = $derived<PlaylistItem>({ kind: "playlist", id: "LM", title: $t("tube.music.liked_music") as string, thumbs: [], channel: null, count_text: null, is_mix: false, first_video_id: null, is_music: true });

  let items = $derived.by<Item[]>(() => (page ? page.shelves.flatMap((s) => s.items) : []));
  let asList = $derived(tab === "songs" || tab === "history" || (items.length > 0 && items.every((i) => songFromItem(i))));
  let tabInfo = $derived(TABS.find((x) => x.id === tab)!);

  async function load() {
    const my = ++seq;
    await loadAccount();
    if (my !== seq) return;
    if (!macct.logged) {
      loading = false;
      page = null;
      return;
    }
    loading = true;
    error = null;
    try {
      const res = await api.musicLibrary(tab);
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
      const res = await api.continuation("music_library", token);
      if (page && page.continuation === token) {
        if (page.shelves.length) for (const s of res.shelves) page.shelves[page.shelves.length - 1].items.push(...s.items);
        else page.shelves.push(...res.shelves);
        page.continuation = res.continuation ?? null;
      }
    } catch {
      if (page) page.continuation = null;
    } finally {
      more = false;
    }
  }

  function setTab(k: MusicLibraryKind) {
    go({ name: "m_library", tab: k }, { replace: true });
  }

  async function doSignIn() {
    if (await signIn()) void load();
  }

  onMount(() => void loadAccount());

  $effect(() => {
    void tab;
    void load();
  });

  let readiness = $derived<"loading" | "ready" | "error" | "empty">(loading ? "loading" : error ? "error" : !macct.logged ? "empty" : "ready");
</script>

<MusicPage {readiness} label={$t("tube.music.library") as string}>
  <div class="lb-top">
    <h1 class="lb-title">{$t("tube.music.library")}</h1>
  </div>
  <div class="lb-chips" role="tablist" aria-label={$t("tube.music.library") as string}>
    {#each TABS as x (x.id)}
      <button class="lb-chip" class:is-on={tab === x.id} role="tab" aria-selected={tab === x.id} onclick={() => setTab(x.id)}>{$t(x.key)}</button>
    {/each}
  </div>

  {#if !macct.loaded || loading}
    <MusicSkeleton variant={asList ? "list" : "grid"} />
  {:else if !macct.logged}
    <MusicState icon="library" title={$t("tube.music.lib_signin_title") as string} text={$t("tube.music.lib_signin_text") as string}>
      <button class="yt-btn lb-signin" onclick={doSignIn}><TIcon name="login" size={18} />{$t("tube.common.sign_in")}</button>
      <button class="yt-btn lb-alt" onclick={() => go({ name: "m_explore" })}>{$t("tube.music.go_explore")}</button>
    </MusicState>
  {:else if error}
    <MusicState icon="library" error title={$t("tube.music.error_title") as string} text={error} onretry={load} />
  {:else}
    {#if tab === "playlists"}
      <div class="lb-grid" role="list">
        <div role="listitem"><SquareCard item={LIKED} fluid subtitle={$t("tube.music.auto_playlist") as string} /></div>
        {#each items as it, i (itemKey(it, i))}
          {#if !(it.kind === "playlist" && (it.id === "LM" || it.id === "VLLM"))}
            <div role="listitem"><SquareCard item={it} fluid /></div>
          {/if}
        {/each}
      </div>
    {:else if !items.length}
      <MusicState icon={tabInfo.icon} title={$t("tube.music.lib_empty_title") as string} text={$t("tube.music.lib_empty_text") as string} />
    {:else if asList}
      <div class="lb-list" role="table" aria-label={$t(tabInfo.key) as string}>
        {#each items as it, i (itemKey(it, i))}
          {@const s = songFromItem(it)}
          {#if s}<SongRow song={s} />{/if}
        {/each}
      </div>
    {:else}
      <div class="lb-grid" role="list">
        {#each items as it, i (itemKey(it, i))}
          <div role="listitem"><SquareCard item={it} fluid /></div>
        {/each}
      </div>
    {/if}
    {#if page?.continuation}
      <div use:nearEnd={loadMore}><MusicSkeleton variant={asList ? "list" : "grid"} /></div>
    {/if}
  {/if}
</MusicPage>

<style>
  .lb-top { display: flex; align-items: center; justify-content: space-between; margin: 8px 0 20px; }
  .lb-title { margin: 0; font: var(--yt-type-display); }
  .lb-chips { display: flex; gap: 12px; flex-wrap: wrap; margin: 0 0 32px; }
  .lb-chip {
    height: 32px; padding: 0 12px; border-radius: 8px; border: 0;
    font: 500 14px/20px var(--yt-font); color: var(--ytm-text, var(--yt-text));
    background: var(--ytm-chip, var(--yt-tonal)); cursor: pointer; white-space: nowrap;
    transition: background var(--yt-dur-base) var(--yt-ease-standard);
  }
  .lb-chip:hover { background: var(--ytm-chip-hover, var(--yt-tonal-hover)); }
  .lb-chip.is-on { background: var(--ytm-text, var(--yt-text)); color: var(--ytm-bg, var(--yt-bg)); }
  .lb-chip:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
  .lb-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 32px 16px; }
  .lb-list { display: flex; flex-direction: column; }
  .lb-signin { background: var(--ytm-text, #fff); color: var(--ytm-bg, #000); }
  .lb-signin:hover { background: color-mix(in srgb, var(--ytm-text, #fff) 85%, transparent); }
  .lb-alt { background: var(--ytm-chip, var(--yt-tonal)); color: var(--ytm-text, var(--yt-text)); }
</style>
