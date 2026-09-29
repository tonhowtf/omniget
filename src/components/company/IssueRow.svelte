<script lang="ts">
  /** One line of the issue list: status, identifier, title, labels, assignee, age. */
  import { t } from "$lib/i18n";
  import type { Issue, Label, Staff } from "$lib/company/rpc";
  import IssuePriority from "./IssuePriority.svelte";
  import IssueStatusBadge from "./IssueStatusBadge.svelte";
  import { staffName } from "./company-format";
  import { relTime } from "./company-ui";

  let {
    issue,
    href,
    staff,
    labels = [],
  }: { issue: Issue; href: string; staff: Staff[]; labels?: Label[] } = $props();

  let assignee = $derived(
    issue.assignee_user ? $t("company.common.owner") : staffName(staff, issue.assignee_staff_id),
  );
</script>

<a class="issue-row list-row interactive compact" {href}>
  <IssuePriority priority={issue.priority} />
  <span class="ident">{issue.identifier}</span>
  <IssueStatusBadge status={issue.status} label={false} />
  <span class="title">{issue.title}</span>
  {#each labels as l (l.id)}
    <span class="label" style:--label={l.color || "var(--gray)"}>{l.name}</span>
  {/each}
  <span class="assignee" class:none={!assignee}>{assignee || $t("company.common.unassigned")}</span>
  <span class="when">{relTime(issue.updated_ms, $t)}</span>
</a>

<style>
  .issue-row {
    text-decoration: none;
    border-radius: 0;
    gap: var(--space-2);
  }
  .ident {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    min-width: 58px;
    flex-shrink: 0;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .label {
    flex-shrink: 0;
    padding: 0 7px;
    border-radius: var(--radius-full);
    font-size: var(--text-xs);
    line-height: 18px;
    color: var(--label);
    background: color-mix(in srgb, var(--label) 14%, transparent);
  }
  .assignee {
    flex-shrink: 0;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .assignee.none {
    color: var(--text-faint);
  }
  .when {
    flex-shrink: 0;
    width: 76px;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
