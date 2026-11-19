<script lang="ts">
  import { moduleById } from "./registry";
  import { onDestroy, onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { formatBytes } from "$lib/washmybelly/api";
  import {
    addTick,
    emptySeries,
    factorValue,
    formatPercent,
    formatRate,
    formatTemp,
    memoryPercent,
    type LiveTick,
    type SystemStatus,
  } from "$lib/washmybelly/monitor";
  import type { Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import HealthRing from "./ui/HealthRing.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import MonitorChart from "./MonitorChart.svelte";
  import OverviewGains from "./OverviewGains.svelte";
  import ProcessesTable from "./ProcessesTable.svelte";

  let _props: { caps: Caps } = $props();

  let status = $state<SystemStatus | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let tick = $state<LiveTick | null>(null);
  let series = $state(emptySeries());
  let unlisten: UnlistenFn | null = null;
  let stopped = false;

  async function refresh() {
    loading = true;
    error = null;
    try {
      status = await api.call<SystemStatus>("wmb_system_status");
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    refresh();
    const u = await listen<LiveTick>("washmybelly-monitor", (e) => {
      tick = e.payload;
      series = addTick(series, e.payload);
    });
    if (stopped) {
      u();
      return;
    }
    unlisten = u;
    api.monitorStart().catch((e) => (error = String(e)));
  });

  onDestroy(() => {
    stopped = true;
    unlisten?.();
    api.monitorStop().catch(() => {});
  });

  function openModule(id: string) {
    void goto(`/washmybelly?m=${id}`, { reset: false, replace: true });
  }

  let problems = $derived(status ? status.factors.filter((f) => f.penalty > 0) : []);
  let fine = $derived(status ? status.factors.filter((f) => f.penalty === 0) : []);
  let hasGpu = $derived(series.gpu.length > 0);
  let hasTemp = $derived(series.temp.length > 0);
</script>

<div class="overview">
  <ModuleHeader tile={moduleById("overview")} title={$t("washmybelly.modules.overview") as string} subtitle={$t("washmybelly.monitor.subtitle") as string}>
    {#snippet actions()}
      <button class="btn btn-secondary" onclick={refresh} disabled={loading}>{$t("washmybelly.monitor.refresh")}</button>
    {/snippet}
  </ModuleHeader>

  <div class="content">
    {#if error && !status}
      <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
        {#snippet action()}<button class="btn" onclick={refresh}>{$t("washmybelly.retry")}</button>{/snippet}
      </EmptyState>
    {:else if !status}
      <EmptyState art="searching" title={$t("washmybelly.loading") as string} />
    {:else}
      <section class="health card">
        <div class="ring">
          <HealthRing score={status.score} size={132} />
          <span class="grade {status.grade}">{$t(`washmybelly.monitor.grade.${status.grade}`)}</span>
        </div>
        <div class="factors">
          <h3>{problems.length ? $t("washmybelly.monitor.lowers_score") : $t("washmybelly.monitor.all_good")}</h3>
          <ul>
            {#each [...problems, ...fine] as f (f.id)}
              <li class={f.severity}>
                <span class="dot" aria-hidden="true"></span>
                <span class="name">{$t(`washmybelly.monitor.factors.${f.id}`)}</span>
                <span class="val">{factorValue(f, formatBytes)}</span>
                {#if f.penalty > 0}<span class="pen">−{f.penalty}</span>{/if}
              </li>
            {/each}
          </ul>
          {#if !status.hooks.junk || !status.hooks.login_items}
            <p class="meta">{$t("washmybelly.monitor.more_after_scans")}</p>
          {/if}
        </div>
      </section>

      <OverviewGains gains={status.gains} onOpen={openModule} />

      <section class="facts">
        <div class="fact"><span class="l">{$t("washmybelly.monitor.cpu")}</span><span class="v">{status.cpu.brand ?? "—"}</span><span class="s">{$t("washmybelly.monitor.cores", { count: status.cpu.cores })}</span></div>
        <div class="fact"><span class="l">{$t("washmybelly.monitor.memory")}</span><span class="v">{formatBytes(status.memory.total)}</span><span class="s">{$t("washmybelly.monitor.swap", { size: formatBytes(status.memory.swap_used) })}</span></div>
        {#if status.disk}
          <div class="fact"><span class="l">{$t("washmybelly.monitor.disk_free")}</span><span class="v">{formatBytes(status.disk.free)}</span><span class="s">{$t("washmybelly.monitor.of_total", { total: formatBytes(status.disk.total) })}</span></div>
        {/if}
        {#if status.battery?.present}
          <div class="fact">
            <span class="l">{$t("washmybelly.monitor.battery")}</span>
            <span class="v">{status.battery.health_percent !== undefined ? `${Math.round(status.battery.health_percent)}%` : "—"}</span>
            <span class="s">{status.battery.cycles !== undefined ? $t("washmybelly.monitor.cycles", { count: status.battery.cycles }) : $t(`washmybelly.monitor.battery_state.${status.battery.state}`)}</span>
          </div>
        {/if}
      </section>
    {/if}

    <section class="live">
      <h3>{$t("washmybelly.monitor.live")}</h3>
      <div class="charts">
        <MonitorChart label={$t("washmybelly.monitor.cpu") as string} value={formatPercent(tick?.cpu.total)} values={series.cpu} max={100} />
        <MonitorChart
          label={$t("washmybelly.monitor.memory") as string}
          value={tick ? formatPercent(memoryPercent(tick.memory)) : "—"}
          values={series.memory}
          max={100}
          color="#30d158"
          sub={tick ? `${formatBytes(tick.memory.used)} / ${formatBytes(tick.memory.total)}` : ""}
        />
        {#if hasGpu}
          <MonitorChart label={$t("washmybelly.monitor.gpu") as string} value={formatPercent(tick?.gpu_percent)} values={series.gpu} max={100} color="#bf5af2" />
        {/if}
        {#if hasTemp}
          <MonitorChart label={$t("washmybelly.monitor.temperature") as string} value={formatTemp(tick?.temp_c)} values={series.temp} max={110} color="#ff9f0a" />
        {/if}
        <MonitorChart
          label={$t("washmybelly.monitor.network") as string}
          value={formatRate(tick?.net.rx_per_s)}
          values={series.rx}
          second={series.tx}
          floor={10_000}
          color="#0a84ff"
          sub={tick ? $t("washmybelly.monitor.net_sub", { down: formatRate(tick.net.rx_per_s), up: formatRate(tick.net.tx_per_s) }) as string : ""}
        />
        <MonitorChart
          label={$t("washmybelly.monitor.disk_io") as string}
          value={formatRate(tick ? tick.disk.read_per_s + tick.disk.write_per_s : 0)}
          values={series.read}
          second={series.write}
          floor={100_000}
          color="#64d2ff"
          sub={tick ? $t("washmybelly.monitor.disk_sub", { read: formatRate(tick.disk.read_per_s), write: formatRate(tick.disk.write_per_s) }) as string : ""}
        />
      </div>
      {#if tick && tick.cpu.cores.length > 1}
        <div class="cores" aria-label={$t("washmybelly.monitor.per_core") as string}>
          {#each tick.cpu.cores as c, i (i)}
            <span class="core" title={`${i + 1}: ${Math.round(c)}%`}><span class="fill" style:height="{Math.min(100, c)}%"></span></span>
          {/each}
        </div>
      {/if}
    </section>

    <ProcessesTable />
  </div>
</div>

<style>
  .overview { display: flex; flex-direction: column; min-height: 0; }
  .content { display: flex; flex-direction: column; gap: 16px; padding: 0 24px 24px; }
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 16px 20px; }
  .health { display: flex; gap: 24px; align-items: center; flex-wrap: wrap; }
  .ring { display: flex; flex-direction: column; align-items: center; gap: 6px; color: var(--text); }
  .grade { font-size: 12px; font-weight: 600; color: var(--text-dim); }
  .grade.excellent, .grade.good { color: #30d158; }
  .grade.fair { color: #ff9f0a; }
  .grade.poor { color: var(--danger, #ff453a); }
  .factors { flex: 1; min-width: 240px; display: flex; flex-direction: column; gap: 8px; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: grid; grid-template-columns: 10px 1fr auto 36px; align-items: center; gap: 8px; padding: 6px 0; font-size: 13px; border-top: var(--hairline, 1px) solid var(--separator); }
  li:first-child { border-top: none; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: #30d158; }
  .warn .dot { background: #ff9f0a; }
  .bad .dot { background: var(--danger, #ff453a); }
  .val { font-variant-numeric: tabular-nums; color: var(--text-dim); }
  .pen { font-size: 12px; font-weight: 600; text-align: right; color: var(--danger, #ff453a); font-variant-numeric: tabular-nums; }
  .warn .pen { color: #ff9f0a; }
  .meta { margin: 0; font-size: 11.5px; color: var(--text-dim); }
  .facts { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 12px; }
  .fact { display: flex; flex-direction: column; gap: 2px; padding: 10px 14px; border-radius: var(--radius-lg, 12px); background: var(--surface-mut, var(--separator)); min-width: 0; }
  .fact .l, .fact .s { font-size: 11px; color: var(--text-dim); }
  .fact .v { font-size: 15px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .live { display: flex; flex-direction: column; gap: 10px; }
  .charts { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 12px; }
  .cores { display: flex; gap: 3px; height: 28px; align-items: flex-end; }
  .core { flex: 1; max-width: 18px; height: 100%; border-radius: 3px; background: var(--surface-mut, var(--separator)); display: flex; align-items: flex-end; overflow: hidden; }
  .core .fill { width: 100%; background: var(--accent); border-radius: 3px; transition: height 0.4s ease; }
  .btn { font: inherit; font-size: 12px; padding: 4px 12px; border-radius: 7px; border: none; cursor: pointer; background: var(--surface-mut, var(--separator)); color: var(--text); }
  .btn:focus-visible { outline: 2px solid var(--accent); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  @media (prefers-reduced-motion: reduce) { .core .fill { transition: none; } }
</style>
