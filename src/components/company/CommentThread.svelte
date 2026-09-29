<script lang="ts">
  /** An issue's comments, oldest first, and the box to add one (Cmd+Enter). */
  import { t } from "$lib/i18n";
  import { rpc, errorText, type Comment, type Staff } from "$lib/company/rpc";
  import IssueMarkdown from "./IssueMarkdown.svelte";
  import { relTime, initials, avatarColor } from "./company-ui";
  import { dateTime, staffName } from "./company-format";

  let {
    company,
    issue,
    comments,
    staff,
  }: { company: string; issue: string; comments: Comment[]; staff: Staff[] } = $props();

  let draft = $state("");
  let busy = $state(false);
  let error = $state("");

  function author(c: Comment): string {
    if (c.author_kind === "user") return $t("company.common.owner");
    if (c.author_kind === "system") return $t("company.issue.system");
    return staffName(staff, c.author_staff_id) || $t("company.issue.agent");
  }

  async function send() {
    const body = draft.trim();
    if (!body || busy) return;
    busy = true;
    error = "";
    try {
      await rpc("issue.comment", { company, issue, body, client_req: crypto.randomUUID() });
      draft = "";
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="thread">
  {#each comments as c (c.id)}
    {@const name = author(c)}
    <article class="comment" class:system={c.author_kind === "system"}>
      {#if c.author_kind === "system"}
        <span class="sys-dot" aria-hidden="true"></span>
        <div class="sys-body">
          <IssueMarkdown text={c.body} />
          <time class="when" title={dateTime(c.created_ms)}>{relTime(c.created_ms, $t)}</time>
        </div>
      {:else}
        <span class="avatar avatar-sm" style:background={c.author_kind === "user" ? "var(--accent)" : avatarColor(name)}>
          {initials(name)}
        </span>
        <div class="body">
          <header class="head">
            <strong>{name}</strong>
            {#if c.author_kind === "agent"}<span class="tag">{$t("company.issue.agent")}</span>{/if}
            <time class="when" title={dateTime(c.created_ms)}>{relTime(c.created_ms, $t)}</time>
          </header>
          <IssueMarkdown text={c.body} />
        </div>
      {/if}
    </article>
  {:else}
    <p class="hint">{$t("company.issue.no_comments")}</p>
  {/each}

  <div class="composer">
    <textarea
      class="input"
      rows="3"
      bind:value={draft}
      placeholder={$t("company.issue.comment_placeholder")}
      aria-label={$t("company.issue.comment")}
      onkeydown={(e) => {
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          void send();
        }
      }}
    ></textarea>
    <div class="composer-foot">
      {#if error}<span class="field-error" role="alert">{error}</span>{/if}
      <span class="hint">{$t("company.issue.comment_hint")}</span>
      <button type="button" class="btn btn-primary btn-sm" disabled={!draft.trim() || busy} onclick={send}>
        {$t("company.issue.comment")}
      </button>
    </div>
  </div>
</div>

<style>
  .thread {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .comment {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
  }
  .avatar {
    color: #fff;
    flex-shrink: 0;
  }
  .body {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: 4px;
    font-size: var(--text-sm);
  }
  .when {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .comment.system {
    align-items: center;
    padding-left: 6px;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .sys-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-faint);
    flex-shrink: 0;
  }
  .sys-body {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
  }
  .sys-body :global(.md) {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .composer {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .composer textarea {
    height: auto;
    resize: vertical;
    font-family: inherit;
  }
  .composer-foot {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    justify-content: flex-end;
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
