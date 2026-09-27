<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { fmtBytes, fmtTime, qualityLabel } from "$lib/music/library";
  import { playTracks } from "$lib/music/player-store.svelte";
  import { removeTracks } from "$lib/music/ui-store.svelte";
  import type { Track } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";

  let groups = $state<Track[][] | null>(null);
  let busy = $state(false);

  async function find() {
    busy = true;
    try {
      groups = await invoke<Track[][]>("music_duplicates");
    } finally {
      busy = false;
    }
  }

  let extras = $derived((groups ?? []).flatMap((g) => g.slice(1)));
  let wasted = $derived(extras.reduce((a, x) => a + x.size, 0));
</script>

<div class="dt">
  <p class="lead">{$t("music.dupes.lead")}</p>
  <div class="acts">
    <button class="btn btn-primary" onclick={find} disabled={busy}>{#if busy}<MusicIcon name="loader" size={14} spin />{:else}<MusicIcon name="copy" size={14} />{/if} {$t("music.dupes.find")}</button>
    {#if groups?.length}
      <span class="sum">{$t("music.dupes.summary", { groups: groups.length, extra: extras.length, size: fmtBytes(wasted) })}</span>
      <button class="btn" onclick={() => removeTracks(extras.map((x) => x.id), false)}>{$t("music.dupes.remove_extras")}</button>
      <button class="btn btn-destructive" onclick={() => removeTracks(extras.map((x) => x.id), true)}>{$t("music.dupes.delete_extras")}</button>
    {/if}
  </div>
  {#if groups && !groups.length}
    <p class="ok"><MusicIcon name="check" size={14} /> {$t("music.dupes.none")}</p>
  {/if}
  {#if groups?.length}
    <ul class="groups">
      {#each groups as g, gi (gi)}
        <li class="grp">
          <div class="gh">{g[0].artist} — {g[0].title}</div>
          {#each g as tr, i (tr.id)}
            <div class="row" class:best={i === 0}>
              <button class="ib" onclick={() => void playTracks([tr.id])} aria-label={$t("music.play") as string}><MusicIcon name="play" size={12} /></button>
              <span class="q">{qualityLabel(tr)}</span>
              <span class="d">{fmtTime(tr.duration_ms)} · {fmtBytes(tr.size)}</span>
              <span class="p" title={tr.path}>{tr.path}</span>
              {#if i === 0}<span class="tag tag-success">{$t("music.dupes.best")}</span>{/if}
            </div>
          {/each}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .dt {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead {
    margin: 0;
    color: var(--text-dim);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .sum {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .ok {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--success);
  }
  .groups {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .grp {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .gh {
    font-weight: 650;
    margin-bottom: 4px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    padding: 2px 0;
    color: var(--text-dim);
  }
  .row.best {
    color: var(--text);
  }
  .ib {
    all: unset;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    cursor: pointer;
  }
  .ib:hover {
    background: var(--fill-2);
  }
  .q {
    width: 170px;
    flex-shrink: 0;
  }
  .d {
    width: 120px;
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .p {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
  }
</style>
