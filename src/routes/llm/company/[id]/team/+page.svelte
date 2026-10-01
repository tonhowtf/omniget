<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { errorText, onCompanyChange, rpc, type ChartNode, type Run, type RuntimeTemplate, type Staff } from "$lib/company/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import OrgChart from "$components/company/OrgChart.svelte";
  import StaffForm from "$components/company/StaffForm.svelte";
  import StatusDot from "$components/company/StatusDot.svelte";
  import StaffScorecards from "$components/company/StaffScorecards.svelte";
  import SecretBindings from "$components/company/SecretBindings.svelte";
  import { avatarColor, flattenChart, initials, microsToUsd, relTime } from "$components/company/company-ui";

  let id = $derived(page.params.id ?? "");
  let chart = $state<ChartNode[]>([]);
  let staff = $state<Staff[]>([]);
  let lastRun = $state<Map<string, Run>>(new Map());
  let templates = $state<RuntimeTemplate[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let openId = $state<string | null>(null);
  let showFormer = $state(false);
  let editing = $state<string | null>(null);
  let busyId = $state<string | null>(null);
  let now = $state(Date.now());

  async function load(cid: string) {
    try {
      const [c, list, runs] = await Promise.all([
        rpc<ChartNode[]>("staff.chart", { company: cid }),
        rpc<Staff[]>("staff.list", { company: cid }),
        rpc<Run[]>("run.list", { company: cid, limit: 200 }).catch(() => [] as Run[]),
      ]);
      chart = c;
      staff = list;
      const m = new Map<string, Run>();
      for (const r of runs) if (!m.has(r.staff_id) || m.get(r.staff_id)!.created_ms < r.created_ms) m.set(r.staff_id, r);
      lastRun = m;
      error = "";
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    const cid = id;
    if (!cid) return;
    void load(cid);
    invoke<RuntimeTemplate[]>("company_runtime_templates").then((l) => (templates = l)).catch(() => {});
    const un = onCompanyChange(cid, () => void load(cid));
    const timer = setInterval(() => (now = Date.now()), 15000);
    return () => {
      clearInterval(timer);
      void un.then((f) => f());
    };
  });

  let byId = $derived(new Map(staff.map((s) => [s.id, s])));
  let ordered = $derived.by(() => {
    const inChart = flattenChart(chart);
    const seen = new Set(inChart.map((s) => s.id));
    return [...inChart.map((s) => byId.get(s.id) ?? s), ...staff.filter((s) => !seen.has(s.id))];
  });
  let active = $derived(ordered.filter((s) => s.status !== "terminated"));
  let former = $derived(ordered.filter((s) => s.status === "terminated"));
  let editingStaff = $derived(editing && editing !== "new" ? byId.get(editing) ?? null : null);

  function select(s: Staff) {
    openId = s.id;
    requestAnimationFrame(() => document.getElementById(`staff-${s.id}`)?.scrollIntoView({ block: "nearest", behavior: "smooth" }));
  }

  async function act(s: Staff, fn: () => Promise<unknown>, ok?: string) {
    busyId = s.id;
    try {
      await fn();
      if (ok) showToast("success", ok);
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      busyId = null;
    }
  }

  function setStatus(s: Staff, action: "pause" | "resume" | "terminate") {
    if (action === "terminate" && !confirm($t("company.team.terminate_confirm", { name: s.name }) as string)) return;
    void act(s, () => rpc("staff.status", { company: id, staff: s.id, action }));
  }

  function wake(s: Staff) {
    void act(s, () => rpc("staff.wake", { company: id, staff: s.id }), $t("company.team.woken", { name: s.name }));
  }

  function runtimeLabel(s: Staff): string {
    if (s.adapter.kind === "process") return `${$t("company.team.form.runtime_process")} · ${s.adapter.command}`;
    if (s.adapter.kind === "http") return `${$t("company.team.form.runtime_http")} · ${s.adapter.url}`;
    const tpl = templates.find((x) => x.id === s.agent_id);
    return tpl ? `${tpl.name}${tpl.model ? ` · ${tpl.model}` : ""}` : s.agent_id;
  }
</script>

<div class="page page-wide team">
  <header class="page-head">
    <div>
      <h2 class="page-title">{$t("company.team.title")}</h2>
      <p class="page-lede">{$t("company.team.lede")}</p>
    </div>
    <button type="button" class="button active" onclick={() => (editing = "new")}>{$t("company.team.hire")}</button>
  </header>

  {#if error}<p class="field-error">{error}</p>{/if}

  {#if loaded && active.length === 0}
    <div class="empty-state">
      <img class="empty-state-art" src="/emoji/sparkles.png" alt="" width="72" height="72" draggable="false" />
      <div class="empty-state-title">{$t("company.team.empty_title")}</div>
      <p class="empty-state-hint">{$t("company.team.empty_hint")}</p>
      <button type="button" class="button active" onclick={() => (editing = "new")}>{$t("company.team.hire_first")}</button>
    </div>
  {:else if active.length}
    <section>
      <h3 class="group-label">{$t("company.team.chart")}</h3>
      <div class="group chart-wrap"><OrgChart nodes={chart} selected={openId} onselect={select} /></div>
    </section>

    <section>
      <h3 class="group-label">{$t("company.team.list", { count: active.length })}</h3>
      <div class="group">
        {#each active as s (s.id)}
          {@render row(s)}
        {/each}
      </div>
    </section>

    {#key id}<StaffScorecards company={id} />{/key}
    <SecretBindings company={id} staff={active} />
  {/if}

  {#if former.length}
    <button type="button" class="section-header-action former-toggle" onclick={() => (showFormer = !showFormer)}>
      {showFormer ? $t("company.team.hide_former") : $t("company.team.show_former", { count: former.length })}
    </button>
    {#if showFormer}
      <div class="group">{#each former as s (s.id)}{@render row(s)}{/each}</div>
    {/if}
  {/if}
</div>

{#snippet row(s: Staff)}
  {@const run = lastRun.get(s.id)}
  {@const open = openId === s.id}
  <div class="staff-row" class:open id="staff-{s.id}">
    <button type="button" class="staff-head" aria-expanded={open} onclick={() => (openId = open ? null : s.id)}>
      <span class="avatar" style:background={avatarColor(s.name)} style:color="#fff">{initials(s.name)}</span>
      <span class="who">
        <span class="nm">{s.name}</span>
        <span class="sub">{s.title || s.role}{#if s.reports_to && byId.get(s.reports_to)} · {$t("company.team.reports_to", { name: byId.get(s.reports_to)?.name ?? "" })}{/if}</span>
      </span>
      <StatusDot status={s.status} label />
      <span class="col">
        <span class="col-label">{$t("company.team.next_heartbeat")}</span>
        <span class="col-val">{s.heartbeat_cron ? relTime(s.heartbeat_next_ms, $t, now) : $t("company.team.on_demand")}</span>
      </span>
      <span class="col">
        <span class="col-label">{$t("company.team.last_run")}</span>
        <span class="col-val">
          {#if run}<StatusDot status={run.state} kind="run" size={6} /> {relTime(run.created_ms, $t, now)}{:else}{$t("company.common.never")}{/if}
        </span>
      </span>
      <span class="chevron" aria-hidden="true">{open ? "⌄" : "›"}</span>
    </button>
    {#if open}
      <div class="staff-detail">
        {#if s.status === "pending_approval"}
          <p class="notice-line">{$t("company.team.pending_hint")} <a href="/llm/company/{id}/inbox">{$t("company.team.open_inbox")}</a></p>
        {/if}
        {#if s.pause_reason}<p class="notice-line">{s.pause_reason}</p>{/if}
        <dl>
          <dt>{$t("company.team.form.runtime")}</dt><dd class="mono">{runtimeLabel(s)}</dd>
          {#if s.capabilities}<dt>{$t("company.team.form.responsibilities")}</dt><dd>{s.capabilities}</dd>{/if}
          {#if s.instructions}<dt>{$t("company.team.form.instructions")}</dt><dd class="clamp">{s.instructions}</dd>{/if}
          <dt>{$t("company.team.form.heartbeat")}</dt><dd class="mono">{s.heartbeat_cron ?? $t("company.team.on_demand")}{s.heartbeat_cron ? ` (${s.heartbeat_tz})` : ""}</dd>
          <dt>{$t("company.team.form.max_concurrent")}</dt><dd>{s.max_concurrent}</dd>
          <dt>{$t("company.team.form.can_hire")}</dt><dd>{s.can_hire ? $t("company.team.yes") : $t("company.team.no")}</dd>
          {#if run}
            <dt>{$t("company.team.last_run")}</dt>
            <dd>
              {$t(`company.common.run_state.${run.state}`)} · US$ {microsToUsd(run.cost_micros)}{#if run.summary} · {run.summary}{/if}
              {#if run.job_id}<span class="mono dim"> · job {run.job_id}</span>{/if}
            </dd>
          {/if}
        </dl>
        {#if s.status !== "terminated"}
          <div class="actions">
            <button type="button" class="button active" disabled={busyId === s.id || s.status === "paused" || s.status === "pending_approval"} onclick={() => wake(s)}>{$t("company.team.run_now")}</button>
            <button type="button" class="button" onclick={() => (editing = s.id)}>{$t("company.common.edit")}</button>
            {#if s.status === "paused"}
              <button type="button" class="button" disabled={busyId === s.id} onclick={() => setStatus(s, "resume")}>{$t("company.team.resume")}</button>
            {:else if s.status !== "pending_approval"}
              <button type="button" class="button" disabled={busyId === s.id} onclick={() => setStatus(s, "pause")}>{$t("company.team.pause")}</button>
            {/if}
            <button type="button" class="button danger" disabled={busyId === s.id} onclick={() => setStatus(s, "terminate")}>{$t("company.team.terminate")}</button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

{#if editing}
  <div class="dialog-backdrop" role="presentation" onkeydown={(e) => e.key === "Escape" && (editing = null)}>
    <div class="dialog-card form-card" role="dialog" aria-modal="true" aria-labelledby="staff-form-title">
      <h2 class="dialog-title" id="staff-form-title">
        {editingStaff ? $t("company.team.edit_title", { name: editingStaff.name }) : $t("company.team.hire_title")}
      </h2>
      {#key editing}
        <StaffForm
          company={id}
          staff={editingStaff}
          all={staff}
          {templates}
          oncancel={() => (editing = null)}
          ondone={(s) => {
            editing = null;
            openId = s.id;
            void load(id);
          }}
        />
      {/key}
    </div>
  </div>
{/if}

<style>
  .team { display: flex; flex-direction: column; gap: var(--space-4); }
  .team .page-head { margin-bottom: 0; }
  .team .page-title { font-size: var(--text-lg); }
  .chart-wrap { padding: var(--space-4) var(--space-3); }
  .former-toggle { align-self: flex-start; }
  .staff-row + .staff-row { border-top: var(--hairline) solid var(--separator); }
  .staff-row.open { background: var(--fill-1); }
  .staff-head { display: flex; align-items: center; gap: var(--space-3); width: 100%; padding: var(--space-2) var(--space-3); background: none; border: 0; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  @media (hover: hover) { .staff-head:hover { background: var(--fill-1); } }
  .staff-head:focus-visible { outline: var(--focus-ring); outline-offset: -2px; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .nm { font-weight: 600; font-size: var(--text-sm); }
  .sub { font-size: var(--text-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .col { display: flex; flex-direction: column; min-width: 96px; flex-shrink: 0; }
  .col-label { font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-faint); }
  .col-val { font-size: var(--text-xs); color: var(--text-muted); font-variant-numeric: tabular-nums; display: inline-flex; align-items: center; gap: 4px; }
  .staff-detail { padding: 0 var(--space-3) var(--space-3) calc(var(--space-3) + 28px + var(--space-3)); display: flex; flex-direction: column; gap: var(--space-3); }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 4px var(--space-4); margin: 0; font-size: var(--text-sm); }
  dt { color: var(--text-dim); }
  dd { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  .clamp { display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); }
  .dim { color: var(--text-dim); }
  .notice-line { margin: 0; font-size: var(--text-sm); color: var(--warning); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .button.danger { color: var(--red); }
  .form-card { width: min(640px, 100%); max-height: calc(100vh - 64px); overflow-y: auto; }
  @media (max-width: 720px) { .col { display: none; } }
</style>
