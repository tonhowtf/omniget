<script lang="ts">
  import { t } from "$lib/i18n";
  import { rpc, errorText, type Approval, type Staff } from "$lib/company/rpc";
  import IssueMarkdown from "./IssueMarkdown.svelte";
  import { relTime, initials, avatarColor } from "./company-ui";
  import { staffName } from "./company-format";

  let { approval, staff }: { approval: Approval; staff: Staff[] } = $props();

  let note = $state("");
  let noteOpen = $state(false);
  let busy = $state(false);
  let error = $state("");

  let p = $derived(approval.payload as Record<string, unknown>);
  let candidate = $derived(
    approval.kind === "hire_agent"
      ? {
          name: String(p.name ?? ""),
          role: String(p.role ?? ""),
          title: String(p.title ?? ""),
          capabilities: String(p.capabilities ?? ""),
          agent: String(p.agent_id ?? ""),
          reportsTo: typeof p.reports_to === "string" ? p.reports_to : null,
        }
      : null,
  );

  async function decide(decision: "approve" | "reject" | "revise", chosen: string | null = null) {
    if (decision === "revise" && !note.trim()) {
      noteOpen = true;
      error = $t("company.inbox.revise_needs_note");
      return;
    }
    busy = true;
    error = "";
    try {
      await rpc("approval.decide", {
        approval: approval.id,
        decision,
        chosen,
        note: note.trim() || null,
        expected_version: approval.version,
      });
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<article class="approval surface-card">
  <header class="head">
    <span class="kind">{$t(`company.inbox.kind.${approval.kind}`)}</span>
    {#if approval.state === "revision_requested"}<span class="tag tag-warning">{$t("company.inbox.revision_requested")}</span>{/if}
    <span class="who">
      {approval.requested_by_staff_id ? staffName(staff, approval.requested_by_staff_id) : $t("company.inbox.system")}
      · {relTime(approval.created_ms, $t)}
    </span>
  </header>
  <h3 class="title">{approval.title}</h3>

  {#if candidate}
    <div class="candidate">
      <span class="avatar avatar-lg" style:background={avatarColor(candidate.name)}>{initials(candidate.name)}</span>
      <div class="cand-body">
        <strong>{candidate.name}</strong>
        <span class="dim">{candidate.title || candidate.role}{candidate.reportsTo ? ` · ${$t("company.inbox.reports_to", { name: staffName(staff, candidate.reportsTo) })}` : ""}</span>
        {#if candidate.agent}<span class="dim mono">{candidate.agent}</span>{/if}
        {#if candidate.capabilities}<p class="caps">{candidate.capabilities}</p>{/if}
      </div>
    </div>
  {:else if approval.body.trim()}
    <div class="text" class:long={approval.kind === "strategy" || approval.kind === "plan"}>
      <IssueMarkdown text={approval.body} />
    </div>
  {/if}

  {#if approval.note}
    <p class="prev-note">{$t("company.inbox.previous_note", { note: approval.note })}</p>
  {/if}

  {#if noteOpen}
    <textarea class="input note" rows="2" bind:value={note} placeholder={$t("company.inbox.note_placeholder")} aria-label={$t("company.inbox.note")}></textarea>
  {/if}
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}

  <div class="actions">
    {#if !noteOpen}
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => (noteOpen = true)}>{$t("company.inbox.add_note")}</button>
    {/if}
    <span class="spacer"></span>
    <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => decide("revise")}>{$t("company.inbox.revise")}</button>
    <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => decide("reject")}>{$t("company.inbox.reject")}</button>
    {#if approval.kind === "decision" && approval.options?.length}
      {#each approval.options as o (o)}
        <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => decide("approve", o)}>{o}</button>
      {/each}
    {:else}
      <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => decide("approve")}>
        {approval.kind === "hire_agent" ? $t("company.inbox.hire") : $t("company.inbox.approve")}
      </button>
    {/if}
  </div>
</article>

<style>
  .approval {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
  }
  .kind {
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
    color: var(--accent-text);
  }
  .who {
    margin-left: auto;
    color: var(--text-dim);
  }
  .title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }
  .candidate {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
  }
  .avatar {
    color: #fff;
    flex-shrink: 0;
  }
  .cand-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .dim {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .caps {
    margin: var(--space-1) 0 0;
    font-size: var(--text-sm);
    white-space: pre-wrap;
  }
  .text.long {
    max-height: 320px;
    overflow-y: auto;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .prev-note {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .note {
    height: auto;
    resize: vertical;
    font-family: inherit;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
</style>
