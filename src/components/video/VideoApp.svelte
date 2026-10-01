<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { page } from "$app/state";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import * as api from "$lib/video/api";
  import { addRoot, app, openPathInfo, refreshStatus, removeRoot, setIncognito, startListening, startScan } from "$lib/video/app.svelte";
  import { baseName, fmtTime, isLink, parentDir, VIDEO_EXT } from "$lib/video/format";
  import { closePlayer, openLink, openPlayer, openResolved, player } from "$lib/video/player.svelte";
  import type { MediaItem, RootKind, TranscriptHit } from "$lib/video/types";
  import { back, go, goRoot, ui } from "$lib/video/ui.svelte";
  import CourseList from "./course/CourseList.svelte";
  import CourseView from "./course/CourseView.svelte";
  import Player from "./player/Player.svelte";
  import ShortcutSheet from "./player/ShortcutSheet.svelte";
  import VIcon from "./player/VIcon.svelte";
  import Sidebar from "./Sidebar.svelte";
  import SourcesView from "./sources/SourcesView.svelte";
  import StudyView from "./study/StudyView.svelte";
  import AddonsView from "./views/AddonsView.svelte";
  import AnimeView from "./views/AnimeView.svelte";
  import FoldersView from "./views/FoldersView.svelte";
  import HistoryView from "./views/HistoryView.svelte";
  import HomeView from "./views/HomeView.svelte";
  import JellyfinView from "./views/JellyfinView.svelte";
  import LiveView from "./views/LiveView.svelte";
  import MoviesView from "./views/MoviesView.svelte";
  import ShowsView from "./views/ShowsView.svelte";
  import ShowView from "./views/ShowView.svelte";

  const WELCOMED_KEY = "omniget.video.welcomed";

  let enabled = $derived(getSettings()?.video?.enabled ?? true);
  let dropping = $state(false);
  let searchEl: HTMLInputElement | undefined = $state();
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let linkOpen = $state(false);
  let linkText = $state("");
  let addOpen = $state(false);
  let settingsOpen = $state(false);
  let keysOpen = $state(false);
  let droppedFolder = $state<string | null>(null);
  let welcomed = $state(readWelcomed());

  function readWelcomed(): boolean {
    try {
      return localStorage.getItem(WELCOMED_KEY) === "1";
    } catch {
      return false;
    }
  }

  function skipWelcome() {
    welcomed = true;
    try {
      localStorage.setItem(WELCOMED_KEY, "1");
    } catch {
    }
  }

  let roots = $derived(app.status?.roots ?? []);
  let showWelcome = $derived(app.loaded && !roots.length && !welcomed && ui.view.name === "home");

  let lastDeepLink = "";
  $effect(() => {
    const key = page.url.searchParams.get("key");
    if (!key) return;
    const t = Number(page.url.searchParams.get("t") ?? "0") || 0;
    const sig = `${key}@${t}`;
    if (sig === lastDeepLink) return;
    lastDeepLink = sig;
    api
      .sourceForKey(key)
      .then((source) => openPlayer(source, { startMs: Math.round(t * 1000) }))
      .catch((e) => showToast("error", String(e)));
  });

  onMount(() => {
    void refreshStatus();
    let stopEvents: (() => void) | null = null;
    let unlistenDrop: (() => void) | null = null;
    let disposed = false;
    void startListening().then((stop) => {
      if (disposed) stop();
      else stopEvents = stop;
    });
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter") dropping = true;
          else if (p.type === "leave") dropping = false;
          else if (p.type === "drop") {
            dropping = false;
            if (p.paths?.length) void handleDropped(p.paths);
          }
        })
        .then((fn) => {
          if (disposed) fn();
          else unlistenDrop = fn;
        })
        .catch(() => {});
    } catch {
    }
    return () => {
      disposed = true;
      stopEvents?.();
      unlistenDrop?.();
    };
  });

  onDestroy(() => {
    if (player.open) closePlayer();
  });

  async function handleDropped(paths: string[]) {
    const files = paths.filter((p) => VIDEO_EXT.test(p));
    const folders = paths.filter((p) => !/\.[a-z0-9]{2,5}$/i.test(baseName(p)));
    if (files.length) await playFile(files[0]);
    if (folders.length === 1 && !files.length) droppedFolder = folders[0];
    else for (const f of folders) await addFolderPath(f, "library");
  }

  async function playFile(path: string) {
    try {
      const info = await openPathInfo(path);
      if (info) {
        await openResolved(info);
        return;
      }
      await addRoot(parentDir(path), "library");
      showToast("info", $t("video.drop.added_folder", { name: baseName(parentDir(path)) }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function addFolderPath(path: string, kind: RootKind) {
    try {
      if (kind === "course") {
        const c = await api.courseImport(path);
        await refreshStatus();
        goRoot({ name: "course", id: c.id });
      } else {
        await addRoot(path, kind);
      }
      showToast("success", $t("video.drop.added", { name: baseName(path) }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function pickFolder(kind: RootKind) {
    addOpen = false;
    let path: string | string[] | null = null;
    try {
      path = await openDialog({ directory: true, multiple: false, title: $t(kind === "course" ? "video.courses.import_title" : "video.library.add_title") as string });
    } catch {
      return;
    }
    if (typeof path === "string") await addFolderPath(path, kind);
  }

  async function pickFile() {
    addOpen = false;
    try {
      const path = await openDialog({ multiple: false, filters: [{ name: "Video", extensions: ["mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "ts", "m2ts", "mpg", "flv"] }] });
      if (typeof path === "string") await playFile(path);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function playLink(url: string) {
    linkOpen = false;
    linkText = "";
    if (!isLink(url)) {
      showToast("error", $t("video.link.invalid") as string);
      return;
    }
    void openLink(url.trim());
  }

  async function rescan() {
    try {
      await startScan();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function typing(e: Event): boolean {
    const el = e.target as HTMLElement | null;
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
  }

  function onPaste(e: ClipboardEvent) {
    if (!enabled || typing(e) || player.open) return;
    const text = e.clipboardData?.getData("text/plain")?.trim() ?? "";
    if (isLink(text)) {
      e.preventDefault();
      playLink(text);
    }
  }

  function keys(e: KeyboardEvent) {
    if (!enabled || player.open || typing(e) || e.metaKey || e.ctrlKey || e.altKey) return;
    if (settingsOpen || keysOpen || droppedFolder) return;
    if (e.key === "/") {
      e.preventDefault();
      searchEl?.focus();
    } else if (e.key === "Escape" && ui.stack.length) back();
  }

  function onSearch(e: Event) {
    const q = (e.currentTarget as HTMLInputElement).value;
    ui.search = q;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      if (q.trim()) {
        if (ui.view.name === "search") ui.view = { name: "search", q };
        else go({ name: "search", q });
      } else if (ui.view.name === "search") back();
    }, 200);
  }

  let found = $state<{ q: string; items: MediaItem[]; hits: TranscriptHit[]; loading: boolean }>({ q: "", items: [], hits: [], loading: false });

  $effect(() => {
    const v = ui.view;
    if (v.name !== "search") return;
    const q = v.q.trim();
    if (!q || q === found.q) return;
    found = { q, items: [], hits: [], loading: true };
    Promise.all([api.items({ search: q, limit: 60 }).catch(() => []), api.transcriptSearch(q).catch(() => [])]).then(([items, hits]) => {
      if (found.q === q) found = { q, items, hits, loading: false };
    });
  });
</script>

<svelte:window onkeydown={keys} onpaste={onPaste} />

{#if !enabled}
  <div class="off">
    <img src="/icons/menu/video-128.webp" alt="" width="56" height="56" />
    <h2>{$t("video.disabled_title")}</h2>
    <p>{$t("superpowers.disabled_hint")}</p>
    <a class="btn btn-primary" href="/settings?tab=advanced">{$t("superpowers.open_settings")}</a>
  </div>
{:else}
  <div class="watch">
    <Sidebar onsettings={() => (settingsOpen = true)} />

    <section class="main">
      <div class="topbar">
        <button class="nb" onclick={back} disabled={!ui.stack.length} aria-label={$t("video.back") as string}><VIcon name="back" size={16} /></button>
        <label class="search">
          <VIcon name="search" size={14} />
          <input bind:this={searchEl} type="search" value={ui.search} oninput={onSearch} placeholder={$t("video.search.placeholder") as string} aria-label={$t("video.search.placeholder") as string} />
          <kbd>/</kbd>
        </label>
        <span class="sp"></span>
        <div class="pop-anchor">
          <button class="btn btn-sm" onclick={() => (linkOpen = !linkOpen)} aria-expanded={linkOpen}><VIcon name="link" size={13} /> {$t("video.link.open")}</button>
          {#if linkOpen}
            <form
              class="pop"
              onsubmit={(e) => {
                e.preventDefault();
                playLink(linkText);
              }}
            >
              <!-- svelte-ignore a11y_autofocus -->
              <input class="input" bind:value={linkText} placeholder="https://…" autofocus onkeydown={(e) => e.key === "Escape" && (linkOpen = false)} aria-label={$t("video.link.open") as string} />
              <button type="submit" class="btn btn-sm btn-primary" disabled={!linkText.trim()}>{$t("video.link.play")}</button>
              <p class="hint">{$t("video.link.hint")}</p>
            </form>
          {/if}
        </div>
        <div class="pop-anchor">
          <button class="btn btn-sm" onclick={() => (addOpen = !addOpen)} aria-expanded={addOpen}><VIcon name="plus" size={13} /> {$t("video.library.add")}</button>
          {#if addOpen}
            <div class="pop menu" role="menu">
              <button role="menuitem" onclick={() => pickFolder("library")}><img src="/icons/video/folders-32.webp" alt="" width="16" height="16" /> {$t("video.library.add_folder")}</button>
              <button role="menuitem" onclick={() => pickFolder("course")}><img src="/icons/video/courses-32.webp" alt="" width="16" height="16" /> {$t("video.library.add_course")}</button>
              <button role="menuitem" onclick={pickFile}><img src="/icons/video/video-32.webp" alt="" width="16" height="16" /> {$t("video.library.open_file")}</button>
            </div>
          {/if}
        </div>
        <button class="nb" onclick={rescan} disabled={!!app.scan || !roots.length} aria-label={$t("video.scan.button") as string} title={$t("video.scan.button") as string}>
          <VIcon name="refresh" size={15} spin={!!app.scan} />
        </button>
      </div>

      <div class="view">
        {#if !app.loaded}
          <div class="loading"><VIcon name="loader" size={22} spin /></div>
        {:else if showWelcome}
          <div class="welcome">
            <img src="/icons/video/folders-128.webp" alt="" width="96" height="96" />
            <h1>{$t("video.welcome.title")}</h1>
            <p>{$t("video.welcome.body")}</p>
            <div class="wa">
              <button class="btn btn-primary btn-lg" onclick={() => pickFolder("library")}>{$t("video.welcome.pick_folder")}</button>
              <button class="btn btn-lg" onclick={() => pickFolder("course")}>{$t("video.welcome.pick_course")}</button>
            </div>
            <form
              class="wl"
              onsubmit={(e) => {
                e.preventDefault();
                playLink(linkText);
              }}
            >
              <VIcon name="link" size={14} />
              <input bind:value={linkText} placeholder={$t("video.welcome.link_placeholder") as string} aria-label={$t("video.link.open") as string} />
              <button type="submit" class="btn btn-sm" disabled={!linkText.trim()}>{$t("video.link.play")}</button>
            </form>
            <p class="small">{$t("video.welcome.drop_hint")}</p>
            <div class="wa2">
              <button class="link" onclick={() => goRoot({ name: "sources" })}>{$t("video.welcome.connect_server")}</button>
              <span>·</span>
              <button class="link" onclick={skipWelcome}>{$t("video.welcome.skip")}</button>
            </div>
            {#if app.status && !app.status.ffmpeg}<p class="warn">{$t("video.welcome.no_ffmpeg")}</p>{/if}
          </div>
        {:else}
          {@const v = ui.view}
          {#if v.name === "home"}<HomeView />
          {:else if v.name === "movies"}<MoviesView />
          {:else if v.name === "shows"}<ShowsView />
          {:else if v.name === "show"}<ShowView show={v.show} />
          {:else if v.name === "courses"}<CourseList />
          {:else if v.name === "course"}{#key v.id}<CourseView id={v.id} />{/key}
          {:else if v.name === "folders"}<FoldersView path={v.path ?? null} />
          {:else if v.name === "history"}<HistoryView />
          {:else if v.name === "live"}<LiveView group={v.group ?? null} />
          {:else if v.name === "anime"}<AnimeView id={v.id ?? null} />
          {:else if v.name === "sources"}<SourcesView />
          {:else if v.name === "jellyfin"}{#key v.source}<JellyfinView source={v.source} parent={v.parent ?? null} title={v.title ?? null} />{/key}
          {:else if v.name === "addons"}<AddonsView addon={v.addon ?? null} type={v.type ?? null} id={v.id ?? null} />
          {:else if v.name === "study"}<StudyView />
          {:else if v.name === "search"}
            <div class="results">
              <h1>{$t("video.search.title", { query: v.q })}</h1>
              {#if found.loading}
                <div class="loading"><VIcon name="loader" size={20} spin /></div>
              {:else if !found.items.length && !found.hits.length}
                <p class="none">{$t("video.search.none")}</p>
              {:else}
                {#if found.items.length}
                  <h2>{$t("video.search.videos")}</h2>
                  <ul>
                    {#each found.items as it (it.id)}
                      {@const thumb = api.serverUrl(it.thumb)}
                      <li>
                        <button class="res" onclick={() => openPlayer({ kind: "local", id: it.id }, { title: it.title })}>
                          <span class="th">{#if thumb}<img src={thumb} alt="" loading="lazy" />{/if}</span>
                          <span class="rt">
                            <strong>{it.title}</strong>
                            <small>{it.show ?? it.module ?? baseName(it.folder)}</small>
                          </span>
                          {#if it.duration_ms}<span class="du">{fmtTime(it.duration_ms)}</span>{/if}
                        </button>
                      </li>
                    {/each}
                  </ul>
                {/if}
                {#if found.hits.length}
                  <h2>{$t("video.search.in_transcripts")}</h2>
                  <ul>
                    {#each found.hits as h, i (`${h.key}:${h.start_ms}:${i}`)}
                      <li>
                        <button
                          class="res"
                          disabled={h.media_id == null}
                          onclick={() => h.media_id != null && openPlayer({ kind: "local", id: h.media_id }, { title: h.title, startMs: h.start_ms })}
                        >
                          <span class="ts">{fmtTime(h.start_ms)}</span>
                          <span class="rt">
                            <strong>{h.text}</strong>
                            <small>{h.title}</small>
                          </span>
                        </button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              {/if}
            </div>
          {/if}
        {/if}
      </div>
    </section>

    {#if dropping}
      <div class="dropzone"><VIcon name="export" size={28} /> {$t("video.drop.hint")}</div>
    {/if}
  </div>

  <Player />
{/if}

{#if droppedFolder}
  {@const path = droppedFolder}
  <div class="dialog-backdrop" role="presentation" onclick={() => (droppedFolder = null)}>
    <div class="dialog-card" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (droppedFolder = null)}>
      <h3 class="dialog-title">{$t("video.drop.folder_title", { name: baseName(path) })}</h3>
      <p class="dialog-body">{$t("video.drop.folder_body")}</p>
      <div class="dialog-actions">
        <button class="btn" onclick={() => (droppedFolder = null)}>{$t("video.cancel")}</button>
        <button
          class="btn"
          onclick={() => {
            droppedFolder = null;
            void addFolderPath(path, "course");
          }}>{$t("video.library.add_course")}</button
        >
        <button
          class="btn btn-primary"
          onclick={() => {
            droppedFolder = null;
            void addFolderPath(path, "library");
          }}>{$t("video.library.add_folder")}</button
        >
      </div>
    </div>
  </div>
{/if}

{#if settingsOpen}
  <div class="dialog-backdrop" role="presentation" onclick={() => (settingsOpen = false)}>
    <div class="dialog-card settings" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (settingsOpen = false)}>
      <h3 class="dialog-title">{$t("video.settings.title")}</h3>

      <h4>{$t("video.settings.folders")}</h4>
      {#if roots.length}
        <ul class="roots">
          {#each roots as r (r.path)}
            <li>
              <img src="/icons/video/{r.kind === 'course' ? 'courses' : 'folders'}-32.webp" alt="" width="18" height="18" />
              <span class="rp" title={r.path}>{r.path}</span>
              {#if !r.online}<span class="badge badge-danger">{$t("video.settings.offline")}</span>{/if}
              <button class="ic" onclick={() => removeRoot(r.path).catch((e) => showToast("error", String(e)))} aria-label={$t("video.settings.remove_folder") as string} title={$t("video.settings.remove_folder_hint") as string}>
                <VIcon name="close" size={13} />
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="dialog-body">{$t("video.settings.no_folders")}</p>
      {/if}
      <div class="row">
        <button class="btn btn-sm" onclick={() => pickFolder("library")}><VIcon name="plus" size={12} /> {$t("video.library.add_folder")}</button>
        <button class="btn btn-sm" onclick={() => pickFolder("course")}><VIcon name="plus" size={12} /> {$t("video.library.add_course")}</button>
        <button class="btn btn-sm" onclick={rescan} disabled={!!app.scan || !roots.length}><VIcon name="refresh" size={12} /> {$t("video.scan.button")}</button>
      </div>

      <h4>{$t("video.settings.privacy")}</h4>
      <label class="tg">
        <input type="checkbox" checked={app.incognito} onchange={(e) => setIncognito((e.currentTarget as HTMLInputElement).checked)} />
        <span>
          <strong>{$t("video.nav.incognito")}</strong>
          <small>{$t("video.settings.incognito_desc")}</small>
        </span>
      </label>

      <h4>{$t("video.settings.player")}</h4>
      <div class="row">
        <button class="btn btn-sm" onclick={() => (keysOpen = true)}><VIcon name="keyboard" size={12} /> {$t("video.keys.title")}</button>
        <a class="btn btn-sm" href="/settings?tab=advanced">{$t("video.settings.app_settings")}</a>
      </div>
      {#if app.status}
        <p class="status-line">
          <i class:ok={app.status.ffmpeg}></i>
          {app.status.ffmpeg ? $t("video.settings.ffmpeg_ok") : $t("video.settings.ffmpeg_missing")}
        </p>
      {/if}

      <div class="dialog-actions">
        <button class="btn btn-primary" onclick={() => (settingsOpen = false)}>{$t("video.done")}</button>
      </div>
    </div>
  </div>
{/if}

{#if keysOpen}
  <div class="keys-host"><ShortcutSheet onclose={() => (keysOpen = false)} /></div>
{/if}

<style>
  .watch {
    position: relative;
    display: grid;
    grid-template-columns: 216px minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }
  .main {
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
  .sp {
    flex: 1;
  }
  .nb {
    all: unset;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--fill-1);
    cursor: pointer;
  }
  .nb:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .nb:focus-visible {
    outline: var(--focus-ring);
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
  .pop-anchor {
    position: relative;
  }
  .pop {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 30;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    width: 340px;
    padding: 10px;
    border-radius: var(--radius-lg);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    box-shadow: var(--elev-3);
  }
  .pop .input {
    flex: 1;
    min-width: 0;
  }
  .pop .hint {
    width: 100%;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .pop.menu {
    flex-direction: column;
    width: 230px;
    padding: 5px;
    gap: 0;
  }
  .pop.menu button {
    all: unset;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .pop.menu button:hover,
  .pop.menu button:focus-visible {
    background: var(--accent);
    color: var(--on-accent);
  }
  .view {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-2) var(--space-5) 0;
  }
  .loading {
    display: grid;
    place-items: center;
    height: 50%;
    color: var(--text-muted);
  }
  .welcome {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    max-width: 520px;
    margin: var(--space-8) auto;
    text-align: center;
  }
  .welcome h1 {
    margin: var(--space-2) 0 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    letter-spacing: var(--track-tight);
  }
  .welcome p {
    margin: 0;
    color: var(--text-dim);
  }
  .wa {
    display: flex;
    gap: 8px;
    margin: var(--space-3) 0;
    flex-wrap: wrap;
    justify-content: center;
  }
  .wl {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 38px;
    padding: 0 6px 0 12px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-muted);
  }
  .wl:focus-within {
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .wl input {
    all: unset;
    flex: 1;
    min-width: 0;
    text-align: left;
    color: var(--text);
    font-size: var(--text-base);
  }
  .welcome .small {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .wa2 {
    display: flex;
    gap: 8px;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .link {
    all: unset;
    color: var(--accent-hi);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  .link:focus-visible {
    outline: var(--focus-ring);
  }
  .warn {
    margin-top: var(--space-3) !important;
    color: var(--warning) !important;
    font-size: var(--text-sm);
  }
  .results {
    max-width: 900px;
    padding-bottom: var(--space-6);
  }
  .results h1 {
    margin: 0 0 var(--space-3);
    font-family: var(--font-display);
    font-size: var(--text-xl);
  }
  .results h2 {
    margin: var(--space-4) 0 var(--space-2);
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
    color: var(--text-muted);
  }
  .results ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .res {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 6px 8px;
    border-radius: var(--radius-md);
    cursor: pointer;
  }
  .res:hover:not(:disabled) {
    background: var(--fill-1);
  }
  .res:focus-visible {
    outline: var(--focus-ring);
  }
  .th {
    width: 96px;
    aspect-ratio: 16 / 9;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    overflow: hidden;
  }
  .th img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .rt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .rt strong {
    font-weight: 550;
    font-size: var(--text-base);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rt small {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .du,
  .ts {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .ts {
    min-width: 48px;
    color: var(--accent-hi);
  }
  .none {
    color: var(--text-dim);
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
  .settings {
    width: min(560px, 100%);
  }
  .settings h4 {
    margin: var(--space-3) 0 var(--space-1);
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
    color: var(--text-muted);
  }
  .roots {
    list-style: none;
    margin: 0 0 var(--space-2);
    padding: 0;
  }
  .roots li {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
  }
  .rp {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ic {
    all: unset;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .ic:hover {
    background: var(--fill-2);
    color: var(--danger);
  }
  .ic:focus-visible {
    outline: var(--focus-ring);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .tg {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    cursor: pointer;
  }
  .tg input {
    margin-top: 3px;
    accent-color: var(--accent);
  }
  .tg span {
    display: flex;
    flex-direction: column;
    font-size: var(--text-sm);
  }
  .tg small {
    color: var(--text-dim);
  }
  .status-line {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: var(--space-2) 0 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .status-line i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--warning);
  }
  .status-line i.ok {
    background: var(--success);
  }
  .keys-host {
    --pl-ink: #fff;
    --pl-shade: #000;
    position: fixed;
    inset: 0;
    z-index: 950;
  }
  @media (max-width: 980px) {
    .watch {
      grid-template-columns: 56px minmax(0, 1fr);
    }
  }
</style>
