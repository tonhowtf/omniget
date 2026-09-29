<script lang="ts">
  /**
   * What a mission task changed (2.4, R6): the files and diff of each job
   * the task ran, read from the job's recorded diff (`llm_job_diff`).
   */
  import { t } from "$lib/i18n";
  import DiffView from "$components/llm/activity/DiffView.svelte";
  import { diffReasonKey, fetchJobDiff, type JobDiffView } from "$lib/stores/llm-jobs-store.svelte";

  let { jobIds }: { jobIds: string[] } = $props();

  let diffs = $state<{ id: string; diff: JobDiffView | null }[]>([]);
  let loading = $state(false);

  async function load(ids: string[]) {
    loading = true;
    diffs = await Promise.all(ids.map(async (id) => ({ id, diff: await fetchJobDiff(id) })));
    loading = false;
  }

  $effect(() => {
    void load(jobIds);
  });
</script>

<div class="changes">
  {#if jobIds.length === 0}
    <p class="hint">{$t("mission.detail.changes_no_job")}</p>
  {:else if loading && diffs.length === 0}
    <p class="hint" role="status">{$t("llm.jobs.diff.loading")}</p>
  {:else}
    {#each diffs as d (d.id)}
      {#if jobIds.length > 1}<div class="job">{$t("mission.detail.changes_job", { job: d.id })}</div>{/if}
      {#if !d.diff}
        <p class="hint">{$t("llm.jobs.diff.unavailable_error")}</p>
      {:else if !d.diff.available}
        <p class="hint">{$t(diffReasonKey(d.diff.reason))}</p>
      {:else if d.diff.files.length === 0}
        <p class="hint">{$t("llm.jobs.diff.none")}</p>
      {:else}
        <p class="hint">
          {$t("llm.jobs.diff.totals", { files: d.diff.file_count, additions: d.diff.additions, deletions: d.diff.deletions })}
        </p>
        <ul class="files">
          {#each d.diff.files as f (f.path)}
            <li>
              <span class="badge">{$t(`assist.runs.file_status.${f.status}`)}</span>
              <code class="path" title={f.path}>{f.path}</code>
              {#if f.binary}
                <span class="dim">{$t("llm.jobs.diff.binary")}</span>
              {:else}
                {#if f.additions != null}<span class="plus">+{f.additions}</span>{/if}
                {#if f.deletions != null}<span class="minus">−{f.deletions}</span>{/if}
              {/if}
            </li>
          {/each}
        </ul>
        {#if d.diff.patch}
          <details class="patch">
            <summary>{$t("assist.runs.show_diff")}</summary>
            <DiffView patch={d.diff.patch} label={$t("llm.jobs.diff.title")} />
          </details>
        {/if}
        {#if d.diff.clipped}<p class="hint">{$t("assist.runs.diff_clipped")}</p>{/if}
      {/if}
    {/each}
  {/if}
</div>

<style>
  .changes { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
  .hint { margin: 0; font-size: var(--text-xs); color: var(--text-dim); }
  .job { font-size: var(--text-xs); font-weight: 600; color: var(--text-muted); }
  .files { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .files li { display: flex; align-items: center; gap: var(--space-2); min-width: 0; font-size: var(--text-xs); }
  .badge { flex-shrink: 0; padding: 0 6px; border-radius: var(--radius-sm); background: var(--fill-2); color: var(--text-muted); }
  .path { font-family: var(--font-mono); color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .dim { color: var(--text-dim); }
  .plus { color: var(--green); font-family: var(--font-mono); }
  .minus { color: var(--red); font-family: var(--font-mono); }
  .patch summary { cursor: pointer; font-size: var(--text-xs); color: var(--text-muted); }
</style>
