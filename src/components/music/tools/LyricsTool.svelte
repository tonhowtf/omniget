<script lang="ts">
  // Fetches missing lyrics for many tracks from LRCLIB, one request at a
  // time with a pause between them (the service asks clients to go easy).
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { library, patchTracksLocal } from "$lib/music/library-store.svelte";
  import type { LrclibRecord, Track } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";

  let { ids = [] }: { ids?: number[] } = $props();

  let onlySynced = $state(true);
  let target = $state<"sidecar" | "tag" | "cache">("sidecar");
  let running = $state(false);
  let stop = $state(false);
  let done = $state(0);
  let found = $state(0);
  let total = $state(0);
  let log = $state<{ t: Track; ok: boolean; note: string }[]>([]);

  let lib = $derived(library());
  let missing = $derived((ids.length ? lib.tracks.filter((x) => ids.includes(x.id)) : lib.tracks).filter((x) => !x.has_lyrics && x.title));

  async function run() {
    running = true;
    stop = false;
    done = 0;
    found = 0;
    log = [];
    const batch = [...missing];
    total = batch.length;
    for (const tr of batch) {
      if (stop) break;
      let ok = false;
      let note = "";
      try {
        const res = await invoke<LrclibRecord[]>("music_lyrics_search", { id: tr.id, artist: null, title: null });
        const dur = tr.duration_ms / 1000;
        const pick = res.find(
          (r) => (r.syncedLyrics || (!onlySynced && r.plainLyrics)) && (r.duration == null || Math.abs(r.duration - dur) <= 3),
        );
        if (pick) {
          await invoke("music_lyrics_save", { id: tr.id, text: pick.syncedLyrics || pick.plainLyrics, target });
          patchTracksLocal([tr.id], { has_lyrics: true });
          ok = true;
          found++;
          note = pick.syncedLyrics ? ($t("music.lyrics.synced") as string) : ($t("music.lyrics.plain") as string);
        } else note = $t("music.lyrics.no_results") as string;
      } catch (e) {
        note = String(e);
      }
      done++;
      log = [{ t: tr, ok, note }, ...log].slice(0, 200);
      await new Promise((r) => setTimeout(r, 700));
    }
    running = false;
  }
</script>

<div class="lt">
  <p class="lead">{$t("music.lyrics_tool.lead", { count: missing.length })}</p>
  <div class="opts">
    <label class="chk"><input type="checkbox" bind:checked={onlySynced} /> {$t("music.lyrics_tool.only_synced")}</label>
    <select class="input sel" bind:value={target}>
      <option value="sidecar">{$t("music.lyrics.target_sidecar")}</option>
      <option value="tag">{$t("music.lyrics.target_tag")}</option>
      <option value="cache">{$t("music.lyrics.target_cache")}</option>
    </select>
  </div>
  <div class="acts">
    {#if running}
      <button class="btn" onclick={() => (stop = true)}>{$t("music.cancel")}</button>
      <span class="prog">{$t("music.lyrics_tool.progress", { done, total, found })}</span>
    {:else}
      <button class="btn btn-primary" onclick={run} disabled={!missing.length}><MusicIcon name="lyrics" size={14} /> {$t("music.lyrics_tool.start", { count: missing.length })}</button>
      {#if done}<span class="prog">{$t("music.lyrics_tool.result", { found, done })}</span>{/if}
    {/if}
  </div>
  {#if log.length}
    <ul class="log">
      {#each log as l, i (i)}
        <li class:ok={l.ok}><MusicIcon name={l.ok ? "check" : "close"} size={12} /> <span class="n">{l.t.artist} — {l.t.title}</span> <span class="note">{l.note}</span></li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .lt {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 760px;
  }
  .lead {
    margin: 0;
    color: var(--text-dim);
  }
  .opts,
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
  }
  .chk input {
    accent-color: var(--accent);
  }
  .sel {
    width: auto;
  }
  .prog {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .log {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 400px;
    overflow: auto;
    font-size: var(--text-sm);
  }
  .log li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 0;
    color: var(--text-muted);
  }
  .log li.ok {
    color: var(--success);
  }
  .n {
    color: var(--text);
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .note {
    font-size: var(--text-xs);
  }
</style>
