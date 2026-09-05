<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { goto, replaceState } from "$app/navigation";
  import { page } from "$app/stores";
  import { pluginInvoke } from "$lib/plugin-invoke";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";
  import PlayerShell from "$lib/study-components/player/PlayerShell.svelte";
  import ConfirmDialog from "$lib/study-components/ConfirmDialog.svelte";

  type BrowseFolder = {
    path: string;
    name: string;
    video_count: number;
    preview_videos?: string[];
    latest_mtime?: number | null;
  };
  type BrowseVideo = {
    path: string;
    name: string;
    stem: string;
    size: number;
    subtitle_path: string | null;
    thumbnail_path?: string | null;
  };
  type BrowseDocument = {
    path: string;
    name: string;
    ext: string;
    size: number;
  };
  type BrowseCrumb = { path: string; name: string };
  type BrowseData = {
    path: string;
    parent: string | null;
    is_root_list: boolean;
    folders: BrowseFolder[];
    videos: BrowseVideo[];
    documents: BrowseDocument[];
    breadcrumb: BrowseCrumb[];
  };

  const currentPath = $derived($page.url.searchParams.get("path") ?? "");
  const currentName = $derived(
    $page.url.searchParams.get("name") ?? deriveName(currentPath),
  );
  const currentSubtitle = $derived(
    $page.url.searchParams.get("subtitle") ?? "",
  );

  let listingPath = $state("");
  let listing = $state<BrowseData | null>(null);
  let listingLoading = $state(false);
  let videoErr = $state(false);

  let returnUrl = $state(
    (typeof window !== "undefined"
      ? sessionStorage.getItem("study.library.return_url")
      : null) ?? "/study/library?mode=browse",
  );

  const videoSrc = $derived(currentPath ? convertFileSrc(currentPath) : "");
  const subtitleSrc = $derived(
    currentSubtitle ? convertFileSrc(currentSubtitle) : "",
  );

  function deriveName(p: string): string {
    if (!p) return "";
    const parts = p.replace(/\\/g, "/").split("/");
    return parts[parts.length - 1] || p;
  }

  function parentDir(p: string): string {
    if (!p) return "";
    const norm = p.replace(/\\/g, "/");
    const idx = norm.lastIndexOf("/");
    if (idx <= 0) return "";
    return p.slice(0, idx);
  }

  async function loadListing(path: string) {
    listingLoading = true;
    try {
      listing = await pluginInvoke<BrowseData>("study", "study:browse:list", {
        path,
      });
      listingPath = listing.path;
    } catch (e) {
      console.error("browse failed", e);
      listing = null;
    } finally {
      listingLoading = false;
    }
  }

  $effect(() => {
    videoErr = false;
    const p = currentPath;
    if (!p) return;
    const parent = parentDir(p);
    if (parent && parent !== listingPath) {
      loadListing(parent);
    }
  });

  onMount(() => {
    const p = $page.url.searchParams.get("path") ?? "";
    if (p) {
      const parent = parentDir(p);
      if (parent) loadListing(parent);
    }
  });

  function playVideo(v: BrowseVideo) {
    const params = new URLSearchParams({ path: v.path, name: v.name });
    if (v.subtitle_path) params.set("subtitle", v.subtitle_path);
    goto(`/study/watch?${params.toString()}`, {
      replaceState: true,
      noScroll: true,
    });
  }

  function navigateFolder(path: string) {
    loadListing(path);
  }

  function navigateCrumb(path: string) {
    loadListing(path);
  }

  async function openDocument(d: BrowseDocument) {
    try {
      await invoke("open_path_default", { path: d.path });
    } catch (e) {
      console.error("open doc failed", e);
    }
  }

  function isTypingTarget(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    const tag = target.tagName;
    return (
      tag === "INPUT" ||
      tag === "TEXTAREA" ||
      tag === "SELECT" ||
      target.isContentEditable
    );
  }

  $effect(() => {
    if (typeof document === "undefined") return;
    function onKey(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      if (isTypingTarget(e.target)) return;
      if (document.fullscreenElement) return;
      e.preventDefault();
      back();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });

  function back() {
    if (typeof window !== "undefined") {
      const folderName =
        listing?.path
          ?.replace(/\\/g, "/")
          .split("/")
          .filter(Boolean)
          .pop() ?? "";
      if (folderName) {
        try {
          sessionStorage.setItem(
            "study.library.return_toast",
            JSON.stringify({ folderName }),
          );
        } catch {
          /* ignore */
        }
      }
      const stored = sessionStorage.getItem("study.library.return_url");
      if (stored) {
        sessionStorage.removeItem("study.library.return_url");
        goto(stored);
        return;
      }
    }
    if (history.length > 1) {
      history.back();
    } else {
      goto("/study/library");
    }
  }

  function onVideoError() {
    videoErr = true;
  }

  function fmtBytes(n: number): string {
    if (!n) return "";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i += 1;
    }
    return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
  }

  type CtxMenuTarget =
    | { kind: "folder"; path: string; name: string }
    | { kind: "video"; path: string; name: string }
    | { kind: "doc"; path: string; name: string }
    | { kind: "current"; path: string; name: string };

  let contextMenu = $state<{ x: number; y: number; target: CtxMenuTarget } | null>(null);
  let renamingPath = $state<string | null>(null);
  let renameValue = $state("");
  let confirmDeleteOpen = $state(false);
  let pendingDelete = $state<CtxMenuTarget | null>(null);
  let headerMenuOpen = $state(false);

  function openContextMenu(event: MouseEvent, target: CtxMenuTarget) {
    event.preventDefault();
    event.stopPropagation();
    contextMenu = { x: event.clientX, y: event.clientY, target };
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  $effect(() => {
    if (!contextMenu) return;
    function onDoc(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (t && t.closest(".ctx-menu")) return;
      closeContextMenu();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") closeContextMenu();
    }
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  });

  $effect(() => {
    if (!headerMenuOpen) return;
    function onDoc(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (t && t.closest(".header-menu, .header-menu-btn")) return;
      headerMenuOpen = false;
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") headerMenuOpen = false;
    }
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  });

  function isTypingFsTarget(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    const tag = target.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || target.isContentEditable;
  }

  async function showInExplorer(path: string) {
    try {
      await pluginInvoke("study", "study:shell:show_in_folder", { path });
    } catch (e) {
      console.error("show_in_folder failed", e);
    }
    closeContextMenu();
    headerMenuOpen = false;
  }

  function startRename(target: CtxMenuTarget) {
    renamingPath = target.path;
    renameValue = target.name;
    closeContextMenu();
    headerMenuOpen = false;
  }

  async function confirmRename(targetPath: string) {
    if (renamingPath !== targetPath) return;
    const trimmed = renameValue.trim();
    if (!trimmed) {
      cancelRename();
      return;
    }
    const wasCurrent = targetPath === currentPath;
    try {
      const res = await pluginInvoke<{ new_path: string; new_name: string }>(
        "study",
        "study:browse:rename",
        { path: targetPath, newName: trimmed },
      );
      renamingPath = null;
      renameValue = "";
      if (wasCurrent && typeof window !== "undefined") {
        const params = new URLSearchParams(window.location.search);
        params.set("path", res.new_path);
        params.set("name", res.new_name);
        try {
          replaceState(`${window.location.pathname}?${params.toString()}`, {});
        } catch {
          window.history.replaceState(null, "", `${window.location.pathname}?${params.toString()}`);
        }
      }
      await loadListing(listingPath);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    }
  }

  function cancelRename() {
    renamingPath = null;
    renameValue = "";
  }

  function startDelete(target: CtxMenuTarget) {
    pendingDelete = target;
    confirmDeleteOpen = true;
    closeContextMenu();
    headerMenuOpen = false;
  }

  async function performDelete() {
    if (!pendingDelete) return;
    const target = pendingDelete;
    const wasCurrent = target.path === currentPath || target.kind === "current";
    try {
      await pluginInvoke("study", "study:browse:delete", { path: target.path });
      showToast(
        "success",
        $t("study.library.delete_toast_done", { name: target.name }) as string,
      );
      if (wasCurrent) {
        back();
      } else {
        await loadListing(listingPath);
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      showToast("error", msg);
    } finally {
      pendingDelete = null;
      confirmDeleteOpen = false;
    }
  }

  function autofocusOnRender(node: HTMLInputElement) {
    node.focus();
    if (node.value.length > 0) node.select();
  }
</script>

<section class="watch-page">
  <header class="head">
    <button type="button" class="back-btn" onclick={back}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M15 18l-6-6 6-6"></path>
      </svg>
      <span>{$t("study.library.watch_back")}</span>
    </button>
    {#if currentName}
      <h1 class="title">{currentName}</h1>
    {/if}
    {#if currentPath}
      <div class="header-menu-wrap">
        <button
          type="button"
          class="header-menu-btn"
          onclick={() => (headerMenuOpen = !headerMenuOpen)}
          aria-haspopup="menu"
          aria-expanded={headerMenuOpen}
          aria-label={$t("study.library.ctx_open") as string}
        >
          <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true">
            <circle cx="5" cy="12" r="2"/>
            <circle cx="12" cy="12" r="2"/>
            <circle cx="19" cy="12" r="2"/>
          </svg>
        </button>
        {#if headerMenuOpen}
          <div class="header-menu" role="menu">
            <button
              type="button"
              class="ctx-item"
              role="menuitem"
              onclick={() => showInExplorer(currentPath)}
            >{$t("study.library.ctx_show_in_explorer")}</button>
            <button
              type="button"
              class="ctx-item"
              role="menuitem"
              onclick={() =>
                startRename({ kind: "current", path: currentPath, name: currentName })}
            >{$t("study.library.ctx_rename")}</button>
            <button
              type="button"
              class="ctx-item danger"
              role="menuitem"
              onclick={() =>
                startDelete({ kind: "current", path: currentPath, name: currentName })}
            >{$t("study.library.ctx_delete")}</button>
          </div>
        {/if}
      </div>
    {/if}
  </header>

  <div class="layout">
    <aside class="playlist">
      {#if listing && (listing.breadcrumb.length > 0 || !listing.is_root_list)}
        <nav class="crumbs" aria-label="breadcrumb">
          <button type="button" class="crumb" onclick={() => navigateCrumb("")}>
            {$t("study.library.browse_breadcrumb_root")}
          </button>
          {#each listing.breadcrumb as cr, i (cr.path)}
            <span class="crumb-sep" aria-hidden="true">›</span>
            {#if i < listing.breadcrumb.length - 1}
              <button
                type="button"
                class="crumb"
                onclick={() => navigateCrumb(cr.path)}
              >
                {cr.name}
              </button>
            {:else}
              <span class="crumb-current">{cr.name}</span>
            {/if}
          {/each}
        </nav>
      {/if}

      {#if listingLoading && !listing}
        <p class="muted small">{$t("study.common.loading")}</p>
      {:else if !listing}
        <p class="muted small">{$t("study.library.browse_empty_roots")}</p>
      {:else}
        <ul class="items">
          {#if listing.is_root_list}
            {#each listing.folders as f (f.path)}
              <li>
                <button
                  type="button"
                  class="item folder"
                  onclick={() => navigateFolder(f.path)}
                >
                  <span class="ico">
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
                      <path d="M3 7v13h18V9h-9l-2-2H3z"/>
                    </svg>
                  </span>
                  <span class="name">{f.name}</span>
                </button>
              </li>
            {/each}
          {:else}
            {#each listing.folders as f (f.path)}
              {@const isRenaming = renamingPath === f.path}
              <li>
                <button
                  type="button"
                  class="item folder"
                  class:renaming={isRenaming}
                  onclick={() => { if (!isRenaming) navigateFolder(f.path); }}
                  oncontextmenu={(e) =>
                    openContextMenu(e, { kind: "folder", path: f.path, name: f.name })}
                >
                  <span class="ico">
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true">
                      <path d="M3 7v13h18V9h-9l-2-2H3z"/>
                    </svg>
                  </span>
                  {#if isRenaming}
                    <input
                      class="rename-input"
                      type="text"
                      bind:value={renameValue}
                      onclick={(e) => e.stopPropagation()}
                      onkeydown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") confirmRename(f.path);
                        else if (e.key === "Escape") cancelRename();
                      }}
                      onblur={() => confirmRename(f.path)}
                      use:autofocusOnRender
                    />
                  {:else}
                    <span class="name">{f.name}</span>
                    {#if f.video_count > 0}
                      <span class="count mono">{f.video_count}</span>
                    {/if}
                  {/if}
                </button>
              </li>
            {/each}
            {#each listing.videos as v (v.path)}
              {@const isRenaming = renamingPath === v.path}
              <li>
                <button
                  type="button"
                  class="item video"
                  class:active={v.path === currentPath}
                  class:renaming={isRenaming}
                  onclick={() => { if (!isRenaming) playVideo(v); }}
                  oncontextmenu={(e) =>
                    openContextMenu(e, { kind: "video", path: v.path, name: v.name })}
                >
                  <span class="ico">
                    {#if v.path === currentPath}
                      <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor" aria-hidden="true">
                        <circle cx="12" cy="12" r="4"/>
                      </svg>
                    {:else}
                      <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor" aria-hidden="true">
                        <path d="M8 5v14l11-7z"/>
                      </svg>
                    {/if}
                  </span>
                  {#if isRenaming}
                    <input
                      class="rename-input"
                      type="text"
                      bind:value={renameValue}
                      onclick={(e) => e.stopPropagation()}
                      onkeydown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") confirmRename(v.path);
                        else if (e.key === "Escape") cancelRename();
                      }}
                      onblur={() => confirmRename(v.path)}
                      use:autofocusOnRender
                    />
                  {:else}
                    <span class="name">{v.name}</span>
                    {#if v.size}
                      <span class="count mono">{fmtBytes(v.size)}</span>
                    {/if}
                  {/if}
                </button>
              </li>
            {/each}
            {#each listing.documents ?? [] as d (d.path)}
              {@const isRenaming = renamingPath === d.path}
              <li>
                <button
                  type="button"
                  class="item doc"
                  class:renaming={isRenaming}
                  onclick={() => { if (!isRenaming) openDocument(d); }}
                  oncontextmenu={(e) =>
                    openContextMenu(e, { kind: "doc", path: d.path, name: d.name })}
                >
                  <span class="ico">
                    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
                      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                      <path d="M14 2v6h6"/>
                    </svg>
                  </span>
                  {#if isRenaming}
                    <input
                      class="rename-input"
                      type="text"
                      bind:value={renameValue}
                      onclick={(e) => e.stopPropagation()}
                      onkeydown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") confirmRename(d.path);
                        else if (e.key === "Escape") cancelRename();
                      }}
                      onblur={() => confirmRename(d.path)}
                      use:autofocusOnRender
                    />
                  {:else}
                    <span class="name">{d.name}</span>
                    <span class="count mono">{d.ext.toUpperCase()}</span>
                  {/if}
                </button>
              </li>
            {/each}
          {/if}
        </ul>
      {/if}
    </aside>

    <div class="player-wrap">
      {#if !currentPath}
        <div class="state">
          <p class="muted">{$t("study.library.watch_not_found")}</p>
        </div>
      {:else if videoErr}
        <div class="state">
          <p class="error">{$t("study.library.watch_file_missing")}</p>
          <p class="muted small mono">{currentPath}</p>
        </div>
      {:else}
        <div class="player-shell">
          {#key currentPath}
            <PlayerShell
              videoSrc={videoSrc}
              title={currentName}
              courseTitle="Pasta local"
              backHref={returnUrl}
              durationMs={null}
              initialSeconds={0}
              initialPlaybackSpeed={1}
              subtitles={subtitleSrc
                ? [
                    {
                      path: currentSubtitle,
                      lang: "default",
                      label: "Padrão",
                      format: "vtt",
                      default: true,
                    },
                  ]
                : []}
              audioTracks={[]}
              skipGaps={null}
              thumbnails={[]}
              settings={null}
              selectedSubtitleLang={subtitleSrc ? "default" : null}
              selectedAudioLang={null}
              theaterMode={false}
              onTimeUpdate={() => {}}
              onSeek={() => {}}
              onPlay={() => {}}
              onPause={() => {}}
              onEnded={() => {}}
              onSubtitleChange={() => {}}
              onAudioChange={() => {}}
              onSpeedChange={() => {}}
              onSkipGap={() => {}}
              onTheaterToggle={() => {}}
              onClose={back}
            />
          {/key}
        </div>
      {/if}
    </div>
  </div>
</section>

<ConfirmDialog
  bind:open={confirmDeleteOpen}
  title={$t("study.library.delete_confirm_title")}
  message={pendingDelete
    ? ($t(
        pendingDelete.kind === "folder"
          ? "study.library.delete_confirm_msg_folder"
          : "study.library.delete_confirm_msg_file",
        { name: pendingDelete.name },
      ) as string)
    : ""}
  confirmLabel={$t("study.library.delete_confirm_btn")}
  variant="danger"
  onConfirm={performDelete}
/>

{#if contextMenu}
  <div
    class="ctx-menu"
    role="menu"
    style:left="{contextMenu.x}px"
    style:top="{contextMenu.y}px"
  >
    {#if contextMenu.target.kind === "folder"}
      <button
        type="button"
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          if (contextMenu) {
            const t = contextMenu.target;
            closeContextMenu();
            navigateFolder(t.path);
          }
        }}
      >{$t("study.library.ctx_open")}</button>
    {:else}
      <button
        type="button"
        class="ctx-item"
        role="menuitem"
        onclick={() => {
          if (contextMenu) {
            const t = contextMenu.target;
            closeContextMenu();
            if (t.kind === "video") {
              const v = listing?.videos.find((vv) => vv.path === t.path);
              if (v) playVideo(v);
            } else if (t.kind === "doc") {
              const d = listing?.documents.find((dd) => dd.path === t.path);
              if (d) void openDocument(d);
            }
          }
        }}
      >{$t("study.library.ctx_open")}</button>
    {/if}
    <button
      type="button"
      class="ctx-item"
      role="menuitem"
      onclick={() => contextMenu && showInExplorer(contextMenu.target.path)}
    >{$t("study.library.ctx_show_in_explorer")}</button>
    <button
      type="button"
      class="ctx-item"
      role="menuitem"
      onclick={() => contextMenu && startRename(contextMenu.target)}
    >{$t("study.library.ctx_rename")}</button>
    <button
      type="button"
      class="ctx-item danger"
      role="menuitem"
      onclick={() => contextMenu && startDelete(contextMenu.target)}
    >{$t("study.library.ctx_delete")}</button>
  </div>
{/if}

<style>
  .watch-page {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
    width: 100%;
    max-width: 100%;
    margin: 0;
    padding: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0 var(--padding);
  }
  .back-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.4rem 0.75rem;
    background: transparent;
    border: none;
    border-radius: var(--border-radius);
    color: var(--secondary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    transition: border-color 150ms ease;
  }
  .back-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .title {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
    color: var(--secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .layout {
    display: grid;
    grid-template-columns: 300px 1fr;
    gap: calc(var(--padding) * 1.5);
    align-items: flex-start;
    padding: 0 var(--padding);
  }
  @media (max-width: 900px) {
    .layout {
      grid-template-columns: 1fr;
    }
    .playlist {
      order: 2;
    }
  }
  .playlist {
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    padding: 0.6rem;
    max-height: calc(100vh - 8rem);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .crumbs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.2rem;
    padding: 0.25rem 0.3rem 0.5rem;
    border-bottom: none;
    font-size: 12px;
  }
  .crumb {
    background: transparent;
    border: 0;
    padding: 0.1rem 0.35rem;
    border-radius: 4px;
    color: var(--tertiary);
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
    transition: color 150ms ease, background 150ms ease;
  }
  .crumb:hover {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .crumb-current {
    padding: 0.1rem 0.35rem;
    color: var(--secondary);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 180px;
  }
  .crumb-sep {
    color: var(--tertiary);
    opacity: 0.5;
  }
  .items {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .item {
    display: grid;
    grid-template-columns: 18px 1fr auto;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    padding: 0.4rem 0.5rem;
    background: transparent;
    border: 0;
    border-radius: var(--border-radius);
    color: var(--secondary);
    text-align: left;
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
    transition: background 150ms ease;
  }
  .item:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .item.active {
    background: color-mix(in oklab, var(--accent) 16%, transparent);
  }
  .item.active .name {
    color: var(--accent);
    font-weight: 500;
  }
  .item .ico {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    color: var(--tertiary);
  }
  .item.video .ico {
    color: var(--accent);
  }
  .item.folder .ico {
    color: var(--accent);
    opacity: 0.8;
  }
  .item.doc .ico {
    color: var(--tertiary);
  }
  .item .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: 1.3;
  }
  .item .count {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    font-size: 10px;
    color: var(--tertiary);
    font-variant-numeric: tabular-nums;
  }
  .player-wrap {
    min-width: 0;
  }
  .player-shell {
    width: 100%;
    background: black;
    border: none;
    border-radius: var(--border-radius);
    overflow: hidden;
    aspect-ratio: 16 / 9;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .player-shell video {
    width: 100%;
    height: 100%;
    background: black;
  }
  .state {
    padding: 4rem 2rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    text-align: center;
    background: var(--button-elevated);
    border: none;
    border-radius: var(--border-radius);
    aspect-ratio: 16 / 9;
    justify-content: center;
  }
  .muted {
    color: var(--tertiary);
    font-size: 14px;
  }
  .small {
    font-size: 11px;
  }
  .mono {
    font-family: var(--font-mono, "IBM Plex Mono", monospace);
    word-break: break-all;
  }
  .error {
    color: var(--error);
    font-size: 14px;
    font-weight: 500;
  }

  .header-menu-wrap {
    position: relative;
    flex-shrink: 0;
  }
  .header-menu-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--secondary);
    cursor: pointer;
    transition: border-color 150ms ease, color 150ms ease, background 150ms ease;
  }
  .header-menu-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .header-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 50;
    min-width: 200px;
    padding: 4px;
    background: var(--surface, var(--button-elevated));
    border: none;
    border-radius: 8px;
    box-shadow: 0 8px 24px color-mix(in oklab, black 22%, transparent);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .ctx-menu {
    position: fixed;
    z-index: 200;
    min-width: 180px;
    max-width: 280px;
    padding: 4px;
    background: var(--surface, var(--button-elevated));
    border: none;
    border-radius: 8px;
    box-shadow: 0 8px 28px color-mix(in oklab, black 22%, transparent);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ctx-item {
    display: flex;
    align-items: center;
    padding: 7px 10px;
    background: transparent;
    border: 0;
    border-radius: 5px;
    color: var(--secondary);
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease;
  }
  .ctx-item:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .ctx-item.danger {
    color: var(--error);
  }
  .ctx-item.danger:hover {
    background: color-mix(in oklab, var(--error) 12%, transparent);
  }

  .rename-input {
    flex: 1 1 auto;
    min-width: 0;
    padding: 2px 4px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--input-bg);
    color: var(--secondary);
    font-family: inherit;
    font-size: inherit;
    outline: none;
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--accent) 18%, transparent);
  }
  .item.renaming {
    cursor: default;
  }
  .item.renaming:hover {
    background: transparent;
  }
</style>
