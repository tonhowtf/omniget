<script lang="ts">
  import { t } from "$lib/i18n";
  import { TRANSITIONS, type Issue, type IssueStatus, type Label, type Staff } from "$lib/company/rpc";
  import IssuePriority from "./IssuePriority.svelte";
  import IssueStatusBadge from "./IssueStatusBadge.svelte";
  import BlockedReasonDialog from "./BlockedReasonDialog.svelte";
  import { BOARD_COLUMNS, issueStatusKey, staffName } from "./company-format";

  let {
    issues,
    staff,
    labelsOf,
    hrefOf,
    onmove,
  }: {
    issues: Issue[];
    staff: Staff[];
    labelsOf: (id: string) => Label[];
    hrefOf: (i: Issue) => string;
    onmove: (issue: Issue, to: IssueStatus, note?: string) => Promise<boolean>;
  } = $props();

  let moved = $state<Record<string, IssueStatus>>({});
  let dragging = $state<Issue | null>(null);
  let over = $state<IssueStatus | null>(null);
  let askBlocked = $state<Issue | null>(null);

  function statusOf(i: Issue): IssueStatus {
    return moved[i.id] ?? i.status;
  }

  let columns = $derived(
    BOARD_COLUMNS.map((s) => ({ status: s, items: issues.filter((i) => statusOf(i) === s) })),
  );

  function allowed(to: IssueStatus): boolean {
    return !!dragging && dragging.status !== to && TRANSITIONS[dragging.status].includes(to);
  }

  async function commit(issue: Issue, to: IssueStatus, note?: string) {
    moved = { ...moved, [issue.id]: to };
    const ok = await onmove(issue, to, note);
    const settle = () => {
      const next = { ...moved };
      delete next[issue.id];
      moved = next;
    };
    if (ok) setTimeout(settle, 1500);
    else settle();
  }

  function drop(e: DragEvent, to: IssueStatus) {
    e.preventDefault();
    over = null;
    const issue = dragging;
    dragging = null;
    if (!issue || issue.status === to || !TRANSITIONS[issue.status].includes(to)) return;
    if (to === "blocked") {
      askBlocked = issue;
      return;
    }
    void commit(issue, to);
  }
</script>

<div class="board" role="list">
  {#each columns as col (col.status)}
    <section
      class="column"
      class:target={dragging && allowed(col.status)}
      class:denied={dragging && dragging.status !== col.status && !allowed(col.status)}
      class:over={over === col.status}
      role="listitem"
      aria-label={$t(issueStatusKey(col.status))}
      ondragover={(e) => {
        if (allowed(col.status)) {
          e.preventDefault();
          if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
          over = col.status;
        }
      }}
      ondragleave={() => over === col.status && (over = null)}
      ondrop={(e) => drop(e, col.status)}
    >
      <header class="col-head">
        <IssueStatusBadge status={col.status} />
        <span class="count">{col.items.length}</span>
      </header>
      <div class="cards">
        {#each col.items as issue (issue.id)}
          {@const labels = labelsOf(issue.id)}
          <a
            class="card-item"
            class:ghost={dragging?.id === issue.id}
            href={hrefOf(issue)}
            draggable="true"
            ondragstart={(e) => {
              dragging = issue;
              e.dataTransfer?.setData("text/plain", issue.id);
              if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
            }}
            ondragend={() => {
              dragging = null;
              over = null;
            }}
          >
            <div class="card-top">
              <span class="ident">{issue.identifier}</span>
              <IssuePriority priority={issue.priority} />
            </div>
            <div class="card-title">{issue.title}</div>
            {#if labels.length || issue.assignee_staff_id || issue.assignee_user}
              <div class="card-foot">
                {#each labels as l (l.id)}
                  <span class="label" style:--label={l.color || "var(--gray)"}>{l.name}</span>
                {/each}
                <span class="assignee">
                  {issue.assignee_user ? $t("company.common.owner") : staffName(staff, issue.assignee_staff_id)}
                </span>
              </div>
            {/if}
            {#if statusOf(issue) === "blocked" && issue.blocked_reason}
              <div class="reason">{issue.blocked_reason}</div>
            {/if}
          </a>
        {:else}
          <p class="col-empty">{$t("company.issues.column_empty")}</p>
        {/each}
      </div>
    </section>
  {/each}
</div>

{#if askBlocked}
  {@const target = askBlocked}
  <BlockedReasonDialog
    identifier={target.identifier}
    oncancel={() => (askBlocked = null)}
    onconfirm={(note) => {
      askBlocked = null;
      void commit(target, "blocked", note);
    }}
  />
{/if}

<style>
  .board {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(220px, 1fr);
    gap: var(--space-3);
    overflow-x: auto;
    padding-bottom: var(--space-2);
    min-height: 0;
  }
  .column {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-2);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
    transition: background var(--duration-fast) var(--ease-out), opacity var(--duration-fast) var(--ease-out);
  }
  .column.target {
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .column.over {
    background: color-mix(in srgb, var(--accent) 10%, var(--fill-1));
  }
  .column.denied {
    opacity: 0.45;
  }
  .col-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px var(--space-1);
  }
  .count {
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 48px;
  }
  .card-item {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    text-decoration: none;
    cursor: grab;
  }
  .card-item:hover {
    background: var(--surface-hi);
  }
  .card-item:focus-visible {
    outline: var(--focus-ring);
  }
  .card-item.ghost {
    opacity: 0.4;
  }
  .card-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .ident {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .card-title {
    font-size: var(--text-base);
    line-height: var(--leading-base);
    overflow-wrap: anywhere;
  }
  .card-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }
  .label {
    padding: 0 6px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    line-height: 16px;
    color: var(--label);
    background: color-mix(in srgb, var(--label) 14%, transparent);
  }
  .assignee {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .reason {
    font-size: var(--text-xs);
    color: var(--red);
    overflow-wrap: anywhere;
  }
  .col-empty {
    margin: 0;
    padding: var(--space-3) var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-faint);
    text-align: center;
  }
</style>
