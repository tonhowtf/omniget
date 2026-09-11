<script lang="ts">
  /** Histórico completo do export do Spotify — tudo local, sem rede. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Bucket = { key: string; plays: number; ms: number };
  type ArtistStat = { name: string; plays: number; ms: number; skips: number; skip_rate: number; first_ts: string; first_track: string; last_ts: string };
  type TrackStat = { track: string; artist: string; plays: number; ms: number };
  type YearTop = { year: string; artist: string; track: string; ms: number };
  type Dropped = { name: string; before_ms: number; after_ms: number; last_ts: string };
  type Result = {
    files: string[];
    plays: number;
    total_ms: number;
    distinct_artists: number;
    distinct_tracks: number;
    first_ts: string;
    last_ts: string;
    by_year: Bucket[];
    by_month: Bucket[];
    by_weekday: Bucket[];
    by_hour: Bucket[];
    top_artists: ArtistStat[];
    top_tracks: TrackStat[];
    top_albums: TrackStat[];
    year_top: YearTop[];
    skipped_artists: ArtistStat[];
    firsts: ArtistStat[];
    streaks: { longest_days: number; longest_start: string; longest_end: string; current_days: number; active_days: number };
    dropped: Dropped[];
    exports: string[];
  };

  let inputs = $state<string[]>([]);
  let outDir = $state<string | null>(null);
  let minSecs = $state(30);
  let top = $state(25);
  let asJson = $state(true);
  let asCsv = $state(true);
  let asMd = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const hours = (ms: number) => Math.round((ms / 3600000) * 10) / 10;
  const bar = (v: number, max: number) => (max > 0 ? Math.max(2, Math.round((v / max) * 100)) : 0);
  const maxOf = (b: Bucket[]) => b.reduce((m, x) => Math.max(m, x.ms), 0);

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "music-history") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!inputs.length || busy) return;
    busy = true;
    result = null;
    progress = null;
    const formats: string[] = [];
    if (outDir) {
      if (asJson) formats.push("json");
      if (asCsv) formats.push("csv");
      if (asMd) formats.push("md");
    }
    try {
      result = await invoke<Result>("tool_music_history", {
        opts: { inputs, min_ms: minSecs * 1000, out_dir: outDir, formats, top },
      });
      showToast("success", `${result.plays}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.mhist.inputs")}</div>
          <div class="group-row-sub mono">{inputs.join(" · ") || $t("tools.mhist.inputs_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if inputs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (inputs = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "Zip", extensions: ["zip", "json"] }]); if (f) inputs = [...inputs, f]; }}>{$t("tools.mhist.add_zip")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) inputs = [...inputs, d]; }}>{$t("tools.mhist.add_folder")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.mhist.min_secs")}</div>
          <div class="group-row-sub">{$t("tools.mhist.min_secs_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row"><input class="input" type="number" min="0" max="120" step="5" bind:value={minSecs} style:width="6em" /><span class="dim">s</span></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.top")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="5" max="200" step="5" bind:value={top} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.mhist.export_to")}</div>
          <div class="group-row-sub mono">{outDir ?? $t("tools.common.optional")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if outDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (outDir = null)}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      {#if outDir}
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.formats")}</div></div>
          <div class="group-row-trailing btn-row">
            <label><input type="checkbox" bind:checked={asJson} /> JSON</label>
            <label><input type="checkbox" bind:checked={asCsv} /> CSV</label>
            <label><input type="checkbox" bind:checked={asMd} /> Markdown</label>
          </div>
        </div>
      {/if}
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !inputs.length} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.mhist.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{hours(result.total_ms)} h · {result.plays} {$t("tools.mhist.plays")}</div>
            <div class="group-row-sub">{result.first_ts} → {result.last_ts} · {result.distinct_artists} {$t("tools.mhist.artists")} · {result.distinct_tracks} {$t("tools.mhist.tracks")}</div>
          </div>
        </div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.streaks.longest_days} {$t("tools.mhist.streak")}</div>
            <div class="group-row-sub">{result.streaks.longest_start} → {result.streaks.longest_end} · {result.streaks.active_days} {$t("tools.mhist.active_days")}</div>
          </div>
        </div>
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.by_year")}</div></div></div>
        {#each result.by_year as b (b.key)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub">{b.key} · {hours(b.ms)} h</div>
              <div class="bar"><div class="bar-fill" style:width="{bar(b.ms, maxOf(result.by_year))}%"></div></div>
            </div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.by_hour")}</div></div></div>
        <div class="group-row">
          <div class="group-row-content">
            <div class="chart">
              {#each result.by_hour as b (b.key)}
                <div class="col" title="{b.key}h">
                  <div class="col-fill" style:height="{bar(b.ms, maxOf(result.by_hour))}%"></div>
                </div>
              {/each}
            </div>
          </div>
        </div>
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.by_weekday")}</div></div></div>
        {#each result.by_weekday as b (b.key)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub">{$t(`tools.mhist.wd_${b.key}`)} · {hours(b.ms)} h</div>
              <div class="bar"><div class="bar-fill" style:width="{bar(b.ms, maxOf(result.by_weekday))}%"></div></div>
            </div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.top_artists")}</div></div></div>
        {#each result.top_artists as a, i (a.name)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub">{i + 1}. {a.name} · {hours(a.ms)} h · {a.plays}</div>
              <div class="group-row-sub dim">{$t("tools.mhist.first_time")} {a.first_ts} — {a.first_track} · {$t("tools.mhist.skip_rate")} {a.skip_rate}%</div>
            </div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="group">
        <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.top_tracks")}</div></div></div>
        {#each result.top_tracks as tr, i (`${tr.track}-${tr.artist}`)}
          <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{i + 1}. {tr.track} — {tr.artist} · {hours(tr.ms)} h · {tr.plays}</div></div></div>
        {/each}
      </div>
    </section>

    {#if result.top_albums.length}
      <section>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.top_albums")}</div></div></div>
          {#each result.top_albums as al, i (`${al.track}-${al.artist}`)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{i + 1}. {al.track} — {al.artist} · {hours(al.ms)} h</div></div></div>
          {/each}
        </div>
      </section>
    {/if}

    {#if result.year_top.length}
      <section>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.year_top")}</div></div></div>
          {#each result.year_top as y (y.year)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{y.year} · {y.artist} · {y.track}</div></div></div>
          {/each}
        </div>
      </section>
    {/if}

    {#if result.skipped_artists.length}
      <section>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.mhist.skipped")}</div>
              <div class="group-row-sub">{$t("tools.mhist.skipped_hint")}</div>
            </div>
          </div>
          {#each result.skipped_artists as a (a.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{a.name} · <strong>{a.skip_rate}%</strong> · {a.skips}/{a.plays}</div></div></div>
          {/each}
        </div>
      </section>
    {/if}

    {#if result.firsts.length}
      <section>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.mhist.firsts")}</div>
              <div class="group-row-sub">{$t("tools.mhist.firsts_hint")}</div>
            </div>
          </div>
          {#each result.firsts as a (a.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{a.first_ts} · {a.name} — {a.first_track}</div></div></div>
          {/each}
        </div>
      </section>
    {/if}

    {#if result.dropped.length}
      <section>
        <div class="group">
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{$t("tools.mhist.dropped")}</div>
              <div class="group-row-sub">{$t("tools.mhist.dropped_hint")}</div>
            </div>
          </div>
          {#each result.dropped as d (d.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{d.name} · {hours(d.before_ms)} h → {hours(d.after_ms)} h · {$t("tools.mhist.last_time")} {d.last_ts}</div></div></div>
          {/each}
        </div>
      </section>
    {/if}

    {#if result.exports.length}
      <section>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.mhist.exports")}</div></div></div>
          {#each result.exports as f (f)}
            <div class="group-row">
              <div class="group-row-content"><div class="group-row-sub mono">{f}</div></div>
              <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(f)}>↗</button></div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .bar { height: 6px; border-radius: 3px; background: var(--fill-1); overflow: hidden; margin-top: 4px; }
  .bar-fill { height: 100%; background: var(--accent); border-radius: 3px; }
  .chart { display: flex; align-items: flex-end; gap: 2px; height: 72px; }
  .col { flex: 1; height: 100%; display: flex; align-items: flex-end; }
  .col-fill { width: 100%; background: var(--accent); border-radius: 2px 2px 0 0; }
</style>
