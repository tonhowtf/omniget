<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import { formatBytes } from "$lib/washmybelly/api";
  import { errorText } from "$lib/washmybelly/format";
  import type { ProcessList, ProcessRow } from "$lib/washmybelly/monitor";
  import {
    appliedKeys,
    candidates,
    explainKey,
    gameErrorKey,
    installLine,
    macGameMode,
    methodKey,
    minutesSince,
    statusPill,
    windowsAutoGameMode,
    type GameStatus,
  } from "$lib/washmybelly/game";
  import { moduleById, type Caps } from "./registry";
  import ModuleHeader from "./ui/ModuleHeader.svelte";
  import EmptyState from "./ui/EmptyState.svelte";
  import Icon from "./ui/Icon.svelte";
  import GameStatusPill from "./GameStatusPill.svelte";
  import GameRow from "./GameRow.svelte";

  let { caps }: { caps: Caps } = $props();

  const mod = moduleById("games");
  const REFRESH_MS = 5000;

  let status = $state<GameStatus | null>(null);
  let procs = $state<ProcessRow[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let notice = $state("");
  let busyPid = $state<number | null>(null);
  let showAll = $state(false);
  let copied = $state(false);
  let clock = $state(Date.now());
  let timer: ReturnType<typeof setInterval> | null = null;

  let activePids = $derived(status?.active.map((a) => a.pid) ?? []);
  let list = $derived(
    showAll
      ? procs.filter((p) => p.mine && !activePids.includes(p.pid)).slice(0, 40)
      : candidates(procs, activePids),
  );
  let pill = $derived(status ? statusPill(status) : null);
  let install = $derived(status?.install ? installLine(status.install) : "");

  async function loadStatus() {
    status = await api.call<GameStatus>("wmb_game_mode", { action: "status" });
  }

  async function loadProcs() {
    const r = await api.call<ProcessList>("wmb_processes", { sort: "cpu", limit: 80 });
    procs = r.processes ?? [];
  }

  async function load() {
    if (typeof document !== "undefined" && document.hidden) return;
    try {
      await Promise.all([loadStatus(), loadProcs()]);
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
      clock = Date.now();
    }
  }

  onMount(() => {
    void load();
    timer = setInterval(load, REFRESH_MS);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  function explainError(e: unknown): string {
    const k = gameErrorKey(e);
    return k ? ($t(k) as string) : errorText(e);
  }

  async function enable(p: ProcessRow) {
    busyPid = p.pid;
    notice = "";
    try {
      await api.call("wmb_game_mode", { action: "enable", pid: p.pid });
      notice = $t("washmybelly.games.enabled", { name: p.name }) as string;
      await loadStatus();
    } catch (e) {
      notice = explainError(e);
    } finally {
      busyPid = null;
    }
  }

  async function disable(pid: number, name: string) {
    busyPid = pid;
    notice = "";
    try {
      await api.call("wmb_game_mode", { action: "disable", pid });
      notice = $t("washmybelly.games.disabled", { name }) as string;
      await loadStatus();
    } catch (e) {
      notice = explainError(e);
    } finally {
      busyPid = null;
    }
  }

  async function copyInstall() {
    try {
      await navigator.clipboard.writeText(install);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      copied = false;
    }
  }

  function onOff(v: boolean | null, unknownKey: string): string {
    if (v === null) return $t(unknownKey) as string;
    return $t(v ? "washmybelly.games.on" : "washmybelly.games.off") as string;
  }

  function procSub(p: ProcessRow): string {
    return $t("washmybelly.games.usage", { cpu: Math.round(p.cpu_percent), memory: formatBytes(p.memory_bytes) }) as string;
  }
</script>

<ModuleHeader title={$t("washmybelly.modules.games") as string} subtitle={$t("washmybelly.games.subtitle") as string} tile={mod}>
  {#snippet meta()}
    {#if pill}<GameStatusPill label={$t(pill.key) as string} tone={pill.tone} />{/if}
  {/snippet}
  {#snippet actions()}
    <button class="btn btn-secondary" type="button" disabled={loading} onclick={() => void load()}>
      <Icon name="refresh" size={13} />{$t("washmybelly.games.refresh")}
    </button>
  {/snippet}
</ModuleHeader>

{#if loading && !status}
  <div class="gm-loading" aria-busy="true">
    {#each [0, 1, 2] as i (i)}<span class="skeleton" style:height="56px"></span>{/each}
  </div>
{:else if error && !status}
  <EmptyState art="worried" title={$t("washmybelly.error_title") as string} body={error}>
    {#snippet action()}
      <button class="btn btn-primary" type="button" onclick={() => void load()}>{$t("washmybelly.retry")}</button>
    {/snippet}
  </EmptyState>
{:else if status}
  <section class="gm-card">
    <p class="gm-explain">{$t(explainKey(status))}</p>
    <dl class="gm-facts">
      {#if status.os === "linux"}
        {#if status.system?.flatpak}
          <div><dt>{$t("washmybelly.games.fact.portal")}</dt><dd>{status.backend === "portal" ? $t("washmybelly.games.fact.present") : $t("washmybelly.games.fact.missing")}</dd></div>
        {:else}
          <div><dt>{$t("washmybelly.games.fact.gamemode")}</dt><dd>{status.system?.installed ? $t("washmybelly.games.fact.installed") : $t("washmybelly.games.fact.not_installed")}</dd></div>
          {#if status.system?.installed}
            <div><dt>{$t("washmybelly.games.fact.daemon")}</dt><dd>{status.system?.daemon_running ? $t("washmybelly.games.fact.running") : $t("washmybelly.games.fact.starts_on_demand")}</dd></div>
            {#if typeof status.system?.client_count === "number"}
              <div><dt>{$t("washmybelly.games.fact.clients")}</dt><dd>{status.system.client_count}</dd></div>
            {/if}
          {/if}
        {/if}
      {:else if status.os === "windows"}
        <div>
          <dt>{$t("washmybelly.games.fact.windows_game_mode")}</dt>
          <dd>{onOff(windowsAutoGameMode(status), "washmybelly.games.fact.default_on")}</dd>
        </div>
        <div>
          <dt>{$t("washmybelly.games.fact.power_plan")}</dt>
          <dd>{status.system?.performance_plan_active ? $t("washmybelly.games.fact.performance") : $t("washmybelly.games.fact.other_plan")}</dd>
        </div>
      {:else}
        <div>
          <dt>{$t("washmybelly.games.fact.mac_game_mode")}</dt>
          <dd>{onOff(macGameMode(status), "washmybelly.games.fact.unknown_needs_xcode")}</dd>
        </div>
        {#if status.system?.policy}
          <div><dt>{$t("washmybelly.games.fact.policy")}</dt><dd>{$t(`washmybelly.games.policy.${status.system.policy}`)}</dd></div>
        {/if}
      {/if}
    </dl>
    {#if status.os === "windows"}
      <p class="gm-hint"><Icon name="info" size={12} />{$t("washmybelly.games.windows_tweak_hint")}</p>
    {/if}
    {#if status.os === "linux" && status.reason === "gamemode_missing"}
      <div class="gm-install">
        <p class="gm-hint"><Icon name="info" size={12} />{$t("washmybelly.games.install_hint")}</p>
        {#if install}
          <div class="gm-cmd">
            <code>{install}</code>
            <button class="btn btn-secondary btn-sm" type="button" onclick={() => void copyInstall()}>
              <Icon name={copied ? "check" : "copy"} size={12} />{copied ? $t("washmybelly.games.copied") : $t("washmybelly.games.copy")}
            </button>
          </div>
        {/if}
      </div>
    {/if}
  </section>

  {#if notice}
    <p class="gm-notice" role="status">{notice}</p>
  {/if}

  {#if status.active.length}
    <span class="group-label">{$t("washmybelly.games.active_title")}</span>
    <div class="group" role="list">
      {#each status.active as a (a.pid)}
        <GameRow name={a.name} live sub={$t("washmybelly.games.active_sub", { method: $t(methodKey(a.method)), minutes: minutesSince(a.since, clock) }) as string}>
          {#snippet extra()}
            <span class="gm-chips">
              {#each appliedKeys(a.applied) as k (k)}<span class="gm-chip">{$t(k)}</span>{/each}
              {#if a.children_lowered}<span class="gm-chip">{$t("washmybelly.games.children_count", { count: a.children_lowered })}</span>{/if}
            </span>
          {/snippet}
          {#snippet trailing()}
            <button class="btn btn-secondary" type="button" disabled={busyPid === a.pid} onclick={() => void disable(a.pid, a.name)}>
              {$t("washmybelly.games.disable")}
            </button>
          {/snippet}
        </GameRow>
      {/each}
    </div>
    <p class="group-footer">{$t("washmybelly.games.auto_off")}</p>
  {/if}

  <div class="gm-listhead">
    <span class="group-label">{$t("washmybelly.games.running_title")}</span>
    <label class="gm-filter">
      <button type="button" class="toggle" class:on={showAll} role="switch" aria-checked={showAll} aria-label={$t("washmybelly.games.show_all") as string} onclick={() => (showAll = !showAll)}>
        <span class="toggle-knob"></span>
      </button>
      <span>{$t("washmybelly.games.show_all")}</span>
    </label>
  </div>
  {#if list.length === 0}
    <EmptyState art="sleeping" compact title={$t("washmybelly.games.none_title") as string} body={$t("washmybelly.games.none_body") as string} />
  {:else}
    <div class="group" role="list">
      {#each list as p (p.pid)}
        <GameRow name={p.name} sub={procSub(p)}>
          {#snippet trailing()}
            <button class="btn btn-primary" type="button" disabled={busyPid !== null} onclick={() => void enable(p)}>
              {busyPid === p.pid ? $t("washmybelly.games.enabling") : $t("washmybelly.games.enable")}
            </button>
          {/snippet}
        </GameRow>
      {/each}
    </div>
    <p class="group-footer">{$t("washmybelly.games.pick_hint")}</p>
  {/if}

  <button class="gm-perf" type="button" onclick={() => void goto("/washmybelly?m=perf", { noScroll: true, keepFocus: true, replaceState: true })}>
    <span class="gm-perf-text">
      <span class="gm-perf-title">{$t("washmybelly.games.perf_title")}</span>
      <span class="gm-perf-sub">{$t("washmybelly.games.perf_body")}</span>
    </span>
    <Icon name="chevron" size={14} />
  </button>
{/if}

<style>
  .gm-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .gm-loading .skeleton {
    display: block;
    border-radius: var(--radius-md);
  }
  .gm-card {
    margin-bottom: var(--space-5);
    padding: var(--space-4);
    border-radius: var(--radius-lg, 12px);
    background: var(--surface);
    border: 1px solid var(--border);
  }
  .gm-explain {
    margin: 0 0 var(--space-3);
    font-size: var(--text-base, 13px);
    color: var(--text);
    line-height: 1.45;
  }
  .gm-facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-5);
    margin: 0;
  }
  .gm-facts div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .gm-facts dt {
    font-size: var(--text-xs, 11px);
    color: var(--text-dim);
  }
  .gm-facts dd {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }
  .gm-hint {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: var(--space-3) 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .gm-cmd {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    flex-wrap: wrap;
  }
  .gm-cmd code {
    flex: 1 1 200px;
    min-width: 0;
    padding: 6px 10px;
    border-radius: var(--radius-md);
    background: var(--surface-2, var(--bg));
    font-size: var(--text-sm);
    overflow-x: auto;
    white-space: nowrap;
  }
  .gm-notice {
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .gm-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .gm-chip {
    padding: 1px 8px;
    border-radius: 999px;
    font-size: var(--text-xs, 11px);
    color: var(--text-muted);
    background: color-mix(in srgb, var(--success) 12%, transparent);
  }
  .gm-listhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }
  .gm-filter {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
    cursor: pointer;
  }
  .group {
    margin-bottom: var(--space-2);
  }
  .gm-perf {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    width: 100%;
    margin-top: var(--space-5);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg, 12px);
    border: 1px dashed var(--border);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .gm-perf:hover {
    background: var(--surface);
  }
  .gm-perf-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .gm-perf-title {
    font-weight: 600;
  }
  .gm-perf-sub {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
