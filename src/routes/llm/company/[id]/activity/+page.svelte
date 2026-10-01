<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { rpc, errorText, type AuditRow, type Issue, type Run, type Staff, type Wake } from "$lib/company/rpc";
  import RunRow from "$components/company/RunRow.svelte";
  import { dateTime, staffName, watchCompany } from "$components/company/company-format";
  import { downloadText, relTime } from "$components/company/company-ui";

  const PAGE = 100;
  const ENTITIES = ["company", "staff", "issue", "approval", "budget", "goal", "project", "routine", "run"];

  let company = $derived(page.params.id ?? "");
  let tab = $state<"runs" | "audit">(page.url.searchParams.get("tab") === "audit" ? "audit" : "runs");

  let staff = $state<Staff[]>([]);
  let issues = $state<Issue[]>([]);

  let runs = $state<Run[]>([]);
  let wakes = $state<Wake[]>([]);
  let runFilter = $state("");
  let runsLoaded = $state(false);

  let rows = $state<AuditRow[]>([]);
  let entity = $state("");
  let actor = $state("");
  let more = $state(false);
  let auditLoaded = $state(false);
  let verify = $state<{ intact: boolean; text: string } | null>(null);
  let verifying = $state(false);
  let openSeq = $state<number | null>(null);

  let loadError = $state("");

  async function loadRuns() {
    const c = company;
    if (!c) return;
    try {
      const [r, w, st, is] = await Promise.all([
        rpc<Run[]>("run.list", { company: c, staff: runFilter || null, limit: 150 }),
        rpc<Wake[]>("wake.list", { company: c, state: "coalesced", limit: 300 }),
        rpc<Staff[]>("staff.list", { company: c }),
        rpc<Issue[]>("issue.list", { company: c, filter: { include_closed: true, limit: 5000 } }),
      ]);
      if (c !== company) return;
      runs = r;
      wakes = w;
      staff = st;
      issues = is;
      loadError = "";
    } catch (e) {
      loadError = errorText(e);
    } finally {
      runsLoaded = true;
    }
  }

  async function loadAudit(append = false) {
    const c = company;
    if (!c) return;
    const before = append && rows.length ? rows[rows.length - 1].seq : null;
    try {
      const chunk = await rpc<AuditRow[]>("audit.list", {
        company: c,
        query: { entity_kind: entity || null, actor_id: actor || null, before_seq: before, limit: PAGE },
      });
      if (c !== company) return;
      rows = append ? [...rows, ...chunk] : chunk;
      more = chunk.length === PAGE;
      if (!staff.length) staff = await rpc<Staff[]>("staff.list", { company: c });
      loadError = "";
    } catch (e) {
      loadError = errorText(e);
    } finally {
      auditLoaded = true;
    }
  }

  $effect(() => {
    void [company, runFilter];
    if (tab === "runs") void loadRuns();
  });
  $effect(() => {
    void [company, entity, actor];
    if (tab === "audit") void loadAudit();
  });
  $effect(() =>
    watchCompany(company, (batch) => {
      if (tab === "runs" && batch.some((c) => c.entity === "run" || c.entity === "wake" || c.entity === "staff")) void loadRuns();
      if (tab === "audit" && rows.length <= PAGE) void loadAudit();
    }),
  );

  function setTab(next: "runs" | "audit") {
    tab = next;
    const url = new URL(page.url);
    if (next === "audit") url.searchParams.set("tab", "audit");
    else url.searchParams.delete("tab");
    history.replaceState(history.state, "", url);
  }

  function issueRef(r: Run) {
    if (!r.issue_id) return null;
    const i = issues.find((x) => x.id === r.issue_id);
    if (!i) return null;
    return { identifier: i.identifier, href: `/llm/company/${encodeURIComponent(company)}/issues/${encodeURIComponent(i.identifier)}` };
  }

  function wakesOf(r: Run): Wake[] {
    return wakes.filter((w) => w.run_id === r.id);
  }

  let running = $derived(runs.filter((r) => r.state === "running" || r.state === "queued").length);

  let actors = $derived.by(() => {
    const seen = new Map<string, string>();
    for (const r of rows) if (!seen.has(r.actor_id)) seen.set(r.actor_id, actorName(r));
    if (actor && !seen.has(actor)) seen.set(actor, actor);
    return [...seen.entries()];
  });

  function actorName(r: AuditRow): string {
    if (r.actor_kind === "user") return $t("company.common.owner");
    if (r.actor_kind === "agent") return staffName(staff, r.actor_id);
    return r.actor_id || r.actor_kind;
  }

  async function runVerify() {
    verifying = true;
    try {
      const r = await rpc<{ lines: number; intact: boolean }>("audit.verify", { company });
      verify = { intact: r.intact, text: $t("company.activity.chain_ok", { count: r.lines }) };
    } catch (e) {
      verify = { intact: false, text: $t("company.activity.chain_broken", { error: errorText(e) }) };
    } finally {
      verifying = false;
    }
  }

  async function exportCsv() {
    try {
      const r = await rpc<{ csv: string }>("audit.csv", { company });
      downloadText(`audit-${company}.csv`, r.csv, "text/csv");
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  function details(v: unknown): string {
    if (v === null || v === undefined) return "";
    return JSON.stringify(v, null, 2);
  }
</script>

<svelte:head><title>{$t("company.activity.title")}</title></svelte:head>

<div class="activity-page">
  <div class="bar">
    <div class="segmented" role="tablist" aria-label={$t("company.activity.title")}>
      <button type="button" class="segmented-btn" role="tab" aria-selected={tab === "runs"} onclick={() => setTab("runs")}>
        {$t("company.activity.tab_runs")}
        {#if running > 0}<span class="badge">{running}</span>{/if}
      </button>
      <button type="button" class="segmented-btn" role="tab" aria-selected={tab === "audit"} onclick={() => setTab("audit")}>
        {$t("company.activity.tab_audit")}
      </button>
    </div>
    <span class="spacer"></span>
    {#if tab === "runs"}
      <select class="input narrow" bind:value={runFilter} aria-label={$t("company.activity.agent")}>
        <option value="">{$t("company.activity.all_agents")}</option>
        {#each staff as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
      </select>
    {:else}
      <select class="input narrow" bind:value={entity} aria-label={$t("company.activity.entity")}>
        <option value="">{$t("company.activity.all_entities")}</option>
        {#each ENTITIES as e (e)}<option value={e}>{e}</option>{/each}
      </select>
      <select class="input narrow" bind:value={actor} aria-label={$t("company.activity.actor")}>
        <option value="">{$t("company.activity.all_actors")}</option>
        {#each actors as [id, name] (id)}<option value={id}>{name}</option>{/each}
      </select>
      <button type="button" class="btn btn-secondary btn-sm" disabled={verifying} onclick={runVerify}>{$t("company.activity.verify")}</button>
      <button type="button" class="btn btn-secondary btn-sm" onclick={exportCsv}>{$t("company.activity.export_csv")}</button>
    {/if}
  </div>

  {#if loadError}<p class="field-error" role="alert">{loadError}</p>{/if}

  {#if tab === "runs"}
    {#if !runsLoaded}
      <div class="skeleton" style="height: 120px"></div>
    {:else if runs.length === 0}
      <div class="empty-state">
        <img class="empty-state-art" src="/emoji/hourglass_not_done.png" alt="" width="72" height="72" draggable="false" />
        <p class="empty-state-title">{$t("company.activity.no_runs")}</p>
        <p class="empty-state-hint">{$t("company.activity.no_runs_hint")}</p>
      </div>
    {:else}
      <div class="group">
        {#each runs as r (r.id)}
          <RunRow run={r} {staff} issueRef={issueRef(r)} wakes={wakesOf(r)} />
        {/each}
      </div>
    {/if}
  {:else}
    {#if verify}
      <div class="notice" class:notice-success={verify.intact} class:notice-warning={!verify.intact} role="status">
        <div class="notice-body">
          <span class="notice-title">{verify.intact ? $t("company.activity.chain_intact") : $t("company.activity.chain_bad")}</span>
          <span class="notice-text">{verify.text}</span>
        </div>
      </div>
    {/if}
    {#if !auditLoaded}
      <div class="skeleton" style="height: 120px"></div>
    {:else if rows.length === 0}
      <div class="empty-state">
        <img class="empty-state-art" src="/emoji/books.png" alt="" width="72" height="72" draggable="false" />
        <p class="empty-state-title">{$t("company.activity.no_audit")}</p>
      </div>
    {:else}
      <div class="group">
        {#each rows as r (r.seq)}
          <div class="audit">
            <button type="button" class="list-row interactive compact line" aria-expanded={openSeq === r.seq} onclick={() => (openSeq = openSeq === r.seq ? null : r.seq)}>
              <span class="seq">#{r.seq}</span>
              <span class="action">{r.action}</span>
              <span class="ent">{r.entity_kind}</span>
              <span class="who">{actorName(r)}</span>
              <span class="chan">{r.channel}</span>
              <time class="when" title={dateTime(r.ts_ms)}>{relTime(r.ts_ms, $t)}</time>
            </button>
            {#if openSeq === r.seq}
              <div class="audit-detail">
                <dl>
                  <dt>{$t("company.activity.entity")}</dt><dd class="mono">{r.entity_kind} · {r.entity_id}</dd>
                  <dt>{$t("company.activity.actor")}</dt><dd class="mono">{r.actor_kind} · {r.actor_id}</dd>
                  {#if r.run_id}<dt>{$t("company.activity.run")}</dt><dd class="mono">{r.run_id}</dd>{/if}
                  <dt>{$t("company.activity.hash")}</dt><dd class="mono">{r.hash}</dd>
                  <dt>{$t("company.activity.when")}</dt><dd>{dateTime(r.ts_ms)}</dd>
                </dl>
                {#if details(r.details)}<pre class="pre">{details(r.details)}</pre>{/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
      {#if more}
        <div class="more"><button type="button" class="btn btn-secondary btn-sm" onclick={() => loadAudit(true)}>{$t("company.activity.load_more")}</button></div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .activity-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
  .narrow {
    width: auto;
    max-width: 180px;
  }
  .badge {
    margin-left: 4px;
  }
  .line {
    border-radius: 0;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .seq,
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .seq {
    min-width: 48px;
    flex-shrink: 0;
  }
  .action {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ent,
  .chan {
    flex-shrink: 0;
    padding: 0 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-xs);
    color: var(--text-muted);
    background: var(--fill-2);
  }
  .who {
    flex-shrink: 0;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    flex-shrink: 0;
    min-width: 72px;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .audit-detail {
    padding: 0 var(--space-3) var(--space-3);
    background: var(--fill-1);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px var(--space-3);
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
  }
  dt {
    color: var(--text-dim);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .pre {
    margin: 0;
    padding: var(--space-2);
    max-height: 260px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    user-select: text;
  }
  .more {
    display: flex;
    justify-content: center;
  }
</style>
