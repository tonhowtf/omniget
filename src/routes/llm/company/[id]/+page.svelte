<script lang="ts">
  /**
   * Company overview: mission and goals, the live org chart, work by status,
   * spend against budgets, open approvals and the latest runs. One
   * `company.dashboard` read, refreshed on every `company://changed`.
   */
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { errorText, onCompanyChange, rpc, type Dashboard, type Issue, type IssueStatus, type PolicyStatus } from "$lib/company/rpc";
  import OrgChart from "$components/company/OrgChart.svelte";
  import StatusDot from "$components/company/StatusDot.svelte";
  import { duration, microsToUsd, relTime, statusTone, usd } from "$components/company/company-ui";

  let id = $derived(page.params.id ?? "");
  let base = $derived(`/llm/company/${id}`);
  let dash = $state<Dashboard | null>(null);
  let issues = $state<Map<string, Issue>>(new Map());
  let error = $state("");
  let now = $state(Date.now());

  const STATUSES: IssueStatus[] = ["backlog", "todo", "in_progress", "in_review", "blocked", "done", "cancelled"];

  async function load(cid: string) {
    try {
      const [d, list] = await Promise.all([
        rpc<Dashboard>("company.dashboard", { company: cid }),
        rpc<Issue[]>("issue.list", { company: cid, filter: { include_closed: true, limit: 500 } }).catch(() => [] as Issue[]),
      ]);
      dash = d;
      issues = new Map(list.map((i) => [i.id, i]));
      error = "";
    } catch (e) {
      error = errorText(e);
    }
  }

  $effect(() => {
    const cid = id;
    if (!cid) return;
    void load(cid);
    const un = onCompanyChange(cid, () => void load(cid));
    const timer = setInterval(() => (now = Date.now()), 15000);
    return () => {
      clearInterval(timer);
      void un.then((f) => f());
    };
  });

  let staffName = $derived(new Map((dash?.staff ?? []).map((s) => [s.id, s.name])));
  let activeGoals = $derived((dash?.goals ?? []).filter((g) => g.status === "active" && g.level !== "task").slice(0, 6));
  let totalIssues = $derived(STATUSES.reduce((n, s) => n + (dash?.issue_counts[s] ?? 0), 0));
  let openIssues = $derived(
    (dash?.issue_counts.todo ?? 0) + (dash?.issue_counts.in_progress ?? 0) + (dash?.issue_counts.in_review ?? 0) + (dash?.issue_counts.blocked ?? 0),
  );
  let hired = $derived((dash?.staff ?? []).filter((s) => s.status !== "terminated").length);

  function scopeName(b: PolicyStatus): string {
    const p = b.policy;
    if (p.scope_type === "company") return $t("company.overview.budget_company");
    if (p.scope_type === "agent") return staffName.get(p.scope_id) ?? p.scope_id;
    return dash?.projects.find((x) => x.id === p.scope_id)?.name ?? p.scope_id;
  }

  function metricValue(metric: string, v: number): string {
    if (metric === "usd") return `US$ ${usd(v)}`;
    return Math.round(v).toLocaleString();
  }

  function barTone(b: PolicyStatus): string {
    if (b.pct >= 100) return "red";
    if (b.pct >= b.policy.warn_pct) return "orange";
    return "green";
  }
</script>

<div class="page page-wide overview">
  {#if error}<p class="field-error">{error}</p>{/if}

  {#if !dash}
    <div class="stats">{#each [0, 1, 2, 3] as i (i)}<div class="stat skeleton" style="height: 72px"></div>{/each}</div>
  {:else}
    <div class="stats">
      <a class="stat surface-card" href="{base}/team">
        <span class="stat-num">{dash.running}<span class="stat-of">/{hired}</span></span>
        <span class="stat-label">{$t("company.overview.running")}</span>
      </a>
      <a class="stat surface-card" href="{base}/issues">
        <span class="stat-num">{openIssues}</span>
        <span class="stat-label">{$t("company.overview.open_tasks")}</span>
      </a>
      <a class="stat surface-card" href="{base}/inbox">
        <span class="stat-num" class:warn={dash.open_approvals > 0}>{dash.open_approvals}</span>
        <span class="stat-label">{$t("company.overview.approvals")}</span>
      </a>
      <a class="stat surface-card" href="{base}/costs">
        <span class="stat-num">US$ {usd(dash.costs.cost_usd)}</span>
        <span class="stat-label">{$t("company.overview.spend_month")}</span>
      </a>
    </div>

    <div class="grid">
      <section class="block">
        <div class="section-header">
          <h2 class="section-header-title">{$t("company.overview.mission")}</h2>
          <a class="section-header-action" href="{base}/goals">{$t("company.overview.see_goals")}</a>
        </div>
        <div class="group pad">
          <p class="mission-text">{dash.company.mission || $t("company.overview.no_mission")}</p>
          {#if activeGoals.length}
            <ul class="goal-list">
              {#each activeGoals as g (g.id)}
                <li>
                  <span class="tag">{$t(`company.common.goal_level.${g.level}`)}</span>
                  <span class="goal-title">{g.title}</span>
                  {#if g.owner_staff_id}<span class="dim">{staffName.get(g.owner_staff_id) ?? ""}</span>{/if}
                </li>
              {/each}
            </ul>
          {:else}
            <p class="hint">{$t("company.overview.no_goals")}</p>
          {/if}
        </div>
      </section>

      <section class="block">
        <div class="section-header">
          <h2 class="section-header-title">{$t("company.overview.tasks")}</h2>
          <a class="section-header-action" href="{base}/issues">{$t("company.overview.see_tasks")}</a>
        </div>
        <div class="group pad">
          {#if totalIssues === 0}
            <p class="hint">{$t("company.overview.no_tasks")}</p>
          {:else}
            <div class="stack-bar" role="img" aria-label={$t("company.overview.tasks")}>
              {#each STATUSES as s (s)}
                {#if dash.issue_counts[s]}
                  <span class="seg {statusTone(s)}" style:flex-grow={dash.issue_counts[s]} title="{$t(`company.common.status.${s}`)}: {dash.issue_counts[s]}"></span>
                {/if}
              {/each}
            </div>
            <ul class="legend">
              {#each STATUSES as s (s)}
                <li class:zero={!dash.issue_counts[s]}>
                  <span class="sw {statusTone(s)}"></span>{$t(`company.common.status.${s}`)}
                  <span class="num">{dash.issue_counts[s] ?? 0}</span>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      </section>
    </div>

    <section class="block">
      <div class="section-header">
        <h2 class="section-header-title">{$t("company.overview.org")}</h2>
        <a class="section-header-action" href="{base}/team">{$t("company.overview.manage_team")}</a>
      </div>
      {#if hired === 0}
        <div class="group empty-state compact-empty">
          <img class="empty-state-art" src="/emoji/sparkles.png" alt="" width="72" height="72" draggable="false" />
          <div class="empty-state-title">{$t("company.overview.no_staff")}</div>
          <a class="button active" href="{base}/team">{$t("company.overview.hire_first")}</a>
        </div>
      {:else}
        <div class="group chart-wrap"><OrgChart staff={dash.staff} compact /></div>
      {/if}
    </section>

    <div class="grid">
      <section class="block">
        <div class="section-header">
          <h2 class="section-header-title">{$t("company.overview.budgets")}</h2>
          <a class="section-header-action" href="{base}/costs">{$t("company.overview.see_costs")}</a>
        </div>
        <div class="group pad">
          {#if dash.budgets.length === 0}
            <p class="hint">{$t("company.overview.no_budgets")}</p>
          {:else}
            {#each dash.budgets as b (b.policy.id)}
              <div class="budget">
                <div class="budget-line">
                  <span class="budget-name">{scopeName(b)} · {$t(`company.overview.window.${b.policy.window}`)}</span>
                  <span class="num">{metricValue(b.policy.metric, b.used)} / {metricValue(b.policy.metric, b.policy.amount)}</span>
                </div>
                <div class="bar"><span class="fill {barTone(b)}" style:width="{Math.min(100, b.pct)}%"></span></div>
              </div>
            {/each}
          {/if}
        </div>
      </section>

      <section class="block">
        <div class="section-header">
          <h2 class="section-header-title">{$t("company.overview.open_approvals")}</h2>
          <a class="section-header-action" href="{base}/inbox">{$t("company.overview.see_inbox")}</a>
        </div>
        <div class="group">
          {#if dash.approvals.length === 0}
            <div class="group-row"><span class="hint">{$t("company.overview.no_approvals")}</span></div>
          {:else}
            {#each dash.approvals.slice(0, 5) as a (a.id)}
              <a class="group-row link-row" href="{base}/inbox">
                <span class="group-row-content">
                  <span class="group-row-title">{a.title}</span>
                  <span class="group-row-sub">
                    {$t(`company.overview.approval_kind.${a.kind}`)}{#if a.requested_by_staff_id} · {staffName.get(a.requested_by_staff_id) ?? ""}{/if} · {relTime(a.created_ms, $t, now)}
                  </span>
                </span>
                <span class="chevron" aria-hidden="true">›</span>
              </a>
            {/each}
          {/if}
        </div>
      </section>
    </div>

    <section class="block">
      <div class="section-header">
        <h2 class="section-header-title">{$t("company.overview.recent_runs")}</h2>
        <a class="section-header-action" href="{base}/activity">{$t("company.overview.see_activity")}</a>
      </div>
      <div class="group">
        {#if dash.recent_runs.length === 0}
          <div class="group-row"><span class="hint">{$t("company.overview.no_runs")}</span></div>
        {:else}
          {#each dash.recent_runs.slice(0, 12) as r (r.id)}
            {@const issue = r.issue_id ? issues.get(r.issue_id) : undefined}
            <div class="group-row run-row">
              <StatusDot status={r.state} kind="run" />
              <span class="run-agent">{staffName.get(r.staff_id) ?? r.staff_id}</span>
              <span class="run-task" title={issue?.title ?? ""}>
                {#if issue}<span class="ident">{issue.identifier}</span> {issue.title}{:else}<span class="dim">{$t(`company.overview.source.${r.source}`)}</span>{/if}
              </span>
              <span class="run-state">{$t(`company.common.run_state.${r.state}`)}</span>
              {#if r.liveness}<span class="tag">{$t(`company.overview.liveness.${r.liveness}`)}</span>{/if}
              <span class="num run-cost">US$ {microsToUsd(r.cost_micros)}</span>
              <span class="num run-dur">{duration(r.started_ms, r.finished_ms ?? (r.state === "running" ? now : r.started_ms))}</span>
              <span class="dim run-when">{relTime(r.created_ms, $t, now)}</span>
            </div>
          {/each}
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .overview { display: flex; flex-direction: column; gap: var(--space-5); }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: var(--space-3); }
  .stat { display: flex; flex-direction: column; gap: 2px; padding: var(--space-3) var(--space-4); text-decoration: none; color: inherit; }
  @media (hover: hover) { a.stat:hover { background: var(--surface-hi); } }
  .stat-num { font-family: var(--font-display); font-size: var(--text-xl); font-weight: 600; font-variant-numeric: tabular-nums; }
  .stat-num.warn { color: var(--orange); }
  .stat-of { font-size: var(--text-sm); color: var(--text-dim); font-weight: 500; }
  .stat-label { font-size: var(--text-sm); color: var(--text-dim); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: var(--space-5); }
  .block { min-width: 0; }
  .pad { padding: var(--space-3) var(--space-4); display: flex; flex-direction: column; gap: var(--space-3); }
  .mission-text { margin: 0; font-size: var(--text-base); line-height: var(--leading-base); white-space: pre-wrap; }
  .goal-list { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 6px; }
  .goal-list li { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); min-width: 0; }
  .goal-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .dim { font-size: var(--text-xs); color: var(--text-dim); }
  .num { font-variant-numeric: tabular-nums; }
  .chart-wrap { padding: var(--space-3); }
  .compact-empty { padding: var(--space-5); }

  .stack-bar { display: flex; height: 10px; border-radius: 5px; overflow: hidden; gap: 2px; background: var(--fill-1); }
  .seg { min-width: 4px; }
  .legend { display: grid; grid-template-columns: repeat(auto-fill, minmax(130px, 1fr)); gap: 4px var(--space-3); margin: 0; padding: 0; list-style: none; font-size: var(--text-sm); }
  .legend li { display: flex; align-items: center; gap: 6px; color: var(--text-muted); }
  .legend li.zero { opacity: 0.5; }
  .legend .num { margin-left: auto; color: var(--text); font-weight: 600; }
  .sw { width: 8px; height: 8px; border-radius: 2px; flex-shrink: 0; }
  .green { background: var(--green); }
  .blue { background: var(--blue); }
  .orange { background: var(--orange); }
  .red { background: var(--red); }
  .yellow { background: var(--warning); }
  .gray { background: var(--text-dim); }
  .faint { background: var(--text-faint); }

  .budget { display: flex; flex-direction: column; gap: 4px; }
  .budget-line { display: flex; justify-content: space-between; gap: var(--space-2); font-size: var(--text-sm); }
  .budget-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bar { height: 6px; border-radius: 3px; background: var(--fill-2); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 3px; }

  .link-row { text-decoration: none; color: inherit; }
  @media (hover: hover) { .link-row:hover { background: var(--fill-1); } }
  .run-row { font-size: var(--text-sm); }
  .run-agent { font-weight: 600; flex-shrink: 0; min-width: 80px; }
  .run-task { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ident { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-dim); }
  .run-state { flex-shrink: 0; color: var(--text-muted); font-size: var(--text-xs); }
  .run-cost, .run-dur { flex-shrink: 0; font-size: var(--text-xs); color: var(--text-muted); min-width: 64px; text-align: right; }
  .run-when { flex-shrink: 0; min-width: 72px; text-align: right; }
  @media (max-width: 720px) { .run-cost, .run-when { display: none; } }
</style>
