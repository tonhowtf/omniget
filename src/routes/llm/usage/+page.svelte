<script lang="ts">
  /**
   * Uso — todas as ferramentas de código numa linha do tempo só.
   *
   * Tudo sai do índice de sessões do núcleo (`sessions_*`): tokens por dia em
   * cinco tipos empilhados, custo por dia (registrado pela ferramenta ×
   * estimado pela tabela de preço), por modelo, por ferramenta, heatmap de um
   * ano com sequências, hora de pico, projetos, agentes/subagentes com os
   * padrões A → B → C e as fontes encontradas nesta máquina.
   *
   * Nada roda em repouso: a tela busca ao abrir, quando um filtro muda e no
   * botão Atualizar. Sem timer.
   */
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import StackedColumns from "$components/central/sessions/StackedColumns.svelte";
  import YearHeatmap from "$components/central/sessions/YearHeatmap.svelte";
  import BarList from "$components/central/sessions/BarList.svelte";
  import FilterBar from "$components/central/sessions/FilterBar.svelte";
  import {
    compact,
    daySpan,
    errText,
    isoDay,
    prefGet,
    prefSet,
    projectName,
    rangeFrom,
    relTime,
    sessionsAgents,
    sessionsHeatmap,
    sessionsRefresh,
    sessionsSources,
    sessionsUsage,
    toMs,
    toolName,
    TOKEN_KINDS,
    usd,
    type AgentsReport,
    type GroupBy,
    type Heatmap,
    type ListFilter,
    type RangeKey,
    type SourceInfo,
    type UsageReport,
  } from "$lib/central/sessions";

  let rangeKey = $state<RangeKey>("30d");
  let tool = $state("");
  let account = $state("");
  let project = $state("");
  let model = $state("");

  let byTime = $state<UsageReport | null>(null);
  let byModel = $state<UsageReport | null>(null);
  let byTool = $state<UsageReport | null>(null);
  let byProject = $state<UsageReport | null>(null);
  let byAccount = $state<UsageReport | null>(null);
  let heat = $state<Heatmap | null>(null);
  let agents = $state<AgentsReport | null>(null);
  let sources = $state<SourceInfo[]>([]);
  let loading = $state(false);
  let refreshing = $state(false);
  let error = $state("");
  let seq = 0;

  let from = $derived(rangeFrom(rangeKey));
  let to = $derived(isoDay(new Date()));
  let grain = $derived<GroupBy>(rangeKey === "all" ? "month" : rangeKey === "year" ? "week" : "day");

  let range = $derived<ListFilter>({ tool, account, project, model, from, to });
  /** O heatmap é sempre de um ano; herda só os filtros de dimensão. */
  let dimRange = $derived<ListFilter>({ tool, account, project, model });

  async function load() {
    const my = ++seq;
    loading = true;
    error = "";
    try {
      const r = range;
      const [a, b, c, d, e, h, ag] = await Promise.all([
        sessionsUsage(r, grain),
        sessionsUsage(r, "model"),
        sessionsUsage({ ...r, tool: "" }, "tool"),
        sessionsUsage(r, "project"),
        sessionsUsage({ ...r, account: "" }, "account"),
        sessionsHeatmap(dimRange, tool || null),
        sessionsAgents(r),
      ]);
      if (my !== seq) return;
      byTime = a;
      byModel = b;
      byTool = c;
      byProject = d;
      byAccount = e;
      heat = h;
      agents = ag;
    } catch (e) {
      if (my === seq) error = errText(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function loadSources() {
    try {
      sources = await sessionsSources();
    } catch (e) {
      error = errText(e);
    }
  }

  async function refresh() {
    refreshing = true;
    try {
      const st = await sessionsRefresh(false);
      showToast("success", `${$t("llm.central.sessions.refreshed")} · ${st.sessions_seen} · ${st.elapsed_ms} ms`);
      await Promise.all([load(), loadSources()]);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      refreshing = false;
    }
  }

  onMount(() => {
    void loadSources();
  });

  $effect(() => {
    // Depende de todos os filtros; cada mudança refaz as consultas.
    void range;
    void grain;
    void load();
  });

  // ---- séries e derivações ----

  const SERIES_COLORS: Record<string, string> = {
    input: "var(--viz-input)",
    output: "var(--viz-output)",
    cache_read: "var(--viz-cache_read)",
    cache_write: "var(--viz-cache_write)",
    reasoning: "var(--viz-reasoning)",
  };

  /** Leitura de cache costuma ser 90%+ do volume e achata o resto: dá para tirar. */
  let showCacheRead = $state(prefGet("usage.cacheRead", "1") === "1");
  $effect(() => prefSet("usage.cacheRead", showCacheRead ? "1" : "0"));

  let tokenSeries = $derived(
    TOKEN_KINDS.filter((k) => showCacheRead || k !== "cache_read").map((k) => ({ id: k, label: $t(`llm.central.sessions.tokens.${k}`), color: SERIES_COLORS[k] })),
  );

  function timeLabel(key: string): string {
    if (grain === "day") return key.slice(5).replace("-", "/");
    if (grain === "month") {
      const [y, m] = key.split("-").map(Number);
      return new Date(y, m - 1, 1).toLocaleDateString(undefined, { month: "short", year: "2-digit" });
    }
    return key.replace(/^\d{4}-/, "");
  }

  let timeRows = $derived.by(() => {
    if (!byTime) return [];
    const map = new Map(byTime.rows.map((r) => [r.key, r]));
    let keys = byTime.rows.map((r) => r.key).sort();
    if (grain === "day" && from) keys = daySpan(from, to);
    return keys.map((k) => {
      const r = map.get(k);
      const u = r?.usage;
      return {
        key: k,
        label: timeLabel(k),
        values: {
          input: u?.input ?? 0,
          output: u?.output ?? 0,
          cache_read: u?.cache_read ?? 0,
          cache_write: u?.cache_write ?? 0,
          reasoning: u?.reasoning ?? 0,
          recorded: r?.recorded_usd ?? 0,
          estimated: Math.max(0, (r?.cost_usd ?? 0) - (r?.recorded_usd ?? 0)),
        },
      };
    });
  });

  let costSeries = $derived([
    { id: "recorded", label: $t("llm.central.sessions.cost.recorded"), color: "var(--viz-seq)" },
    { id: "estimated", label: $t("llm.central.sessions.cost.computed"), color: "color-mix(in srgb, var(--viz-seq) 42%, var(--viz-empty))" },
  ]);

  let totals = $derived(byTime?.totals ?? null);
  let estimated = $derived(totals ? Math.max(0, totals.cost_usd - totals.recorded_usd) : 0);
  let subagentSessions = $derived((agents?.subagent_sessions ?? []).reduce((s, [, n]) => s + n, 0));

  let unpriced = $derived(byTime?.unpriced_models ?? []);

  let hourRows = $derived(
    (heat?.stats.hours ?? []).map((v, h) => ({ key: String(h), label: String(h).padStart(2, "0"), values: { v } })),
  );
  let weekdayNames = $derived(
    [0, 1, 2, 3, 4, 5, 6].map((i) => new Date(2024, 0, 1 + i).toLocaleDateString(undefined, { weekday: "short" })),
  );

  let toolOptions = $derived.by(() => {
    const seen = new Set<string>();
    const out: { value: string; label: string }[] = [];
    for (const r of byTool?.rows ?? []) {
      if (!seen.has(r.key)) out.push({ value: r.key, label: toolName(r.key) });
      seen.add(r.key);
    }
    for (const s of sources) if (s.sessions > 0 && !seen.has(s.tool)) out.push({ value: s.tool, label: toolName(s.tool) }), seen.add(s.tool);
    return out;
  });
  let accountOptions = $derived(
    (byAccount?.rows ?? []).filter((r) => r.key).map((r) => ({ value: r.key, label: r.key })),
  );
  let modelOptions = $derived(
    (byModel?.rows ?? []).filter((r) => r.key && r.key !== "?").sort((a, b) => b.total_tokens - a.total_tokens).map((r) => ({ value: r.key, label: r.key })),
  );
  let projectOptions = $derived(
    (byProject?.rows ?? []).filter((r) => r.key).sort((a, b) => b.total_tokens - a.total_tokens).slice(0, 60).map((r) => ({ value: r.key, label: projectName(r.key) || r.key })),
  );

  let modelRows = $derived([...(byModel?.rows ?? [])].sort((a, b) => b.cost_usd - a.cost_usd || b.total_tokens - a.total_tokens));
  let modelMax = $derived(Math.max(1e-9, ...modelRows.map((r) => r.total_tokens)));

  function openDay(day: string) {
    if (grain !== "day") return;
    void goto(`/llm/sessions?from=${day}&to=${day}${tool ? `&tool=${tool}` : ""}`);
  }
</script>

<div class="page" aria-busy={loading}>
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.central.sessions.usage.title")}</h1>
      <p class="lede">{$t("llm.central.sessions.usage.lede")}</p>
    </div>
    <button type="button" class="btn btn-secondary" onclick={refresh} disabled={refreshing}>
      {refreshing ? $t("llm.central.sessions.refreshing") : $t("llm.central.sessions.refresh")}
    </button>
  </header>

  <FilterBar
    bind:rangeKey
    bind:tool
    bind:account
    bind:project
    bind:model
    tools={toolOptions}
    accounts={accountOptions}
    projects={projectOptions}
    models={modelOptions}
  />

  {#if error}
    <div class="notice notice-danger"><div class="notice-body"><div class="notice-text">{error}</div></div></div>
  {/if}

  <div class="content" class:stale={loading && !!byTime}>
    <section class="kpis" aria-label={$t("llm.central.sessions.usage.kpis")}>
      <div class="kpi surface-card">
        <span class="kpi-label">{$t("llm.central.sessions.usage.kpi_tokens")}</span>
        <strong class="kpi-value">{compact(totals?.total_tokens ?? 0)}</strong>
        <span class="kpi-sub">{compact(totals?.usage.output ?? 0)} {$t("llm.central.sessions.tokens.output").toLowerCase()} · {compact(totals?.usage.cache_read ?? 0)} {$t("llm.central.sessions.tokens.cache_read").toLowerCase()}</span>
      </div>
      <div class="kpi surface-card">
        <span class="kpi-label">{$t("llm.central.sessions.usage.kpi_cost")}</span>
        <strong class="kpi-value">{usd(totals?.cost_usd ?? 0)}</strong>
        <span class="kpi-sub">
          {usd(totals?.recorded_usd ?? 0)} {$t("llm.central.sessions.usage.recorded_short")} · {usd(estimated)} {$t("llm.central.sessions.usage.estimated_short")}
        </span>
      </div>
      <div class="kpi surface-card">
        <span class="kpi-label">{$t("llm.central.sessions.usage.kpi_sessions")}</span>
        <strong class="kpi-value">{(totals?.sessions ?? 0).toLocaleString()}</strong>
        <span class="kpi-sub">{compact(totals?.responses ?? 0)} {$t("llm.central.sessions.usage.responses")}</span>
      </div>
      <div class="kpi surface-card">
        <span class="kpi-label">{$t("llm.central.sessions.usage.kpi_subagents")}</span>
        <strong class="kpi-value">{(agents?.total_invocations ?? 0).toLocaleString()}</strong>
        <span class="kpi-sub">{agents?.agent_types ?? 0} {$t("llm.central.sessions.usage.agent_types")} · {subagentSessions} {$t("llm.central.sessions.usage.subagent_files")}</span>
      </div>
    </section>

    {#if unpriced.length}
      <p class="fineprint">{$t("llm.central.sessions.tips.unpriced_model")} <code>{unpriced.join(", ")}</code></p>
    {/if}

    <section class="card surface-card wide">
      <div class="card-head">
        <h2 class="card-title">{$t(`llm.central.sessions.usage.tokens_by_${grain}`)}</h2>
        <label class="check">
          <input type="checkbox" bind:checked={showCacheRead} />
          {$t("llm.central.sessions.usage.include_cache_read")}
        </label>
      </div>
      {#if timeRows.length}
        <StackedColumns rows={timeRows} series={tokenSeries} format={compact} label={$t(`llm.central.sessions.usage.tokens_by_${grain}`)} onpick={grain === "day" ? openDay : undefined} />
      {:else}
        <p class="empty">{loading ? $t("llm.central.sessions.loading") : $t("llm.central.sessions.usage.empty")}</p>
      {/if}
    </section>

    <section class="card surface-card wide">
      <h2 class="card-title">{$t(`llm.central.sessions.usage.cost_by_${grain}`)}</h2>
      {#if timeRows.length}
        <StackedColumns rows={timeRows} series={costSeries} format={usd} height={140} label={$t(`llm.central.sessions.usage.cost_by_${grain}`)} onpick={grain === "day" ? openDay : undefined} />
        <p class="fineprint">{$t("llm.central.sessions.usage.cost_note")}</p>
      {/if}
    </section>

    <div class="grid">
      <section class="card surface-card">
        <h2 class="card-title">{$t("llm.central.sessions.usage.by_model")}</h2>
        {#if modelRows.length}
          <div class="table-scroll">
            <table class="table">
              <thead>
                <tr>
                  <th scope="col">{$t("llm.central.sessions.model")}</th>
                  <th scope="col" class="num">{$t("llm.central.sessions.tokens.input")}</th>
                  <th scope="col" class="num">{$t("llm.central.sessions.tokens.output")}</th>
                  <th scope="col" class="num">{$t("llm.central.sessions.tokens.cache_read")}</th>
                  <th scope="col" class="num">{$t("llm.central.sessions.cost_label")}</th>
                </tr>
              </thead>
              <tbody>
                {#each modelRows as m (m.key)}
                  <tr>
                    <th scope="row" class="bar-cell">
                      <span class="bar" style:width="{(m.total_tokens / modelMax) * 100}%"></span>
                      <button type="button" class="linkish" onclick={() => (model = model === m.key ? "" : m.key)} title={m.key}>{m.key}</button>
                    </th>
                    <td class="num">{compact(m.usage.input)}</td>
                    <td class="num">{compact(m.usage.output + m.usage.reasoning)}</td>
                    <td class="num">{compact(m.usage.cache_read)}</td>
                    <td class="num">
                      {m.unpriced_tokens > 0 && m.cost_usd === 0 ? "—" : usd(m.cost_usd)}
                      {#if m.recorded_usd > 0}<span class="src" title={$t("llm.central.sessions.cost.recorded")}>●</span>{/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          <p class="fineprint"><span class="src">●</span> {$t("llm.central.sessions.cost.recorded")}</p>
        {:else}
          <p class="empty">{$t("llm.central.sessions.usage.empty")}</p>
        {/if}
      </section>

      <section class="card surface-card">
        <h2 class="card-title">{$t("llm.central.sessions.usage.by_tool")}</h2>
        <BarList
          items={(byTool?.rows ?? []).map((r) => ({ key: r.key, label: toolName(r.key), value: r.total_tokens, sub: `${r.sessions} ${$t("llm.central.sessions.sessions_unit")} · ${usd(r.cost_usd)}` })).sort((a, b) => b.value - a.value)}
          format={compact}
          onpick={(k) => (tool = tool === k ? "" : k)}
          empty={$t("llm.central.sessions.usage.empty")}
        />
      </section>

      <section class="card surface-card wide">
        <div class="card-head">
          <h2 class="card-title">{$t("llm.central.sessions.usage.heatmap")}</h2>
          {#if heat}
            <dl class="streaks">
              <div><dt>{$t("llm.central.sessions.usage.current_streak")}</dt><dd>{heat.stats.current_streak} {$t("llm.central.sessions.days_unit")}</dd></div>
              <div title={heat.stats.longest_streak_start ? `${heat.stats.longest_streak_start} → ${heat.stats.longest_streak_end}` : ""}><dt>{$t("llm.central.sessions.usage.longest_streak")}</dt><dd>{heat.stats.longest_streak} {$t("llm.central.sessions.days_unit")}</dd></div>
              <div><dt>{$t("llm.central.sessions.usage.active_days")}</dt><dd>{heat.stats.active_days}</dd></div>
              {#if heat.stats.busiest_day}<div><dt>{$t("llm.central.sessions.usage.busiest_day")}</dt><dd>{heat.stats.busiest_day}</dd></div>{/if}
            </dl>
          {/if}
        </div>
        {#if heat}
          <YearHeatmap days={heat.days} from={heat.from} to={heat.to} onpick={(d) => goto(`/llm/sessions?from=${d}&to=${d}`)} />
        {/if}
      </section>

      <section class="card surface-card">
        <h2 class="card-title">
          {$t("llm.central.sessions.usage.peak_hour")}
          {#if heat?.stats.peak_hour != null}<span class="title-value">{String(heat.stats.peak_hour).padStart(2, "0")}:00</span>{/if}
        </h2>
        {#if hourRows.length}
          <StackedColumns rows={hourRows} series={[{ id: "v", label: $t("llm.central.sessions.messages_unit"), color: "var(--viz-seq)" }]} height={110} tickEvery={3} label={$t("llm.central.sessions.usage.peak_hour")} />
        {/if}
        {#if heat}
          <ul class="weekdays" aria-label={$t("llm.central.sessions.usage.weekdays")}>
            {#each heat.stats.weekdays as v, i (i)}
              {@const wmax = Math.max(1, ...heat.stats.weekdays)}
              <li class:peak={heat.stats.peak_weekday === i}>
                <span class="wbar" style:height="{(v / wmax) * 100}%" title={`${weekdayNames[i]}: ${v}`}></span>
                <span class="wlbl">{weekdayNames[i]}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="card surface-card">
        <h2 class="card-title">{$t("llm.central.sessions.usage.top_projects")}</h2>
        <BarList
          items={(byProject?.rows ?? []).filter((r) => r.key).map((r) => ({ key: r.key, label: projectName(r.key), title: r.key, value: r.total_tokens, sub: `${r.sessions} ${$t("llm.central.sessions.sessions_unit")} · ${usd(r.cost_usd)}` })).sort((a, b) => b.value - a.value)}
          format={compact}
          onpick={(k) => goto(`/llm/sessions?project=${encodeURIComponent(k)}`)}
          empty={$t("llm.central.sessions.usage.empty")}
        />
      </section>

      <section class="card surface-card wide">
        <h2 class="card-title">{$t("llm.central.sessions.usage.agents")}</h2>
        {#if agents && agents.total_invocations > 0}
          <div class="agents">
            <div>
              <p class="mini-kpis">
                <span><strong>{agents.total_invocations}</strong> {$t("llm.central.sessions.usage.invocations")}</span>
                <span><strong>{agents.agent_types}</strong> {$t("llm.central.sessions.usage.agent_types")}</span>
                {#if agents.top_agent}<span>{$t("llm.central.sessions.usage.top_agent")} <strong>{agents.top_agent}</strong></span>{/if}
                <span><strong>{Math.round(agents.adoption_rate)}%</strong> {$t("llm.central.sessions.usage.adoption")}</span>
              </p>
              <BarList
                items={agents.agents.map((a) => ({ key: a.name, label: a.name, value: a.invocations, sub: `${a.sessions} ${$t("llm.central.sessions.sessions_unit")}${a.last ? ` · ${relTime(toMs(a.last))}` : ""}`, title: a.prompts[0] ?? a.name }))}
                limit={10}
              />
            </div>
            <div>
              <h3 class="sub-title">{$t("llm.central.sessions.usage.patterns")}</h3>
              {#if agents.patterns.length}
                <ol class="patterns">
                  {#each agents.patterns as p (p.pattern)}
                    <li><span class="pattern">{p.pattern}</span><span class="num">{p.count}×</span></li>
                  {/each}
                </ol>
              {:else}
                <p class="empty">{$t("llm.central.sessions.usage.no_patterns")}</p>
              {/if}
              {#if agents.by_tool.length}
                <h3 class="sub-title">{$t("llm.central.sessions.usage.agents_by_tool")}</h3>
                <p class="chips">{#each agents.by_tool as [tl, n] (tl)}<span class="chip">{toolName(tl)} <strong>{n}</strong></span>{/each}</p>
              {/if}
            </div>
          </div>
        {:else}
          <p class="empty">{$t("llm.central.sessions.usage.no_agents")}</p>
        {/if}
      </section>

      <section class="card surface-card wide">
        <h2 class="card-title">{$t("llm.central.sessions.usage.sources")}</h2>
        <ul class="sources">
          {#each [...sources].sort((a, b) => Number(b.found) - Number(a.found) || b.sessions - a.sessions) as s (s.tool + s.roots.map((r) => r.path).join())}
            {@const root = s.roots.find((r) => r.exists) ?? s.roots[0]}
            <li class:off={!s.found}>
              <span class="src-name">
                {toolName(s.tool)}
                {#if s.beta}<span class="tag" title={$t("llm.central.sessions.usage.beta_hint")}>beta</span>{/if}
              </span>
              <span class="src-count">{s.found ? `${s.sessions} ${$t("llm.central.sessions.sessions_unit")}` : $t("llm.central.sessions.usage.not_found")}</span>
              <code class="src-root" title={s.roots.map((r) => r.path).join("\n")}>{root?.path ?? "—"}</code>
              <span class="src-when">{s.last_activity ? relTime(toMs(s.last_activity)) : ""}</span>
            </li>
          {/each}
        </ul>
      </section>
    </div>
  </div>
</div>

<style>
  .page {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    box-sizing: border-box;
  }
  .page > :global(*) {
    flex-shrink: 0;
    min-width: 0;
  }
  .page-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .page-title {
    margin: 0;
  }
  .lede {
    margin: 4px 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    max-width: 62ch;
  }
  .content {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    transition: opacity var(--duration-base) var(--ease-out);
  }
  .content.stale {
    opacity: 0.6;
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: var(--space-3);
  }
  .kpi {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .kpi-label {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .kpi-value {
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }
  .kpi-sub {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 360px), 1fr));
    gap: var(--space-4);
    align-items: start;
  }
  .card {
    padding: var(--space-4);
    min-width: 0;
    position: relative;
  }
  .card.wide {
    grid-column: 1 / -1;
  }
  .card-head {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px 16px;
  }
  .card-title {
    margin: 0 0 var(--space-3);
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-muted);
    margin-bottom: var(--space-3);
  }
  .title-value {
    font-weight: 400;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .sub-title {
    margin: var(--space-3) 0 var(--space-2);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-muted);
  }
  .sub-title:first-child {
    margin-top: 0;
  }
  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .fineprint {
    margin: 6px 0 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .fineprint code {
    font-size: var(--text-caption);
  }
  .table-scroll {
    overflow-x: auto;
  }
  .table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
    min-width: 420px;
  }
  .table th,
  .table td {
    padding: 4px 6px;
    text-align: left;
    font-weight: 400;
    white-space: nowrap;
  }
  .table thead th {
    font-size: var(--text-xs);
    color: var(--text-dim);
    border-bottom: 1px solid var(--separator);
  }
  .num {
    text-align: right !important;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  .bar-cell {
    position: relative;
    max-width: 200px;
  }
  .bar-cell .bar {
    position: absolute;
    left: 0;
    top: 4px;
    bottom: 4px;
    background: color-mix(in srgb, var(--viz-seq) 18%, transparent);
    border-radius: var(--radius-xs);
  }
  .linkish {
    position: relative;
    background: none;
    border: none;
    padding: 0 4px;
    font: inherit;
    color: var(--text);
    cursor: pointer;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: block;
    text-align: left;
  }
  .linkish:focus-visible {
    outline: var(--focus-ring);
  }
  .src {
    color: var(--viz-seq);
    font-size: 8px;
    vertical-align: middle;
  }
  .streaks {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 18px;
    margin: 0 0 var(--space-3);
  }
  .streaks div {
    display: flex;
    gap: 6px;
    align-items: baseline;
    font-size: var(--text-sm);
  }
  .streaks dt {
    color: var(--text-dim);
  }
  .streaks dd {
    margin: 0;
    color: var(--text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .weekdays {
    list-style: none;
    margin: var(--space-3) 0 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 6px;
    height: 64px;
  }
  .weekdays li {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    align-items: center;
    gap: 3px;
    min-height: 0;
  }
  .wbar {
    width: 60%;
    max-width: 22px;
    min-height: 2px;
    background: color-mix(in srgb, var(--viz-seq) 45%, var(--viz-empty));
    border-radius: 4px 4px 0 0;
  }
  .weekdays li.peak .wbar {
    background: var(--viz-seq);
  }
  .wlbl {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .agents {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr));
    gap: var(--space-5);
  }
  .mini-kpis {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .mini-kpis strong {
    color: var(--text);
  }
  .patterns {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .patterns li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: var(--text-sm);
    padding: 4px 6px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }
  .pattern {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0;
  }
  .chip {
    font-size: var(--text-xs);
    color: var(--text-muted);
    background: var(--fill-1);
    border-radius: var(--radius-full);
    padding: 2px 8px;
  }
  .chip strong {
    color: var(--text);
  }
  .sources {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .sources li {
    display: grid;
    grid-template-columns: minmax(110px, 160px) minmax(90px, 120px) minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    padding: 6px 4px;
    border-bottom: 1px solid var(--separator);
    font-size: var(--text-sm);
  }
  .sources li:last-child {
    border-bottom: none;
  }
  .sources li.off {
    color: var(--text-dim);
  }
  .src-name {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--text);
  }
  .off .src-name {
    color: var(--text-dim);
  }
  .tag {
    font-size: var(--text-caption);
    color: var(--text-muted);
    background: var(--fill-2);
    border-radius: var(--radius-xs);
    padding: 0 4px;
  }
  .src-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  .src-root {
    font-size: var(--text-caption);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .src-when {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .page {
      padding: var(--space-3);
    }
    .sources li {
      grid-template-columns: 1fr auto;
    }
    .src-root {
      grid-column: 1 / -1;
    }
    .src-when {
      display: none;
    }
  }
</style>
