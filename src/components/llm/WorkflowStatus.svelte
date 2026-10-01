<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type RepoCommand = { kind: "after_create" | "before_remove" | "check"; command: string };
  type WorkflowConfig = {
    agent?: string;
    run_mode?: string;
    isolation?: string;
    base?: string;
    deadline_min?: number;
    budget_usd?: number;
    model?: string;
    effort?: string;
    include?: string[];
  };
  export type WorkflowStatusView = {
    root: string;
    path: string;
    state: "none" | "active" | "invalid";
    error?: string;
    active?: { config: WorkflowConfig; template: string; sha256: string };
    from_last_good: boolean;
    trusted: boolean;
    trusted_sha256?: string;
    commands: RepoCommand[];
    gated: string[];
    skipped_reason?: string | null;
  };

  let {
    folder,
    ondefaults,
  }: {
    folder: string;
    ondefaults?: (config: WorkflowConfig, trusted: boolean) => void;
  } = $props();

  let status = $state<WorkflowStatusView | null>(null);
  let busy = $state(false);
  let showTemplate = $state(false);

  async function load(dir: string) {
    try {
      const next = await invoke<WorkflowStatusView>("llm_workflow_status", { folder: dir });
      if (dir !== folder) return;
      status = next;
      if (next.active) ondefaults?.(next.active.config, next.trusted);
    } catch (e) {
      status = null;
      showToast("error", String(e));
    }
  }

  $effect(() => {
    status = null;
    void load(folder);
  });

  async function setTrust(revoke: boolean) {
    if (!status?.active) return;
    busy = true;
    try {
      status = await invoke<WorkflowStatusView>("llm_workflow_trust", {
        folder,
        sha256: status.active.sha256,
        revoke,
      });
      showToast("success", $t(revoke ? "llm.jobs.workflow.untrusted_toast" : "llm.jobs.workflow.trusted_toast") as string);
    } catch (e) {
      showToast("error", String(e));
      void load(folder);
    } finally {
      busy = false;
    }
  }

  const tone = $derived(
    !status || status.state === "none"
      ? "none"
      : status.state === "invalid"
        ? "invalid"
        : status.trusted || status.gated.length === 0
          ? "trusted"
          : "untrusted",
  );
  const defaults = $derived(
    status?.active
      ? Object.entries(status.active.config).filter(
          ([k, v]) => !["hooks", "checks"].includes(k) && v !== null && v !== undefined && !(Array.isArray(v) && v.length === 0),
        )
      : [],
  );
</script>

{#if status && status.state !== "none"}
  <section class="workflow" aria-label={$t("llm.jobs.workflow.title")}>
    <div class="head">
      <span class="field-label">{$t("llm.jobs.workflow.title")}</span>
      <span class="chip {tone}">{$t(`llm.jobs.workflow.state.${tone}`)}</span>
      {#if status.active}
        <code class="sha" title={status.active.sha256}>{status.active.sha256.slice(0, 12)}</code>
      {/if}
    </div>

    {#if status.state === "invalid"}
      <p class="error" role="alert">{status.error}</p>
      <p class="hint">
        {status.from_last_good ? $t("llm.jobs.workflow.last_good") : $t("llm.jobs.workflow.no_last_good")}
      </p>
    {/if}

    {#if status.active}
      {#if defaults.length}
        <p class="hint">
          {$t("llm.jobs.workflow.defaults")}
          {#each defaults as [key, value] (key)}
            <code class="kv">{key}: {Array.isArray(value) ? value.join(", ") : value}</code>
          {/each}
        </p>
      {/if}
      {#if status.commands.length}
        <div class="commands">
          <span class="hint">{$t("llm.jobs.workflow.commands")}</span>
          <ul>
            {#each status.commands as c, i (i)}
              <li><span class="kind">{$t(`llm.jobs.workflow.kind.${c.kind}`)}</span> <code>{c.command}</code></li>
            {/each}
          </ul>
        </div>
      {/if}
      {#if !status.trusted && status.gated.length}
        <p class="hint">{$t("llm.jobs.workflow.untrusted_hint")}</p>
      {/if}
      <div class="actions">
        {#if status.gated.length && !status.trusted}
          <button type="button" class="button active" disabled={busy} onclick={() => void setTrust(false)}>
            {$t("llm.jobs.workflow.trust")}
          </button>
        {:else if status.trusted}
          <button type="button" class="button" disabled={busy} onclick={() => void setTrust(true)}>
            {$t("llm.jobs.workflow.revoke")}
          </button>
        {/if}
        <button type="button" class="button" onclick={() => (showTemplate = !showTemplate)}>
          {$t("llm.jobs.workflow.template")}
        </button>
      </div>
      {#if showTemplate}
        <pre class="template">{status.active.template}</pre>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .workflow {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
  }
  .head,
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .chip {
    font-size: var(--text-xs);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-2);
  }
  .chip.trusted {
    color: var(--green);
  }
  .chip.untrusted {
    color: var(--orange);
  }
  .chip.invalid {
    color: var(--red);
  }
  .sha,
  .kv {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .kv {
    margin-left: var(--space-1);
  }
  .hint,
  .error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .error {
    color: var(--red);
  }
  .commands ul {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-4);
    font-size: var(--text-sm);
  }
  .commands code {
    font-family: var(--font-mono);
    overflow-wrap: anywhere;
  }
  .kind {
    color: var(--text-muted);
  }
  .template {
    margin: 0;
    padding: var(--space-2);
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 240px;
    overflow: auto;
  }
</style>
