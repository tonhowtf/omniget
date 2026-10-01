<script lang="ts" module>
  let booted = false;
</script>

<script lang="ts">
  import { onMount, tick } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { page } from "$app/state";
  import { locale } from "$lib/i18n";
  import { registerSemantic } from "$lib/native-agent/semantic";
  import { setFormatLocale } from "$lib/tube/format";
  import { installKeys, keys } from "$lib/tube/keys.svelte";
  import { currentSong, mplayer } from "$lib/tube/music.svelte";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import { current, go, goRoot, routeFromUrl, ui, type Route } from "$lib/tube/ui.svelte";
  import { openVideo, vplayer } from "$lib/tube/video.svelte";
  import "./tube.css";
  import Guide from "./shell/Guide.svelte";
  import Masthead from "./shell/Masthead.svelte";
  import MiniGuide from "./shell/MiniGuide.svelte";
  import ShortcutSheet from "./shell/ShortcutSheet.svelte";
  import { look, watchScheme } from "./shell/look.svelte";
  import { loadSession } from "./shell/session.svelte";
  import NowPlaying from "./music/NowPlaying.svelte";
  import PlayerBar from "./music/PlayerBar.svelte";
  import HomeView from "./views/video/HomeView.svelte";
  import SearchView from "./views/video/SearchView.svelte";
  import ChannelView from "./views/video/ChannelView.svelte";
  import PlaylistView from "./views/video/PlaylistView.svelte";
  import SubscriptionsView from "./views/video/SubscriptionsView.svelte";
  import LibraryView from "./views/video/LibraryView.svelte";
  import HistoryView from "./views/video/HistoryView.svelte";
  import ShortsView from "./views/video/ShortsView.svelte";
  import WatchView from "./watch/WatchView.svelte";
  import MusicHomeView from "./views/music/MusicHomeView.svelte";
  import ExploreView from "./views/music/ExploreView.svelte";
  import BrowseView from "./views/music/BrowseView.svelte";
  import MusicSearchView from "./views/music/MusicSearchView.svelte";
  import AlbumView from "./views/music/AlbumView.svelte";
  import ArtistView from "./views/music/ArtistView.svelte";
  import MusicPlaylistView from "./views/music/MusicPlaylistView.svelte";
  import MusicLibraryView from "./views/music/MusicLibraryView.svelte";
  import TubeSettingsView from "./views/settings/TubeSettingsView.svelte";
  import ImportView from "./views/ImportView.svelte";

  const DOCK_MIN = 1000;
  const MINI_MIN = 640;

  let root = $state<HTMLDivElement>();
  let scroller = $state<HTMLElement>();
  let width = $state(0);
  let drawer = $state(false);
  let scrolled = $state(false);
  let loading = $state(false);

  let prefs = $derived(tubePrefs());
  let route = $derived.by(() => {
    void ui.nav;
    return current();
  });
  let onWatch = $derived(route.name === "watch");
  let docked = $derived(width >= DOCK_MIN && ui.guideOpen && !onWatch);
  let mini = $derived(!docked && !onWatch && width >= MINI_MIN);
  let scheme = $derived(ui.face === "music" && prefs.look === "classic" ? "dark" : look.scheme);
  let showBar = $derived(!!currentSong() && !(onWatch && vplayer.videoId && vplayer.active) && !(vplayer.mini && vplayer.active));

  let viewKey = $derived(route.name === "watch" ? "watch" : JSON.stringify(route));
  let posKey = $derived(`${ui.face}:${ui.stacks[ui.face].length}:${viewKey}`);

  function toggleGuide() {
    if (width >= DOCK_MIN && !onWatch) ui.guideOpen = !ui.guideOpen;
    else drawer = !drawer;
  }

  let lastQuery = "";
  $effect(() => {
    const qs = page.url.search;
    if (qs === lastQuery) return;
    lastQuery = qs;
    const r = routeFromUrl(page.url.searchParams);
    if (!r) return;
    if (r.name === "watch") openVideo(r.id, { list: r.list, t: r.t });
    else go(r);
  });

  const positions = new Map<string, number>();
  let activeKey = "";
  function onScroll() {
    if (!scroller) return;
    positions.set(activeKey, scroller.scrollTop);
    scrolled = scroller.scrollTop > 4;
  }
  $effect(() => {
    const key = posKey;
    if (key === activeKey) return;
    activeKey = key;
    const want = positions.get(key) ?? 0;
    void tick().then(() => restore(key, want));
  });
  function restore(key: string, want: number) {
    const el = scroller;
    if (!el) return;
    el.scrollTop = want;
    if (want <= 0) return;
    const started = performance.now();
    const step = () => {
      if (activeKey !== key || !scroller) return;
      if (Math.abs(scroller.scrollTop - want) < 2 || performance.now() - started > 1500) return;
      scroller.scrollTop = want;
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  $effect(() => {
    const el = root;
    const box = scroller;
    if (!el || !box) return;
    const onScreen = (n: Element) => {
      const r = n.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && r.bottom > 0 && r.top < innerHeight;
    };
    const check = () => (loading = [...el.querySelectorAll('[data-agent-readiness="loading"]')].some((n) => n !== el && onScreen(n)));
    const mo = new MutationObserver(check);
    mo.observe(box, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-agent-readiness"] });
    box.addEventListener("scroll", check, { passive: true });
    const tick = setInterval(() => loading && check(), 1000);
    check();
    return () => {
      mo.disconnect();
      clearInterval(tick);
      box.removeEventListener("scroll", check);
    };
  });

  $effect(() => setFormatLocale($locale));


  $effect(() => {
    void ui.nav;
    drawer = false;
  });

  onMount(() => {
    watchScheme();
    void loadSession();
    if (!booted) {
      booted = true;
      const deep = routeFromUrl(page.url.searchParams);
      if (!deep) {
        ui.face = prefs.start_face;
        const s = ui.stacks.video;
        if (prefs.start_page !== "home" && s.length === 1 && s[0].name === "home") {
          const start: Route = prefs.start_page === "subscriptions" ? { name: "subscriptions" } : { name: "library" };
          goRoot(start);
          ui.face = prefs.start_face;
        }
      }
    }
    const off = installKeys();
    const offSem = registerSemantic("omnitube", () => ({
      ready: !loading,
      face: ui.face,
      route: current(),
      video: vplayer.videoId
        ? { id: vplayer.videoId, title: vplayer.info?.title ?? vplayer.streams?.title ?? null, playing: vplayer.playing, position: Math.round(vplayer.position), duration: Math.round(vplayer.duration), mode: vplayer.mode, error: vplayer.error, failures: vplayer.failures }
        : null,
      music: currentSong() ? { id: currentSong()!.id, title: currentSong()!.title, playing: mplayer.playing, queue: mplayer.queue.length } : null,
    }));
    return () => {
      off();
      offSem();
    };
  });
</script>

<div
  class="omnitube"
  bind:this={root}
  bind:clientWidth={width}
  data-look={prefs.look}
  data-scheme={scheme}
  data-face={ui.face}
  class:with-bar={showBar}
  data-agent-readiness={loading ? "loading" : "ready"}
>
  <Masthead {scrolled} {loading} ontoggle={toggleGuide} />

  <div class="body" class:docked class:mini>
    {#if docked}
      <aside class="side"><Guide /></aside>
    {:else if mini}
      <aside class="side"><MiniGuide /></aside>
    {/if}

    <main class="content yt-scroll" bind:this={scroller} onscroll={onScroll}>
      {#key viewKey}
        {#if route.name === "home"}<HomeView {route} />
        {:else if route.name === "search"}<SearchView {route} />
        {:else if route.name === "watch"}<WatchView {route} />
        {:else if route.name === "channel"}<ChannelView {route} />
        {:else if route.name === "playlist"}<PlaylistView {route} />
        {:else if route.name === "subscriptions"}<SubscriptionsView {route} />
        {:else if route.name === "shorts"}<ShortsView {route} />
        {:else if route.name === "library"}<LibraryView {route} />
        {:else if route.name === "history"}<HistoryView {route} />
        {:else if route.name === "settings"}<TubeSettingsView {route} />
        {:else if route.name === "import"}<ImportView {route} />
        {:else if route.name === "m_home"}<MusicHomeView {route} />
        {:else if route.name === "m_explore"}<ExploreView {route} />
        {:else if route.name === "m_library"}<MusicLibraryView {route} />
        {:else if route.name === "m_search"}<MusicSearchView {route} />
        {:else if route.name === "m_album"}<AlbumView {route} />
        {:else if route.name === "m_artist"}<ArtistView {route} />
        {:else if route.name === "m_playlist"}<MusicPlaylistView {route} />
        {:else if route.name === "m_browse"}<BrowseView {route} />
        {/if}
      {/key}
    </main>

    {#if ui.nowPlaying && currentSong()}
      <NowPlaying />
    {/if}

    {#if drawer}
      <div class="scrim" role="presentation" onclick={() => (drawer = false)} transition:fade={{ duration: 200 }}></div>
      <aside class="drawer" transition:fly={{ x: -260, duration: 250, easing: cubicOut, opacity: 1 }}>
        <Guide onnavigate={() => (drawer = false)} />
      </aside>
    {/if}
  </div>

  {#if showBar}<PlayerBar />{/if}
  {#if keys.help}<ShortcutSheet />{/if}
</div>

<style>
  .omnitube {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .omnitube[data-face="music"] {
    background: var(--ytm-bg);
  }
  .body {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    flex: 1;
    min-height: 0;
  }
  .body.docked {
    grid-template-columns: var(--yt-guide-w) minmax(0, 1fr);
  }
  .body.mini {
    grid-template-columns: var(--yt-mini-guide-w) minmax(0, 1fr);
  }
  .side {
    min-height: 0;
    overflow: hidden;
  }
  .content {
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    overscroll-behavior: contain;
  }
  .scrim {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    z-index: 40;
  }
  .drawer {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    background: var(--yt-bg);
    box-shadow: 8px 0 32px rgba(0, 0, 0, 0.35);
    z-index: 41;
  }
  .omnitube[data-face="music"] .drawer {
    background: var(--ytm-bg-a);
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer,
    .scrim {
      transition: none;
    }
  }
</style>
