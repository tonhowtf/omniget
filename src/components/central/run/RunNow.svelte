<script lang="ts">
  /**
   * "Run now": one Job with a catalog agent (or command) as the role and the
   * runner the user picks. A coding-CLI runner can go into OmniGet's sandbox
   * (Docker on a copy of the project, or E2B), with the key from the vault
   * in the child's environment and the result as a diff to review.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import ProjectPicker from "$components/central/catalog/ProjectPicker.svelte";
  import { errText } from "$lib/central/catalog";
  import RunDialog from "./RunDialog.svelte";
  import RunnerPicker from "./RunnerPicker.svelte";
  import SandboxOptions from "./SandboxOptions.svelte";
  import { runAgentNow, type RunnerSpec, type SandboxOpts } from "./run-api";

  let {
    agentId = null,
    itemName = "",
    workspace: initialWorkspace = null,
    label = "",
    onsubmitted,
  }: {
    agentId?: string | null;
    itemName?: string;
    workspace?: string | null;
    label?: string;
    onsubmitted?: (jobId: string) => void;
  } = $props();

  let open = $state(false);
  let runner = $state<RunnerSpec>({ kind: "agent", id: "omni" });
  let workspace = $state<string | null>(null);
  let prompt = $state("");
  let sandbox = $state<SandboxOpts | null>(null);
  let busy = $state(false);
  let error = $state("");

  function show() {
    workspace = initialWorkspace;
    error = "";
    open = true;
  }

  async function submit() {
    busy = true;
    error = "";
    try {
      const job = await runAgentNow({
        agent: agentId,
        runnerSpec: runner,
        prompt: prompt.trim(),
        workspace,
        sandbox: runner.kind === "tool" ? sandbox : null,
      });
      showToast("success", $t("llm.central.run.now.submitted"));
      onsubmitted?.(job.id);
      open = false;
      prompt = "";
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<button type="button" class="button" onclick={show}>{label || $t("llm.central.run.now.button")}</button>

{#if open}
  <RunDialog title={$t("llm.central.run.now.title")} subtitle={itemName || agentId || ""} {busy} onclose={() => (open = false)}>
    <RunnerPicker bind:value={runner} />
    <div class="field">
      <span class="field-label">{$t("llm.central.run.project")}</span>
      <ProjectPicker bind:value={workspace} allowNone />
    </div>
    <label class="field">
      <span class="field-label">{$t("llm.central.run.now.prompt")}</span>
      <textarea class="input" rows="4" bind:value={prompt} placeholder={$t("llm.central.run.now.prompt_placeholder")}></textarea>
    </label>
    {#if runner.kind === "tool"}
      <SandboxOptions tool={runner.id} bind:value={sandbox} hasProject={!!workspace} />
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
    {#snippet footer()}
      <button type="button" class="button" disabled={busy} onclick={() => (open = false)}>{$t("llm.central.run.cancel")}</button>
      <button type="button" class="button active" disabled={busy || !prompt.trim() || (!!sandbox && !workspace)} onclick={submit}>
        {$t("llm.central.run.now.start")}
      </button>
    {/snippet}
  </RunDialog>
{/if}

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  textarea.input {
    resize: vertical;
    height: auto;
    font-family: inherit;
  }
  .err {
    margin: 0;
    color: var(--red);
    font-size: var(--text-sm);
  }
</style>
