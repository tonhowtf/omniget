<script lang="ts">
  /**
   * Análise de uma sessão: duração, tempo do agente × seu tempo de pensar,
   * eficiência de cache, custo (gravado × estimado), mix de modelos, tools,
   * componentes (subagentes, comandos, skills, MCP) e dicas.
   */
  import { t } from "$lib/i18n";
  import Modal from "./Modal.svelte";
  import BarList from "./BarList.svelte";
  import { compact, duration, errText, sessionsAnalysis, usd, type SessionAnalysis } from "$lib/central/sessions";

  interface Props {
    tool: string;
    id: string;
    onclose: () => void;
  }

  let { tool, id, onclose }: Props = $props();

  let a = $state<SessionAnalysis | null>(null);
  let error = $state("");

  $effect(() => {
    sessionsAnalysis(tool, id)
      .then((r) => (a = r))
      .catch((e) => (error = errText(e)));
  });

  function tip(code: string, fallback: string): string {
    const key = `llm.central.sessions.tips.${code}`;
    const s = $t(key);
    return s && s !== key ? s : fallback;
  }

  let timeTotal = $derived(a ? a.time.agent_ms + a.time.think_ms + a.time.away_ms : 0);
  const pctOf = (v: number, total: number) => (total > 0 ? (v / total) * 100 : 0);
</script>

<Modal title={$t("llm.central.sessions.analysis.title")} width={720} {onclose}>
  {#if error}
    <p class="err">{error}</p>
  {:else if !a}
    <p>{$t("llm.central.sessions.loading")}</p>
  {:else}
    <div class="an">
      <section class="tiles">
        <div class="tile"><span>{$t("llm.central.sessions.analysis.duration")}</span><strong>{duration(a.duration_ms)}</strong></div>
        <div class="tile"><span>{$t("llm.central.sessions.analysis.messages")}</span><strong>{a.counts.user} / {a.counts.assistant}</strong><em>{$t("llm.central.sessions.analysis.user_assistant")}</em></div>
        <div class="tile"><span>{$t("llm.central.sessions.analysis.tool_calls")}</span><strong>{a.counts.tool_calls}</strong>{#if a.counts.tool_errors}<em>{a.counts.tool_errors} {$t("llm.central.sessions.analysis.errors")}</em>{/if}</div>
        <div class="tile">
          <span>{$t("llm.central.sessions.cost_label")}</span>
          <strong>{usd(a.cost.cost_usd)}</strong>
          <em>{$t(`llm.central.sessions.cost.${a.cost.source}`)}</em>
        </div>
        <div class="tile"><span>{$t("llm.central.sessions.analysis.max_context")}</span><strong>{compact(a.max_context)}</strong></div>
      </section>

      <section>
        <h3>{$t("llm.central.sessions.analysis.time")}</h3>
        <div class="split" role="img" aria-label={`${$t("llm.central.sessions.analysis.agent_time")} ${duration(a.time.agent_ms)}, ${$t("llm.central.sessions.analysis.think_time")} ${duration(a.time.think_ms)}, ${$t("llm.central.sessions.analysis.away_time")} ${duration(a.time.away_ms)}`}>
          <i class="s-agent" style:width="{pctOf(a.time.agent_ms, timeTotal)}%"></i>
          <i class="s-think" style:width="{pctOf(a.time.think_ms, timeTotal)}%"></i>
          <i class="s-away" style:width="{pctOf(a.time.away_ms, timeTotal)}%"></i>
        </div>
        <ul class="legend">
          <li><span class="sw s-agent"></span>{$t("llm.central.sessions.analysis.agent_time")} <strong>{duration(a.time.agent_ms)}</strong> ({Math.round(a.time.agent_pct)}%)</li>
          <li><span class="sw s-think"></span>{$t("llm.central.sessions.analysis.think_time")} <strong>{duration(a.time.think_ms)}</strong> ({Math.round(a.time.think_pct)}%)</li>
          <li><span class="sw s-away"></span>{$t("llm.central.sessions.analysis.away_time")} <strong>{duration(a.time.away_ms)}</strong></li>
          <li>{a.time.exchanges} {$t("llm.central.sessions.analysis.exchanges")}</li>
        </ul>
      </section>

      <section class="two">
        <div>
          <h3>{$t("llm.central.sessions.analysis.cache")}</h3>
          <p class="big">{Math.round(a.cache.efficiency)}%</p>
          <p class="small">{$t("llm.central.sessions.analysis.cache_efficiency_hint")}</p>
          <p class="small">{compact(a.cache.read)} {$t("llm.central.sessions.tokens.cache_read").toLowerCase()} · {compact(a.cache.write)} {$t("llm.central.sessions.tokens.cache_write").toLowerCase()} · {Math.round(a.cache.hit_ratio)}% {$t("llm.central.sessions.analysis.hit_ratio")}</p>
        </div>
        <div>
          <h3>{$t("llm.central.sessions.analysis.cost_split")}</h3>
          <p class="small">{$t("llm.central.sessions.cost.recorded")}: <strong>{usd(a.cost.recorded_usd)}</strong></p>
          <p class="small">{$t("llm.central.sessions.cost.computed")}: <strong>{usd(a.cost.computed_usd)}</strong></p>
          {#if a.cost.unpriced_tokens > 0}
            <p class="small">{compact(a.cost.unpriced_tokens)} {$t("llm.central.sessions.analysis.unpriced")} <code>{a.cost.unpriced_models.join(", ")}</code></p>
          {/if}
          <p class="small">
            in {compact(a.usage.input)} · out {compact(a.usage.output)} · r {compact(a.usage.reasoning)}
          </p>
        </div>
      </section>

      {#if a.models.length}
        <section>
          <h3>{$t("llm.central.sessions.analysis.models")}</h3>
          <BarList items={a.models.map((m) => ({ key: m.model, label: m.model, value: m.pct, sub: `${m.responses} · ${compact(m.tokens)} tok${m.cost_usd != null ? ` · ${usd(m.cost_usd)}` : ""}` }))} format={(v) => `${Math.round(v)}%`} />
        </section>
      {/if}

      {#if a.tools.length}
        <section>
          <h3>{$t("llm.central.sessions.analysis.tools")}</h3>
          <div class="tscroll">
            <table>
              <thead><tr><th scope="col">{$t("llm.central.sessions.analysis.tool")}</th><th scope="col">{$t("llm.central.sessions.analysis.calls")}</th><th scope="col">{$t("llm.central.sessions.analysis.errors")}</th><th scope="col">{$t("llm.central.sessions.analysis.avg")}</th><th scope="col">{$t("llm.central.sessions.analysis.total_time")}</th></tr></thead>
              <tbody>
                {#each a.tools as tl (tl.name)}
                  <tr>
                    <td title={tl.raw.join(", ")}>{tl.name}</td>
                    <td>{tl.count}</td>
                    <td class:bad={tl.errors > 0}>{tl.errors}</td>
                    <td>{tl.avg_ms != null ? duration(tl.avg_ms) : "—"}</td>
                    <td>{tl.total_ms ? duration(tl.total_ms) : "—"}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </section>
      {/if}

      {#if a.components.agents.length || a.components.slash_commands.length || a.components.skills.length || a.components.mcp_servers.length}
        <section>
          <h3>{$t("llm.central.sessions.analysis.components")}</h3>
          <dl class="comps">
            {#each [["agents", a.components.agents], ["slash_commands", a.components.slash_commands], ["skills", a.components.skills], ["mcp_servers", a.components.mcp_servers]] as [k, list] (k)}
              {#if (list as [string, number][]).length}
                <div>
                  <dt>{$t(`llm.central.sessions.analysis.comp_${k}`)}</dt>
                  <dd>{#each list as [n, c] (n)}<span class="chip">{n} <strong>{c}</strong></span>{/each}</dd>
                </div>
              {/if}
            {/each}
          </dl>
        </section>
      {/if}

      {#if a.tips.length}
        <section>
          <h3>{$t("llm.central.sessions.analysis.tips")}</h3>
          <ul class="tips">
            {#each a.tips as tp (tp.code)}
              <li class={tp.severity}><span class="sev">{tp.severity === "warn" ? "!" : "i"}</span>{tip(tp.code, tp.message)}</li>
            {/each}
          </ul>
        </section>
      {/if}
    </div>
  {/if}
</Modal>

<style>
  .an {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    color: var(--text-muted);
  }
  h3 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: 8px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-xs);
  }
  .tile strong {
    font-size: var(--text-lg);
    color: var(--text);
    font-weight: 600;
  }
  .tile em {
    font-style: normal;
    color: var(--text-dim);
  }
  .split {
    display: flex;
    gap: 2px;
    height: 12px;
    border-radius: var(--radius-full);
    overflow: hidden;
    background: var(--viz-empty);
  }
  .split i {
    display: block;
    height: 100%;
  }
  .s-agent {
    background: var(--viz-seq);
  }
  .s-think {
    background: color-mix(in srgb, var(--viz-seq) 45%, var(--viz-empty));
  }
  .s-away {
    background: var(--fill-3);
  }
  .legend {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    font-size: var(--text-xs);
  }
  .legend li {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .legend strong {
    color: var(--text);
  }
  .sw {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    display: inline-block;
  }
  .two {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-4);
  }
  .big {
    margin: 0;
    font-size: var(--text-2xl);
    font-weight: 700;
    color: var(--text);
  }
  .small {
    margin: 2px 0;
    font-size: var(--text-xs);
  }
  .small strong {
    color: var(--text);
  }
  .tscroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }
  th,
  td {
    text-align: right;
    padding: 3px 6px;
    border-bottom: 1px solid var(--separator);
    white-space: nowrap;
    font-weight: 400;
  }
  th:first-child,
  td:first-child {
    text-align: left;
    color: var(--text);
  }
  thead th {
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  td.bad {
    color: var(--danger);
  }
  .comps {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .comps div {
    display: grid;
    grid-template-columns: 120px 1fr;
    gap: 8px;
  }
  .comps dt {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .comps dd {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    font-size: var(--text-xs);
    background: var(--fill-1);
    border-radius: var(--radius-full);
    padding: 1px 8px;
    color: var(--text-muted);
  }
  .chip strong {
    color: var(--text);
  }
  .tips {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .tips li {
    display: flex;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--text);
    padding: 8px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .sev {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 10px;
    font-weight: 700;
    background: var(--fill-3);
    color: var(--text);
  }
  .tips li.warn .sev {
    background: var(--warning);
    color: var(--on-warning);
  }
  .err {
    color: var(--danger);
  }
  code {
    font-size: var(--text-caption);
  }
</style>
