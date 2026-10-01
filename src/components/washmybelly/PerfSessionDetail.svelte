<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import { formatFps, formatMs, smoothness, type Chart, type Summary } from "$lib/washmybelly/perf";
  import StatTile from "./ui/StatTile.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import FrametimeChart from "./FrametimeChart.svelte";
  import FrametimeHistogram from "./FrametimeHistogram.svelte";

  let { sessionId, onDelete }: { sessionId: string; onDelete?: () => void } = $props();

  let summary = $state<Summary | null>(null);
  let chart = $state<Chart | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let confirmDelete = $state(false);

  $effect(() => {
    const id = sessionId;
    loading = true;
    error = null;
    summary = null;
    chart = null;
    Promise.all([api.call<Summary>("wmb_perf_summary", { session_id: id }), api.ui<Chart>("perf.chart", { session_id: id, points: 700 })])
      .then(([s, c]) => {
        if (id !== sessionId) return;
        summary = s;
        chart = c;
      })
      .catch((e) => (error = errorText(e)))
      .finally(() => (loading = false));
  });

  async function remove() {
    try {
      await api.ui("perf.delete_session", { session_id: sessionId });
      onDelete?.();
    } catch (e) {
      error = errorText(e);
    }
  }

  let tone = $derived(summary ? smoothness(summary.fps_avg, summary.low_1) : "default");
  let secondary = $derived(
    summary
      ? [
          ["median", formatFps(summary.fps_median)],
          ["p1", formatFps(summary.fps_p1)],
          ["low_1_integral", formatFps(summary.low_1_integral)],
          ["low_01_integral", formatFps(summary.low_01_integral)],
          ["frametime_avg", formatMs(summary.frametime_avg_ms)],
          ["frametime_p99", formatMs(summary.frametime_p99_ms)],
          ["stutter_time", summary.stutter_time_pct == null ? "—" : `${summary.stutter_time_pct.toFixed(2)}%`],
          ["low_fps_time", summary.low_fps_time_pct == null ? "—" : `${summary.low_fps_time_pct.toFixed(2)}%`],
        ]
      : [],
  );
</script>

{#if loading}
  <p class="dim">{$t("washmybelly.perf.loading")}</p>
{:else if error}
  <p class="error" role="alert">{error}</p>
{:else if summary}
  {#if summary.frames === 0}
    <EmptyState
      art="worried"
      compact
      title={$t("washmybelly.perf.no_frames_title") as string}
      body={$t(`washmybelly.perf.reason.${summary.reason ?? "no_frames_seen"}`) as string}
    />
  {:else}
    <div class="tiles">
      <StatTile label={$t("washmybelly.perf.fps_avg") as string} value={formatFps(summary.fps_avg)} unit="FPS" tone="accent" sub={$t("washmybelly.perf.frames_count", { count: summary.frames.toLocaleString() }) as string} />
      <StatTile label={$t("washmybelly.perf.low_1") as string} value={formatFps(summary.low_1)} unit="FPS" {tone} />
      <StatTile label={$t("washmybelly.perf.low_01") as string} value={formatFps(summary.low_01)} unit="FPS" {tone} />
      <StatTile label={$t("washmybelly.perf.stutters") as string} value={summary.stutters.toLocaleString()} tone={summary.stutters > 0 ? "warn" : "good"} />
      <StatTile label={$t("washmybelly.perf.freezes") as string} value={summary.freezes.toLocaleString()} tone={summary.freezes > 0 ? "bad" : "good"} />
    </div>

    {#if chart}
      <section class="card">
        <div class="head">
          <h4>{$t("washmybelly.perf.timeline_title")}</h4>
          <span class="dim">{$t(summary.source === "display" ? "washmybelly.perf.source_display" : "washmybelly.perf.source_present")}</span>
        </div>
        <FrametimeChart {chart} />
        <div class="legend">
          <span><i class="k stutter"></i>{$t("washmybelly.perf.legend_stutter")}</span>
          <span><i class="k freeze"></i>{$t("washmybelly.perf.legend_freeze")}</span>
        </div>
      </section>
      <section class="card">
        <div class="head"><h4>{$t("washmybelly.perf.histogram_title")}</h4></div>
        <FrametimeHistogram histogram={chart.histogram} />
      </section>
    {/if}

    <section class="card">
      <div class="head"><h4>{$t("washmybelly.perf.more_title")}</h4></div>
      <dl class="grid">
        {#each secondary as [k, v] (k)}
          <div><dt>{$t(`washmybelly.perf.stat.${k}`)}</dt><dd>{v}</dd></div>
        {/each}
      </dl>
      <p class="formula" title={summary.formulas}>{$t("washmybelly.perf.formulas_note")}</p>
    </section>
  {/if}

  <section class="card">
    <div class="head"><h4>{$t("washmybelly.perf.sensors_title")}</h4></div>
    {#if summary.sensors.samples > 0}
      <dl class="grid">
        <div><dt>{$t("washmybelly.perf.cpu_avg")}</dt><dd>{summary.sensors.cpu_load_avg == null ? "—" : `${summary.sensors.cpu_load_avg}%`}</dd></div>
        <div><dt>{$t("washmybelly.perf.gpu_avg")}</dt><dd>{summary.sensors.gpu_load_avg == null ? "—" : `${summary.sensors.gpu_load_avg}%`}</dd></div>
        <div><dt>{$t("washmybelly.perf.cpu_temp_max")}</dt><dd>{summary.sensors.cpu_temp_max == null ? "—" : `${summary.sensors.cpu_temp_max} °C`}</dd></div>
        <div><dt>{$t("washmybelly.perf.gpu_temp_max")}</dt><dd>{summary.sensors.gpu_temp_max == null ? "—" : `${summary.sensors.gpu_temp_max} °C`}</dd></div>
      </dl>
    {/if}
    {#if summary.correlation.findings.length}
      <ul class="findings">
        {#each summary.correlation.findings as f (f)}
          <li>{$t(`washmybelly.perf.finding.${f}`, { count: summary.correlation.stutters_hot, pct: summary.correlation.gpu_full_pct ?? 0 })}</li>
        {/each}
      </ul>
    {:else}
      <p class="dim">{$t("washmybelly.perf.finding_none")}</p>
    {/if}
    {#if summary.warnings.length}
      <ul class="warnings">
        {#each summary.warnings as w (w)}<li>{$t(`washmybelly.perf.warning.${w}`)}</li>{/each}
      </ul>
    {/if}
  </section>

  <div class="actions">
    {#if confirmDelete}
      <span class="dim">{$t("washmybelly.perf.delete_confirm")}</span>
      <button class="btn btn-ghost btn-sm" type="button" onclick={() => (confirmDelete = false)}>{$t("washmybelly.perf.cancel")}</button>
      <button class="btn btn-destructive btn-sm" type="button" onclick={remove}>{$t("washmybelly.perf.delete")}</button>
    {:else}
      <button class="btn btn-ghost btn-sm" type="button" onclick={() => (confirmDelete = true)}>{$t("washmybelly.perf.delete")}</button>
    {/if}
  </div>
{/if}

<style>
  .tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(128px, 1fr)); gap: 10px; }
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; }
  .head { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
  h4 { margin: 0; font-size: 13px; font-weight: 600; }
  .dim { margin: 0; font-size: 11.5px; color: var(--text-dim); }
  .error { margin: 0; font-size: 12px; color: var(--danger); }
  .legend { display: flex; gap: 14px; font-size: 11px; color: var(--text-dim); }
  .legend span { display: inline-flex; align-items: center; gap: 5px; }
  .k { display: inline-block; width: 2px; height: 9px; border-radius: 1px; }
  .k.stutter { background: var(--warning); }
  .k.freeze { background: var(--danger); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 0; margin: 0; }
  .grid div { display: flex; justify-content: space-between; gap: 8px; padding: 6px 0; border-bottom: var(--hairline, 1px) solid var(--content-border, var(--separator)); margin-right: 16px; }
  dt { font-size: 12px; color: var(--text-dim); }
  dd { margin: 0; font-size: 12.5px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .formula { margin: 0; font-size: 11px; color: var(--text-dim); cursor: help; }
  .findings, .warnings { margin: 0; padding-left: 18px; font-size: 12.5px; display: flex; flex-direction: column; gap: 4px; }
  .warnings { color: var(--text-dim); font-size: 11.5px; }
  .actions { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
</style>
