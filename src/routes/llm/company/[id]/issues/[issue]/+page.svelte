<script lang="ts">
  /**
   * One issue: editable title and body, status bar with the valid moves,
   * properties, comments, documents, deliverables, relations, runs and the
   * subtree controls.
   */
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    rpc,
    errorText,
    TRANSITIONS,
    type Issue,
    type IssueDetail,
    type IssueStatus,
    type Label,
    type Project,
    type Run,
    type Staff,
  } from "$lib/company/rpc";
  import IssueStatusBadge from "$components/company/IssueStatusBadge.svelte";
  import IssuePriority from "$components/company/IssuePriority.svelte";
  import IssueMarkdown from "$components/company/IssueMarkdown.svelte";
  import CommentThread from "$components/company/CommentThread.svelte";
  import DocEditor from "$components/company/DocEditor.svelte";
  import IssueWorkProducts from "$components/company/IssueWorkProducts.svelte";
  import IssueRelations from "$components/company/IssueRelations.svelte";
  import IssueCreateDialog from "$components/company/IssueCreateDialog.svelte";
  import BlockedReasonDialog from "$components/company/BlockedReasonDialog.svelte";
  import RunRow from "$components/company/RunRow.svelte";
  import { PRIORITIES, issueStatusKey, priorityKey, watchCompany } from "$components/company/company-format";
  import { relTime } from "$components/company/company-ui";

  let company = $derived(page.params.id ?? "");
  let ref = $derived(page.params.issue ?? "");

  let detail = $state<IssueDetail | null>(null);
  let staff = $state<Staff[]>([]);
  let projects = $state<Project[]>([]);
  let labels = $state<Label[]>([]);
  let issues = $state<Issue[]>([]);
  let runs = $state<Run[]>([]);
  let loadError = $state("");

  let editing = $state(false);
  let draftTitle = $state("");
  let draftBody = $state("");
  let editSeq = $state<number | null>(null);
  let saving = $state(false);
  let askBlocked = $state(false);
  let subtask = $state(false);
  let labelText = $state("");
  let labelsFocused = $state(false);

  let issue = $derived(detail?.issue ?? null);
  let base = $derived(`/llm/company/${encodeURIComponent(company)}`);

  async function load() {
    const c = company;
    const r = ref;
    if (!c || !r) return;
    try {
      const d = await rpc<IssueDetail>("issue.get", { company: c, issue: r });
      const [st, pr, lb, all, rn] = await Promise.all([
        rpc<Staff[]>("staff.list", { company: c }),
        rpc<Project[]>("project.list", { company: c }),
        rpc<Label[]>("label.list", { company: c }),
        rpc<Issue[]>("issue.list", { company: c, filter: { include_closed: true, limit: 2000 } }),
        rpc<Run[]>("run.list", { company: c, issue: d.issue.id, limit: 50 }),
      ]);
      if (c !== company || r !== ref) return;
      detail = d;
      staff = st;
      projects = pr;
      labels = lb;
      issues = all;
      runs = rn;
      if (!labelsFocused) labelText = d.labels.map((l) => l.name).join(", ");
      loadError = "";
    } catch (e) {
      loadError = errorText(e);
    }
  }

  $effect(() => {
    void [company, ref];
    editing = false;
    void load();
  });

  $effect(() => watchCompany(company, () => void load()));

  function hrefOf(identifier: string): string {
    return `${base}/issues/${encodeURIComponent(identifier)}`;
  }

  function startEdit() {
    if (!issue) return;
    draftTitle = issue.title;
    draftBody = issue.body;
    editSeq = issue.seq;
    editing = true;
  }

  async function patch(p: Record<string, unknown>, expected: number | null = issue?.seq ?? null) {
    if (!issue) return false;
    try {
      await rpc("issue.update", { company, issue: issue.id, patch: { ...p, expected_seq: expected } });
      return true;
    } catch (e) {
      showToast("error", errorText(e));
      void load();
      return false;
    }
  }

  async function saveText() {
    if (!draftTitle.trim()) return;
    saving = true;
    const ok = await patch({ title: draftTitle.trim(), body: draftBody }, editSeq);
    saving = false;
    if (ok) editing = false;
  }

  async function move(to: IssueStatus, note?: string) {
    if (!issue) return;
    if (to === "blocked" && note === undefined) {
      askBlocked = true;
      return;
    }
    try {
      await rpc("issue.status", { company, issue: issue.id, status: to, note: note ?? null, expected_seq: issue.seq });
    } catch (e) {
      showToast("error", errorText(e));
      void load();
    }
  }

  function assigneeValue(i: Issue): string {
    return i.assignee_user ? "me" : i.assignee_staff_id ?? "";
  }

  function setAssignee(v: string) {
    if (v === "me") void patch({ assignee_user: true, assignee_staff_id: null });
    else void patch({ assignee_user: false, assignee_staff_id: v || null });
  }

  async function saveLabels() {
    if (!issue) return;
    const names = labelText
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      await rpc("issue.labels", { company, issue: issue.id, labels: names });
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  async function tree(action: "hold" | "release" | "cancel") {
    if (!issue) return;
    if (action === "cancel" && !confirm($t("company.issue.tree_cancel_confirm", { identifier: issue.identifier }))) return;
    try {
      const r = await rpc<{ changed: number }>("issue.tree", { company, issue: issue.id, action });
      showToast("success", $t("company.issue.tree_done", { count: r.changed }));
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  async function wake() {
    if (!issue?.assignee_staff_id) return;
    try {
      await rpc("staff.wake", { company, staff: issue.assignee_staff_id, issue: issue.id, reason: "woken by the owner" });
      showToast("success", $t("company.issue.woken"));
    } catch (e) {
      showToast("error", errorText(e));
    }
  }
</script>

<svelte:head><title>{issue ? `${issue.identifier} · ${issue.title}` : $t("company.issues.title")}</title></svelte:head>

{#if loadError && !detail}
  <div class="empty-state">
    <img class="empty-state-art" src="/emoji/paperclip.png" alt="" width="72" height="72" draggable="false" />
    <p class="empty-state-title">{$t("company.issue.not_found")}</p>
    <p class="empty-state-hint">{loadError}</p>
    <a class="btn btn-secondary" href={`${base}/issues`}>{$t("company.issue.back")}</a>
  </div>
{:else if !detail || !issue}
  <div class="skeleton" style="height: 160px"></div>
{:else}
  <div class="issue-page">
    <div class="main">
      <nav class="crumbs">
        <a href={`${base}/issues`}>{$t("company.issues.title")}</a>
        <span aria-hidden="true">›</span>
        <span class="mono">{issue.identifier}</span>
      </nav>

      {#if editing}
        <div class="edit">
          <input class="input input-lg" type="text" bind:value={draftTitle} aria-label={$t("company.issues.field_title")} />
          <textarea class="input body" rows="10" bind:value={draftBody} aria-label={$t("company.issues.field_body")}></textarea>
          <div class="row end">
            <button type="button" class="btn btn-secondary btn-sm" onclick={() => (editing = false)}>{$t("company.common.cancel")}</button>
            <button type="button" class="btn btn-primary btn-sm" disabled={saving || !draftTitle.trim()} onclick={saveText}>{$t("company.common.save")}</button>
          </div>
        </div>
      {:else}
        <header class="title-row">
          <h1 class="title">{issue.title}</h1>
          <button type="button" class="btn btn-ghost btn-sm" onclick={startEdit}>{$t("company.common.edit")}</button>
        </header>
        {#if issue.body.trim()}
          <IssueMarkdown text={issue.body} />
        {:else}
          <button type="button" class="add-body" onclick={startEdit}>{$t("company.issue.add_description")}</button>
        {/if}
      {/if}

      <div class="status-bar">
        <IssueStatusBadge status={issue.status} />
        {#if issue.status === "blocked" && issue.blocked_reason}
          <span class="blocked-reason">{issue.blocked_reason}</span>
        {/if}
        <span class="spacer"></span>
        {#each TRANSITIONS[issue.status] as to (to)}
          <button type="button" class="btn btn-secondary btn-sm" onclick={() => move(to)}>
            <IssueStatusBadge status={to} label={false} />
            {$t(issueStatusKey(to))}
          </button>
        {/each}
      </div>

      <section class="section">
        <h2 class="section-header-title">{$t("company.issue.comments")}</h2>
        <CommentThread {company} issue={issue.id} comments={detail.comments} {staff} />
      </section>

      <section class="section">
        <h2 class="section-header-title">{$t("company.issue.documents")}</h2>
        <DocEditor {company} issue={issue.id} documents={detail.documents} />
      </section>

      <section class="section">
        <h2 class="section-header-title">{$t("company.issue.work_products")}</h2>
        <IssueWorkProducts {company} items={detail.work_products} />
      </section>

      <section class="section">
        <h2 class="section-header-title">{$t("company.issue.relations")}</h2>
        <IssueRelations {company} {detail} {issues} {hrefOf} onsubtask={() => (subtask = true)} />
      </section>

      <section class="section">
        <h2 class="section-header-title">{$t("company.issue.runs")}</h2>
        {#if runs.length === 0}
          <p class="hint">{$t("company.issue.no_runs")}</p>
        {:else}
          <div class="group">
            {#each runs as r (r.id)}
              <RunRow run={r} {staff} />
            {/each}
          </div>
        {/if}
      </section>
    </div>

    <aside class="side">
      <div class="group props">
        <label class="setting-row prop">
          <span class="k">{$t("company.issues.field_assignee")}</span>
          <select class="input" value={assigneeValue(issue)} onchange={(e) => setAssignee(e.currentTarget.value)}>
            <option value="">{$t("company.common.unassigned")}</option>
            <option value="me">{$t("company.common.owner")}</option>
            {#each staff.filter((s) => s.status !== "terminated") as s (s.id)}
              <option value={s.id}>{s.name}</option>
            {/each}
          </select>
        </label>
        <label class="setting-row prop">
          <span class="k">{$t("company.issues.field_priority")}</span>
          <span class="v">
            <IssuePriority priority={issue.priority} />
            <select class="input" value={issue.priority} onchange={(e) => void patch({ priority: e.currentTarget.value })}>
              {#each PRIORITIES as p (p)}<option value={p}>{$t(priorityKey(p))}</option>{/each}
            </select>
          </span>
        </label>
        <label class="setting-row prop">
          <span class="k">{$t("company.issues.field_project")}</span>
          <select class="input" value={issue.project_id ?? ""} onchange={(e) => void patch({ project_id: e.currentTarget.value || null })}>
            <option value="">{$t("company.common.none")}</option>
            {#each projects as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
        <label class="setting-row prop">
          <span class="k">{$t("company.issues.field_mode")}</span>
          <select class="input" value={issue.work_mode} onchange={(e) => void patch({ work_mode: e.currentTarget.value })}>
            {#each ["standard", "ask", "planning"] as m (m)}<option value={m}>{$t(`company.issues.mode.${m}`)}</option>{/each}
          </select>
        </label>
        <label class="setting-row prop">
          <span class="k">{$t("company.issues.field_review")}</span>
          <select class="input" value={issue.review_policy} onchange={(e) => void patch({ review_policy: e.currentTarget.value })}>
            {#each ["none", "anyone", "not_creator", "human_only"] as r (r)}<option value={r}>{$t(`company.issues.review.${r}`)}</option>{/each}
          </select>
        </label>
        <label class="setting-row prop col">
          <span class="k">{$t("company.issues.field_labels")}</span>
          <input
            class="input"
            type="text"
            list="company-issue-labels"
            bind:value={labelText}
            onfocus={() => (labelsFocused = true)}
            onblur={() => (labelsFocused = false)}
            onchange={saveLabels}
            placeholder={$t("company.issues.labels_placeholder")}
          />
          <datalist id="company-issue-labels">
            {#each labels as l (l.id)}<option value={l.name}></option>{/each}
          </datalist>
        </label>
        {#if issue.parent_id}
          {@const parent = issues.find((i) => i.id === issue?.parent_id)}
          {#if parent}
            <div class="setting-row prop">
              <span class="k">{$t("company.issues.field_parent")}</span>
              <a class="mono" href={hrefOf(parent.identifier)}>{parent.identifier}</a>
            </div>
          {/if}
        {/if}
      </div>

      <div class="facts">
        <span>{$t("company.issue.created", { when: relTime(issue.created_ms, $t) })}</span>
        <span>{$t("company.issue.updated", { when: relTime(issue.updated_ms, $t) })}</span>
        {#if issue.lease_run_id}<span class="live">{$t("company.issue.checked_out")}</span>{/if}
      </div>

      {#if issue.assignee_staff_id}
        <button type="button" class="btn btn-primary btn-block" onclick={wake}>{$t("company.issue.wake")}</button>
      {/if}

      <div class="group-label">{$t("company.issue.tree")}</div>
      <div class="tree">
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => tree("hold")}>{$t("company.issue.tree_hold")}</button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => tree("release")}>{$t("company.issue.tree_release")}</button>
        <button type="button" class="btn btn-destructive btn-sm" onclick={() => tree("cancel")}>{$t("company.issue.tree_cancel")}</button>
      </div>
      <p class="hint">{$t("company.issue.tree_hint")}</p>
    </aside>
  </div>
{/if}

{#if askBlocked && issue}
  <BlockedReasonDialog
    identifier={issue.identifier}
    oncancel={() => (askBlocked = false)}
    onconfirm={(note) => {
      askBlocked = false;
      void move("blocked", note);
    }}
  />
{/if}

{#if subtask && issue}
  <IssueCreateDialog
    {company}
    {staff}
    {projects}
    {issues}
    {labels}
    parentId={issue.id}
    projectId={issue.project_id}
    onclose={() => (subtask = false)}
    oncreated={(i) => {
      subtask = false;
      void goto(hrefOf(i.identifier));
    }}
  />
{/if}

<style>
  .issue-page {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 280px;
    gap: var(--space-5);
    align-items: start;
  }
  @media (max-width: 860px) {
    .issue-page {
      grid-template-columns: 1fr;
    }
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }
  .crumbs {
    display: flex;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .crumbs a {
    color: var(--text-muted);
    text-decoration: none;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .title-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }
  .title {
    flex: 1;
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    line-height: var(--leading-xl);
    letter-spacing: var(--track-tight);
    overflow-wrap: anywhere;
    user-select: text;
  }
  .add-body {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-dim);
    font: inherit;
    cursor: text;
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .body {
    height: auto;
    resize: vertical;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .end {
    justify-content: flex-end;
  }
  .status-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
  }
  .blocked-reason {
    font-size: var(--text-sm);
    color: var(--red);
  }
  .spacer {
    flex: 1;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    position: sticky;
    top: 0;
  }
  .prop {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
  }
  .prop.col {
    flex-direction: column;
    align-items: stretch;
  }
  .prop .input {
    width: 150px;
  }
  .prop.col .input {
    width: 100%;
  }
  .k {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .v {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  .v .input {
    width: 124px;
  }
  .facts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    padding: 0 var(--space-2);
  }
  .live {
    color: var(--blue);
  }
  .tree {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
