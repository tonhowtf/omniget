<script lang="ts">
  import { t } from "$lib/i18n";
  import ContextGuardian from "./ContextGuardian.svelte";
  import Sparkline from "./Sparkline.svelte";
  import {
    aggregateWindow,
    cacheHitRate,
    contextRatio,
    guardianBand,
    windowTotals,
    WINDOWS,
    type AgentTelemetry,
    type WindowKey,
  } from "$lib/stores/llm-telemetry-store.svelte";

  interface Props {
    agent: AgentTelemetry;
    windowKey: WindowKey;
    nowMs: number;
    onReset: (agentId: string) => void;
  }

  let { agent, windowKey, nowMs, onReset }: Props = $props();

  type Metric = "tps" | "tokens_out" | "cost_usd";
  const METRICS: Metric[] = ["tps", "tokens_out", "cost_usd"];
  let metric = $state<Metric>("tps");

  const BUCKETS = 96;

  let buckets = $derived(aggregateWindow(agent.history, nowMs, WINDOWS[windowKey], BUCKETS));
  let totals = $derived(windowTotals(buckets));
  let series = $derived(buckets.map((b) => b[metric]));
  let times = $derived(buckets.map((b) => b.t_ms));
  let ratio = $derived(contextRatio(agent.context_used_tokens, agent.context_window));
  let band = $derived(guardianBand(ratio));
  let cache = $derived(cacheHitRate(agent));

  function k(n: number): string {
    if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
    if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
    return String(Math.round(n));
  }

  function usd(v: number): string {
    return v >= 10 ? `$${v.toFixed(2)}` : `$${v.toFixed(3)}`;
  }

  function ms(v: number | null): string {
    if (v === null || !Number.isFinite(v)) return "—";
    return v >= 1000 ? `${(v / 1000).toFixed(2)} s` : `${Math.round(v)} ms`;
  }

  function fmtMetric(v: number): string {
    if (metric === "cost_usd") return usd(v);
    if (metric === "tokens_out") return k(v);
    return `${v.toFixed(1)} tok/s`;
  }

  function clock(tMs: number): string {
    return new Date(tMs).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }

  const EVENT_GLYPH: Record<string, string> = {
    turn_started: "▸",
    turn_finished: "✓",
    rerouted: "⇄",
    error: "!",
    budget: "$",
  };

  let timeline = $derived([...agent.events].sort((a, b) => b.t_ms - a.t_ms).slice(0, 6));
</script>

<article class="agent-card band-{band}">
  <header>
    <div class="who">
      <span class="dot state-{agent.state}" aria-hidden="true"></span>
      <div class="names">
        <span class="name">{agent.agent_name}</span>
        <span class="model">{agent.provider}/{agent.model}</span>
      </div>
    </div>
    <span class="state-label">{$t(`llm.observatory.state_${agent.state}`)}</span>
  </header>

  <div class="tiles">
    <div class="tile">
      <span class="tile-label">{$t("llm.observatory.throughput")}</span>
      <span class="tile-value">{agent.tps_last === null ? "—" : `${agent.tps_last.toFixed(1)}`}<em>tok/s</em></span>
    </div>
    <div class="tile">
      <span class="tile-label">{$t("llm.observatory.first_token")}</span>
      <span class="tile-value">{ms(agent.first_token_ms_p50)}</span>
      <span class="tile-sub">{$t("llm.observatory.last")} {ms(agent.first_token_ms_last)}</span>
    </div>
    <div class="tile">
      <span class="tile-label">{$t("llm.observatory.tokens")}</span>
      <span class="tile-value">{k(totals.tokens_in)}<em>{$t("llm.observatory.tokens_in_short")}</em></span>
      <span class="tile-sub">{k(totals.tokens_out)} {$t("llm.observatory.tokens_out_short")} · {agent.turns} {$t("llm.observatory.turns")}</span>
    </div>
    <div class="tile">
      <span class="tile-label">{$t("llm.observatory.cache_hit")}</span>
      <span class="tile-value">{Math.round(cache * 100)}<em>%</em></span>
      <span class="tile-sub">{k(agent.cache_read_tokens)} {$t("llm.observatory.cached")}</span>
    </div>
    <div class="tile">
      <span class="tile-label">{$t("llm.observatory.cost")}</span>
      <span class="tile-value">{usd(totals.cost_usd)}</span>
      <span class="tile-sub">{$t(`llm.observatory.window_${windowKey}`)}</span>
    </div>
  </div>

  <div class="plot">
    <div class="plot-head">
      <span class="group-label plain">{$t(`llm.observatory.metric_${metric}`)}</span>
      <div class="segmented">
        {#each METRICS as m (m)}
          <button class="segmented-btn" class:active={metric === m} type="button" onclick={() => (metric = m)}>
            {$t(`llm.observatory.metric_${m}_short`)}
          </button>
        {/each}
      </div>
    </div>
    <Sparkline
      values={series}
      {times}
      format={fmtMetric}
      label="{agent.agent_name} — {$t(`llm.observatory.metric_${metric}`)}"
      tone={band === "critical" ? "error" : band === "warn" ? "warn" : "accent"}
    />
  </div>

  <ContextGuardian used={agent.context_used_tokens} window={agent.context_window} estimated={agent.context_estimated} />

  {#if timeline.length}
    <ul class="timeline">
      {#each timeline as ev (ev.t_ms + ev.kind)}
        <li class="ev kind-{ev.kind}">
          <span class="ev-glyph" aria-hidden="true">{EVENT_GLYPH[ev.kind] ?? "·"}</span>
          <span class="ev-time">{clock(ev.t_ms)}</span>
          <span class="ev-text">
            {$t(`llm.observatory.event_${ev.kind}`)}{#if ev.detail}<span class="ev-detail">{ev.detail}</span>{/if}
          </span>
          {#if ev.code}<code class="ev-code">{ev.code}</code>{/if}
        </li>
      {/each}
    </ul>
  {/if}

  <footer>
    {#if agent.last_error_code}
      <code class="err">{agent.last_error_code}</code>
    {/if}
    <button class="btn btn-ghost btn-sm" type="button" onclick={() => onReset(agent.agent_id)}>
      {$t("llm.observatory.reset_agent")}
    </button>
  </footer>
</article>

<style>
  .agent-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .who {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .names {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .model {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--text-dim);
    flex: none;
  }
  .dot.state-streaming { background: var(--success); }
  .dot.state-waiting_tool { background: var(--warning); }
  .dot.state-error { background: var(--error); }
  .state-label {
    margin-left: auto;
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(96px, 1fr));
    gap: var(--space-3);
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .tile-label {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .tile-value {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
  }
  .tile-value em {
    font-style: normal;
    font-size: var(--text-xs);
    font-weight: 400;
    color: var(--text-dim);
    margin-left: 3px;
  }
  .tile-sub {
    font-size: var(--text-caption);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .plot-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    min-width: 0;
  }
  .group-label.plain {
    padding: 0;
    margin: 0;
  }
  .plot-head .segmented {
    margin-left: auto;
    max-width: 100%;
    overflow: hidden;
  }
  .segmented-btn.active {
    background: var(--surface-hi);
    color: var(--text);
  }
  .timeline {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ev {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-muted);
    min-width: 0;
  }
  .ev-glyph {
    color: var(--text-dim);
    width: 10px;
    flex: none;
    text-align: center;
  }
  .ev.kind-rerouted .ev-glyph { color: var(--warning); }
  .ev.kind-error .ev-glyph { color: var(--error); }
  .ev-time {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex: none;
  }
  .ev-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ev-detail {
    font-family: var(--font-mono);
    color: var(--text);
    margin-left: var(--space-1);
  }
  .ev-code,
  .err {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--error);
    flex: none;
  }
  footer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: auto;
  }
  footer .btn {
    margin-left: auto;
  }
</style>
