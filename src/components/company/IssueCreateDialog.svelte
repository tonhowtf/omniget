<script lang="ts">
  /** New issue form (`issue.create`). Title is the only required field. */
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { rpc, errorText, type Issue, type Label, type Project, type Staff } from "$lib/company/rpc";
  import { PRIORITIES, priorityKey } from "./company-format";

  let {
    company,
    staff,
    projects,
    issues,
    labels,
    parentId = null,
    projectId = null,
    oncreated,
    onclose,
  }: {
    company: string;
    staff: Staff[];
    projects: Project[];
    issues: Issue[];
    labels: Label[];
    parentId?: string | null;
    projectId?: string | null;
    oncreated: (issue: Issue) => void;
    onclose: () => void;
  } = $props();

  let title = $state("");
  let body = $state("");
  /** "" = nobody, "me" = the owner, else a staff id. */
  let assignee = $state("");
  let priority = $state("medium");
  let project = $state(untrack(() => projectId) ?? "");
  let parent = $state(untrack(() => parentId) ?? "");
  let blockedBy = $state<string[]>([]);
  let labelText = $state("");
  let workMode = $state("standard");
  let review = $state("none");
  let busy = $state(false);
  let error = $state("");
  let titleBox = $state<HTMLInputElement | null>(null);

  let hireable = $derived(staff.filter((s) => s.status !== "terminated" && s.status !== "pending_approval"));
  let openIssues = $derived(issues.filter((i) => i.status !== "done" && i.status !== "cancelled"));

  $effect(() => {
    titleBox?.focus();
  });

  async function submit(e: Event) {
    e.preventDefault();
    if (!title.trim() || busy) return;
    busy = true;
    error = "";
    try {
      const created = await rpc<Issue>("issue.create", {
        company,
        issue: {
          title: title.trim(),
          body,
          priority,
          project_id: project || null,
          parent_id: parent || null,
          assignee_staff_id: assignee && assignee !== "me" ? assignee : null,
          assignee_user: assignee === "me",
          work_mode: workMode,
          review_policy: review,
          labels: labelText
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean),
          blocked_by: blockedBy,
        },
      });
      oncreated(created);
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit(e);
  }
</script>

<div class="dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="new-issue-title" tabindex="-1" onkeydown={keydown} onclick={(e) => e.target === e.currentTarget && onclose()}>
  <form class="dialog-card create" onsubmit={submit}>
    <h2 class="dialog-title" id="new-issue-title">{$t("company.issues.new_title")}</h2>
    <input
      class="input input-lg"
      type="text"
      bind:this={titleBox}
      bind:value={title}
      placeholder={$t("company.issues.title_placeholder")}
      aria-label={$t("company.issues.field_title")}
    />
    <textarea
      class="input body"
      rows="5"
      bind:value={body}
      placeholder={$t("company.issues.body_placeholder")}
      aria-label={$t("company.issues.field_body")}
    ></textarea>

    <div class="grid">
      <label class="field">
        <span class="field-label">{$t("company.issues.field_assignee")}</span>
        <select class="input" bind:value={assignee}>
          <option value="">{$t("company.common.unassigned")}</option>
          <option value="me">{$t("company.common.owner")}</option>
          {#each hireable as s (s.id)}
            <option value={s.id}>{s.name} · {s.title || s.role}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("company.issues.field_priority")}</span>
        <select class="input" bind:value={priority}>
          {#each PRIORITIES as p (p)}
            <option value={p}>{$t(priorityKey(p))}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("company.issues.field_project")}</span>
        <select class="input" bind:value={project}>
          <option value="">{$t("company.common.none")}</option>
          {#each projects as p (p.id)}
            <option value={p.id}>{p.name}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("company.issues.field_parent")}</span>
        <select class="input" bind:value={parent}>
          <option value="">{$t("company.common.none")}</option>
          {#each openIssues as i (i.id)}
            <option value={i.id}>{i.identifier} · {i.title}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("company.issues.field_mode")}</span>
        <select class="input" bind:value={workMode}>
          {#each ["standard", "ask", "planning"] as m (m)}
            <option value={m}>{$t(`company.issues.mode.${m}`)}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("company.issues.field_review")}</span>
        <select class="input" bind:value={review}>
          {#each ["none", "anyone", "not_creator", "human_only"] as r (r)}
            <option value={r}>{$t(`company.issues.review.${r}`)}</option>
          {/each}
        </select>
      </label>
    </div>

    <label class="field">
      <span class="field-label">{$t("company.issues.field_labels")}</span>
      <input class="input" type="text" list="company-label-names" bind:value={labelText} placeholder={$t("company.issues.labels_placeholder")} />
      <datalist id="company-label-names">
        {#each labels as l (l.id)}<option value={l.name}></option>{/each}
      </datalist>
    </label>

    {#if openIssues.length > 0}
      <label class="field">
        <span class="field-label">{$t("company.issues.field_blocked_by")}</span>
        <select class="input multi" multiple bind:value={blockedBy}>
          {#each openIssues as i (i.id)}
            <option value={i.id}>{i.identifier} · {i.title}</option>
          {/each}
        </select>
      </label>
    {/if}

    {#if error}<p class="field-error" role="alert">{error}</p>{/if}
    <div class="dialog-actions">
      <span class="hint">{$t("company.issues.create_hint")}</span>
      <button type="button" class="btn btn-secondary" onclick={onclose}>{$t("company.common.cancel")}</button>
      <button type="submit" class="btn btn-primary" disabled={!title.trim() || busy}>{$t("company.issues.create")}</button>
    </div>
  </form>
</div>

<style>
  .create {
    width: min(620px, 100%);
    max-height: calc(100vh - 80px);
    overflow-y: auto;
  }
  .body {
    height: auto;
    resize: vertical;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: var(--space-3);
  }
  .multi {
    height: 88px;
    padding: 4px;
  }
  .hint {
    margin-right: auto;
    align-self: center;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
