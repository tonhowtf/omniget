<script lang="ts">
  /**
   * A company's issues: a list (grouped by status or flat) and a kanban board,
   * with filters, debounced search and the new-issue form (`c`).
   */
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    rpc,
    errorText,
    type Issue,
    type IssueStatus,
    type Label,
    type Project,
    type Staff,
  } from "$lib/company/rpc";
  import IssueRow from "$components/company/IssueRow.svelte";
  import IssueBoard from "$components/company/IssueBoard.svelte";
  import IssueStatusBadge from "$components/company/IssueStatusBadge.svelte";
  import IssueCreateDialog from "$components/company/IssueCreateDialog.svelte";
  import { LIST_ORDER, PRIORITIES, priorityKey, watchCompany } from "$components/company/company-format";

  type SearchHit = { issue: Issue; score: number };
  const VIEW_KEY = "omniget.company.issues.view";

  let company = $derived(page.params.id ?? "");

  let issues = $state<Issue[]>([]);
  let staff = $state<Staff[]>([]);
  let projects = $state<Project[]>([]);
  let labels = $state<Label[]>([]);
  /** issue id → its labels (names come with `issue.list`). */
  let labelMap = $state<Record<string, Label[]>>({});
  let loaded = $state(false);
  let loadError = $state("");

  let view = $state<"list" | "board">(readView());
  let grouped = $state(true);
  let fAssignee = $state("");
  let fProject = $state("");
  let fLabel = $state("");
  let fPriority = $state("");
  let includeDone = $state(false);
  let query = $state("");
  let hits = $state<Issue[] | null>(null);
  let creating = $state(false);

  function readView(): "list" | "board" {
    try {
      return localStorage.getItem(VIEW_KEY) === "board" ? "board" : "list";
    } catch {
      return "list";
    }
  }

  function setView(v: "list" | "board") {
    view = v;
    try {
      localStorage.setItem(VIEW_KEY, v);
    } catch {
      /* private window: the choice just is not remembered */
    }
  }

  async function load() {
    const c = company;
    if (!c) return;
    try {
      const filter: Record<string, unknown> = { include_closed: includeDone, limit: 2000 };
      if (fAssignee === "me") filter.assignee_user = true;
      else if (fAssignee) filter.assignee_staff_id = fAssignee;
      if (fProject) filter.project_id = fProject;
      if (fLabel) filter.label = fLabel;
      const [list, st, pr, lb] = await Promise.all([
        rpc<Issue[]>("issue.list", { company: c, filter }),
        rpc<Staff[]>("staff.list", { company: c }),
        rpc<Project[]>("project.list", { company: c }),
        rpc<Label[]>("label.list", { company: c }),
      ]);
      const byName = new Map(lb.map((l) => [l.name.toLowerCase(), l]));
      const map: Record<string, Label[]> = {};
      for (const i of list) {
        const ls = (i.labels ?? []).map((n) => byName.get(n.toLowerCase())).filter((l): l is Label => !!l);
        if (ls.length) map[i.id] = ls;
      }
      if (c !== company) return;
      issues = list;
      staff = st;
      projects = pr;
      labels = lb;
      labelMap = map;
      loadError = "";
    } catch (e) {
      loadError = errorText(e);
    } finally {
      loaded = true;
    }
  }

  // Reload when the company or a server-side filter changes.
  $effect(() => {
    void [company, fAssignee, fProject, fLabel, includeDone];
    void load();
  });

  // Debounced full-text search.
  $effect(() => {
    const q = query.trim();
    const c = company;
    if (!q) {
      hits = null;
      return;
    }
    const timer = setTimeout(async () => {
      try {
        const res = await rpc<SearchHit[]>("issue.search", { company: c, query: q, limit: 100 });
        if (query.trim() === q) hits = res.map((h) => h.issue);
      } catch (e) {
        showToast("error", errorText(e));
      }
    }, 220);
    return () => clearTimeout(timer);
  });

  $effect(() =>
    watchCompany(company, (batch) => {
      if (batch.some((c) => ["issue", "label", "staff", "project"].includes(c.entity))) void load();
    }),
  );

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "c" || e.metaKey || e.ctrlKey || e.altKey || creating) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName))) return;
      e.preventDefault();
      creating = true;
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  let shown = $derived.by(() => {
    let xs = hits ?? issues;
    if (hits) {
      xs = xs.filter((i) => {
        if (!includeDone && (i.status === "done" || i.status === "cancelled")) return false;
        if (fAssignee === "me" && !i.assignee_user) return false;
        if (fAssignee && fAssignee !== "me" && i.assignee_staff_id !== fAssignee) return false;
        if (fProject && i.project_id !== fProject) return false;
        if (fLabel && !(labelMap[i.id] ?? []).some((l) => l.name === fLabel)) return false;
        return true;
      });
    }
    if (fPriority) xs = xs.filter((i) => i.priority === fPriority);
    return xs;
  });

  let groups = $derived(
    LIST_ORDER.map((s) => ({ status: s, items: shown.filter((i) => i.status === s) })).filter((g) => g.items.length > 0),
  );

  let filtered = $derived(!!(fAssignee || fProject || fLabel || fPriority || query.trim()));

  function hrefOf(i: Issue): string {
    return `/llm/company/${encodeURIComponent(company)}/issues/${encodeURIComponent(i.identifier)}`;
  }

  function labelsOf(id: string): Label[] {
    return labelMap[id] ?? [];
  }

  async function move(issue: Issue, to: IssueStatus, note?: string): Promise<boolean> {
    try {
      await rpc("issue.status", { company, issue: issue.id, status: to, note: note ?? null, expected_seq: issue.seq });
      return true;
    } catch (e) {
      showToast("error", errorText(e));
      void load();
      return false;
    }
  }

  function clearFilters() {
    fAssignee = "";
    fProject = "";
    fLabel = "";
    fPriority = "";
    query = "";
  }
</script>

<svelte:head><title>{$t("company.issues.title")}</title></svelte:head>

<div class="issues-page">
  <div class="bar">
    <div class="segmented" role="tablist" aria-label={$t("company.issues.view")}>
      <button type="button" class="segmented-btn" role="tab" aria-selected={view === "list"} onclick={() => setView("list")}>
        {$t("company.issues.view_list")}
      </button>
      <button type="button" class="segmented-btn" role="tab" aria-selected={view === "board"} onclick={() => setView("board")}>
        {$t("company.issues.view_board")}
      </button>
    </div>
    <input
      class="input input-search search"
      type="search"
      bind:value={query}
      placeholder={$t("company.issues.search_placeholder")}
      aria-label={$t("company.issues.search")}
    />
    <button type="button" class="btn btn-primary" onclick={() => (creating = true)} title={$t("company.issues.new_shortcut")}>
      {$t("company.issues.new")}
      <kbd class="kbd">C</kbd>
    </button>
  </div>

  <div class="filters">
    <select class="input" bind:value={fAssignee} aria-label={$t("company.issues.field_assignee")}>
      <option value="">{$t("company.issues.any_assignee")}</option>
      <option value="me">{$t("company.common.owner")}</option>
      {#each staff as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
    </select>
    <select class="input" bind:value={fProject} aria-label={$t("company.issues.field_project")}>
      <option value="">{$t("company.issues.any_project")}</option>
      {#each projects as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
    </select>
    {#if labels.length > 0}
      <select class="input" bind:value={fLabel} aria-label={$t("company.issues.field_labels")}>
        <option value="">{$t("company.issues.any_label")}</option>
        {#each labels as l (l.id)}<option value={l.name}>{l.name}</option>{/each}
      </select>
    {/if}
    <select class="input" bind:value={fPriority} aria-label={$t("company.issues.field_priority")}>
      <option value="">{$t("company.issues.any_priority")}</option>
      {#each PRIORITIES as p (p)}<option value={p}>{$t(priorityKey(p))}</option>{/each}
    </select>
    <label class="check">
      <input type="checkbox" class="checkbox" bind:checked={includeDone} />
      {$t("company.issues.include_done")}
    </label>
    {#if view === "list"}
      <label class="check">
        <input type="checkbox" class="checkbox" bind:checked={grouped} />
        {$t("company.issues.group_by_status")}
      </label>
    {/if}
    {#if filtered}
      <button type="button" class="btn btn-ghost btn-sm" onclick={clearFilters}>{$t("company.issues.clear_filters")}</button>
    {/if}
    <span class="total">{$t("company.issues.count", { count: shown.length })}</span>
  </div>

  {#if loadError}
    <p class="field-error" role="alert">{loadError}</p>
  {/if}

  {#if !loaded}
    <div class="skeleton" style="height: 120px"></div>
  {:else if shown.length === 0 && view === "list"}
    <div class="empty-state">
      <img class="empty-state-art" src="/emoji/paperclip.png" alt="" width="72" height="72" draggable="false" />
      {#if filtered}
        <p class="empty-state-title">{$t("company.issues.empty_filtered")}</p>
        <button type="button" class="btn btn-secondary" onclick={clearFilters}>{$t("company.issues.clear_filters")}</button>
      {:else}
        <p class="empty-state-title">{$t("company.issues.empty_title")}</p>
        <p class="empty-state-hint">{$t("company.issues.empty_hint")}</p>
        <button type="button" class="btn btn-primary" onclick={() => (creating = true)}>{$t("company.issues.new")}</button>
      {/if}
    </div>
  {:else if view === "board"}
    <IssueBoard issues={shown} {staff} {labelsOf} {hrefOf} onmove={move} />
  {:else if grouped}
    {#each groups as g (g.status)}
      <h3 class="group-label head"><IssueStatusBadge status={g.status} /> <span class="n">{g.items.length}</span></h3>
      <div class="group">
        {#each g.items as issue (issue.id)}
          <IssueRow {issue} href={hrefOf(issue)} {staff} labels={labelsOf(issue.id)} />
        {/each}
      </div>
    {/each}
  {:else}
    <div class="group">
      {#each shown as issue (issue.id)}
        <IssueRow {issue} href={hrefOf(issue)} {staff} labels={labelsOf(issue.id)} />
      {/each}
    </div>
  {/if}
</div>

{#if creating}
  <IssueCreateDialog
    {company}
    {staff}
    {projects}
    {issues}
    {labels}
    onclose={() => (creating = false)}
    oncreated={(i) => {
      creating = false;
      showToast("success", $t("company.issues.created", { identifier: i.identifier }));
      void load();
    }}
  />
{/if}

<style>
  .issues-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .bar,
  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .search {
    flex: 1;
    min-width: 180px;
  }
  .filters .input {
    width: auto;
    max-width: 180px;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .total {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }
  .n {
    font-weight: 400;
    font-variant-numeric: tabular-nums;
  }
  .kbd {
    margin-left: 4px;
    opacity: 0.7;
  }
</style>
