<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { errorText, formatTime } from "$lib/washmybelly/format";
  import { candidates } from "$lib/washmybelly/game";
  import type { ProcessList, ProcessRow } from "$lib/washmybelly/monitor";
  import { formatClock, formatFps, type LaunchCommand, type LiveStatus, type PerfCaps, type SessionSummary } from "$lib/washmybelly/perf";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Sparkline from "./ui/Sparkline.svelte";
  import Icon from "./ui/Icon.svelte";
  import PerfSessionDetail from "./PerfSessionDetail.svelte";
  import PerfCompare from "./PerfCompare.svelte";

  let { caps }: { caps: Caps } = $props();

  const mod = moduleById("perf");
  const DURATIONS = [30, 60, 120, 300, 600, 1800];

  let pcaps = $state<PerfCaps | null>(null);
  let live = $state<LiveStatus>({ active: false });
  let sessions = $state<SessionSummary[]>([]);
  let procs = $state<ProcessRow[]>([]);
  let selected = $state<string | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  let pid = $state<number | null>(null);
  let processName = $state("");
  let seconds = $state(60);
  let label = $state("");
  let setup = $state<{ commands: string[]; undo_commands: string[]; group: string } | null>(null);
  let copied = $state<string | null>(null);

  let poll: ReturnType<typeof setInterval> | null = null;

  let picks = $derived(candidates(procs, [], 10));
  let os = $derived(pcaps?.os ?? caps.os);

  async function loadSessions() {
    const r = await api.call<{ sessions: SessionSummary[] }>("wmb_perf_sessions", { limit: 50 });
    sessions = r.sessions ?? [];
  }

  async function loadProcs() {
    if (os === "linux") return;
    try {
      const r = await api.call<ProcessList>("wmb_processes", { sort: "cpu", limit: 80 });
      procs = r.processes;
    } catch {
      procs = [];
    }
  }

  async function tick() {
    const was = live.active;
    try {
      live = await api.ui<LiveStatus>("perf.status");
    } catch {
      live = { active: false };
    }
    if (was && !live.active) {
      await loadSessions();
      selected = sessions[0]?.session_id ?? null;
    }
  }

  onMount(async () => {
    try {
      pcaps = await api.call<PerfCaps>("wmb_perf_capabilities");
    } catch (e) {
      error = errorText(e);
    }
    await Promise.all([loadSessions().catch((e) => (error = errorText(e))), loadProcs(), tick()]);
    poll = setInterval(tick, 1000);
  });
  onDestroy(() => {
    if (poll) clearInterval(poll);
  });

  async function start() {
    busy = true;
    error = null;
    try {
      const input: Record<string, unknown> = { max_seconds: seconds };
      if (label.trim()) input.label = label.trim();
      if (os !== "linux" && pid) input.pid = pid;
      else if (processName.trim()) input.process_name = processName.trim();
      await api.call("wmb_perf_capture_start", input);
      await tick();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function launchMac() {
    const path = await openDialog({ directory: false, multiple: false, defaultPath: "/Applications" });
    if (!path || Array.isArray(path)) return;
    busy = true;
    error = null;
    try {
      await api.ui("perf.launch", { path, max_seconds: seconds, ...(label.trim() ? { label: label.trim() } : {}) });
      await tick();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function stop() {
    busy = true;
    try {
      await api.call("wmb_perf_capture_stop", {});
      await tick();
      await loadSessions();
      selected = sessions[0]?.session_id ?? null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function prepareSetup() {
    error = null;
    try {
      setup = await api.ui("perf.setup_access");
    } catch (e) {
      error = errorText(e);
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = text;
      setTimeout(() => (copied = copied === text ? null : copied), 1500);
    } catch {
    }
  }

  function when(s: SessionSummary): string {
    return formatTime(Math.floor(s.started_ms / 1000), undefined, true);
  }

  let liveLaunch = $derived<LaunchCommand[]>(live.active ? live.launch : []);
</script>

<ModuleHeader title={$t("washmybelly.modules.perf") as string} subtitle={$t("washmybelly.perf.subtitle") as string} tile={mod}>
  {#snippet meta()}
    {#if pcaps}
      <span class="pill" class:ok={pcaps.available}>{$t(`washmybelly.perf.method.${pcaps.method}`)}</span>
    {/if}
  {/snippet}
</ModuleHeader>

<div class="perf">
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if pcaps && (!pcaps.available || pcaps.reason)}
    <section class="card note" class:warn={!pcaps.available}>
      <Icon name={pcaps.available ? "info" : "warning"} size={16} />
      <div class="grow">
        <p>{$t(`washmybelly.perf.reason.${pcaps.reason ?? "unavailable"}`)}</p>
        {#if os === "windows" && pcaps.windows?.setup_action}
          {#if setup}
            <p class="dim">{$t("washmybelly.perf.setup_manual", { group: setup.group })}</p>
            {#each setup.commands as c (c)}
              <div class="cmd"><code>{c}</code><button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(c)}><Icon name="copy" size={12} />{copied === c ? $t("washmybelly.perf.copied") : $t("washmybelly.perf.copy")}</button></div>
            {/each}
            <p class="dim">{$t("washmybelly.perf.setup_sign_out")}</p>
          {:else}
            <button class="btn btn-secondary btn-sm" type="button" onclick={prepareSetup}>{$t("washmybelly.perf.setup_button")}</button>
          {/if}
        {/if}
        {#if os === "linux" && pcaps.missing.includes("mangohud")}
          <p class="dim">{$t("washmybelly.perf.install_mangohud")}</p>
        {/if}
      </div>
    </section>
  {/if}

  <section class="card">
    {#if live.active}
      <div class="live">
        <div class="big">
          <span class="fps">{formatFps(live.fps_now)}</span>
          <span class="unit">FPS</span>
        </div>
        <div class="grow">
          <strong>{live.label}</strong>
          <p class="dim">
            {live.status === "waiting_launch" && live.frames === 0 ? $t("washmybelly.perf.waiting_launch") : $t("washmybelly.perf.recording")}
            · {formatClock(live.elapsed_s)}{#if live.max_seconds} / {formatClock(live.max_seconds)}{/if}
            · {$t("washmybelly.perf.frames_count", { count: live.frames.toLocaleString() })}
          </p>
        </div>
        {#if live.recent.length > 2}<Sparkline values={live.recent} width={140} height={36} />{/if}
        <button class="btn btn-primary btn-sm" type="button" disabled={busy} onclick={stop}>{$t("washmybelly.perf.stop")}</button>
      </div>
      {#if liveLaunch.length}
        <p class="dim">{$t("washmybelly.perf.launch_hint")}</p>
        {#each liveLaunch as l (l.kind)}
          <div class="cmd">
            <span class="kind">{$t(`washmybelly.perf.launch_kind.${l.kind}`)}</span>
            <code>{l.command}</code>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => copy(l.command)}><Icon name="copy" size={12} />{copied === l.command ? $t("washmybelly.perf.copied") : $t("washmybelly.perf.copy")}</button>
          </div>
        {/each}
      {/if}
    {:else}
      <h3>{$t("washmybelly.perf.new_capture")}</h3>
      {#if os !== "linux"}
        <p class="dim">{$t(os === "macos" ? "washmybelly.perf.pick_hint_mac" : "washmybelly.perf.pick_hint")}</p>
        {#if picks.length}
          <ul class="procs" role="listbox" aria-label={$t("washmybelly.perf.pick_game") as string}>
            {#each picks as p (p.pid)}
              <li>
                <button type="button" role="option" aria-selected={pid === p.pid} class:on={pid === p.pid} onclick={() => { pid = pid === p.pid ? null : p.pid; processName = ""; }}>
                  <span class="pname">{p.name}</span>
                  <span class="dim">PID {p.pid} · {p.cpu_percent.toFixed(0)}% CPU</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="row">
          <button class="btn btn-ghost btn-sm" type="button" onclick={loadProcs}><Icon name="refresh" size={12} />{$t("washmybelly.perf.refresh")}</button>
        </div>
      {:else}
        <p class="dim">{$t("washmybelly.perf.linux_hint")}</p>
      {/if}
      <div class="form">
        {#if os !== "macos"}
          <label>
            <span>{$t(os === "linux" ? "washmybelly.perf.game_name_optional" : "washmybelly.perf.process_name")}</span>
            <input type="text" bind:value={processName} placeholder={os === "windows" ? "League of Legends.exe" : ""} oninput={() => (pid = null)} />
          </label>
        {/if}
        <label>
          <span>{$t("washmybelly.perf.duration")}</span>
          <select bind:value={seconds}>
            {#each DURATIONS as d (d)}<option value={d}>{formatClock(d)}</option>{/each}
          </select>
        </label>
        <label>
          <span>{$t("washmybelly.perf.label")}</span>
          <input type="text" bind:value={label} placeholder={$t("washmybelly.perf.label_placeholder") as string} />
        </label>
      </div>
      <div class="row end">
        {#if os === "macos"}
          <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={launchMac}>{$t("washmybelly.perf.open_with_hud")}</button>
        {/if}
        <button
          class="btn btn-primary btn-sm"
          type="button"
          disabled={busy || !pcaps?.available || (os === "windows" && !pid && !processName.trim())}
          onclick={start}
        >{$t(os === "linux" ? "washmybelly.perf.prepare" : os === "macos" ? "washmybelly.perf.start_telemetry" : "washmybelly.perf.start")}</button>
      </div>
      {#if os === "windows" && pcaps?.presentmon?.download_on_first_use}
        <p class="dim">{$t("washmybelly.perf.presentmon_download", { version: pcaps.presentmon.version })}</p>
      {/if}
    {/if}
  </section>

  <section class="card">
    <h3>{$t("washmybelly.perf.sessions_title")}</h3>
    {#if sessions.length === 0}
      <EmptyState art="sleeping" compact title={$t("washmybelly.perf.empty_title") as string} body={$t("washmybelly.perf.empty_body") as string} />
    {:else}
      <ul class="sessions">
        {#each sessions as s (s.session_id)}
          <li>
            <button type="button" class:on={selected === s.session_id} onclick={() => (selected = selected === s.session_id ? null : s.session_id)}>
              <span class="grow">
                <strong>{s.label}</strong>
                <span class="dim">{when(s)} · {$t(`washmybelly.perf.method.${s.method}`)}{#if s.status !== "done"} · {$t(`washmybelly.perf.status.${s.status}`)}{/if}</span>
              </span>
              {#if s.fps_avg != null}
                <span class="num"><b>{formatFps(s.fps_avg)}</b><small>FPS</small></span>
                <span class="num"><b>{formatFps(s.low_1)}</b><small>1%</small></span>
              {/if}
              <Icon name="chevron" size={12} />
            </button>
            {#if selected === s.session_id && s.status !== "recording" && s.status !== "waiting_launch"}
              <div class="detail">
                <PerfSessionDetail sessionId={s.session_id} onDelete={() => { selected = null; void loadSessions(); }} />
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="card">
    <h3>{$t("washmybelly.perf.compare_title")}</h3>
    <PerfCompare {sessions} />
  </section>
</div>

<style>
  .perf { display: flex; flex-direction: column; gap: 16px; }
  .card { border-radius: var(--radius-lg, 12px); background: var(--surface); box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, var(--separator)); padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; }
  .note { flex-direction: row; align-items: flex-start; }
  .note.warn { color: var(--text); }
  .note p { margin: 0; font-size: 12.5px; }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; }
  .dim { margin: 0; font-size: 11.5px; color: var(--text-dim); }
  .error { margin: 0; font-size: 12px; color: var(--danger); }
  .pill { font-size: 11px; font-weight: 600; padding: 2px 8px; border-radius: 999px; background: var(--surface-mut, var(--separator)); color: var(--text-dim); }
  .pill.ok { background: color-mix(in srgb, var(--success) 16%, transparent); color: var(--success); }
  .live { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
  .big { display: flex; align-items: baseline; gap: 4px; min-width: 88px; }
  .fps { font-family: var(--font-display); font-size: 34px; font-weight: 700; font-variant-numeric: tabular-nums; letter-spacing: var(--track-tight); line-height: 1; }
  .unit { font-size: 12px; font-weight: 600; color: var(--text-dim); }
  .cmd { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .cmd code { flex: 1; min-width: 0; overflow-x: auto; white-space: nowrap; font-size: 11.5px; padding: 5px 8px; border-radius: 7px; background: var(--surface-mut, var(--separator)); user-select: all; }
  .kind { font-size: 11px; color: var(--text-dim); min-width: 56px; }
  .procs, .sessions { list-style: none; margin: 0; padding: 0; }
  .procs li button, .sessions li > button { all: unset; box-sizing: border-box; width: 100%; display: flex; align-items: center; gap: 10px; padding: 7px 8px; border-radius: 8px; cursor: pointer; }
  .procs li + li, .sessions li + li { border-top: var(--hairline, 1px) solid var(--content-border, var(--separator)); }
  .procs li button:hover, .sessions li > button:hover { background: var(--surface-hi, var(--surface-mut)); }
  .procs li button.on { background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .sessions li > button.on { background: var(--surface-hi, var(--surface-mut)); }
  .procs li button:focus-visible, .sessions li > button:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .pname { font-size: 12.5px; font-weight: 600; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sessions strong { font-size: 12.5px; }
  .num { display: flex; align-items: baseline; gap: 3px; min-width: 58px; justify-content: flex-end; font-variant-numeric: tabular-nums; }
  .num b { font-size: 13px; }
  .num small { font-size: 10.5px; color: var(--text-dim); }
  .detail { display: flex; flex-direction: column; gap: 12px; padding: 8px 0 14px; }
  .form { display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 10px; }
  label { display: flex; flex-direction: column; gap: 3px; font-size: 11px; color: var(--text-dim); }
  input, select { font: inherit; font-size: 12.5px; color: var(--text); padding: 5px 7px; border-radius: 7px; border: var(--hairline, 1px) solid var(--content-border, var(--separator)); background: var(--surface); }
  .row { display: flex; gap: 8px; align-items: center; }
  .row.end { justify-content: flex-end; }
</style>
