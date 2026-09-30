<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { page } from "$app/state";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { focusWhenReady, parseMusicTarget } from "$lib/settings-navigation";
  import { applyMusicTarget } from "$lib/destination-targets";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { albumKey, groupAlbums } from "$lib/music/library";
  import { library, loadLibrary, patchTracksLocal, upsertTracks } from "$lib/music/library-store.svelte";
  import {
    cycleRepeat,
    hydrateCurrent,
    initPlayer,
    next,
    player,
    prefs,
    prev,
    seek,
    seekBy,
    setVolume,
    toggle,
    toggleMute,
    toggleShuffle,
  } from "$lib/music/player-store.svelte";
  import { back, createPlaylistWith, go, loadPlaylists, ui, type View } from "$lib/music/ui-store.svelte";
  import type { Track } from "$lib/music/types";
  import ContextMenu from "$components/music/ContextMenu.svelte";
  import EqPanel from "$components/music/EqPanel.svelte";
  import LyricsPanel from "$components/music/LyricsPanel.svelte";
  import Modal from "$components/music/Modal.svelte";
  import MusicIcon from "$components/music/MusicIcon.svelte";
  import PlayerBar from "$components/music/PlayerBar.svelte";
  import QueuePanel from "$components/music/QueuePanel.svelte";
  import TagEditor from "$components/music/TagEditor.svelte";
  import AlbumsView from "$components/music/views/AlbumsView.svelte";
  import AlbumView from "$components/music/views/AlbumView.svelte";
  import ArtistsView from "$components/music/views/ArtistsView.svelte";
  import ArtistView from "$components/music/views/ArtistView.svelte";
  import DownloadView from "$components/music/views/DownloadView.svelte";
  import FoldersView from "$components/music/views/FoldersView.svelte";
  import GenresView from "$components/music/views/GenresView.svelte";
  import HomeView from "$components/music/views/HomeView.svelte";
  import NowPlayingView from "$components/music/views/NowPlayingView.svelte";
  import PlaylistsView from "$components/music/views/PlaylistsView.svelte";
  import PlaylistView from "$components/music/views/PlaylistView.svelte";
  import RadioView from "$components/music/views/RadioView.svelte";
  import SettingsView from "$components/music/views/SettingsView.svelte";
  import SongsView from "$components/music/views/SongsView.svelte";
  import StatsView from "$components/music/views/StatsView.svelte";
  import ToolsView from "$components/music/views/ToolsView.svelte";

  let enabled = $derived(getSettings()?.music?.enabled ?? true);
  let lib = $derived(library());
  /** For the native agent: `data-agent-readiness` on the library area. */
  let readiness = $derived(lib.loading && !lib.loaded ? "loading" : lib.error ? "error" : "ready");
  let peaks = $state<number[]>([]);
  let peaksFor = $state<number | null>(null);
  let dropping = $state(false);
  let newName = $state("");
  let searchEl: HTMLInputElement | undefined = $state();
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  const NAV: { kind: View["kind"]; icon: string }[] = [
    { kind: "home", icon: "home" },
    { kind: "nowplaying", icon: "wave" },
    { kind: "songs", icon: "note" },
    { kind: "albums", icon: "disc" },
    { kind: "artists", icon: "artist" },
    { kind: "genres", icon: "tag" },
    { kind: "folders", icon: "folder" },
    { kind: "liked", icon: "heart" },
    { kind: "recent", icon: "history" },
  ];
  const NAV2: { kind: View["kind"]; icon: string }[] = [
    { kind: "radio", icon: "radio" },
    { kind: "download", icon: "download" },
    { kind: "tools", icon: "tools" },
    { kind: "stats", icon: "stats" },
    { kind: "settings", icon: "settings" },
  ];

  onMount(() => {
    initPlayer();
    void loadLibrary().then(() => hydrateCurrent());
    void loadPlaylists();

    // files dropped on the window: audio goes into the library, playlists get imported
    let unlisten: (() => void) | null = null;
    let disposed = false;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter") dropping = true;
          else if (p.type === "leave") dropping = false;
          else if (p.type === "drop") {
            dropping = false;
            if (p.paths?.length) void importDropped(p.paths);
          }
        })
        .then((fn) => {
          if (disposed) fn();
          else unlisten = fn;
        })
        .catch(() => {});
    } catch {
      /* outside Tauri */
    }
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  async function importDropped(paths: string[]) {
    const lists = paths.filter((p) => /\.(m3u8?|pls)$/i.test(p));
    const rest = paths.filter((p) => !lists.includes(p));
    for (const l of lists) {
      try {
        const r = await invoke<{ playlist_id: number; matched: number }>("music_playlist_import", { path: l });
        if (r.playlist_id) go({ kind: "playlist", id: r.playlist_id });
      } catch (e) {
        showToast("error", String(e));
      }
    }
    if (lists.length) await loadPlaylists();
    if (!rest.length) return;
    try {
      const rows = await invoke<Track[]>("music_import_paths", { paths: rest, sourceUrl: null });
      upsertTracks(rows);
      showToast(rows.length ? "success" : "info", $t("music.toast.dropped", { count: rows.length }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  // `/music?view=settings&panel=eq` (links from Settings): reveal through the
  // store navigation and focus. Never enables Music, plays, scans or edits.
  $effect(() => {
    const target = parseMusicTarget(page.url.searchParams);
    const selector = untrack(() =>
      applyMusicTarget(target, {
        currentKind: ui.view.kind,
        go,
        showEq: () => { ui.side = "eq"; },
      }),
    );
    if (!selector) return;
    return focusWhenReady(selector, { timeoutMs: 8000 });
  });

  // waveform for the current track
  $effect(() => {
    const id = player.current?.id ?? null;
    if (id === peaksFor) return;
    peaksFor = id;
    peaks = [];
    if (id == null) return;
    invoke<number[]>("music_peaks", { id })
      .then((p) => {
        if (peaksFor === id) peaks = p;
      })
      .catch(() => {});
  });

  function onSearch(e: Event) {
    const q = (e.currentTarget as HTMLInputElement).value;
    ui.search = q;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      if (q.trim()) {
        if (ui.view.kind === "search") ui.view = { kind: "search", q };
        else go({ kind: "search", q });
      } else if (ui.view.kind === "search") back();
    }, 180);
  }

  function typing(e: KeyboardEvent): boolean {
    const el = e.target as HTMLElement | null;
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
  }

  function keys(e: KeyboardEvent) {
    if (!enabled || typing(e) || ui.tagEditor || ui.confirm || ui.playlistPicker || e.altKey) return;
    if (e.metaKey || e.ctrlKey) return;
    const k = e.key;
    if (k === " " || k === "k") {
      e.preventDefault();
      void toggle();
    } else if (k === "ArrowRight" && e.shiftKey) void next();
    else if (k === "ArrowLeft" && e.shiftKey) void prev();
    else if (k === "ArrowRight") seekBy(10);
    else if (k === "ArrowLeft") seekBy(-10);
    else if (k === "ArrowUp") {
      e.preventDefault();
      setVolume(prefs.volume + 0.05);
    } else if (k === "ArrowDown") {
      e.preventDefault();
      setVolume(prefs.volume - 0.05);
    } else if (k === "m") toggleMute();
    else if (k === "s") toggleShuffle();
    else if (k === "r") cycleRepeat();
    else if (k === "L" || k === "l") {
      const c = player.current;
      if (c) {
        patchTracksLocal([c.id], { liked: !c.liked });
        void invoke("music_set_liked", { ids: [c.id], liked: !c.liked });
      }
    } else if (/^[0-9]$/.test(k) && player.duration > 0) void seek((Number(k) / 10) * player.duration);
    else if (k === "/") {
      e.preventDefault();
      searchEl?.focus();
    } else if (k === "Escape" && ui.view.kind !== "home") back();
    else return;
  }

  async function createFromPicker() {
    const ids = ui.playlistPicker;
    if (!ids) return;
    await createPlaylistWith(ids, newName);
    newName = "";
    ui.playlistPicker = null;
  }

  function navLabel(kind: string) {
    return $t(`music.nav.${kind}`) as string;
  }

  function isActive(kind: string) {
    const v = ui.view.kind;
    if (v === kind) return true;
    if (kind === "albums" && v === "album") return true;
    if (kind === "artists" && v === "artist") return true;
    if (kind === "genres" && v === "genre") return true;
    if (kind === "playlists" && v === "playlist") return true;
    return false;
  }

  function dropOnPlaylist(e: DragEvent, id: number) {
    const raw = e.dataTransfer?.getData("application/x-omniget-tracks");
    if (!raw) return;
    e.preventDefault();
    void invoke("music_playlist_add", { id, trackIds: JSON.parse(raw) }).then(() => {
      void loadPlaylists();
      showToast("success", $t("music.toast.added_to_playlist", { name: ui.playlists.find((p) => p.id === id)?.name ?? "" }) as string);
    });
  }

  let albumTitle = $derived.by(() => {
    const v = ui.view;
    if (v.kind !== "album") return "";
    return groupAlbums(lib.tracks.filter((x) => albumKey(x) === v.key))[0]?.title ?? "";
  });
</script>

<svelte:window onkeydown={keys} />

{#if !enabled}
  <div class="off">
    <MusicIcon name="note" size={32} />
    <h2>{$t("music.disabled_title")}</h2>
    <p>{$t("superpowers.disabled_hint")}</p>
    <a class="btn btn-primary" href="/settings?tab=integrations&section=music">{$t("superpowers.open_settings")}</a>
  </div>
{:else}
  <div class="music" class:with-side={!!ui.side}>
    <nav class="m-nav" aria-label={$t("music.nav_label") as string}>
      <div class="brand">
        <img src="/icons/menu/music-64.webp" srcset="/icons/menu/music-64.webp 1x, /icons/menu/music-128.webp 2x" alt="" width="30" height="30" />
        <span>{$t("music.title")}</span>
      </div>
      <ul>
        {#each NAV as n (n.kind)}
          <li>
            <button class="ni" class:on={isActive(n.kind)} onclick={() => go({ kind: n.kind } as View)} aria-current={isActive(n.kind)}>
              <MusicIcon name={n.icon} size={16} /> <span>{navLabel(n.kind)}</span>
            </button>
          </li>
        {/each}
      </ul>
      <div class="sec">
        <button class="sec-h" class:on={isActive("playlists")} onclick={() => go({ kind: "playlists" })}>{$t("music.nav.playlists")}</button>
        <button class="sec-add" onclick={() => (ui.playlistPicker = [])} aria-label={$t("music.playlist.new") as string} title={$t("music.playlist.new") as string}><MusicIcon name="plus" size={13} /></button>
      </div>
      <ul class="pls">
        {#each ui.playlists as p (p.id)}
          <li>
            <button
              class="ni pl"
              class:on={ui.view.kind === "playlist" && ui.view.id === p.id}
              onclick={() => go({ kind: "playlist", id: p.id })}
              ondragover={(e) => {
                if (!p.smart && e.dataTransfer?.types.includes("application/x-omniget-tracks")) e.preventDefault();
              }}
              ondrop={(e) => dropOnPlaylist(e, p.id)}
            >
              <MusicIcon name={p.smart ? "smart" : "playlist"} size={14} /> <span>{p.name}</span>
            </button>
          </li>
        {/each}
      </ul>
      <ul class="nav2">
        {#each NAV2 as n (n.kind)}
          <li>
            <button class="ni" class:on={isActive(n.kind)} onclick={() => go({ kind: n.kind } as View)} aria-current={isActive(n.kind)}>
              <MusicIcon name={n.icon} size={16} /> <span>{navLabel(n.kind)}</span>
            </button>
          </li>
        {/each}
      </ul>
      {#if lib.scan || lib.analysis}
        <div class="status" role="status">
          {#if lib.scan}
            <span><MusicIcon name="loader" size={12} spin /> {$t("music.scan.short", { done: lib.scan.done, total: lib.scan.total })}</span>
          {/if}
          {#if lib.analysis}
            <span><MusicIcon name="activity" size={12} /> {$t("music.tools.analyzing", { done: lib.analysis.done, total: lib.analysis.total })}</span>
          {/if}
        </div>
      {/if}
    </nav>

    <section class="m-main">
      <div class="topbar">
        <button class="nb" onclick={back} disabled={!ui.stack.length} aria-label={$t("music.back") as string}><MusicIcon name="back" size={16} /></button>
        <label class="search">
          <MusicIcon name="search" size={14} />
          <input bind:this={searchEl} type="search" value={ui.search} oninput={onSearch} placeholder={$t("music.search.placeholder") as string} aria-label={$t("music.search.placeholder") as string} />
          <kbd>/</kbd>
        </label>
        {#if ui.view.kind === "album" && albumTitle}<span class="crumb">{albumTitle}</span>{/if}
      </div>
      <div class="view" class:fill={["songs", "liked", "recent", "genre", "search", "nowplaying", "playlist"].includes(ui.view.kind)} data-agent-readiness={readiness}>
        {#if lib.loading && !lib.loaded}
          <div class="loading" role="status"><MusicIcon name="loader" size={22} spin /><span>{$t("settings.rebrand.state.loading")}</span></div>
        {:else if lib.error}
          <p class="err" role="alert">{lib.error}</p>
        {:else}
          {@const v = ui.view}
          {#if v.kind === "home"}<HomeView />
          {:else if v.kind === "songs"}<SongsView mode="all" />
          {:else if v.kind === "liked"}<SongsView mode="liked" />
          {:else if v.kind === "recent"}<SongsView mode="recent" />
          {:else if v.kind === "search"}<SongsView mode="search" query={v.q} />
          {:else if v.kind === "genre"}<SongsView mode="genre" genre={v.name} />
          {:else if v.kind === "albums"}<AlbumsView />
          {:else if v.kind === "album"}<AlbumView key={v.key} />
          {:else if v.kind === "artists"}<ArtistsView />
          {:else if v.kind === "artist"}<ArtistView name={v.name} />
          {:else if v.kind === "genres"}<GenresView />
          {:else if v.kind === "folders"}<FoldersView path={v.path ?? ""} />
          {:else if v.kind === "playlists"}<PlaylistsView />
          {:else if v.kind === "playlist"}<PlaylistView id={v.id} />
          {:else if v.kind === "radio"}<RadioView />
          {:else if v.kind === "download"}<DownloadView q={v.q ?? ""} />
          {:else if v.kind === "tools"}<ToolsView tab={v.tab ?? "analyze"} ids={v.ids ?? []} />
          {:else if v.kind === "stats"}<StatsView />
          {:else if v.kind === "nowplaying"}<NowPlayingView />
          {:else if v.kind === "settings"}<div id="music-settings-view" class="view-anchor" tabindex="-1"><SettingsView /></div>
          {/if}
        {/if}
      </div>
    </section>

    {#if ui.side}
      <aside class="m-side" aria-label={$t(`music.side.${ui.side}`) as string}>
        <div class="side-tabs mac-segmented">
          <button class="mac-segmented-btn" class:active={ui.side === "queue"} onclick={() => (ui.side = "queue")}>{$t("music.queue.title")}</button>
          <button class="mac-segmented-btn" class:active={ui.side === "lyrics"} onclick={() => (ui.side = "lyrics")}>{$t("music.lyrics.title")}</button>
          <button class="mac-segmented-btn" class:active={ui.side === "eq"} onclick={() => (ui.side = "eq")}>{$t("music.eq.short")}</button>
          <button class="close" onclick={() => (ui.side = null)} aria-label={$t("music.close") as string}><MusicIcon name="close" size={13} /></button>
        </div>
        <div class="side-body">
          {#if ui.side === "queue"}<QueuePanel />
          {:else if ui.side === "lyrics"}<LyricsPanel />
          {:else}<div id="music-eq-panel" class="view-anchor" tabindex="-1"><EqPanel /></div>{/if}
        </div>
      </aside>
    {/if}

    <div class="m-player"><PlayerBar {peaks} /></div>

    {#if dropping}
      <div class="dropzone"><MusicIcon name="download" size={28} /> {$t("music.drop_hint")}</div>
    {/if}
  </div>
{/if}

{#if ui.menu}
  <ContextMenu x={ui.menu.x} y={ui.menu.y} items={ui.menu.items} onclose={() => (ui.menu = null)} />
{/if}

{#if ui.tagEditor}
  <TagEditor ids={ui.tagEditor} onclose={() => (ui.tagEditor = null)} />
{/if}

{#if ui.playlistPicker}
  <Modal title={$t("music.playlist.new") as string} width={400} onclose={() => (ui.playlistPicker = null)}>
    <form
      class="nf"
      onsubmit={(e) => {
        e.preventDefault();
        void createFromPicker();
      }}
    >
      {#if ui.playlistPicker.length}<p class="dialog-body">{$t("music.playlist.with_songs", { count: ui.playlistPicker.length })}</p>{/if}
      <input class="input" bind:value={newName} placeholder={$t("music.playlist.untitled") as string} />
      <div class="dialog-actions">
        <button type="button" class="btn" onclick={() => (ui.playlistPicker = null)}>{$t("music.cancel")}</button>
        <button type="submit" class="btn btn-primary">{$t("music.create")}</button>
      </div>
    </form>
  </Modal>
{/if}

{#if ui.confirm}
  {@const c = ui.confirm}
  <Modal title={c.title} width={420} onclose={() => (ui.confirm = null)}>
    <p class="dialog-body">{c.body}</p>
    <div class="dialog-actions">
      <button class="btn" onclick={() => (ui.confirm = null)}>{$t("music.cancel")}</button>
      <button
        class="btn"
        class:btn-destructive={c.danger}
        class:btn-primary={!c.danger}
        onclick={() => {
          ui.confirm = null;
          c.run();
        }}>{c.ok}</button
      >
    </div>
  </Modal>
{/if}

<style>
  .music {
    position: relative;
    display: grid;
    grid-template-columns: 208px minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) auto;
    grid-template-areas:
      "nav main"
      "player player";
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }
  .music.with-side {
    grid-template-columns: 208px minmax(0, 1fr) 320px;
    grid-template-areas:
      "nav main side"
      "player player player";
  }
  .m-nav {
    grid-area: nav;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3) var(--space-2);
    border-right: var(--hairline) solid var(--content-border);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px var(--space-3);
    font-family: var(--font-display);
    font-weight: 750;
    font-size: var(--text-lg);
  }
  .m-nav ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .ni {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-sm);
    font-size: var(--text-base);
    color: var(--text-dim);
    cursor: pointer;
  }
  .ni span {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ni:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .ni.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
    font-weight: 600;
  }
  .ni:focus-visible,
  .sec-h:focus-visible,
  .sec-add:focus-visible,
  .nb:focus-visible {
    outline: var(--focus-ring);
  }
  .sec {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: var(--space-3);
    padding: 0 6px 0 10px;
  }
  .sec-h {
    all: unset;
    font-size: var(--text-xs);
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    cursor: pointer;
  }
  .sec-h.on,
  .sec-h:hover {
    color: var(--text);
  }
  .sec-add {
    all: unset;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .sec-add:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .pls {
    flex: 1;
    min-height: 40px;
  }
  .ni.pl {
    height: 28px;
    font-size: var(--text-sm);
  }
  .nav2 {
    border-top: var(--hairline) solid var(--content-border);
    padding-top: var(--space-2) !important;
    margin-top: var(--space-2) !important;
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: var(--space-2);
    padding: 8px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .status span {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .m-main {
    grid-area: main;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-2);
  }
  .nb {
    all: unset;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: var(--fill-1);
    cursor: pointer;
  }
  .nb:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 1 420px;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-muted);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .search:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .search input {
    all: unset;
    flex: 1;
    min-width: 0;
    font-size: var(--text-base);
    color: var(--text);
  }
  .search kbd {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    padding: 0 5px;
    border-radius: 4px;
    background: var(--fill-2);
  }
  .crumb {
    font-size: var(--text-sm);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .view {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-2) var(--space-5) 0;
  }
  .view.fill {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding-bottom: var(--space-3);
  }
  .view.fill > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .loading {
    display: grid;
    place-items: center;
    align-content: center;
    gap: var(--space-2);
    height: 60%;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .view-anchor {
    outline: none;
  }
  .view-anchor:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }
  .err {
    color: var(--danger);
  }
  .m-side {
    grid-area: side;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: var(--hairline) solid var(--content-border);
    background: color-mix(in srgb, var(--surface) 40%, var(--bg));
  }
  .side-tabs {
    display: flex;
    align-items: center;
    margin: var(--space-2);
  }
  .side-tabs .mac-segmented-btn {
    flex: 1;
  }
  .close {
    all: unset;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    margin-left: 4px;
    border-radius: 50%;
    cursor: pointer;
    color: var(--text-muted);
  }
  .close:hover {
    background: var(--fill-2);
  }
  .side-body {
    flex: 1;
    min-height: 0;
  }
  .side-body > .view-anchor {
    height: 100%;
  }
  .m-player {
    grid-area: player;
  }
  .dropzone {
    position: absolute;
    inset: 12px;
    z-index: 50;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--bg) 80%, transparent);
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--accent-hi);
    pointer-events: none;
  }
  .off {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-8) auto;
    text-align: center;
    color: var(--text-dim);
  }
  .off h2 {
    margin: 0;
    color: var(--text);
  }
  .nf {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  @media (max-width: 980px) {
    .music,
    .music.with-side {
      grid-template-columns: 56px minmax(0, 1fr);
      grid-template-areas:
        "nav main"
        "player player";
    }
    .music.with-side .m-side {
      position: absolute;
      right: 0;
      top: 0;
      bottom: 84px;
      width: min(340px, 90%);
      z-index: 20;
      box-shadow: -12px 0 30px rgba(0, 0, 0, 0.3);
    }
    .ni span,
    .brand span,
    .sec,
    .pls,
    .status {
      display: none;
    }
    .ni {
      justify-content: center;
      padding: 0;
    }
  }
</style>
