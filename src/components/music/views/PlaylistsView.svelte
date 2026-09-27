<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtLong } from "$lib/music/library";
  import { go, loadPlaylists, ui } from "$lib/music/ui-store.svelte";
  import type { SmartQuery } from "$lib/music/types";
  import Cover from "../Cover.svelte";
  import Modal from "../Modal.svelte";
  import MusicIcon from "../MusicIcon.svelte";
  import SmartEditor from "../SmartEditor.svelte";

  let creating = $state<"manual" | "smart" | null>(null);
  let name = $state("");
  let query = $state<SmartQuery>({ rules: [{ field: "liked", op: "is_true", value: null }], match_all: true, order: "added", descending: true, limit: null });

  onMount(() => void loadPlaylists());

  async function create() {
    const n = name.trim() || ($t("music.playlist.untitled") as string);
    const id = await invoke<number>("music_playlist_create", {
      name: n,
      smart: creating === "smart" ? $state.snapshot(query) : null,
      trackIds: null,
    });
    creating = null;
    name = "";
    await loadPlaylists();
    go({ kind: "playlist", id });
  }

  async function importFile() {
    const path = await open({ multiple: false, filters: [{ name: "Playlist", extensions: ["m3u", "m3u8", "pls"] }] });
    if (!path || Array.isArray(path)) return;
    try {
      const r = await invoke<{ playlist_id: number; matched: number; missing: string[]; streams: { location: string; title: string | null; logo: string | null }[] }>("music_playlist_import", { path });
      for (const s of r.streams) {
        await invoke("music_radio_save", {
          station: { id: `url:${s.location}`, name: s.title || s.location, url: s.location, homepage: "", favicon: s.logo ?? "", tags: "", country: "", codec: "", bitrate: 0, hls: /\.m3u8/i.test(s.location) },
        }).catch(() => {});
      }
      await loadPlaylists();
      showToast("success", $t("music.toast.imported", { matched: r.matched, missing: r.missing.length, streams: r.streams.length }) as string);
      if (r.playlist_id) go({ kind: "playlist", id: r.playlist_id });
      else if (r.streams.length) go({ kind: "radio" });
    } catch (e) {
      showToast("error", String(e));
    }
  }
</script>

<div class="pv">
  <header>
    <h1>{$t("music.nav.playlists")} <small>{ui.playlists.length}</small></h1>
    <div class="acts">
      <button class="btn btn-primary" onclick={() => (creating = "manual")}><MusicIcon name="plus" size={14} /> {$t("music.playlist.new")}</button>
      <button class="btn" onclick={() => (creating = "smart")}><MusicIcon name="smart" size={14} /> {$t("music.playlist.new_smart")}</button>
      <button class="btn" onclick={importFile}><MusicIcon name="upload" size={14} /> {$t("music.playlist.import")}</button>
    </div>
  </header>

  {#if !ui.playlists.length}
    <p class="dim">{$t("music.playlist.none")}</p>
  {/if}

  <div class="grid">
    {#each ui.playlists as p (p.id)}
      <button class="pl" onclick={() => go({ kind: "playlist", id: p.id })}>
        <div class="art">
          {#if p.covers.length >= 4}
            <div class="collage">{#each p.covers.slice(0, 4) as c (c)}<Cover hash={c} seed={c} size="100%" radius={0} />{/each}</div>
          {:else}
            <Cover hash={p.covers[0] ?? null} seed={p.name} size="100%" radius={8} icon={p.smart ? "smart" : "playlist"} />
          {/if}
          {#if p.smart}<span class="badge-smart"><MusicIcon name="smart" size={11} /></span>{/if}
        </div>
        <span class="n">{p.name}</span>
        <span class="c">{$t("music.count_songs", { count: p.count })} · {fmtLong(p.duration_ms)}</span>
      </button>
    {/each}
  </div>
</div>

{#if creating}
  <Modal title={$t(creating === "smart" ? "music.playlist.new_smart" : "music.playlist.new") as string} width={creating === "smart" ? 720 : 400} onclose={() => (creating = null)}>
    {#if creating === "smart"}
      <SmartEditor bind:name bind:query onsave={create} oncancel={() => (creating = null)} />
    {:else}
      <form
        class="nf"
        onsubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <input class="input" bind:value={name} placeholder={$t("music.playlist.untitled") as string} />
        <div class="dialog-actions">
          <button type="button" class="btn" onclick={() => (creating = null)}>{$t("music.cancel")}</button>
          <button type="submit" class="btn btn-primary">{$t("music.create")}</button>
        </div>
      </form>
    {/if}
  </Modal>
{/if}

<style>
  .pv {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  h1 small {
    font-size: var(--text-md);
    color: var(--text-muted);
    font-weight: 500;
  }
  .acts {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: var(--space-5) var(--space-4);
  }
  .pl {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: 6px;
    cursor: pointer;
    min-width: 0;
  }
  .pl:focus-visible {
    outline: var(--focus-ring);
    border-radius: 8px;
  }
  .art {
    position: relative;
    aspect-ratio: 1;
    border-radius: 8px;
    overflow: hidden;
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.25);
  }
  .art :global(.cover) {
    width: 100% !important;
    height: 100% !important;
  }
  .collage {
    display: grid;
    grid-template-columns: 1fr 1fr;
    width: 100%;
    height: 100%;
  }
  .badge-smart {
    position: absolute;
    top: 6px;
    right: 6px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
  }
  .n {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .c {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .nf {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
</style>
