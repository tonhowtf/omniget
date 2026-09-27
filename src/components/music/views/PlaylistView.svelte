<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtLong, shuffled } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { enqueue, playTracks } from "$lib/music/player-store.svelte";
  import { back, loadPlaylists, showTrackMenu, ui } from "$lib/music/ui-store.svelte";
  import type { SmartQuery, Track } from "$lib/music/types";
  import CollectionHeader from "../CollectionHeader.svelte";
  import Modal from "../Modal.svelte";
  import MusicIcon from "../MusicIcon.svelte";
  import SmartEditor from "../SmartEditor.svelte";
  import TrackList from "../TrackList.svelte";

  let { id }: { id: number } = $props();

  let tracks = $state<Track[]>([]);
  let loading = $state(true);
  let editing = $state(false);
  let renaming = $state(false);
  let name = $state("");
  let query = $state<SmartQuery>({ rules: [], match_all: true, order: null, descending: false, limit: null });
  let selected = $state(new Set<number>());

  let pl = $derived(ui.playlists.find((p) => p.id === id) ?? null);
  let ids = $derived(tracks.map((x) => x.id));

  async function load() {
    loading = true;
    try {
      tracks = await invoke<Track[]>("music_playlist_tracks", { id });
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void id;
    void library().version;
    void load();
    if (!ui.playlists.length) void loadPlaylists();
  });

  async function reorder(from: number, to: number) {
    const next = [...tracks];
    const [x] = next.splice(from, 1);
    next.splice(to, 0, x);
    tracks = next;
    await invoke("music_playlist_set_items", { id, trackIds: next.map((t) => t.id) });
  }

  async function removeItems(sel: number[]) {
    const drop = new Set(sel);
    const next = tracks.filter((x) => !drop.has(x.id));
    tracks = next;
    await invoke("music_playlist_set_items", { id, trackIds: next.map((x) => x.id) });
    void loadPlaylists();
  }

  function editSmart() {
    if (!pl?.smart) return;
    name = pl.name;
    query = JSON.parse(JSON.stringify(pl.smart));
    editing = true;
  }

  async function saveSmart() {
    await invoke("music_playlist_update", { id, name, smart: $state.snapshot(query) });
    editing = false;
    await loadPlaylists();
    await load();
  }

  async function rename() {
    await invoke("music_playlist_update", { id, name, smart: null });
    renaming = false;
    await loadPlaylists();
  }

  async function exportM3u() {
    const path = await save({ defaultPath: `${pl?.name ?? "playlist"}.m3u8`, filters: [{ name: "M3U", extensions: ["m3u8", "m3u"] }] });
    if (!path) return;
    try {
      const n = await invoke<number>("music_playlist_export", { id, path });
      showToast("success", $t("music.toast.exported", { count: n }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function remove() {
    ui.confirm = {
      title: $t("music.playlist.delete_title") as string,
      body: $t("music.playlist.delete_body", { name: pl?.name ?? "" }) as string,
      danger: true,
      ok: $t("music.delete") as string,
      run: async () => {
        await invoke("music_playlist_delete", { id });
        await loadPlaylists();
        back();
      },
    };
  }

  function drop(e: DragEvent) {
    const raw = e.dataTransfer?.getData("application/x-omniget-tracks");
    if (!raw || pl?.smart) return;
    e.preventDefault();
    void invoke("music_playlist_add", { id, trackIds: JSON.parse(raw) }).then(() => {
      void load();
      void loadPlaylists();
    });
  }
</script>

<div
  class="plv"
  role="region"
  aria-label={pl?.name ?? ""}
  ondragover={(e) => {
    if (!pl?.smart && e.dataTransfer?.types.includes("application/x-omniget-tracks") && !e.dataTransfer.types.includes("application/x-omniget-row")) e.preventDefault();
  }}
  ondrop={drop}
>
  {#if pl}
    <CollectionHeader
      kicker={$t(pl.smart ? "music.playlist.smart" : "music.playlist.kind") as string}
      title={pl.name}
      subtitle={`${$t("music.count_songs", { count: tracks.length })} · ${fmtLong(tracks.reduce((a, x) => a + x.duration_ms, 0))}`}
      covers={pl.covers}
      icon={pl.smart ? "smart" : "playlist"}
      onplay={tracks.length ? () => void playTracks(ids, undefined, pl?.name) : undefined}
      onshuffle={tracks.length ? () => void playTracks(shuffled(ids), undefined, pl?.name) : undefined}
    >
      {#snippet actions()}
        <button class="btn btn-lg" onclick={() => enqueue(ids)} disabled={!tracks.length} aria-label={$t("music.menu.add_queue") as string} title={$t("music.menu.add_queue") as string}><MusicIcon name="queue" size={15} /></button>
        {#if pl?.smart}
          <button class="btn btn-lg" onclick={editSmart}><MusicIcon name="smart" size={15} /> {$t("music.smart.edit")}</button>
        {:else}
          <button class="btn btn-lg" onclick={() => { name = pl?.name ?? ""; renaming = true; }} aria-label={$t("music.rename") as string} title={$t("music.rename") as string}><MusicIcon name="edit" size={15} /></button>
        {/if}
        <button class="btn btn-lg" onclick={exportM3u} aria-label={$t("music.playlist.export") as string} title={$t("music.playlist.export") as string}><MusicIcon name="upload" size={15} /></button>
        <button class="btn btn-lg btn-ghost" onclick={remove} aria-label={$t("music.delete") as string} title={$t("music.delete") as string}><MusicIcon name="trash" size={15} /></button>
      {/snippet}
    </CollectionHeader>
  {/if}

  {#if !loading && !tracks.length}
    <p class="dim">{$t(pl?.smart ? "music.smart.no_matches" : "music.playlist.empty_hint")}</p>
  {:else}
    <div class="list">
      <TrackList
        {tracks}
        bind:selected
        columns={["album", "plays"]}
        reorderable={!pl?.smart}
        onreorder={reorder}
        onplay={(tr) => void playTracks(ids, tr.id, pl?.name)}
        onmenu={(e, sel) =>
          showTrackMenu(e, sel, pl?.smart ? [] : [{ divider: true }, { label: $t("music.playlist.remove_from") as string, icon: "close", action: () => void removeItems(sel) }])}
      />
    </div>
  {/if}
</div>

{#if editing}
  <Modal title={$t("music.smart.edit") as string} width={720} onclose={() => (editing = false)}>
    <SmartEditor bind:name bind:query onsave={saveSmart} oncancel={() => (editing = false)} />
  </Modal>
{/if}

{#if renaming}
  <Modal title={$t("music.rename") as string} width={400} onclose={() => (renaming = false)}>
    <form
      class="nf"
      onsubmit={(e) => {
        e.preventDefault();
        void rename();
      }}
    >
      <input class="input" bind:value={name} />
      <div class="dialog-actions">
        <button type="button" class="btn" onclick={() => (renaming = false)}>{$t("music.cancel")}</button>
        <button type="submit" class="btn btn-primary">{$t("music.save")}</button>
      </div>
    </form>
  </Modal>
{/if}

<style>
  .plv {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    height: 100%;
    min-height: 0;
  }
  .list {
    flex: 1;
    min-height: 240px;
  }
  .dim {
    color: var(--text-dim);
    text-align: center;
    margin-top: var(--space-6);
  }
  .nf {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
</style>
