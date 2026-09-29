<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  /**
   * Observatory — the instrument panel of the LLM section.
   *
   * Top to bottom: quota as energy (one `QuotaMeter` per account, in router
   * order, with the active candidate marked), cost per day, one card per agent
   * (throughput, first token, tokens, cache, cost, Context Guardian, timeline),
   * the wire probe, and the diagnostics list.
   *
   * Cadence: the page creates no timer. It pulls one snapshot on mount and
   * then rides `llm://telemetry`, which the backend pushes at 2 Hz only while
   * a turn is alive. Leaving the route unsubscribes; a closed section costs
   * zero.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import QuotaMeter from "$components/llm/QuotaMeter.svelte";
  import AgentTelemetryCard from "$components/llm/observatory/AgentTelemetryCard.svelte";
  import CostByDay from "$components/llm/observatory/CostByDay.svelte";
  import DiagnosticsPanel from "$components/llm/observatory/DiagnosticsPanel.svelte";
  import WireProbePanel from "$components/llm/observatory/WireProbePanel.svelte";
  import AuditPanel from "$components/llm/observatory/AuditPanel.svelte";
  import {
    getTelemetry,
    getTelemetryError,
    getTelemetryState,
    isDemo,
    loadCostFallback,
    loadTelemetry,
    resetAgentHistory,
    startTelemetry,
    WINDOWS,
    type QuotaStatus,
    type WindowKey,
  } from "$lib/stores/llm-telemetry-store.svelte";

  const WINDOW_KEYS = Object.keys(WINDOWS) as WindowKey[];

  let windowKey = $state<WindowKey>("24h");
  let busy = $state(false);
  let diagnosticsOpen = $state(false);
  let auditOpen = $state(false);

  let snapshot = $derived(getTelemetry());
  let dataState = $derived(getTelemetryState());
  let errorCode = $derived(getTelemetryError());
  let demo = $derived(isDemo());
  /** Clock of the data, not of the wall: reading it costs no timer. */
  let nowMs = $derived(snapshot.ts_ms || Date.now());

  /** Accounts in router order, so the fallback chain reads top to bottom. */
  let chain = $derived.by<QuotaStatus[]>(() => {
    const byId = new Map(snapshot.quotas.map((q) => [q.id, q]));
    const ordered = snapshot.fallback_chain.map((id) => byId.get(id)).filter((q): q is QuotaStatus => !!q);
    const rest = snapshot.quotas.filter((q) => !snapshot.fallback_chain.includes(q.id));
    return [...ordered, ...rest];
  });

  function projection(q: QuotaStatus): string | undefined {
    if (!q.exhausts_at_ms) return undefined;
    const mins = Math.max(0, Math.round((q.exhausts_at_ms - nowMs) / 60_000));
    const h = Math.floor(mins / 60);
    const left = h > 0 ? `${h} h ${mins % 60} min` : `${mins} min`;
    return `${$t("llm.observatory.quota_projection")} ${left}`;
  }

  async function refresh() {
    busy = true;
    try {
      await loadTelemetry();
      await loadCostFallback();
    } finally {
      busy = false;
    }
  }

  async function cancelTurn(requestId: string) {
    try {
      await invoke("llm_turn_cancel", { requestId });
      await loadTelemetry();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  onMount(() => {
    let stop: (() => void) | null = null;
    let disposed = false;
    const demoFlag = $page.url.searchParams.get("demo") === "1";
    startTelemetry({ demo: demoFlag }).then((fn) => {
      if (disposed) { fn(); return; }
      stop = fn;
      if (!demoFlag) void loadCostFallback();
    });
    return () => { disposed = true; stop?.(); };
  });
</script>

<div class="observatory">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.observatory.title")}</h1>
      <p class="page-lede">{$t("llm.observatory.lede")}</p>
    </div>
    <div class="segmented">
      {#each WINDOW_KEYS as key (key)}
        <button class="segmented-btn" class:active={windowKey === key} type="button" onclick={() => (windowKey = key)}>
          {$t(`llm.observatory.window_${key}`)}
        </button>
      {/each}
    </div>
  </header>
  <SurfaceGuide text={$t("llm.surface.observatory_hint")} href="/help?article=activity#guide" />

  {#if dataState === "unavailable" && snapshot.agents.length === 0 && !demo}
    <div class="empty-state">
      <div class="empty-state-title">{$t("llm.observatory.empty_title")}</div>
      <div class="empty-state-hint">{$t("llm.observatory.empty_hint")} <code>{errorCode}</code></div>
      <button class="btn btn-secondary" type="button" disabled={busy} onclick={refresh}>
        {$t("llm.observatory.refresh")}
      </button>
    </div>
  {:else}
    <div class="activity-summary">
      <a href="/llm/work"><strong>{snapshot.agents.filter(a => a.state === "streaming").length}</strong>{$t("llm.observatory.running")}</a>
      <a href="/llm/work"><strong>{snapshot.agents.filter(a => a.state === "waiting_tool").length}</strong>{$t("llm.observatory.waiting")}</a>
      <a href="/llm/work"><strong>{snapshot.agents.filter(a => a.state === "error").length}</strong>{$t("llm.observatory.failures")}</a>
    </div>
    {#if chain.length}
      <section>
        <span class="group-label">{$t("llm.observatory.quota")}</span>
        <div class="card quotas">
          {#each chain as q, i (q.id)}
            <div class="quota-row" class:active={q.active}>
              <span class="rank" aria-hidden="true">{i + 1}</span>
              <QuotaMeter
                label={q.label}
                used={q.used}
                resetsAt={q.resets_at_ms ?? undefined}
                projection={projection(q)}
                active={q.active}
              />
              {#if q.source === "estimated"}
                <span class="src-tag">{$t("llm.observatory.source_estimated")}</span>
              {/if}
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {#if snapshot.cost_by_day.length}
      <section>
        <span class="group-label">{$t("llm.observatory.cost_by_day")}</span>
        <div class="card chart-card">
          <CostByDay days={snapshot.cost_by_day} />
        </div>
      </section>
    {/if}

    <section>
      <span class="group-label">{$t("llm.observatory.agents")}</span>
      {#if snapshot.agents.length}
        <div class="agents">
          {#each snapshot.agents as agent (agent.agent_id)}
            <AgentTelemetryCard {agent} {windowKey} {nowMs} onReset={resetAgentHistory} />
          {/each}
        </div>
      {:else}
        <div class="card"><div class="setting-row">{$t("llm.observatory.no_agents")}</div></div>
      {/if}
    </section>

    <details class="diagnostic-disclosure" ontoggle={(event) => auditOpen = event.currentTarget.open}>
      <summary>{$t("llm.audit.title")}</summary>
      {#if auditOpen}<AuditPanel />{/if}
    </details>

    <details class="diagnostic-disclosure" ontoggle={(event) => diagnosticsOpen = event.currentTarget.open}>
      <summary>{$t("llm.surface.advanced")}</summary>
      {#if diagnosticsOpen}<WireProbePanel />{/if}
    </details>

    <section>
      <span class="group-label">{$t("llm.observatory.diagnostics")}</span>
      <DiagnosticsPanel
        agents={snapshot.agents}
        state={dataState}
        {errorCode}
        tsMs={snapshot.ts_ms}
        {demo}
        {busy}
        onRefresh={refresh}
        onCancel={cancelTurn}
        onReset={resetAgentHistory}
      />
    </section>
  {/if}
</div>

<style>
  .diagnostic-disclosure summary { cursor:pointer; padding:16px; font-weight:600; color:var(--text); border-bottom:1px solid var(--separator); }
  .activity-summary { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: var(--space-3); }
  .activity-summary a { display: flex; flex-direction: column; padding: var(--space-4); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-md); color: var(--text-muted); text-decoration: none; }
  .activity-summary strong { font-size: var(--text-xl); color: var(--text); }
  .observatory {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    padding: var(--space-5);
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: auto;
    width: 100%;
    box-sizing: border-box;
  }
  .observatory > :global(*) { flex-shrink: 0; min-width: 0; }
  .page-head {
    flex-wrap: wrap;
    display: flex;
    align-items: flex-end;
    gap: var(--space-4);
  }
  .page-head .segmented {
    margin-left: auto;
  }
  .segmented-btn.active {
    background: var(--surface-hi);
    color: var(--text);
  }
  section {
    display: flex;
    flex-direction: column;
  }
  .quotas {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
  }
  .quota-row {
    display: grid;
    grid-template-columns: 18px 1fr auto;
    align-items: center;
    gap: var(--space-3);
  }
  .rank {
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
  .quota-row.active .rank {
    color: var(--accent);
  }
  .src-tag {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .chart-card {
    padding: var(--space-4);
  }
  .agents {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 340px), 1fr));
    gap: var(--space-4);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
  }
</style>
