<script lang="ts">
  /**
   * Goals and projects. Goals form a tree under the company mission (team →
   * agent → task); projects group work with a lead, a folder and an
   * isolation mode (shared folder or a git worktree per task).
   */
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { errorText, onCompanyChange, rpc, type Company, type Goal, type Project, type Staff } from "$lib/company/rpc";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { pickFolder } from "$lib/stores/llm-jobs-store.svelte";
  import StatusDot from "$components/company/StatusDot.svelte";
  import { statusTone } from "$components/company/company-ui";

  type GoalNode = { goal: Goal; children: GoalNode[] };
  const LEVELS = ["company", "team", "agent", "task"] as const;
  const RANK: Record<string, number> = { company: 0, team: 1, agent: 2, task: 3 };
  const GOAL_STATUSES = ["planned", "active", "achieved", "cancelled"] as const;
  const PROJECT_STATUSES = ["backlog", "planned", "in_progress", "completed", "cancelled"] as const;
  const COLORS = ["#0a84ff", "#30d158", "#ff9f0a", "#ff453a", "#bf5af2", "#40c8e0", "#ffd60a", "#8e8e93"];

  let id = $derived(page.params.id ?? "");
  let company = $state<Company | null>(null);
  let goals = $state<Goal[]>([]);
  let projects = $state<Project[]>([]);
  let staff = $state<Staff[]>([]);
  let loaded = $state(false);
  let error = $state("");

  async function load(cid: string) {
    try {
      const [c, g, p, s] = await Promise.all([
        rpc<Company>("company.get", { company: cid }),
        rpc<Goal[]>("goal.list", { company: cid }),
        rpc<Project[]>("project.list", { company: cid }),
        rpc<Staff[]>("staff.list", { company: cid }),
      ]);
      company = c;
      goals = g;
      projects = p;
      staff = s;
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
    const un = onCompanyChange(cid, () => void load(cid));
    return () => void un.then((f) => f());
  });

  let alive = $derived(staff.filter((s) => s.status !== "terminated"));
  let staffName = $derived(new Map(staff.map((s) => [s.id, s.name])));
  let goalTitle = $derived(new Map(goals.map((g) => [g.id, g.title])));
  let tree = $derived.by(() => {
    const ids = new Set(goals.map((g) => g.id));
    const kids = new Map<string | null, Goal[]>();
    for (const g of goals) {
      const p = g.parent_id && ids.has(g.parent_id) ? g.parent_id : null;
      kids.set(p, [...(kids.get(p) ?? []), g]);
    }
    const build = (g: Goal): GoalNode => ({ goal: g, children: (kids.get(g.id) ?? []).map(build) });
    return (kids.get(null) ?? []).sort((a, b) => RANK[a.level] - RANK[b.level]).map(build);
  });

  let subGoals = $derived(goals.filter((g) => !(g.level === "company" && !g.parent_id)).length);

  // ── Goal form ──
  let goalForm = $state<null | { id: string | null; title: string; body: string; level: string; parent: string; owner: string; status: string }>(null);
  let goalBusy = $state(false);
  let goalError = $state("");

  function newGoal(parent: Goal | null) {
    const level = parent ? LEVELS[Math.min(3, RANK[parent.level] + 1)] : "team";
    goalForm = { id: null, title: "", body: "", level, parent: parent?.id ?? "", owner: "", status: "active" };
    goalError = "";
  }

  function editGoal(g: Goal) {
    goalForm = { id: g.id, title: g.title, body: g.body, level: g.level, parent: g.parent_id ?? "", owner: g.owner_staff_id ?? "", status: g.status };
    goalError = "";
  }

  let parentOptions = $derived(goalForm ? goals.filter((g) => g.id !== goalForm?.id && RANK[g.level] <= RANK[goalForm?.level ?? "task"]) : []);

  async function saveGoal(e: Event) {
    e.preventDefault();
    const f = goalForm;
    if (!f || !f.title.trim()) return;
    goalBusy = true;
    goalError = "";
    try {
      if (f.id) {
        await rpc("goal.update", {
          company: id,
          goal_id: f.id,
          patch: { title: f.title.trim(), body: f.body, status: f.status, owner_staff_id: f.owner || null, parent_id: f.parent || null },
        });
      } else {
        await rpc("goal.create", {
          company: id,
          goal: { level: f.level, parent_id: f.parent || null, owner_staff_id: f.owner || null, title: f.title.trim(), body: f.body, status: f.status },
        });
      }
      goalForm = null;
      void load(id);
    } catch (err) {
      goalError = errorText(err);
    } finally {
      goalBusy = false;
    }
  }

  async function goalAction(fn: () => Promise<unknown>) {
    try {
      await fn();
      void load(id);
    } catch (e) {
      showToast("error", errorText(e));
    }
  }

  function achieve(g: Goal) {
    void goalAction(() => rpc("goal.update", { company: id, goal_id: g.id, patch: { status: "achieved" } }));
  }

  function removeGoal(g: Goal) {
    if (!confirm($t("company.goals.delete_confirm", { title: g.title }) as string)) return;
    void goalAction(() => rpc("goal.delete", { company: id, goal_id: g.id }));
  }

  // ── Project form ──
  type ProjectDraft = {
    id: string | null; name: string; description: string; goal: string; lead: string;
    workspace: string | null; isolation: "shared" | "worktree"; color: string; status: string;
  };
  let projectForm = $state<ProjectDraft | null>(null);
  let projectBusy = $state(false);
  let projectError = $state("");

  function newProject() {
    projectForm = { id: null, name: "", description: "", goal: "", lead: "", workspace: null, isolation: "shared", color: COLORS[projects.length % COLORS.length], status: "planned" };
    projectError = "";
  }

  function editProject(p: Project) {
    projectForm = {
      id: p.id, name: p.name, description: p.description, goal: p.goal_id ?? "", lead: p.lead_staff_id ?? "",
      workspace: p.workspace, isolation: p.isolation, color: p.color ?? COLORS[0], status: p.status,
    };
    projectError = "";
  }

  async function saveProject(e: Event) {
    e.preventDefault();
    const f = projectForm;
    if (!f || !f.name.trim()) return;
    projectBusy = true;
    projectError = "";
    const body = {
      name: f.name.trim(), description: f.description, goal_id: f.goal || null, lead_staff_id: f.lead || null,
      workspace: f.workspace, isolation: f.isolation, color: f.color, status: f.status,
    };
    try {
      if (f.id) await rpc("project.update", { company: id, project_id: f.id, patch: body });
      else await rpc("project.create", { company: id, project: body });
      projectForm = null;
      void load(id);
    } catch (err) {
      projectError = errorText(err);
    } finally {
      projectBusy = false;
    }
  }

  function removeProject(p: Project) {
    if (!confirm($t("company.goals.project_delete_confirm", { name: p.name }) as string)) return;
    void goalAction(() => rpc("project.delete", { company: id, project_id: p.id }));
  }
</script>

<div class="page page-wide goals-page">
  {#if error}<p class="field-error">{error}</p>{/if}

  <section>
    <div class="section-header">
      <h2 class="section-header-title">{$t("company.goals.title")}</h2>
      <button type="button" class="button active" onclick={() => newGoal(tree[0]?.goal ?? null)}>{$t("company.goals.new")}</button>
    </div>
    <div class="group">
      {#if tree.length === 0 && company?.mission}
        <div class="goal-row mission-row"><span class="tag tag-accent">{$t("company.goals.mission")}</span><span class="goal-title">{company.mission}</span></div>
      {/if}
      {#each tree as n (n.goal.id)}{@render goalBranch(n, 0)}{/each}
      {#if loaded && subGoals === 0}
        <div class="empty-state small-empty">
          <img class="empty-state-art" src="/emoji/sparkles.png" alt="" width="72" height="72" draggable="false" />
          <div class="empty-state-title">{$t("company.goals.empty_title")}</div>
          <p class="empty-state-hint">{$t("company.goals.empty_hint")}</p>
        </div>
      {/if}
    </div>
  </section>

  <section>
    <div class="section-header">
      <h2 class="section-header-title">{$t("company.goals.projects")}</h2>
      <button type="button" class="section-header-action" onclick={newProject}>{$t("company.goals.new_project")}</button>
    </div>
    <div class="group">
      {#if projects.length === 0}
        <div class="group-row"><span class="hint">{$t("company.goals.no_projects")}</span></div>
      {:else}
        {#each projects as p (p.id)}
          <div class="group-row project-row">
            <span class="swatch" style:background={p.color ?? "var(--fill-3)"}></span>
            <span class="group-row-content">
              <span class="group-row-title">{p.name}</span>
              <span class="group-row-sub">
                {#if p.lead_staff_id}{$t("company.goals.lead")}: {staffName.get(p.lead_staff_id) ?? "—"}{/if}
                {#if p.goal_id} · {goalTitle.get(p.goal_id) ?? ""}{/if}
                · {$t(`company.common.isolation.${p.isolation}`)}
                {#if p.workspace} · <code title={p.workspace}>{p.workspace}</code>{/if}
              </span>
            </span>
            <span class="group-row-trailing">
              <span class="tag tone-{statusTone(p.status)}">{$t(`company.common.project_status.${p.status}`)}</span>
              <button type="button" class="button" onclick={() => editProject(p)}>{$t("company.common.edit")}</button>
              <button type="button" class="icon-btn" aria-label={$t("company.common.delete")} onclick={() => removeProject(p)}>×</button>
            </span>
          </div>
        {/each}
      {/if}
    </div>
  </section>
</div>

{#snippet goalBranch(n: GoalNode, depth: number)}
  {@render goalRow(n.goal, depth)}
  {#each n.children as c (c.goal.id)}{@render goalBranch(c, depth + 1)}{/each}
{/snippet}

{#snippet goalRow(g: Goal, depth: number)}
  {@const isMission = g.level === "company" && !g.parent_id}
  <div class="goal-row" class:mission-row={isMission} class:done={g.status === "achieved" || g.status === "cancelled"} style:padding-left="calc(var(--space-3) + {depth * 22}px)">
    {#if isMission}
      <span class="tag tag-accent">{$t("company.goals.mission")}</span>
    {:else}
      <StatusDot status={g.status === "achieved" ? "done" : g.status === "cancelled" ? "cancelled" : g.status === "active" ? "in_progress" : "todo"} kind="issue" />
      <span class="tag">{$t(`company.common.goal_level.${g.level}`)}</span>
    {/if}
    <span class="goal-title" title={g.body || g.title}>{g.title}</span>
    {#if g.owner_staff_id}<span class="dim">{staffName.get(g.owner_staff_id) ?? ""}</span>{/if}
    {#if !isMission}<span class="dim">{$t(`company.common.goal_status.${g.status}`)}</span>{/if}
    <span class="row-actions">
      {#if g.level !== "task"}<button type="button" class="link" onclick={() => newGoal(g)}>{$t("company.goals.add_child")}</button>{/if}
      {#if !isMission && g.status !== "achieved"}<button type="button" class="link" onclick={() => achieve(g)}>{$t("company.goals.achieve")}</button>{/if}
      <button type="button" class="link" onclick={() => editGoal(g)}>{$t("company.common.edit")}</button>
      {#if !isMission}<button type="button" class="link danger" onclick={() => removeGoal(g)}>{$t("company.common.delete")}</button>{/if}
    </span>
  </div>
{/snippet}

{#if goalForm}
  <div class="dialog-backdrop" role="presentation" onkeydown={(e) => e.key === "Escape" && (goalForm = null)}>
    <div class="dialog-card wide" role="dialog" aria-modal="true" aria-labelledby="goal-form-title">
      <form class="dialog-form" onsubmit={saveGoal}>
        <h2 class="dialog-title" id="goal-form-title">{goalForm.id ? $t("company.goals.edit_title") : $t("company.goals.new_title")}</h2>
        <label class="field">
          <span class="field-label">{$t("company.goals.form.title")}</span>
          <input class="input" type="text" bind:value={goalForm.title} required />
        </label>
        <label class="field">
          <span class="field-label">{$t("company.goals.form.body")}</span>
          <textarea class="input" rows="3" bind:value={goalForm.body}></textarea>
        </label>
        <div class="row">
          {#if !goalForm.id}
            <label class="field grow">
              <span class="field-label">{$t("company.goals.form.level")}</span>
              <select class="input" bind:value={goalForm.level}>
                {#each LEVELS.slice(1) as l (l)}<option value={l}>{$t(`company.common.goal_level.${l}`)}</option>{/each}
              </select>
            </label>
          {/if}
          {#if !(goalForm.level === "company" && !goalForm.parent)}
            <label class="field grow">
              <span class="field-label">{$t("company.goals.form.parent")}</span>
              <select class="input" bind:value={goalForm.parent}>
                <option value="">{$t("company.goals.form.under_mission")}</option>
                {#each parentOptions as p (p.id)}<option value={p.id}>{p.title}</option>{/each}
              </select>
            </label>
          {/if}
        </div>
        <div class="row">
          <label class="field grow">
            <span class="field-label">{$t("company.goals.form.owner")}</span>
            <select class="input" bind:value={goalForm.owner}>
              <option value="">{$t("company.common.nobody")}</option>
              {#each alive as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
            </select>
          </label>
          <label class="field grow">
            <span class="field-label">{$t("company.goals.form.status")}</span>
            <select class="input" bind:value={goalForm.status}>
              {#each GOAL_STATUSES as st (st)}<option value={st}>{$t(`company.common.goal_status.${st}`)}</option>{/each}
            </select>
          </label>
        </div>
        {#if goalError}<p class="field-error">{goalError}</p>{/if}
        <div class="dialog-actions">
          <button type="button" class="button" disabled={goalBusy} onclick={() => (goalForm = null)}>{$t("company.common.cancel")}</button>
          <button type="submit" class="button active" disabled={goalBusy || !goalForm.title.trim()}>{$t("company.common.save")}</button>
        </div>
      </form>
    </div>
  </div>
{/if}

{#if projectForm}
  <div class="dialog-backdrop" role="presentation" onkeydown={(e) => e.key === "Escape" && (projectForm = null)}>
    <div class="dialog-card wide" role="dialog" aria-modal="true" aria-labelledby="project-form-title">
      <form class="dialog-form" onsubmit={saveProject}>
        <h2 class="dialog-title" id="project-form-title">{projectForm.id ? $t("company.goals.edit_project") : $t("company.goals.new_project")}</h2>
        <label class="field">
          <span class="field-label">{$t("company.goals.form.name")}</span>
          <input class="input" type="text" bind:value={projectForm.name} required />
        </label>
        <label class="field">
          <span class="field-label">{$t("company.goals.form.description")}</span>
          <textarea class="input" rows="2" bind:value={projectForm.description}></textarea>
        </label>
        <div class="row">
          <label class="field grow">
            <span class="field-label">{$t("company.goals.form.goal")}</span>
            <select class="input" bind:value={projectForm.goal}>
              <option value="">{$t("company.common.none")}</option>
              {#each goals as g (g.id)}<option value={g.id}>{g.title}</option>{/each}
            </select>
          </label>
          <label class="field grow">
            <span class="field-label">{$t("company.goals.lead")}</span>
            <select class="input" bind:value={projectForm.lead}>
              <option value="">{$t("company.common.nobody")}</option>
              {#each alive as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
            </select>
          </label>
        </div>
        <div class="field">
          <span class="field-label">{$t("company.goals.form.workspace")}</span>
          <div class="folder">
            <button type="button" class="button" onclick={async () => { if (projectForm) projectForm.workspace = (await pickFolder()) ?? projectForm.workspace; }}>{$t("company.common.pick_folder")}</button>
            {#if projectForm.workspace}
              <code class="path" title={projectForm.workspace}>{projectForm.workspace}</code>
            {:else}
              <span class="hint">{$t("company.goals.form.workspace_hint")}</span>
            {/if}
          </div>
        </div>
        <div class="field">
          <span class="field-label">{$t("company.goals.form.isolation")}</span>
          <div class="segmented" role="radiogroup" aria-label={$t("company.goals.form.isolation")}>
            {#each ["shared", "worktree"] as const as iso (iso)}
              <button type="button" class="segmented-btn" role="radio" aria-checked={projectForm.isolation === iso} onclick={() => projectForm && (projectForm.isolation = iso)}>{$t(`company.common.isolation.${iso}`)}</button>
            {/each}
          </div>
        </div>
        <div class="row">
          <div class="field grow">
            <span class="field-label">{$t("company.goals.form.color")}</span>
            <div class="colors" role="radiogroup" aria-label={$t("company.goals.form.color")}>
              {#each COLORS as c (c)}
                <button type="button" class="color" role="radio" aria-checked={projectForm.color === c} aria-label={c} style:background={c} onclick={() => projectForm && (projectForm.color = c)}></button>
              {/each}
            </div>
          </div>
          <label class="field grow">
            <span class="field-label">{$t("company.goals.form.status")}</span>
            <select class="input" bind:value={projectForm.status}>
              {#each PROJECT_STATUSES as st (st)}<option value={st}>{$t(`company.common.project_status.${st}`)}</option>{/each}
            </select>
          </label>
        </div>
        {#if projectError}<p class="field-error">{projectError}</p>{/if}
        <div class="dialog-actions">
          <button type="button" class="button" disabled={projectBusy} onclick={() => (projectForm = null)}>{$t("company.common.cancel")}</button>
          <button type="submit" class="button active" disabled={projectBusy || !projectForm.name.trim()}>{$t("company.common.save")}</button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .goals-page { display: flex; flex-direction: column; gap: var(--space-5); }
  .goal-row { display: flex; align-items: center; gap: var(--space-2); min-height: 40px; padding: 6px var(--space-3); font-size: var(--text-sm); position: relative; }
  .goal-row + .goal-row { border-top: var(--hairline) solid var(--separator); }
  .mission-row { background: var(--fill-1); font-weight: 600; }
  .goal-row.done .goal-title { color: var(--text-dim); text-decoration: line-through; }
  .goal-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-actions { display: flex; gap: var(--space-3); flex-shrink: 0; opacity: 0; transition: opacity 120ms; }
  .goal-row:hover .row-actions, .goal-row:focus-within .row-actions { opacity: 1; }
  @media (hover: none) { .row-actions { opacity: 1; } }
  .link { background: none; border: 0; padding: 0; font: inherit; font-size: var(--text-xs); color: var(--accent-hi); cursor: pointer; }
  .link:hover { text-decoration: underline; }
  .link.danger { color: var(--red); }
  .dim { font-size: var(--text-xs); color: var(--text-dim); flex-shrink: 0; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .small-empty { padding: var(--space-5); }
  .swatch { width: 12px; height: 12px; border-radius: 4px; flex-shrink: 0; }
  .project-row code { font-family: var(--font-mono); font-size: var(--text-xs); }
  .icon-btn { background: none; border: 0; font-size: 18px; line-height: 1; color: var(--text-dim); cursor: pointer; padding: 2px 6px; border-radius: var(--radius-sm); }
  .icon-btn:hover { background: var(--fill-2); color: var(--red); }
  .tone-green { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .tone-blue { color: var(--blue); background: color-mix(in srgb, var(--blue) 14%, transparent); }
  .tone-faint { color: var(--text-dim); }
  .dialog-form { display: flex; flex-direction: column; gap: var(--space-3); }
  .wide { width: min(560px, 100%); max-height: calc(100vh - 64px); overflow-y: auto; }
  .row { display: flex; flex-wrap: wrap; gap: var(--space-3); }
  .grow { flex: 1; min-width: 180px; }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  .folder { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .path { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .segmented { align-self: flex-start; }
  .colors { display: flex; gap: 6px; flex-wrap: wrap; padding-top: 4px; }
  .color { width: 22px; height: 22px; border-radius: 50%; border: 0; cursor: pointer; }
  .color[aria-checked="true"] { box-shadow: 0 0 0 2px var(--surface), 0 0 0 4px var(--accent); }
  .color:focus-visible { outline: var(--focus-ring); }
</style>
