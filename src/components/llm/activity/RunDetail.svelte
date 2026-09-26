<script lang="ts">
  /**
   * One run, reviewed without a terminal: what was asked, the files it
   * really changed (the diff from the folder's snapshots, not the model's
   * word), the commands and tests it ran, the permissions, and what you can
   * do next. Jargon (runtime, session, launch id) stays in the collapsed
   * advanced details.
   */
  import { t } from "$lib/i18n";
  import {
    answerPermission,
    cancelRun,
    fetchRun,
    fetchRunDiff,
    getRunTick,
    resolveRun,
    shortTime,
    type RunDetail,
    type RunDiff,
  } from "$lib/stores/llm-jobs-store.svelte";
  import DiffView from "./DiffView.svelte";
  import { parseResolution, runStatus, secondsLeft } from "./run-status";

  let { runId }: { runId: string } = $props();

  let detail = $state<RunDetail | null>(null);
  let diff = $state<RunDiff | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let now = $state(Date.now());

  async function load(id: string) {
    const [d, f] = await Promise.all([fetchRun(id), fetchRunDiff(id)]);
    if (id !== runId) return;
    detail = d;
    diff = f;
    loading = false;
  }

  $effect(() => {
    const id = runId;
    loading = true;
    void load(id);
  });

  // A state change of any run may be this one: refresh quietly.
  $effect(() => {
    const tick = getRunTick();
    if (tick > 0 && !busy) void load(runId);
  });

  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  let status = $derived(detail ? runStatus(detail.state, detail.error) : null);
  let pending = $derived(detail?.permissions.filter((p) => p.state === "pending") ?? []);
  let answered = $derived(detail?.permissions.filter((p) => p.state !== "pending") ?? []);
  let resolution = $derived(parseResolution(detail?.resolution ?? null));
  let tests = $derived(detail?.commands.filter((c) => c.is_test) ?? []);

  async function act(fn: () => Promise<unknown>) {
    busy = true;
    await fn();
    busy = false;
    await load(runId);
  }
</script>

{#if loading && !detail}
  <p class="hint" role="status">{$t("assist.runs.loading")}</p>
{:else if !detail}
  <p class="hint">{$t("assist.runs.not_found")}</p>
{:else}
  <div class="run-detail">
    {#if detail.state === "unknown" && !detail.resolution}
      <p class="notice warn" role="status">{$t("assist.runs.unknown_hint")}</p>
    {:else if detail.state === "interrupted" && !detail.resolution}
      <p class="notice warn" role="status">{$t("assist.runs.interrupted_hint")}</p>
    {/if}
    {#if resolution}
      <p class="notice">{$t(`assist.runs.resolution.${resolution.kind}`)}</p>
    {/if}

    <div class="field-label">{$t("assist.runs.request_title")}</div>
    <div class="text">{detail.input_preview}</div>

    {#if detail.summary}
      <div class="field-label">{$t("assist.runs.summary_title")}</div>
      <div class="text">{detail.summary}</div>
    {/if}
    {#if detail.error && status === "failed"}
      <div class="field-label">{$t("assist.runs.error_title")}</div>
      <div class="text err">{detail.error}</div>
    {/if}

    {#if pending.length > 0}
      <div class="field-label">{$t("assist.runs.permissions_title")}</div>
      {#each pending as p (p.id)}
        <div class="ask" role="group" aria-label={$t("assist.runs.permission_label", { action: p.action })}>
          <div class="ask-title">{$t("assist.runs.permission_label", { action: p.action })}</div>
          <pre class="pre">{p.preview}</pre>
          <div class="actions">
            <button type="button" class="button active" disabled={busy} onclick={() => act(() => answerPermission(p.id, "once"))}>
              {$t("assist.runs.answer_once")}
            </button>
            <button type="button" class="button" disabled={busy} onclick={() => act(() => answerPermission(p.id, "always"))}>
              {$t("assist.runs.answer_always")}
            </button>
            <button type="button" class="button" disabled={busy} onclick={() => act(() => answerPermission(p.id, "deny"))}>
              {$t("assist.runs.answer_deny")}
            </button>
            <span class="dim">{$t("assist.runs.expires_in", { seconds: secondsLeft(p.deadline_ms, now) })}</span>
          </div>
        </div>
      {/each}
    {/if}

    <div class="field-label">{$t("assist.runs.files_title")}</div>
    {#if !diff || !diff.available}
      <p class="hint">
        {#if diff && !diff.available && diff.reason === "no_workspace"}
          {$t("assist.runs.no_workspace")}
        {:else if diff && !diff.available && diff.reason === "error"}
          {$t("assist.runs.diff_error")}
        {:else}
          {$t("assist.runs.no_changes")}
        {/if}
      </p>
      {#if detail.files_touched.length > 0}
        <p class="hint">{$t("assist.runs.files_claimed", { files: detail.files_touched.join(", ") })}</p>
      {/if}
    {:else}
      {#if !diff.diff.complete}
        <p class="hint">{$t("assist.runs.diff_partial")}</p>
      {/if}
      <ul class="files">
        {#each diff.diff.files as f (f.path)}
          <li>
            <span class="badge st-{f.status}">{$t(`assist.runs.file_status.${f.status}`)}</span>
            <code class="path" title={f.path}>{f.path}</code>
            {#if f.additions != null}<span class="plus">+{f.additions}</span>{/if}
            {#if f.deletions != null}<span class="minus">−{f.deletions}</span>{/if}
          </li>
        {/each}
      </ul>
      {#if diff.diff.patch}
        <details class="patch">
          <summary>{$t("assist.runs.show_diff")}</summary>
          <DiffView patch={diff.diff.patch} label={$t("assist.runs.files_title")} />
          {#if diff.diff.clipped}<p class="hint">{$t("assist.runs.diff_clipped")}</p>{/if}
        </details>
      {/if}
    {/if}

    <div class="field-label">{$t("assist.runs.commands_title")}</div>
    {#if detail.commands.length === 0}
      <p class="hint">{$t("assist.runs.commands_empty")}</p>
    {:else}
      {#if tests.length > 0}
        <p class="hint">
          {$t("assist.runs.tests_summary", {
            passed: tests.filter((c) => c.ok === true).length,
            total: tests.length,
          })}
        </p>
      {/if}
      <ul class="commands">
        {#each detail.commands as c, i (i)}
          <li>
            <details>
              <summary>
                {#if c.is_test}<span class="badge">{$t("assist.runs.test_badge")}</span>{/if}
                <span class="badge {c.ok === false ? 'bad' : c.ok ? 'good' : ''}">
                  {c.ok === false ? $t("assist.runs.cmd_failed") : c.ok ? $t("assist.runs.cmd_ok") : $t("assist.runs.cmd_unknown")}
                </span>
                <code class="path">{c.command}</code>
              </summary>
              {#if c.output}<pre class="pre">{c.output}</pre>{/if}
            </details>
          </li>
        {/each}
      </ul>
    {/if}

    {#if answered.length > 0}
      <div class="field-label">{$t("assist.runs.permissions_history")}</div>
      <ul class="plain">
        {#each answered as p (p.id)}
          <li>
            <span class="badge">{$t(`assist.runs.permission_state.${p.state}`)}</span>
            <span class="dim">{p.action}</span>
            <span class="dim">{shortTime(p.resolved_ms ?? p.created_ms)}</span>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="actions">
      {#if detail.actions.includes("continue")}
        <button type="button" class="button active" disabled={busy} onclick={() => act(() => resolveRun(detail!.id, "continue"))}>
          {$t("assist.runs.action_continue")}
        </button>
      {/if}
      {#if detail.actions.includes("mark_done")}
        <button type="button" class="button" disabled={busy} onclick={() => act(() => resolveRun(detail!.id, "mark_done"))}>
          {$t("assist.runs.action_mark_done")}
        </button>
      {/if}
      {#if detail.actions.includes("discard")}
        <button type="button" class="button" disabled={busy} onclick={() => act(() => resolveRun(detail!.id, "discard"))}>
          {$t("assist.runs.action_discard")}
        </button>
      {/if}
      {#if detail.actions.includes("cancel")}
        <button type="button" class="button" disabled={busy} onclick={() => act(() => cancelRun(detail!.id))}>
          {$t("assist.runs.action_cancel")}
        </button>
      {/if}
    </div>
    {#if detail.actions.includes("continue")}
      <p class="hint">{$t("assist.runs.continue_hint")}</p>
    {/if}

    <details class="advanced">
      <summary>{$t("assist.runs.advanced")}</summary>
      <dl>
        <dt>{$t("assist.runs.adv_runtime")}</dt><dd><code>{detail.runtime ?? "—"}</code></dd>
        <dt>{$t("assist.runs.adv_resume")}</dt><dd>{detail.resume_kind ? $t(`assist.runs.resume.${detail.resume_kind}`) : "—"}</dd>
        {#if detail.session}
          <dt>{$t("assist.runs.adv_session")}</dt>
          <dd><code>{detail.session.provider_handle ?? "—"}</code> · {$t("assist.runs.adv_generation", { count: detail.session.generation })}</dd>
          {#if detail.session.exe_version}<dt>{$t("assist.runs.adv_version")}</dt><dd><code>{detail.session.exe_version}</code></dd>{/if}
          <dt>{$t("assist.runs.adv_context")}</dt><dd>{detail.session.context_kind}</dd>
        {/if}
        <dt>{$t("assist.runs.adv_folder")}</dt><dd><code>{detail.cwd ?? "—"}</code></dd>
        <dt>{$t("assist.runs.adv_process")}</dt><dd><code>{detail.launch_id ?? "—"}</code>{#if detail.pid} · pid {detail.pid}{/if}</dd>
        <dt>{$t("assist.runs.adv_usage")}</dt>
        <dd>
          {detail.tokens_in} in · {detail.tokens_out} out ·
          {detail.cost_usd == null ? $t("assist.runs.cost_unknown") : `$${detail.cost_usd.toFixed(4)}`}
        </dd>
        <dt>{$t("assist.runs.adv_events")}</dt>
        <dd>{detail.events.length}{#if detail.events_dropped > 0} (+{detail.events_dropped} {$t("assist.runs.adv_dropped")}){/if}</dd>
      </dl>
      <pre class="pre">{detail.events.map((e) => `${e.seq} ${e.kind} ${JSON.stringify(e.payload)}`).join("\n")}</pre>
    </details>
  </div>
{/if}

<style>
  .run-detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3);
    min-width: 0;
  }

  .notice {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-size: var(--text-sm);
  }

  .notice.warn {
    background: color-mix(in srgb, var(--orange) 12%, transparent);
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .dim {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .text {
    font-size: var(--text-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .text.err {
    color: var(--red);
  }

  .pre {
    margin: 0;
    padding: var(--space-2);
    max-height: 260px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .ask {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--orange) 8%, transparent);
  }

  .ask-title {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .files,
  .commands,
  .plain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .files li,
  .plain li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    font-size: var(--text-sm);
  }

  .commands summary {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
    min-width: 0;
  }

  .commands summary:focus-visible,
  .patch summary:focus-visible,
  .advanced summary:focus-visible {
    outline: var(--focus-ring);
    border-radius: var(--radius-sm);
  }

  .path {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .badge {
    flex-shrink: 0;
    padding: 0 6px;
    border-radius: 999px;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    background: var(--fill-2);
  }

  .badge.good,
  .badge.st-A {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 14%, transparent);
  }

  .badge.bad,
  .badge.st-D {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 14%, transparent);
  }

  .plus {
    font-size: var(--text-xs);
    color: var(--green);
  }

  .minus {
    font-size: var(--text-xs);
    color: var(--red);
  }

  .patch summary,
  .advanced summary {
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--accent-text);
  }

  .advanced dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--space-1) var(--space-3);
    margin: var(--space-2) 0;
    font-size: var(--text-xs);
  }

  .advanced dt {
    color: var(--text-dim);
  }

  .advanced dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>
