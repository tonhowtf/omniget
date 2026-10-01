<script lang="ts">
  import { untrack } from "svelte";
  import DiffViewer from "$components/central/vcs/DiffViewer.svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import type { FileChange, Job } from "$lib/stores/llm-jobs-store.svelte";
  import type { ReviewComment } from "$lib/central/vcs";
  import {
    addNote,
    deleteNote,
    discardHunk,
    discardWork,
    errorCode,
    fetchHunks,
    fetchIntegratePreview,
    fetchNotes,
    fetchWalkthrough,
    hunkAt,
    integrateJob,
    noteFromComment,
    noteRange,
    sendNotes,
    viewerFiles,
    type HunksView,
    type IntegratePreview,
    type JobNote,
    type Walkthrough,
  } from "$lib/llm/job-review";

  let { job, files, onchanged }: { job: Job; files: FileChange[]; onchanged: () => void } = $props();

  let view = $state<HunksView | null>(null);
  let notes = $state<JobNote[]>([]);
  let preview = $state<IntegratePreview | null>(null);
  let walk = $state<Walkthrough | null>(null);
  let walkError = $state("");
  let walking = $state(false);
  let busy = $state(false);
  let strategy = $state<"merge" | "squash">("merge");

  const worktree = $derived(!!job.branch);
  const pending = $derived(notes.filter((n) => n.sent_ms == null));
  const viewer = $derived(view ? viewerFiles(files, view) : []);

  async function load() {
    const id = job.id;
    try {
      const [v, n] = await Promise.all([fetchHunks(id), fetchNotes(id)]);
      if (id !== job.id) return;
      view = v;
      notes = n;
    } catch (e) {
      showToast("error", String(e));
    }
    preview = null;
    if (job.branch) {
      try {
        const p = await fetchIntegratePreview(id);
        if (id === job.id) preview = p;
      } catch {
        preview = null;
      }
    }
  }

  $effect(() => {
    void job.id;
    void job.state_seq;
    untrack(() => void load());
  });

  function failed(e: unknown) {
    const code = errorCode(e);
    if (code === "ERR_HUNK_STALE") showToast("error", $t("llm.jobs.review.hunk_stale"));
    else if (code === "ERR_REVIEW_BUSY") showToast("error", $t("llm.jobs.review.busy"));
    else showToast("error", String(e));
  }

  async function act(run: () => Promise<void>) {
    if (busy) return;
    busy = true;
    try {
      await run();
    } catch (e) {
      failed(e);
    } finally {
      busy = false;
      await load();
      onchanged();
    }
  }

  function onHunk(path: string, index: number) {
    const h = view && hunkAt(view, path, index);
    if (!h) return;
    void act(async () => {
      await discardHunk(job.id, h);
      showToast("success", $t("llm.jobs.review.hunk_discarded"));
    });
  }

  function hunkEnabled(path: string, index: number): boolean {
    const h = view && hunkAt(view, path, index);
    return !!(view?.actionable && h?.complete && !busy);
  }

  async function onComment(c: ReviewComment) {
    if (!c.text.trim()) return;
    try {
      const n = await addNote(job.id, noteFromComment(c));
      notes = [...notes, n];
    } catch (e) {
      failed(e);
    }
  }

  async function removeNote(n: JobNote) {
    try {
      await deleteNote(job.id, n.id);
      notes = notes.filter((x) => x.id !== n.id);
    } catch (e) {
      failed(e);
    }
  }

  function send() {
    void act(async () => {
      const next = await sendNotes(job.id);
      showToast("success", $t("llm.jobs.review.notes_sent", { job: next.id }));
    });
  }

  function integrate() {
    void act(async () => {
      const { outcome } = await integrateJob(job.id, strategy);
      const target = outcome.target_branch;
      if (outcome.status === "merged") showToast("success", $t("llm.jobs.review.integrated", { target }));
      else if (outcome.status === "up_to_date") showToast("success", $t("llm.jobs.review.up_to_date", { target }));
      else if (outcome.status === "conflict")
        showToast("error", $t("llm.jobs.review.conflict", { files: outcome.conflicts.join(", ") }));
      else showToast("error", $t("llm.jobs.review.failed"));
    });
  }

  function discard() {
    if (!confirm($t("llm.jobs.review.discard_confirm", { branch: job.branch ?? "" }) as string)) return;
    void act(async () => {
      const out = await discardWork(job.id);
      showToast("success", $t("llm.jobs.review.discarded", { branch: out.branch, tip: (out.tip ?? "").slice(0, 7) }));
    });
  }

  async function explain(refresh = false) {
    walking = true;
    walkError = "";
    try {
      walk = await fetchWalkthrough(job.id, refresh);
    } catch (e) {
      walk = null;
      walkError =
        errorCode(e) === "ERR_WALKTHROUGH_TOO_LARGE" ? ($t("llm.jobs.review.walkthrough_too_large") as string) : String(e);
    } finally {
      walking = false;
    }
  }

  function hunkPath(id: string): string {
    return view?.hunks.find((h) => h.id === id)?.path ?? id.split(":")[0];
  }
</script>

<section class="review" aria-label={$t("llm.jobs.review.title")}>
  <div class="field-label">{$t("llm.jobs.review.title")}</div>
  <p class="hint">{$t("llm.jobs.review.lede")}</p>
  {#if view && !view.actionable && worktree && !job.worktree_path}
    <p class="hint">{$t("llm.jobs.review.not_actionable")}</p>
  {/if}

  {#if view}
    <DiffViewer
      files={viewer}
      height="420px"
      collapsedByDefault={viewer.length > 8}
      oncomment={onComment}
      onhunk={view.actionable ? onHunk : undefined}
      hunkLabel={$t("llm.jobs.review.discard_hunk") as string}
      {hunkEnabled}
    />
  {/if}

  <div class="block">
    <div class="row">
      <span class="sub">{$t("llm.jobs.review.notes_title")}</span>
      <span class="grow"></span>
      <button type="button" class="button active" disabled={busy || pending.length === 0} onclick={send}>
        {$t("llm.jobs.review.notes_send", { count: pending.length })}
      </button>
    </div>
    <p class="hint">{$t("llm.jobs.review.notes_hint")}</p>
    {#if notes.length === 0}
      <p class="hint">{$t("llm.jobs.review.notes_empty")}</p>
    {:else}
      <ul class="notes">
        {#each notes as n (n.id)}
          <li class:sent={n.sent_ms != null}>
            <code class="path">{n.path}</code>
            <span class="dim">{n.start_line == null ? $t("llm.jobs.review.note_file") : noteRange(n)}</span>
            <span class="body">{n.body}</span>
            {#if n.sent_ms != null}
              <span class="dim">{$t("llm.jobs.review.note_sent", { job: n.sent_job ?? "" })}</span>
            {:else}
              <button type="button" class="link" aria-label={$t("llm.jobs.review.note_delete")} onclick={() => void removeNote(n)}>×</button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="block">
    <div class="row">
      <span class="sub">{$t("llm.jobs.review.walkthrough_title")}</span>
      <span class="grow"></span>
      <button type="button" class="button" disabled={walking} onclick={() => void explain(!!walk)}>
        {walking
          ? $t("llm.jobs.review.walkthrough_loading")
          : walk
            ? $t("llm.jobs.review.walkthrough_refresh")
            : $t("llm.jobs.review.walkthrough")}
      </button>
    </div>
    {#if walkError}<p class="hint err">{walkError}</p>{/if}
    {#if walk}
      {#if walk.raw}
        <p class="text">{walk.raw}</p>
      {:else}
        {#if walk.title}<p class="text strong">{walk.title}</p>{/if}
        {#if walk.focus}<p class="hint">{walk.focus}</p>{/if}
        <ol class="stops">
          {#each walk.stops as s, i (i)}
            <li class="imp-{s.importance}">
              <span class="strong">{s.title}</span>
              {#if s.importance === "critical"}<span class="badge">{$t("llm.jobs.review.importance_critical")}</span>{/if}
              <p class="text">{s.prose}</p>
              <span class="dim">{[...new Set(s.hunks.map(hunkPath))].join(", ")}</span>
            </li>
          {/each}
        </ol>
      {/if}
      {#if walk.uncovered.length}
        <p class="hint">{$t("llm.jobs.review.walkthrough_uncovered", { count: walk.uncovered.length })}</p>
      {/if}
      <p class="hint">{walk.model}{walk.cached ? ` · ${$t("llm.jobs.review.walkthrough_cached")}` : ""}</p>
    {/if}
  </div>

  {#if worktree}
    <div class="block">
      <div class="sub">{$t("llm.jobs.review.integrate_title")}</div>
      {#if preview}
        <p class="hint">
          {$t("llm.jobs.review.integrate_into", { branch: preview.branch, target: preview.target_branch ?? "?" })}
          · {$t("llm.jobs.review.commits", { count: preview.commits.length })}
        </p>
        {#if preview.worktree_dirty.length}
          <p class="hint">{$t("llm.jobs.review.worktree_dirty", { count: preview.worktree_dirty.length })}</p>
        {/if}
        {#if preview.blocker}<p class="hint err">{$t("llm.jobs.review.blocked", { reason: preview.blocker })}</p>{/if}
      {/if}
      <div class="row">
        <select class="input" bind:value={strategy} aria-label={$t("llm.jobs.review.integrate_title")}>
          <option value="merge">{$t("llm.jobs.review.strategy_merge")}</option>
          <option value="squash">{$t("llm.jobs.review.strategy_squash")}</option>
        </select>
        <button type="button" class="button active" disabled={busy || !!preview?.blocker} onclick={integrate}>
          {$t("llm.jobs.review.integrate")}
        </button>
        <button type="button" class="button" disabled={busy} onclick={discard}>
          {$t("llm.jobs.review.discard_work")}
        </button>
      </div>
    </div>
  {/if}
</section>

<style>
  .review {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .block {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-top: var(--space-2);
    border-top: 1px solid var(--separator);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .grow {
    flex: 1;
  }

  .sub,
  .strong {
    font-weight: 600;
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .err {
    color: var(--red);
  }

  .text {
    margin: 0;
    white-space: pre-wrap;
  }

  .dim {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .notes,
  .stops {
    margin: 0;
    padding-left: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .notes {
    list-style: none;
    padding-left: 0;
  }

  .notes li {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    flex-wrap: wrap;
  }

  .notes li.sent {
    opacity: 0.7;
  }

  .body {
    flex: 1;
    min-width: 12ch;
    white-space: pre-wrap;
  }

  .path {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }

  .link {
    background: none;
    border: 0;
    color: var(--text-muted);
    cursor: pointer;
    font-size: var(--text-sm);
  }

  .badge {
    margin-left: var(--space-1);
    padding: 0 6px;
    border-radius: 6px;
    font-size: var(--text-xs);
    color: var(--red);
    background: color-mix(in srgb, var(--red) 12%, transparent);
  }
</style>
