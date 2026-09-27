<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { library, upsertTracks } from "$lib/music/library-store.svelte";
  import type { Track } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";

  let { ids = [] }: { ids?: number[] } = $props();

  let lib = $derived(library());
  let scope = $derived(ids.length ? lib.tracks.filter((x) => ids.includes(x.id)) : lib.tracks);
  let pending = $derived(scope.filter((x) => !x.analyzed));
  let analyzed = $derived(scope.filter((x) => x.analyzed));
  let prog = $derived(lib.analysis);
  let writing = $state(false);

  let loudest = $derived([...analyzed].filter((x) => x.loudness_lufs != null).sort((a, b) => (b.loudness_lufs ?? 0) - (a.loudness_lufs ?? 0)).slice(0, 5));
  let avgLufs = $derived(analyzed.length ? analyzed.reduce((a, x) => a + (x.loudness_lufs ?? 0), 0) / analyzed.length : null);

  function run(list: Track[]) {
    if (!list.length) return;
    void invoke("music_analyze", { ids: list.map((x) => x.id) });
  }

  async function writeTags() {
    writing = true;
    try {
      const r = await invoke<{ updated: Track[]; failed: [number, string][] }>("music_write_replaygain", { ids: analyzed.map((x) => x.id) });
      upsertTracks(r.updated);
      showToast(r.failed.length ? "info" : "success", $t("music.tools.rg_written", { ok: r.updated.length, failed: r.failed.length }) as string);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      writing = false;
    }
  }
</script>

<div class="at">
  <p class="lead">{$t("music.tools.analyze_lead")}</p>
  <div class="cards">
    <div class="card-s"><b>{analyzed.length}</b><span>{$t("music.tools.analyzed")}</span></div>
    <div class="card-s"><b>{pending.length}</b><span>{$t("music.tools.pending")}</span></div>
    <div class="card-s"><b>{avgLufs != null ? avgLufs.toFixed(1) : "—"}</b><span>{$t("music.tools.avg_lufs")}</span></div>
  </div>

  {#if prog}
    <div class="progress">
      <span>{$t("music.tools.analyzing", { done: prog.done, total: prog.total })}</span>
      <span class="bar"><i style:width="{(prog.done / Math.max(1, prog.total)) * 100}%"></i></span>
      <button class="btn btn-sm" onclick={() => void invoke("music_analysis_cancel")}>{$t("music.cancel")}</button>
    </div>
  {/if}

  <div class="acts">
    <button class="btn btn-primary" onclick={() => run(pending)} disabled={!pending.length || !!prog}><MusicIcon name="activity" size={14} /> {$t("music.tools.analyze_pending", { count: pending.length })}</button>
    <button class="btn" onclick={() => run(scope)} disabled={!scope.length || !!prog}>{$t("music.tools.reanalyze_all")}</button>
    <button class="btn" onclick={writeTags} disabled={!analyzed.length || writing}><MusicIcon name="save" size={14} /> {$t("music.tools.write_rg")}</button>
  </div>
  <p class="hint">{$t("music.tools.write_rg_hint")}</p>

  {#if loudest.length}
    <h3>{$t("music.tools.loudest")}</h3>
    <ul class="loud">
      {#each loudest as x (x.id)}
        <li><span class="n">{x.artist} — {x.title}</span><span class="v">{x.loudness_lufs?.toFixed(1)} LUFS · {x.rg_track_gain != null ? `${x.rg_track_gain > 0 ? "+" : ""}${x.rg_track_gain.toFixed(1)} dB` : ""}</span></li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .at {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 760px;
  }
  .lead,
  .hint {
    margin: 0;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .hint {
    font-size: var(--text-xs);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-2);
  }
  .card-s {
    display: flex;
    flex-direction: column;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
  }
  .card-s b {
    font-size: var(--text-2xl);
    font-weight: 750;
    font-variant-numeric: tabular-nums;
  }
  .card-s span {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .progress {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .bar {
    flex: 1;
    height: 6px;
    border-radius: 3px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.3s;
  }
  .acts {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  h3 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-md);
  }
  .loud {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .loud li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    padding: 4px 0;
    font-size: var(--text-sm);
    border-bottom: var(--hairline) solid var(--content-border);
  }
  .n {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .v {
    color: var(--text-dim);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
</style>
