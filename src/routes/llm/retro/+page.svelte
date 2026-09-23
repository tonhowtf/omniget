<script lang="ts">
  /**
   * Retrospectiva — qualquer período, qualquer ferramenta (ou todas).
   *
   * Cartões com o que marcou o período (modelos, tools, projetos, sequência,
   * dia mais produtivo, marcos) e uma animação em canvas que reproduz os dias.
   * Busca só ao abrir e quando o período ou a ferramenta mudam.
   */
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import RetroCanvas from "$components/central/sessions/RetroCanvas.svelte";
  import BarList from "$components/central/sessions/BarList.svelte";
  import YearHeatmap from "$components/central/sessions/YearHeatmap.svelte";
  import {
    compact,
    duration,
    errText,
    fill,
    isoDay,
    projectName,
    sessionsRetro,
    sessionsSources,
    toolName,
    usd,
    type Milestone,
    type Retro,
  } from "$lib/central/sessions";

  type Preset = "this_month" | "last_month" | "this_year" | "last_year" | "12m" | "all" | "custom";
  const PRESETS: Preset[] = ["this_month", "last_month", "12m", "this_year", "last_year", "all", "custom"];

  let preset = $state<Preset>("12m");
  let customFrom = $state("");
  let customTo = $state(isoDay(new Date()));
  let tool = $state("");
  let tools = $state<string[]>([]);
  let retro = $state<Retro | null>(null);
  let loading = $state(false);
  let error = $state("");
  let seq = 0;

  function bounds(p: Preset): [string | null, string | null] {
    const now = new Date();
    const y = now.getFullYear();
    const m = now.getMonth();
    switch (p) {
      case "this_month":
        return [isoDay(new Date(y, m, 1)), isoDay(now)];
      case "last_month":
        return [isoDay(new Date(y, m - 1, 1)), isoDay(new Date(y, m, 0))];
      case "this_year":
        return [isoDay(new Date(y, 0, 1)), isoDay(now)];
      case "last_year":
        return [isoDay(new Date(y - 1, 0, 1)), isoDay(new Date(y - 1, 11, 31))];
      case "12m": {
        const d = new Date(now);
        d.setDate(d.getDate() - 364);
        return [isoDay(d), isoDay(now)];
      }
      case "custom":
        return [customFrom || null, customTo || null];
      default:
        return [null, null];
    }
  }

  let range = $derived(bounds(preset));

  async function load() {
    const my = ++seq;
    loading = true;
    error = "";
    try {
      const r = await sessionsRetro(range[0], range[1], tool || null);
      if (my === seq) retro = r;
    } catch (e) {
      if (my === seq) error = errText(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  onMount(() => {
    sessionsSources()
      .then((s) => (tools = s.filter((x) => x.sessions > 0).map((x) => x.tool)))
      .catch(() => {});
  });

  $effect(() => {
    void range;
    void tool;
    void load();
  });

  function milestoneText(m: Milestone): string {
    const key = `llm.central.sessions.milestone.${m.kind}`;
    const vars: Record<string, string | number> = {
      n: m.kind === "sessions" ? Number(m.label).toLocaleString() : m.kind === "tokens" ? compact(m.value) : m.value,
      name: m.kind === "new_tool" ? toolName(m.label) : m.label,
    };
    const base = fill($t(key, vars), vars);
    switch (m.kind) {
      case "biggest_session":
        return `${base}: ${m.label} · ${compact(m.value)} tokens`;
      case "longest_session":
        return `${base}: ${m.label} · ${duration(m.value)}`;
      case "first_session":
        return `${base}: ${m.label}`;
      default:
        return base;
    }
  }

  function weekdayName(i: number | null): string {
    if (i == null) return "—";
    return new Date(2024, 0, 1 + i).toLocaleDateString(undefined, { weekday: "long" });
  }

  function longDate(date: string): string {
    const [y, m, d] = date.split("-").map(Number);
    return new Date(y, m - 1, d).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
  }
</script>

<div class="page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.central.sessions.retro.title")}</h1>
      <p class="lede">{$t("llm.central.sessions.retro.lede")}</p>
    </div>
  </header>

  <div class="filters" role="group" aria-label={$t("llm.central.sessions.filters")}>
    <select class="input" bind:value={preset} aria-label={$t("llm.central.sessions.period")}>
      {#each PRESETS as p (p)}<option value={p}>{$t(`llm.central.sessions.retro.preset_${p}`)}</option>{/each}
    </select>
    {#if preset === "custom"}
      <label class="date"><span>{$t("llm.central.sessions.from")}</span><input class="input" type="date" bind:value={customFrom} /></label>
      <label class="date"><span>{$t("llm.central.sessions.to")}</span><input class="input" type="date" bind:value={customTo} /></label>
    {/if}
    <select class="input" bind:value={tool} aria-label={$t("llm.central.sessions.tool")}>
      <option value="">{$t("llm.central.sessions.all_tools")}</option>
      {#each tools as tl (tl)}<option value={tl}>{toolName(tl)}</option>{/each}
    </select>
    {#if loading}<span class="muted">{$t("llm.central.sessions.loading")}</span>{/if}
  </div>

  {#if error}
    <div class="notice notice-danger"><div class="notice-body"><div class="notice-text">{error}</div></div></div>
  {/if}

  {#if retro}
    {#if retro.totals.sessions === 0}
      <div class="empty-state">
        <div class="empty-state-title">{$t("llm.central.sessions.retro.empty_title")}</div>
        <div class="empty-state-hint">{$t("llm.central.sessions.retro.empty_hint")}</div>
      </div>
    {:else}
      <p class="period">{longDate(retro.from)} → {longDate(retro.to)}{tool ? ` · ${toolName(tool)}` : ""}</p>

      <section class="replay" aria-label={$t("llm.central.sessions.retro.replay")}>
        <RetroCanvas {retro} {milestoneText} />
      </section>

      <section class="hero">
        <div class="stat"><strong>{retro.totals.sessions.toLocaleString()}</strong><span>{$t("llm.central.sessions.sessions_unit")}</span></div>
        <div class="stat"><strong>{retro.totals.user_messages.toLocaleString()}</strong><span>{$t("llm.central.sessions.retro.prompts")}</span></div>
        <div class="stat"><strong>{compact(retro.totals.total_tokens)}</strong><span>tokens</span></div>
        <div class="stat"><strong>{retro.totals.tool_calls.toLocaleString()}</strong><span>{$t("llm.central.sessions.tool_calls_unit")}</span></div>
        <div class="stat"><strong>{usd(retro.totals.cost_usd)}</strong><span>{$t("llm.central.sessions.retro.cost_est")}</span></div>
        <div class="stat"><strong>{retro.totals.active_days}</strong><span>{$t("llm.central.sessions.usage.active_days")}</span></div>
        <div class="stat"><strong>{retro.totals.projects}</strong><span>{$t("llm.central.sessions.retro.projects")}</span></div>
      </section>

      <div class="cards">
        <section class="card surface-card">
          <h2>{$t("llm.central.sessions.retro.rhythm")}</h2>
          <dl class="facts">
            <div><dt>{$t("llm.central.sessions.usage.current_streak")}</dt><dd>{retro.current_streak} {$t("llm.central.sessions.days_unit")}</dd></div>
            <div><dt>{$t("llm.central.sessions.usage.longest_streak")}</dt><dd>{retro.longest_streak} {$t("llm.central.sessions.days_unit")}</dd></div>
            {#if retro.most_productive_day}
              <div>
                <dt>{$t("llm.central.sessions.retro.best_day")}</dt>
                <dd>
                  <button type="button" class="linkish" onclick={() => goto(`/llm/sessions?from=${retro!.most_productive_day!.date}&to=${retro!.most_productive_day!.date}`)}>{longDate(retro.most_productive_day.date)}</button>
                  <small>{retro.most_productive_day.messages} {$t("llm.central.sessions.messages_unit")} · {compact(retro.most_productive_day.tokens)} tokens</small>
                </dd>
              </div>
            {/if}
            <div><dt>{$t("llm.central.sessions.usage.peak_hour")}</dt><dd>{retro.peak_hour != null ? `${String(retro.peak_hour).padStart(2, "0")}:00` : "—"}</dd></div>
            <div><dt>{$t("llm.central.sessions.retro.busiest_weekday")}</dt><dd>{weekdayName(retro.busiest_weekday)}</dd></div>
          </dl>
        </section>

        <section class="card surface-card">
          <h2>{$t("llm.central.sessions.retro.top_models")}</h2>
          <BarList items={retro.top_models.map((r) => ({ key: r.name, label: r.name, value: r.count, sub: `${Math.round(r.pct)}% · ${compact(r.tokens)} tokens` }))} format={compact} limit={6} />
        </section>

        <section class="card surface-card">
          <h2>{$t("llm.central.sessions.retro.top_tools")}</h2>
          <BarList items={retro.top_tools.map((r) => ({ key: r.name, label: r.name, value: r.count, sub: `${Math.round(r.pct)}%` }))} format={compact} limit={8} />
        </section>

        <section class="card surface-card">
          <h2>{$t("llm.central.sessions.retro.top_projects")}</h2>
          <BarList
            items={retro.top_projects.map((r) => ({ key: r.name, label: projectName(r.name) || r.name, title: r.name, value: r.count, sub: `${compact(r.tokens)} tokens` }))}
            format={compact}
            limit={6}
            onpick={(k) => goto(`/llm/sessions?project=${encodeURIComponent(k)}`)}
          />
        </section>

        {#if retro.by_tool.length > 1 || !tool}
          <section class="card surface-card">
            <h2>{$t("llm.central.sessions.usage.by_tool")}</h2>
            <BarList items={retro.by_tool.map((r) => ({ key: r.name, label: toolName(r.name), value: r.count, sub: `${compact(r.tokens)} tokens` }))} format={compact} limit={8} onpick={(k) => (tool = k)} />
          </section>
        {/if}

        {#if retro.top_agents.length || retro.top_skills.length || retro.top_slash_commands.length || retro.top_mcp.length}
          <section class="card surface-card">
            <h2>{$t("llm.central.sessions.retro.components")}</h2>
            {#each [["agents", retro.top_agents], ["skills", retro.top_skills], ["slash_commands", retro.top_slash_commands], ["mcp_servers", retro.top_mcp]] as [k, list] (k)}
              {#if (list as { name: string; count: number }[]).length}
                <h3>{$t(`llm.central.sessions.analysis.comp_${k}`)}</h3>
                <p class="chips">{#each (list as { name: string; count: number }[]).slice(0, 8) as r (r.name)}<span class="chip">{r.name} <strong>{r.count}</strong></span>{/each}</p>
              {/if}
            {/each}
          </section>
        {/if}

        <section class="card surface-card wide">
          <h2>{$t("llm.central.sessions.retro.calendar")}</h2>
          <YearHeatmap days={retro.heatmap} from={retro.from} to={retro.to} onpick={(d) => goto(`/llm/sessions?from=${d}&to=${d}`)} />
        </section>

        {#if retro.milestones.length}
          <section class="card surface-card wide">
            <h2>{$t("llm.central.sessions.retro.milestones")}</h2>
            <ol class="milestones">
              {#each retro.milestones as m (m.date + m.kind + m.label)}
                <li><time datetime={m.date}>{longDate(m.date)}</time><span>{milestoneText(m)}</span></li>
              {/each}
            </ol>
          </section>
        {/if}
      </div>
    {/if}
  {/if}
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
  .page-title {
    margin: 0;
  }
  .lede {
    margin: 4px 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    max-width: 62ch;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .filters .input {
    height: 28px;
    font-size: var(--text-sm);
    padding-block: 0;
    max-width: 220px;
  }
  .date {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .muted {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .period {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .hero {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: var(--space-3);
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .stat strong {
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }
  .stat span {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 300px), 1fr));
    gap: var(--space-4);
    align-items: start;
  }
  .card {
    padding: var(--space-4);
    min-width: 0;
  }
  .card.wide {
    grid-column: 1 / -1;
  }
  .card h2 {
    margin: 0 0 var(--space-3);
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text);
  }
  .card h3 {
    margin: var(--space-2) 0 4px;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .facts {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .facts div {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: var(--text-sm);
  }
  .facts dt {
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    color: var(--text);
    font-weight: 600;
    text-align: right;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
  }
  .facts small {
    font-weight: 400;
    color: var(--text-dim);
    font-size: var(--text-xs);
  }
  .linkish {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-weight: 600;
    color: var(--accent-hi);
    cursor: pointer;
  }
  .linkish:focus-visible {
    outline: var(--focus-ring);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 0;
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
  .milestones {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-left: 2px solid var(--separator);
    padding-left: 12px;
  }
  .milestones li {
    display: grid;
    grid-template-columns: 150px 1fr;
    gap: 10px;
    font-size: var(--text-sm);
    color: var(--text);
  }
  .milestones time {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  @media (max-width: 640px) {
    .page {
      padding: var(--space-3);
    }
    .milestones li {
      grid-template-columns: 1fr;
      gap: 0;
    }
  }
</style>
