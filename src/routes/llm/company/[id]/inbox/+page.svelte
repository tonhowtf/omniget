<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    rpc,
    errorText,
    type Approval,
    type Incident,
    type Issue,
    type PolicyStatus,
    type Project,
    type Staff,
  } from "$lib/company/rpc";
  import ApprovalCard from "$components/company/ApprovalCard.svelte";
  import BudgetIncidentCard from "$components/company/BudgetIncidentCard.svelte";
  import IssuePriority from "$components/company/IssuePriority.svelte";
  import IssueStatusBadge from "$components/company/IssueStatusBadge.svelte";
  import { staffName, watchCompany } from "$components/company/company-format";
  import { relTime } from "$components/company/company-ui";

  type InboxItem = { issue: Issue; why: "review" | "blocked" | "assigned" | "mentioned"; read: boolean };
  type Inbox = { issues: InboxItem[]; approvals: Approval[]; incidents: Incident[] };

  let company = $derived(page.params.id ?? "");
  let inbox = $state<Inbox | null>(null);
  let budgets = $state<PolicyStatus[]>([]);
  let staff = $state<Staff[]>([]);
  let projects = $state<Project[]>([]);
  let loadError = $state("");

  async function load() {
    const c = company;
    if (!c) return;
    try {
      const [ib, bs, st, pr] = await Promise.all([
        rpc<Inbox>("inbox.list", { company: c }),
        rpc<PolicyStatus[]>("budget.status", { company: c }),
        rpc<Staff[]>("staff.list", { company: c }),
        rpc<Project[]>("project.list", { company: c }),
      ]);
      if (c !== company) return;
      inbox = ib;
      budgets = bs;
      staff = st;
      projects = pr;
      loadError = "";
    } catch (e) {
      loadError = errorText(e);
    }
  }

  $effect(() => {
    void company;
    void load();
  });
  $effect(() => watchCompany(company, () => void load()));

  let approvals = $derived(inbox?.approvals ?? []);
  let incidents = $derived(inbox?.incidents ?? []);
  let items = $derived(inbox?.issues ?? []);
  let unread = $derived(items.filter((i) => !i.read).length);
  let total = $derived(approvals.length + incidents.length + items.length);

  function statusOf(policyId: string): PolicyStatus | null {
    return budgets.find((b) => b.policy.id === policyId) ?? null;
  }

  function href(i: Issue): string {
    return `/llm/company/${encodeURIComponent(company)}/issues/${encodeURIComponent(i.identifier)}`;
  }

  async function mark(i: Issue, action: "read" | "unread" | "archive" | "snooze", untilMs?: number) {
    try {
      await rpc("inbox.mark", { company, issue: i.id, action, until_ms: untilMs ?? null });
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  function snoozeUntil(kind: string): number {
    const d = new Date();
    if (kind === "hours") return Date.now() + 3 * 3600_000;
    d.setHours(9, 0, 0, 0);
    d.setDate(d.getDate() + (kind === "week" ? 7 : 1));
    return d.getTime();
  }
</script>

<svelte:head><title>{$t("company.inbox.title")}</title></svelte:head>

<div class="inbox-page">
  {#if loadError}<p class="field-error" role="alert">{loadError}</p>{/if}

  {#if !inbox}
    <div class="skeleton" style="height: 120px"></div>
  {:else if total === 0}
    <div class="empty-state">
      <img class="empty-state-art" src="/emoji/sparkles.png" alt="" width="72" height="72" draggable="false" />
      <p class="empty-state-title">{$t("company.inbox.empty_title")}</p>
      <p class="empty-state-hint">{$t("company.inbox.empty_hint")}</p>
    </div>
  {:else}
    {#if approvals.length > 0}
      <h2 class="group-label">{$t("company.inbox.approvals", { count: approvals.length })}</h2>
      <div class="cards">
        {#each approvals as a (a.id)}
          <ApprovalCard approval={a} {staff} />
        {/each}
      </div>
    {/if}

    {#if incidents.length > 0}
      <h2 class="group-label">{$t("company.inbox.incidents", { count: incidents.length })}</h2>
      <div class="cards">
        {#each incidents as inc (inc.id)}
          <BudgetIncidentCard incident={inc} status={statusOf(inc.policy_id)} {staff} {projects} />
        {/each}
      </div>
    {/if}

    {#if items.length > 0}
      <h2 class="group-label">
        {$t("company.inbox.issues", { count: items.length })}
        {#if unread > 0}<span class="badge">{unread}</span>{/if}
      </h2>
      <div class="group">
        {#each items as it (it.issue.id)}
          <div class="list-row compact item" class:unread={!it.read}>
            <span class="dot" aria-hidden="true"></span>
            <span class="why why-{it.why}">{$t(`company.inbox.why.${it.why}`)}</span>
            <IssuePriority priority={it.issue.priority} />
            <IssueStatusBadge status={it.issue.status} label={false} />
            <a class="ident" href={href(it.issue)} onclick={() => !it.read && void mark(it.issue, "read")}>{it.issue.identifier}</a>
            <a class="title" href={href(it.issue)} onclick={() => !it.read && void mark(it.issue, "read")}>{it.issue.title}</a>
            {#if it.issue.assignee_staff_id}<span class="dim">{staffName(staff, it.issue.assignee_staff_id)}</span>{/if}
            <span class="dim when">{relTime(it.issue.updated_ms, $t)}</span>
            <div class="acts">
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => mark(it.issue, it.read ? "unread" : "read")}>
                {it.read ? $t("company.inbox.mark_unread") : $t("company.inbox.mark_read")}
              </button>
              <select
                class="input snooze"
                aria-label={$t("company.inbox.snooze")}
                onchange={(e) => {
                  const v = e.currentTarget.value;
                  e.currentTarget.value = "";
                  if (v) void mark(it.issue, "snooze", snoozeUntil(v));
                }}
              >
                <option value="">{$t("company.inbox.snooze")}</option>
                <option value="hours">{$t("company.inbox.snooze_hours")}</option>
                <option value="tomorrow">{$t("company.inbox.snooze_tomorrow")}</option>
                <option value="week">{$t("company.inbox.snooze_week")}</option>
              </select>
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => mark(it.issue, "archive")}>{$t("company.inbox.archive")}</button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .inbox-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 920px;
  }
  .group-label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .item {
    border-radius: 0;
    gap: var(--space-2);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    background: transparent;
  }
  .unread .dot {
    background: var(--accent);
  }
  .unread .title {
    font-weight: 600;
  }
  .why {
    flex-shrink: 0;
    padding: 0 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-xs);
    color: var(--text-muted);
    background: var(--fill-2);
  }
  .why-review {
    color: var(--purple);
    background: color-mix(in srgb, var(--purple) 14%, transparent);
  }
  .why-blocked {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 14%, transparent);
  }
  .why-assigned {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 14%, transparent);
  }
  .ident {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-decoration: none;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    text-decoration: none;
  }
  .dim {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .when {
    font-variant-numeric: tabular-nums;
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease-out);
  }
  .item:hover .acts,
  .item:focus-within .acts {
    opacity: 1;
  }
  .snooze {
    width: auto;
    height: 24px;
    font-size: var(--text-xs);
  }
</style>
