<script lang="ts">
  /**
   * Review of a sandboxed job: what the agent changed in the copy, file by
   * file, and "apply" for the ones picked. A file the user edited in the
   * project meanwhile is a conflict and is never overwritten.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText } from "$lib/central/catalog";
  import {
    runSandboxApply,
    runSandboxDiff,
    runSandboxDiscard,
    type SandboxChange,
    type SandboxRecord,
  } from "./run-api";

  let { jobId, finished = true }: { jobId: string; finished?: boolean } = $props();

  let record = $state<SandboxRecord | null>(null);
  let changes = $state<SandboxChange[]>([]);
  let picked = $state<string[]>([]);
  let error = $state("");
  let busy = $state(false);
  let gone = $state(false);

  async function load() {
    error = "";
    try {
      const r = await runSandboxDiff(jobId);
      record = r.record;
      changes = r.changes;
      picked = r.changes.filter((c) => !c.conflict && !r.record.applied.includes(c.path)).map((c) => c.path);
    } catch (e) {
      error = errText(e);
    }
  }

  $effect(() => {
    if (finished && jobId) void load();
  });

  function toggle(p: string) {
    picked = picked.includes(p) ? picked.filter((x) => x !== p) : [...picked, p];
  }

  async function apply() {
    busy = true;
    try {
      const r = await runSandboxApply(jobId, picked);
      showToast("success", $t("llm.central.run.sandbox.applied", { count: String(r.applied.length) }));
      if (r.skipped.length) showToast("info", $t("llm.central.run.sandbox.skipped", { list: r.skipped.join(", ") }));
      await load();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function discard() {
    busy = true;
    try {
      await runSandboxDiscard(jobId);
      gone = true;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function lineClass(l: string): string {
    if (l.startsWith("+++") || l.startsWith("---")) return "meta";
    if (l.startsWith("+")) return "add";
    if (l.startsWith("-")) return "del";
    if (l.startsWith("@@")) return "hunk";
    return "";
  }
</script>

<section class="review">
  <div class="field-label">{$t("llm.central.run.sandbox.review")}</div>
  {#if gone}
    <p class="hint">{$t("llm.central.run.sandbox.discarded")}</p>
  {:else if !finished}
    <p class="hint">{$t("llm.central.run.sandbox.running")}</p>
  {:else if error}
    <p class="err">{error}</p>
  {:else if record}
    <p class="hint mono">{record.provider} · {record.image ?? ""} · {record.env_names.length ? record.env_names.join(", ") : $t("llm.central.run.sandbox.no_env")}</p>
    {#if record.bind_original}
      <p class="hint">{$t("llm.central.run.sandbox.bind_hint")}</p>
    {:else if changes.length === 0}
      <p class="hint">{$t("llm.central.run.sandbox.no_changes")}</p>
    {:else}
      <ul class="files">
        {#each changes as c (c.path)}
          <li>
            <label class="head">
              <input type="checkbox" checked={picked.includes(c.path)} disabled={c.conflict || record.applied.includes(c.path)} onchange={() => toggle(c.path)} />
              <span class="st {c.status}">{$t(`llm.central.run.sandbox.status.${c.status}`)}</span>
              <code class="mono">{c.path}</code>
              {#if c.conflict}<span class="warn">{$t("llm.central.run.sandbox.conflict")}</span>{/if}
              {#if record.applied.includes(c.path)}<span class="ok">{$t("llm.central.run.sandbox.done")}</span>{/if}
            </label>
            {#if c.diff}
              <details>
                <summary class="hint">{$t("llm.central.run.sandbox.show_diff")}</summary>
                <pre class="diff">{#each c.diff.split("\n") as l, i (i)}<span class={lineClass(l)}>{l}
</span>{/each}</pre>
              </details>
            {:else if c.binary}
              <span class="hint">{$t("llm.central.run.sandbox.binary")}</span>
            {/if}
          </li>
        {/each}
      </ul>
      <div class="actions">
        <button type="button" class="button active" disabled={busy || picked.length === 0} onclick={apply}>
          {$t("llm.central.run.sandbox.apply", { count: String(picked.length) })}
        </button>
        <button type="button" class="button" disabled={busy} onclick={discard}>{$t("llm.central.run.sandbox.discard")}</button>
      </div>
    {/if}
  {/if}
</section>

<style>
  .review {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .st {
    font-size: var(--text-xs);
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
  }
  .st.added {
    color: var(--green);
  }
  .st.deleted {
    color: var(--red);
  }
  .st.modified {
    color: var(--blue);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .warn {
    font-size: var(--text-xs);
    color: var(--warning);
  }
  .ok {
    font-size: var(--text-xs);
    color: var(--green);
  }
  .err {
    margin: 0;
    color: var(--red);
    font-size: var(--text-sm);
  }
  .diff {
    margin: 4px 0 0;
    padding: var(--space-2);
    max-height: 360px;
    overflow: auto;
    background: var(--fill-2);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    user-select: text;
  }
  .diff .add {
    color: var(--green);
  }
  .diff .del {
    color: var(--red);
  }
  .diff .hunk,
  .diff .meta {
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
