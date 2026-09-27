<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtTime, fmtLong } from "$lib/music/library";
  import { trackById, library } from "$lib/music/library-store.svelte";
  import { clearQueue, moveInQueue, player, playIndex, removeFromQueue, setSleepTimer } from "$lib/music/player-store.svelte";
  import { createPlaylistWith, showTrackMenu } from "$lib/music/ui-store.svelte";
  import Cover from "./Cover.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let dragFrom = $state<number | null>(null);
  let dropAt = $state<number | null>(null);
  let list: HTMLOListElement | undefined = $state();

  let rows = $derived.by(() => {
    void library().version;
    return player.queue.map((id, i) => ({ i, t: trackById(id) }));
  });
  let upcoming = $derived(rows.slice(player.index + 1));
  let remainingMs = $derived(upcoming.reduce((a, r) => a + (r.t?.duration_ms ?? 0), 0));

  $effect(() => {
    void player.index;
    const el = list?.querySelector<HTMLElement>(".cur");
    el?.scrollIntoView({ block: "nearest" });
  });

  async function saveAsPlaylist() {
    await createPlaylistWith(player.queue, `${$t("music.queue.title")} ${new Date().toLocaleDateString()}`);
  }

  async function exportM3u() {
    const path = await save({ defaultPath: "queue.m3u8", filters: [{ name: "M3U", extensions: ["m3u8", "m3u"] }] });
    if (!path) return;
    try {
      const n = await invoke<number>("music_export_m3u", { trackIds: player.queue, path });
      showToast("success", $t("music.toast.exported", { count: n }) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function drop(to: number) {
    if (dragFrom != null && dragFrom !== to) moveInQueue(dragFrom, to);
    dragFrom = null;
    dropAt = null;
  }

  const SLEEP = [15, 30, 45, 60, 90];
</script>

<div class="qp">
  <header class="qp-head">
    <h3>{$t("music.queue.title")}</h3>
    <span class="meta">{$t("music.queue.remaining", { count: upcoming.length, time: fmtLong(remainingMs) })}</span>
  </header>

  <div class="qp-actions">
    <button class="btn btn-sm btn-ghost" onclick={saveAsPlaylist} disabled={!player.queue.length}><MusicIcon name="save" size={13} /> {$t("music.queue.save")}</button>
    <button class="btn btn-sm btn-ghost" onclick={exportM3u} disabled={!player.queue.length}><MusicIcon name="upload" size={13} /> M3U</button>
    <button class="btn btn-sm btn-ghost" onclick={() => clearQueue(true)} disabled={player.queue.length < 2}><MusicIcon name="trash" size={13} /> {$t("music.queue.clear")}</button>
  </div>

  {#if !player.queue.length}
    <p class="empty">{$t("music.queue.empty")}</p>
  {:else}
    <ol class="qp-list" bind:this={list}>
      {#each rows as r (r.i + ":" + (r.t?.id ?? 0))}
        {#if r.t}
          <li
            class="qi"
            class:cur={r.i === player.index}
            class:past={r.i < player.index}
            class:drop={dropAt === r.i}
            draggable="true"
            ondragstart={() => (dragFrom = r.i)}
            ondragover={(e) => {
              e.preventDefault();
              dropAt = r.i;
            }}
            ondragleave={() => (dropAt = null)}
            ondrop={() => drop(r.i)}
            oncontextmenu={(e) => r.t && showTrackMenu(e, [r.t.id])}
          >
            <span class="grip"><MusicIcon name="grip" size={14} /></span>
            <button class="qi-main" ondblclick={() => void playIndex(r.i)} onclick={(e) => e.detail === 0 && void playIndex(r.i)}>
              <Cover hash={r.t.cover} seed={r.t.album || r.t.title} size={34} radius={4} />
              <span class="qi-txt">
                <span class="t1">{r.t.title}</span>
                <span class="t2">{r.t.artist}</span>
              </span>
            </button>
            <span class="dur">{fmtTime(r.t.duration_ms)}</span>
            <button class="rm" onclick={() => removeFromQueue(r.i)} aria-label={$t("music.queue.remove") as string}><MusicIcon name="close" size={13} /></button>
          </li>
        {/if}
      {/each}
    </ol>
  {/if}

  <footer class="qp-sleep">
    <span class="lbl"><MusicIcon name="moon" size={14} /> {$t("music.sleep.title")}</span>
    <div class="chips">
      {#each SLEEP as m (m)}
        <button class="btn btn-pill btn-sm" onclick={() => setSleepTimer(m)}>{m} min</button>
      {/each}
      <button class="btn btn-pill btn-sm" class:active={player.sleepEndOfTrack} onclick={() => setSleepTimer("track")}>{$t("music.sleep.end_of_track")}</button>
      {#if player.sleepAt || player.sleepEndOfTrack}
        <button class="btn btn-pill btn-sm" onclick={() => setSleepTimer(null)}>{$t("music.sleep.off")}</button>
      {/if}
    </div>
    {#if player.sleepAt}
      <span class="until">{$t("music.sleep.until", { time: new Date(player.sleepAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) })}</span>
    {/if}
  </footer>
</div>

<style>
  .qp {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .qp-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-3) var(--space-1);
  }
  h3 {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
  }
  .meta {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .qp-actions {
    display: flex;
    gap: 2px;
    padding: 0 var(--space-2) var(--space-2);
    flex-wrap: wrap;
  }
  .empty {
    padding: var(--space-5);
    color: var(--text-dim);
    font-size: var(--text-sm);
    text-align: center;
  }
  .qp-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0 var(--space-2);
  }
  .qi {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px;
    border-radius: var(--radius-sm);
  }
  .qi:hover {
    background: var(--fill-1);
  }
  .qi.cur {
    background: var(--accent-soft);
  }
  .qi.cur .t1 {
    color: var(--accent-hi);
  }
  .qi.past {
    opacity: 0.5;
  }
  .qi.drop {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .grip {
    color: var(--text-muted);
    cursor: grab;
    opacity: 0;
  }
  .qi:hover .grip {
    opacity: 1;
  }
  .qi-main {
    all: unset;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    cursor: default;
  }
  .qi-main:focus-visible {
    outline: var(--focus-ring);
  }
  .qi-txt {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .t1,
  .t2 {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .t1 {
    font-size: var(--text-sm);
    font-weight: 550;
  }
  .t2 {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .dur {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .rm {
    all: unset;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0;
  }
  .qi:hover .rm,
  .rm:focus-visible {
    opacity: 1;
  }
  .rm:hover {
    background: var(--fill-2);
    color: var(--text);
  }
  .qp-sleep {
    border-top: var(--hairline) solid var(--content-border);
    padding: var(--space-2) var(--space-3) var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lbl {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .until {
    font-size: var(--text-xs);
    color: var(--accent-hi);
  }
</style>
