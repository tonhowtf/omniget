<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { fmtBytes, fmtLong } from "$lib/music/library";
  import { trackById } from "$lib/music/library-store.svelte";
  import { playTracks } from "$lib/music/player-store.svelte";
  import { go } from "$lib/music/ui-store.svelte";
  import type { Stats } from "$lib/music/types";
  import Cover from "../Cover.svelte";

  let stats = $state<Stats | null>(null);
  let hover = $state<{ x: number; label: string } | null>(null);

  onMount(async () => {
    stats = await invoke<Stats>("music_stats", { tzOffsetMin: -new Date().getTimezoneOffset() }).catch(() => null);
  });

  // last 30 days, one bar per day, zero-filled
  let days = $derived.by(() => {
    if (!stats) return [];
    const map = new Map(stats.per_day.map(([d, ms]) => [new Date(d).toDateString(), ms]));
    const out: { label: string; ms: number }[] = [];
    for (let i = 29; i >= 0; i--) {
      const d = new Date(Date.now() - i * 86_400_000);
      out.push({ label: d.toLocaleDateString(undefined, { day: "numeric", month: "short" }), ms: map.get(d.toDateString()) ?? 0 });
    }
    return out;
  });
  let dayMax = $derived(Math.max(1, ...days.map((d) => d.ms)));
  let hourMax = $derived(Math.max(1, ...(stats?.per_hour ?? [0])));
  let fmtMax = $derived(Math.max(1, ...(stats?.formats.map((f) => f[1]) ?? [0])));
  let top = $derived((stats?.top_tracks ?? []).map(([id, n]) => ({ t: trackById(id), n })).filter((x) => x.t));
</script>

<div class="stv">
  <h1>{$t("music.nav.stats")}</h1>
  {#if !stats}
    <p class="dim">…</p>
  {:else}
    <div class="tiles">
      <div class="tile"><b>{stats.tracks.toLocaleString()}</b><span>{$t("music.stats.tracks")}</span></div>
      <div class="tile"><b>{stats.albums.toLocaleString()}</b><span>{$t("music.stats.albums")}</span></div>
      <div class="tile"><b>{stats.artists.toLocaleString()}</b><span>{$t("music.stats.artists")}</span></div>
      <div class="tile"><b>{fmtLong(stats.total_ms)}</b><span>{$t("music.stats.duration")}</span></div>
      <div class="tile"><b>{fmtBytes(stats.total_bytes)}</b><span>{$t("music.stats.size")}</span></div>
      <div class="tile hl"><b>{fmtLong(stats.listened_ms_30d)}</b><span>{$t("music.stats.listened_30d", { plays: stats.plays_30d })}</span></div>
    </div>

    <section class="chart">
      <h2>{$t("music.stats.per_day")}</h2>
      <div class="bars days" role="img" aria-label={$t("music.stats.per_day") as string} onpointerleave={() => (hover = null)}>
        {#each days as d, i (i)}
          <div
            class="col"
            role="presentation"
            onpointerenter={() => (hover = { x: i / days.length, label: `${d.label}: ${fmtLong(d.ms)}` })}
          >
            <span class="bar" style:height="{Math.max(d.ms ? 2 : 0, (d.ms / dayMax) * 100)}%"></span>
          </div>
        {/each}
        {#if hover}<span class="tip" style:left="{hover.x * 100}%">{hover.label}</span>{/if}
      </div>
      <div class="axis"><span>{days[0]?.label}</span><span>{days[days.length - 1]?.label}</span></div>
    </section>

    <section class="chart">
      <h2>{$t("music.stats.per_hour")}</h2>
      <div class="bars hours" role="img" aria-label={$t("music.stats.per_hour") as string}>
        {#each stats.per_hour as ms, h (h)}
          <div class="col" title="{h}:00 · {fmtLong(ms)}"><span class="bar" style:height="{Math.max(ms ? 2 : 0, (ms / hourMax) * 100)}%"></span></div>
        {/each}
      </div>
      <div class="axis"><span>0h</span><span>6h</span><span>12h</span><span>18h</span><span>23h</span></div>
    </section>

    <div class="cols">
      <section>
        <h2>{$t("music.stats.top_tracks")}</h2>
        <ol class="rank">
          {#each top.slice(0, 15) as r, i (r.t!.id)}
            <li>
              <span class="pos">{i + 1}</span>
              <button class="rk" onclick={() => r.t && void playTracks(top.map((x) => x.t!.id), r.t.id)}>
                <Cover hash={r.t!.cover} seed={r.t!.album} size={32} radius={4} />
                <span class="nm"><b>{r.t!.title}</b><small>{r.t!.artist}</small></span>
              </button>
              <span class="n">{r.n}×</span>
            </li>
          {:else}
            <li class="dim">{$t("music.stats.no_plays")}</li>
          {/each}
        </ol>
      </section>
      <section>
        <h2>{$t("music.stats.top_artists")}</h2>
        <ol class="rank">
          {#each stats.top_artists as [name, n], i (name)}
            <li>
              <span class="pos">{i + 1}</span>
              <button class="rk" onclick={() => go({ kind: "artist", name })}><span class="nm"><b>{name}</b></span></button>
              <span class="n">{n}×</span>
            </li>
          {:else}
            <li class="dim">{$t("music.stats.no_plays")}</li>
          {/each}
        </ol>
        <h2 class="mt">{$t("music.stats.formats")}</h2>
        <ul class="hbars">
          {#each stats.formats as [codec, n] (codec)}
            <li><span class="lbl">{codec || "?"}</span><span class="track"><i style:width="{(n / fmtMax) * 100}%"></i></span><span class="n">{n}</span></li>
          {/each}
        </ul>
      </section>
    </div>
  {/if}
</div>

<style>
  .stv {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-md);
    font-weight: 650;
  }
  .mt {
    margin-top: var(--space-4);
  }
  .dim {
    color: var(--text-dim);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-2);
  }
  .tile {
    display: flex;
    flex-direction: column;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
  }
  .tile.hl {
    background: var(--accent-soft);
  }
  .tile b {
    font-size: var(--text-xl);
    font-weight: 750;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .tile span {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .bars {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 120px;
    border-bottom: var(--hairline) solid var(--content-border);
  }
  .bars.hours {
    height: 80px;
  }
  .col {
    flex: 1;
    height: 100%;
    display: flex;
    align-items: flex-end;
  }
  .bar {
    width: 100%;
    border-radius: 4px 4px 0 0;
    background: var(--accent);
    opacity: 0.85;
  }
  .col:hover .bar {
    opacity: 1;
  }
  .tip {
    position: absolute;
    top: -6px;
    transform: translate(-30%, -100%);
    padding: 3px 7px;
    border-radius: 5px;
    background: var(--surface-hi);
    box-shadow: 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-xs);
    color: var(--text);
    white-space: nowrap;
    pointer-events: none;
  }
  .axis {
    display: flex;
    justify-content: space-between;
    font-size: var(--text-caption);
    color: var(--text-muted);
    margin-top: 4px;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: var(--space-5);
  }
  .rank {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rank li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 3px 0;
  }
  .pos {
    width: 22px;
    text-align: right;
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .rk {
    all: unset;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 6px;
  }
  .rk:hover {
    background: var(--fill-1);
  }
  .rk:focus-visible {
    outline: var(--focus-ring);
  }
  .nm {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .nm b,
  .nm small {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .nm b {
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .nm small {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .n {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .hbars {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .hbars li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .lbl {
    width: 70px;
    color: var(--text-dim);
  }
  .track {
    flex: 1;
    height: 8px;
  }
  .track i {
    display: block;
    height: 100%;
    border-radius: 0 4px 4px 0;
    background: var(--accent);
  }
</style>
