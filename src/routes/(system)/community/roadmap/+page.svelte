<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-shell";
  import { t, locale } from "$lib/i18n";
  import { roadmapColumnKey, roadmapColumns, roadmapErrorText, type Roadmap } from "$lib/roadmap";

  type ChatAnswer = { ok: boolean; state: string; id?: number | null };

  const MAX_TITLE = 90;
  const MAX_DETAILS = 600;
  const NEW_ISSUE_GITHUB = "https://github.com/tonhowtf/omniget/issues/new";

  const STATE_KEYS: Record<string, string> = {
    slow: "community.chat.state_slow",
    limit: "community.chat.state_limit",
    frozen: "community.chat.state_frozen",
    ticket: "community.chat.state_error",
    error: "community.chat.state_error",
  };

  let roadmap = $state<Roadmap | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let request = 0;

  let activeCol = $state<string>("all");
  let query = $state("");

  let composerOpen = $state(false);
  let issueTitle = $state("");
  let issueDetails = $state("");
  let submitting = $state(false);
  let submitNotice = $state<{ kind: "ok" | "error"; text: string } | null>(null);

  let columns = $derived(roadmap ? roadmapColumns(roadmap) : []);
  let totalCount = $derived(columns.reduce((sum, col) => sum + col.items.length, 0));

  let visibleColumns = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return columns
      .filter((col) => activeCol === "all" || col.id === activeCol)
      .map((col) => ({
        ...col,
        items: q
          ? col.items.filter(
              (item) =>
                item.title.toLowerCase().includes(q) ||
                item.key.toLowerCase().includes(q) ||
                item.area.toLowerCase().includes(q) ||
                item.type.toLowerCase().includes(q),
            )
          : col.items,
      }))
      .filter((col) => col.items.length > 0);
  });

  async function load() {
    const mine = ++request;
    loading = true;
    error = null;
    try {
      const got = await invoke<Roadmap>("roadmap_get", {});
      if (mine !== request) return;
      roadmap = got;
    } catch (e) {
      if (mine !== request) return;
      error = roadmapErrorText(e);
    } finally {
      if (mine === request) loading = false;
    }
  }

  function openLink(url: string) {
    open(url).catch(() => {});
  }

  function toggleComposer() {
    composerOpen = !composerOpen;
    if (composerOpen) submitNotice = null;
  }

  async function submitIdea(e: Event) {
    e.preventDefault();
    const title = issueTitle.trim();
    if (!title || submitting) return;
    submitting = true;
    submitNotice = null;
    try {
      const answer = await invoke<ChatAnswer>("community_chat_send", {
        text: issueDetails.trim(),
        lang: $locale ?? "en",
        title,
      });
      if (answer.ok) {
        issueTitle = "";
        issueDetails = "";
        submitNotice = { kind: "ok", text: $t("roadmap.issue_sent") };
      } else {
        submitNotice = { kind: "error", text: $t(STATE_KEYS[answer.state] ?? "community.chat.state_error") };
      }
    } catch {
      submitNotice = { kind: "error", text: $t("community.chat.state_offline") };
    } finally {
      submitting = false;
    }
  }

  onMount(load);
</script>

<div class="roadmap-page" data-agent-readiness={loading ? "loading" : error ? "error" : "ready"}>
  <header class="head">
    <div class="head-copy">
      <h1>{$t("roadmap.title")}</h1>
      <p class="desc">{$t("roadmap.desc")}</p>
      {#if roadmap}
        <p class="meta">
          <span>{$t("roadmap.updated", { date: roadmap.updated })}</span>
          <button type="button" class="quiet-action" onclick={() => openLink(roadmap!.board)}>{$t("roadmap.open_board")}</button>
        </p>
      {/if}
    </div>
    <button type="button" class="btn btn-primary" onclick={toggleComposer}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
      {$t("roadmap.new_issue")}
    </button>
  </header>

  {#if composerOpen}
    <form class="surface-card issue-form" onsubmit={submitIdea}>
      <div class="issue-form-head">
        <div>
          <h2>{$t("roadmap.new_issue_title")}</h2>
          <p class="desc">{$t("roadmap.new_issue_desc")}</p>
        </div>
      </div>

      <label class="field">
        <span class="field-label">{$t("roadmap.issue_title_label")}</span>
        <input
          type="text"
          class="input"
          bind:value={issueTitle}
          maxlength={MAX_TITLE}
          placeholder={$t("roadmap.issue_title_placeholder")}
          required
        />
      </label>

      <label class="field">
        <span class="field-label">{$t("roadmap.issue_details_label")}</span>
        <textarea
          class="input field-textarea"
          bind:value={issueDetails}
          maxlength={MAX_DETAILS}
          rows="3"
          placeholder={$t("roadmap.issue_details_placeholder")}
        ></textarea>
      </label>

      {#if submitNotice}
        <p class="form-notice" class:is-error={submitNotice.kind === "error"} role="status">{submitNotice.text}</p>
      {/if}

      <div class="issue-form-actions">
        <button type="submit" class="btn btn-primary btn-sm" disabled={submitting || !issueTitle.trim()}>
          {submitting ? $t("roadmap.issue_submitting") : $t("roadmap.issue_submit")}
        </button>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => { composerOpen = false; }}>{$t("roadmap.cancel")}</button>
        <button type="button" class="quiet-action github-fallback" onclick={() => openLink(NEW_ISSUE_GITHUB)}>
          {$t("roadmap.issue_open_github")}
        </button>
      </div>
    </form>
  {/if}

  {#if loading}
    <div class="state" role="status">
      <span class="spinner" aria-hidden="true"></span>
      <span>{$t("common.loading")}</span>
    </div>
  {:else if error}
    <div class="surface-card state error" role="alert">
      <span><bdi>{$t("roadmap.failed", { msg: error })}</bdi></span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={load}>{$t("roadmap.retry")}</button>
    </div>
  {:else if columns.length === 0}
    <div class="surface-card state">{$t("roadmap.empty")}</div>
  {:else}
    <div class="roadmap-toolbar">
      <div class="filter-pills" role="tablist" aria-label={$t("roadmap.title")}>
        <button
          type="button"
          class="filter-pill"
          class:active={activeCol === "all"}
          role="tab"
          aria-selected={activeCol === "all"}
          onclick={() => { activeCol = "all"; }}
        >
          <span>{$t("roadmap.filter_all")}</span>
          <span class="pill-count">{totalCount}</span>
        </button>
        {#each columns as col (col.id)}
          <button
            type="button"
            class="filter-pill"
            class:active={activeCol === col.id}
            role="tab"
            aria-selected={activeCol === col.id}
            onclick={() => { activeCol = col.id; }}
          >
            <span class="status-dot" data-col={col.id} aria-hidden="true"></span>
            <span>{(roadmapColumnKey(col.id) ? $t(roadmapColumnKey(col.id)!) : col.id)}</span>
            <span class="pill-count">{col.items.length}</span>
          </button>
        {/each}
      </div>

      <input
        type="search"
        class="input search-input"
        bind:value={query}
        placeholder={$t("roadmap.search_placeholder")}
        aria-label={$t("roadmap.search_placeholder")}
      />
    </div>

    {#if visibleColumns.length === 0}
      <div class="surface-card state">{$t("roadmap.no_matches")}</div>
    {:else}
      <div class="lanes">
        {#each visibleColumns as col (col.id)}
          <section class="surface-card lane" aria-labelledby={`lane-${col.id}`}>
            <header class="lane-head">
              <span class="status-dot" data-col={col.id} aria-hidden="true"></span>
              <h2 id={`lane-${col.id}`}>{(roadmapColumnKey(col.id) ? $t(roadmapColumnKey(col.id)!) : col.id)}</h2>
              <span class="lane-count">{col.items.length}</span>
            </header>
            <ul class="tickets">
              {#each col.items as ticket (ticket.key)}
                <li class="ticket">
                  <button type="button" class="ticket-main" onclick={() => openLink(ticket.url)}>
                    <span class="key" dir="ltr">{ticket.key}</span>
                    <span class="ticket-body">
                      <span class="title" dir="ltr" lang="en">{ticket.title}</span>
                      <span class="ticket-tags">
                        {#if ticket.type}<span class="meta-chip" data-kind={ticket.type}>{ticket.type}</span>{/if}
                        {#if ticket.priority === "high"}<span class="meta-chip high">{ticket.priority}</span>{/if}
                        {#if ticket.area && ticket.area !== "all"}<span class="meta-chip">{ticket.area}</span>{/if}
                      </span>
                    </span>
                  </button>
                  {#if ticket.issue}
                    <button type="button" class="issue-btn" onclick={() => openLink(ticket.issue!)}>{$t("roadmap.issue")}</button>
                  {/if}
                </li>
              {/each}
            </ul>
          </section>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .roadmap-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .head-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    flex: 1 1 260px;
  }

  .head h1 {
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    line-height: var(--leading-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    margin: 0;
  }

  .desc,
  .meta {
    font-size: var(--text-sm);
    color: var(--text-muted);
    margin: 0;
    line-height: var(--leading-base);
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-1);
    color: var(--text-dim);
  }

  .quiet-action {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    font-size: var(--text-sm);
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
    white-space: nowrap;
  }

  .quiet-action:hover {
    color: var(--accent-text, var(--accent));
  }

  .issue-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
  }

  .issue-form-head h2 {
    margin: 0 0 2px;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .field-label {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
  }

  .field-textarea {
    height: auto;
    min-height: 76px;
    padding: var(--space-2) var(--space-3);
    resize: vertical;
    line-height: var(--leading-base);
  }

  .form-notice {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--success);
  }

  .form-notice.is-error {
    color: var(--error, var(--danger));
  }

  .issue-form-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .github-fallback {
    margin-inline-start: auto;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .roadmap-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .filter-pills {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .filter-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px;
    border-radius: var(--radius-full);
    border: none;
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }

  .filter-pill:hover {
    background: var(--surface-hi);
    color: var(--text);
  }

  .filter-pill.active {
    background: var(--surface-hi);
    color: var(--text);
    font-weight: 600;
    box-shadow: inset 0 0 0 var(--hairline) var(--accent);
  }

  .pill-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }

  .search-input {
    width: 210px;
    max-width: 100%;
    height: 28px;
    font-size: var(--text-sm);
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .status-dot[data-col="progress"] { background: var(--warning); }
  .status-dot[data-col="todo"] { background: var(--info); }
  .status-dot[data-col="backlog"] { background: var(--text-dim); }
  .status-dot[data-col="done"] { background: var(--success); }
  .status-dot[data-col="wontfix"] { background: var(--error, var(--danger)); }

  .lanes {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .lane {
    padding: 0;
    overflow: hidden;
  }

  .lane-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-block-end: 1px solid var(--separator);
    background: color-mix(in srgb, var(--surface-hi) 40%, var(--surface));
  }

  .lane-head h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    flex: 1;
  }

  .lane-count {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    padding: 2px 8px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
  }

  .tickets {
    list-style: none;
    margin: 0;
    padding: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .ticket {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    border-radius: var(--radius-md);
  }

  .ticket:hover {
    background: var(--fill-1);
  }

  .ticket-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    background: none;
    border: 0;
    padding: var(--space-2) var(--space-3);
    font: inherit;
    color: inherit;
    text-align: start;
    cursor: pointer;
  }

  .key {
    flex: none;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    background: var(--surface-hi);
    padding: 2px 7px;
    border-radius: var(--radius-xs);
    font-variant-numeric: tabular-nums;
    margin-top: 1px;
  }

  .ticket-body {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }

  .title {
    font-size: var(--text-base);
    line-height: 1.45;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .ticket-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .meta-chip {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    background: var(--fill-2);
    color: var(--text-dim);
  }

  .meta-chip[data-kind="bug"] {
    color: var(--error, var(--danger));
    background: color-mix(in srgb, var(--error, var(--danger)) 14%, transparent);
  }

  .meta-chip[data-kind="feature"] {
    color: var(--info);
    background: color-mix(in srgb, var(--info) 14%, transparent);
  }

  .meta-chip.high {
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 16%, transparent);
  }

  .issue-btn {
    flex: none;
    margin-inline-end: var(--space-3);
    padding: 3px 9px;
    border-radius: var(--radius-sm);
    border: none;
    background: var(--surface-hi);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font: inherit;
    font-size: var(--text-xs);
    color: var(--text-muted);
    cursor: pointer;
    white-space: nowrap;
  }

  .issue-btn:hover {
    color: var(--text);
  }

  .state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-6);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .spinner {
    width: 18px;
    height: 18px;
    border: 2px solid var(--content-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner { animation: none; }
  }
</style>
