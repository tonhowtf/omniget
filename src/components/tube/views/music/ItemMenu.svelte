<script lang="ts">
  // ⋮ menu of songs, albums, playlists and artists (YT Music order).
  import { onMount, tick } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { playRadio } from "$lib/tube/music.svelte";
  import { go } from "$lib/tube/ui.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../../TIcon.svelte";
  import type { Item, SongItem } from "$lib/tube/types";
  import {
    addToQueue,
    copyLink,
    downloadSongs,
    likeSong,
    linkFor,
    playItem,
    playNext,
    saveToLibrary,
    songFromItem,
    songsOf,
  } from "./shared.svelte";

  let { item, x, y, onclose }: { item: Item; x: number; y: number; onclose: () => void } = $props();

  let el = $state<HTMLDivElement>();
  let left = $state(0);
  let top = $state(0);
  let song = $derived(songFromItem(item));
  let view = $state<"main" | "save">("main");
  let lists = $state<{ id: string; title: string; contains: boolean }[] | null>(null);
  let listsError = $state(false);
  let newTitle = $state("");

  async function place() {
    await tick();
    if (!el) return;
    const r = el.getBoundingClientRect();
    left = Math.max(8, Math.min(x, window.innerWidth - r.width - 8));
    top = y + r.height > window.innerHeight - 8 ? Math.max(8, y - r.height) : y;
  }

  onMount(() => {
    void place();
    el?.querySelector<HTMLButtonElement>("button")?.focus();
    const down = (e: PointerEvent) => {
      if (el && !el.contains(e.target as Node)) onclose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onclose();
      }
    };
    const scroll = () => onclose();
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("wheel", scroll, { passive: true });
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("wheel", scroll);
    };
  });

  function run(fn: () => unknown) {
    onclose();
    void fn();
  }

  async function openSave() {
    view = "save";
    void place();
    if (!song) return;
    try {
      lists = await api.savePlaylists(song.id);
    } catch {
      listsError = true;
    }
    void place();
  }

  async function toggleIn(pl: { id: string; title: string; contains: boolean }) {
    if (!song) return;
    try {
      await api.action(pl.contains ? "playlist_remove" : "playlist_add", { video_id: song.id, playlist_id: pl.id, set_video_id: song.set_video_id ?? null });
      pl.contains = !pl.contains;
      showToast("success", $t(pl.contains ? "tube.music.toast_added_to" : "tube.music.toast_removed_from", { name: pl.title }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function createWith() {
    const title = newTitle.trim();
    if (!title || !song) return;
    try {
      await api.createPlaylist(title, [song.id], true);
      showToast("success", $t("tube.music.toast_added_to", { name: title }) as string);
      onclose();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function collectionSongs(): Promise<SongItem[]> {
    if (item.kind === "album") return (await api.musicAlbum(item.id)).tracks;
    if (item.kind === "playlist") return songsOf((await api.musicPlaylist(item.id)).items);
    return song ? [song] : [];
  }

  async function queueCollection() {
    try {
      addToQueue(await collectionSongs());
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function downloadCollection() {
    try {
      await downloadSongs(await collectionSongs());
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

<div
  class="yt-menu im"
  role="menu"
  tabindex="-1"
  bind:this={el}
  style:left="{left}px"
  style:top="{top}px"
  aria-label={$t("tube.music.more_actions") as string}
>
  {#if view === "save"}
    <div class="im-head">
      <button class="yt-btn yt-btn--icon" onclick={() => ((view = "main"), void place())} aria-label={$t("tube.common.back") as string}><TIcon name="back" size={20} /></button>
      <span>{$t("tube.music.save_to_playlist")}</span>
    </div>
    {#if listsError}
      <p class="im-note">{$t("tube.music.save_needs_account")}</p>
    {:else if !lists}
      {#each [0, 1, 2] as i (i)}<div class="im-skel yt-skel"></div>{/each}
    {:else}
      <div class="im-lists yt-scroll">
        {#each lists as pl (pl.id)}
          <button class="yt-menu-item" role="menuitemcheckbox" aria-checked={pl.contains} onclick={() => toggleIn(pl)}>
            <TIcon name={pl.contains ? "check" : "playlist"} size={20} />
            <span class="yt-clamp-1">{pl.title}</span>
          </button>
        {/each}
      </div>
      <div class="yt-menu-sep"></div>
      <form class="im-new" onsubmit={(e) => (e.preventDefault(), void createWith())}>
        <input bind:value={newTitle} placeholder={$t("tube.music.new_playlist") as string} aria-label={$t("tube.music.new_playlist") as string} />
        <button class="yt-btn yt-btn--s" type="submit" disabled={!newTitle.trim()}>{$t("tube.music.create")}</button>
      </form>
    {/if}
  {:else if song}
    <button class="yt-menu-item" role="menuitem" onclick={() => run(() => playNext(song!))}><TIcon name="queue" size={20} />{$t("tube.music.play_next")}</button>
    <button class="yt-menu-item" role="menuitem" onclick={() => run(() => addToQueue([song!]))}><TIcon name="playlist-add" size={20} />{$t("tube.music.add_to_queue")}</button>
    <button class="yt-menu-item" role="menuitem" onclick={() => run(() => playRadio(song!.id))}><TIcon name="radio" size={20} />{$t("tube.music.start_radio")}</button>
    <div class="yt-menu-sep"></div>
    <button class="yt-menu-item" role="menuitem" onclick={() => run(() => likeSong(song!, !song!.liked))}>
      <TIcon name={song.liked ? "thumbs-up-fill" : "thumbs-up"} size={20} />{song.liked ? $t("tube.music.unlike") : $t("tube.music.like")}
    </button>
    <button class="yt-menu-item" role="menuitem" onclick={openSave}><TIcon name="save" size={20} />{$t("tube.music.save_to_playlist")}</button>
    {#if song.artists.length || song.album}
      <div class="yt-menu-sep"></div>
    {/if}
    {#each song.artists.filter((a) => a.id).slice(0, 3) as a (a.id)}
      <button class="yt-menu-item" role="menuitem" onclick={() => run(() => go({ name: "m_artist", id: a.id }))}><TIcon name="artist" size={20} /><span class="yt-clamp-1">{$t("tube.music.go_to_artist_named", { name: a.name })}</span></button>
    {/each}
    {#if song.album?.id}
      <button class="yt-menu-item" role="menuitem" onclick={() => run(() => go({ name: "m_album", id: song!.album!.id }))}><TIcon name="album" size={20} />{$t("tube.music.go_to_album")}</button>
    {/if}
    <div class="yt-menu-sep"></div>
    <button class="yt-menu-item" role="menuitem" onclick={() => run(() => downloadSongs([song!]))}><TIcon name="download" size={20} />{$t("tube.music.download_audio")}</button>
    <button class="yt-menu-item" role="menuitem" onclick={() => run(() => copyLink(linkFor(item)))}><TIcon name="link" size={20} />{$t("tube.music.copy_link")}</button>
  {:else}
    <button class="yt-menu-item" role="menuitem" onclick={() => run(() => playItem(item))}><TIcon name="play" size={20} />{$t("tube.music.play")}</button>
    {#if item.kind !== "mood"}
      <button class="yt-menu-item" role="menuitem" onclick={() => run(() => playItem(item, true))}><TIcon name="shuffle" size={20} />{$t("tube.music.shuffle")}</button>
    {/if}
    {#if item.kind === "album" || item.kind === "playlist"}
      <button class="yt-menu-item" role="menuitem" onclick={() => run(queueCollection)}><TIcon name="playlist-add" size={20} />{$t("tube.music.add_to_queue")}</button>
      {@const pid = item.kind === "album" ? item.playlist_id : item.id}
      {#if pid}
        <button class="yt-menu-item" role="menuitem" onclick={() => run(() => saveToLibrary(pid))}><TIcon name="plus" size={20} />{$t("tube.music.save_to_library")}</button>
      {/if}
      <button class="yt-menu-item" role="menuitem" onclick={() => run(downloadCollection)}><TIcon name="download" size={20} />{$t("tube.music.download_all")}</button>
    {/if}
    {#if item.kind === "album"}
      {#each item.artists.filter((a) => a.id).slice(0, 2) as a (a.id)}
        <button class="yt-menu-item" role="menuitem" onclick={() => run(() => go({ name: "m_artist", id: a.id }))}><TIcon name="artist" size={20} /><span class="yt-clamp-1">{$t("tube.music.go_to_artist_named", { name: a.name })}</span></button>
      {/each}
    {/if}
    {#if item.kind !== "mood"}
      <div class="yt-menu-sep"></div>
      <button class="yt-menu-item" role="menuitem" onclick={() => run(() => copyLink(linkFor(item)))}><TIcon name="link" size={20} />{$t("tube.music.copy_link")}</button>
    {/if}
  {/if}
</div>

<style>
  .im {
    position: fixed;
    z-index: 60;
    width: 280px;
    background: var(--ytm-bg-raised, var(--yt-bg-menu));
    color: var(--ytm-text, var(--yt-text));
    animation: im-in var(--yt-dur-fast) var(--yt-ease-enter);
  }
  .im :global(.yt-menu-item) { height: 40px; }
  .im :global(.yt-menu-item:hover) { background: var(--ytm-chip, var(--yt-tonal)); }
  .im-head { display: flex; align-items: center; gap: 8px; padding: 0 8px 8px; font: var(--yt-type-body-strong); border-bottom: 1px solid var(--ytm-divider, var(--yt-divider)); margin-bottom: 8px; }
  .im-lists { max-height: 240px; }
  .im-skel { height: 28px; margin: 6px 16px; }
  .im-note { margin: 8px 16px; color: var(--ytm-text-2, var(--yt-text-2)); font: var(--yt-type-meta); }
  .im-new { display: flex; gap: 8px; padding: 0 12px 4px; }
  .im-new input {
    flex: 1; min-width: 0; height: 32px; padding: 0 10px; border-radius: 8px;
    border: 1px solid var(--ytm-divider, var(--yt-divider)); background: transparent; color: inherit; font: var(--yt-type-meta);
  }
  .im-new input:focus { outline: 2px solid var(--yt-link); outline-offset: 0; }
  @keyframes im-in { from { opacity: 0; transform: translateY(-4px); } }
  @media (prefers-reduced-motion: reduce) { .im { animation: none; } }
</style>
