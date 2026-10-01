<script lang="ts">
  import { t } from "$lib/i18n";
  import CostByDay from "$components/llm/observatory/CostByDay.svelte";
  import ToolHeatmap from "./ToolHeatmap.svelte";
  import type { CliUsageReport } from "$lib/stores/llm-accounts-store.svelte";

  interface Props {
    report: CliUsageReport;
  }

  let { report }: Props = $props();

  let topToolMax = $derived(Math.max(1, ...report.top_tools.map((x) => x.calls)));
  let modelMax = $derived(Math.max(0.01, ...report.by_model.map((m) => m.cost_usd)));
  let days = $derived(
    report.by_day.map((d) => ({ day: d.key, cost_usd: d.cost_usd, calls: d.calls })),
  );

  function usd(v: number): string {
    return v >= 10 ? `$${v.toFixed(0)}` : `$${v.toFixed(2)}`;
  }

  function compact(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
    return String(n);
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }
</script>

<div class="dashboard">
  {#if report.by_day.length > 0}
    <section class="card surface-card">
      <h3 class="card-title">{$t("llm.accounts.cost_by_day")}</h3>
      <CostByDay {days} />
    </section>
  {/if}

  {#if report.by_model.length > 0}
    <section class="card surface-card">
      <h3 class="card-title">{$t("llm.accounts.by_model")}</h3>
      <table class="table">
        <thead>
          <tr>
            <th scope="col">{$t("llm.accounts.model")}</th>
            <th scope="col" class="num">{$t("llm.accounts.calls")}</th>
            <th scope="col" class="num">{$t("llm.accounts.tokens_in")}</th>
            <th scope="col" class="num">{$t("llm.accounts.tokens_out")}</th>
            <th scope="col" class="num">{$t("llm.accounts.cost")}</th>
          </tr>
        </thead>
        <tbody>
          {#each report.by_model as m (m.key)}
            <tr>
              <th scope="row" class="model">
                <span class="bar" style:width="{(m.cost_usd / modelMax) * 100}%"></span>
                <span class="model-name">{m.key}</span>
              </th>
              <td class="num">{m.calls}</td>
              <td class="num">{compact(m.input_tokens)}</td>
              <td class="num">{compact(m.output_tokens)}</td>
              <td class="num">{usd(m.cost_usd)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}

  {#if report.by_tool_hour.length > 0}
    <section class="card surface-card wide">
      <h3 class="card-title">{$t("llm.accounts.tool_heatmap")}</h3>
      <ToolHeatmap grid={report.by_tool_hour} />
    </section>
  {/if}

  {#if report.top_tools.length > 0}
    <section class="card surface-card">
      <h3 class="card-title">{$t("llm.accounts.top_tools")}</h3>
      <ul class="tools">
        {#each report.top_tools as tool (tool.tool)}
          <li>
            <span class="tool-name">{tool.tool}</span>
            <span class="tool-track"><i style:width="{(tool.calls / topToolMax) * 100}%"></i></span>
            <span class="num">{tool.calls}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if report.sessions.length > 0}
    <section class="card surface-card">
      <h3 class="card-title">{$t("llm.accounts.sessions_title")}</h3>
      <ul class="prompts">
        {#each report.sessions as s (s.id)}
          <li>
            <span class="prompt-text" title={s.project}>{s.project || s.account_id}</span>
            <span class="prompt-meta">
              {when(s.started_ms)} · {s.account_id}{s.model ? ` · ${s.model}` : ""} ·
              {s.turns} {$t("llm.accounts.turns")} · <strong>{usd(s.cost_usd)}</strong>
              {#if !s.ended_ms}· {$t("llm.accounts.session_live")}{/if}
            </span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <p class="scan-note">
    {$t("llm.accounts.scan_note")}
    {report.scanned_files} · {report.scan_ms} ms · {$t("llm.accounts.total_cost")}
    {usd(report.total_cost_usd)}
  </p>
</div>

<style>
  .dashboard {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: var(--space-3);
    align-items: start;
  }
  .card {
    padding: var(--space-4);
    min-width: 0;
    overflow: hidden;
  }
  .card.wide {
    grid-column: 1 / -1;
  }
  .card-title {
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-muted);
  }
  .table {
    width: 100%;
    table-layout: fixed;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }
  .table th:first-child,
  .table td:first-child {
    width: 40%;
  }
  .table th,
  .table td {
    padding: 4px 6px;
    text-align: left;
    font-weight: 400;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .table thead th {
    font-size: var(--text-caption);
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  .model {
    position: relative;
    min-width: 0;
  }
  .model .bar {
    position: absolute;
    left: 0;
    top: 3px;
    bottom: 3px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    border-radius: var(--radius-xs);
  }
  .model-name {
    position: relative;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: block;
  }
  .tools {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .tools li {
    display: grid;
    grid-template-columns: 5.5rem 1fr 3rem;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .tool-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .tool-track {
    height: 8px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-hi));
    overflow: hidden;
  }
  .tool-track i {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: var(--radius-full);
  }
  .prompts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .prompts li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .prompt-text {
    font-size: var(--text-sm);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .prompt-meta {
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .scan-note {
    grid-column: 1 / -1;
    margin: 0;
    font-size: var(--text-caption);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
