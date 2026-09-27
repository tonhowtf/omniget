<script lang="ts">
  /**
   * Importar uma sessão exportada (JSON neutro do OmniGet). Destino Claude ou
   * Codex grava uma sessão retomável com id novo; outro destino devolve o
   * Markdown de contexto para colar na ferramenta.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import Modal from "./Modal.svelte";
  import {
    copyText,
    errText,
    NATIVE_IMPORT,
    pickJsonFile,
    saveTextAs,
    sessionsImport,
    TOOL_NAMES,
    toolName,
    type ImportOutcome,
  } from "$lib/central/sessions";

  interface Props {
    onclose: () => void;
    ondone?: () => void;
  }

  let { onclose, ondone }: Props = $props();

  let path = $state("");
  let target = $state("");
  let busy = $state(false);
  let out = $state<ImportOutcome | null>(null);
  let error = $state("");

  async function pick() {
    const p = await pickJsonFile();
    if (p) path = p;
  }

  async function run() {
    if (!path) return;
    busy = true;
    error = "";
    try {
      out = await sessionsImport(path, target || null);
      if (out.written) ondone?.();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function copy(s: string) {
    await copyText(s);
    showToast("success", $t("llm.central.sessions.copied"));
  }
</script>

<Modal title={$t("llm.central.sessions.import.title")} {onclose}>
  <div class="form">
    <p class="hint">{$t("llm.central.sessions.import.hint")}</p>
    <div class="row">
      <input class="input" readonly value={path} placeholder={$t("llm.central.sessions.import.no_file")} aria-label={$t("llm.central.sessions.import.file")} />
      <button type="button" class="btn btn-secondary" onclick={pick}>{$t("llm.central.sessions.import.choose")}</button>
    </div>
    <label class="field">
      <span class="field-label">{$t("llm.central.sessions.import.target")}</span>
      <select class="input" bind:value={target}>
        <option value="">{$t("llm.central.sessions.import.same_tool")}</option>
        {#each Object.keys(TOOL_NAMES) as id (id)}
          <option value={id}>{toolName(id)}{NATIVE_IMPORT.includes(id) ? ` · ${$t("llm.central.sessions.import.resumable")}` : ` · ${$t("llm.central.sessions.import.as_context")}`}</option>
        {/each}
      </select>
    </label>
    {#if error}<p class="err">{error}</p>{/if}
    {#if out}
      <div class="result surface-card">
        <p><strong>{toolName(out.source_tool)}</strong> → <strong>{toolName(out.target_tool)}</strong></p>
        {#if out.written}
          <p class="mono">{out.written}</p>
        {/if}
        {#if out.resume_command}
          <div class="cmd"><code>{out.resume_command}</code><button type="button" class="btn btn-sm" onclick={() => copy(out!.resume_command!)}>{$t("llm.central.sessions.copy")}</button></div>
        {/if}
        {#if out.context}
          <p>{$t("llm.central.sessions.import.context_ready")} ({out.context.turns_included}/{out.context.turns_total})</p>
          <div class="btn-row">
            <button type="button" class="btn btn-sm" onclick={() => copy(out!.context!.content)}>{$t("llm.central.sessions.copy")}</button>
            <button type="button" class="btn btn-sm" onclick={() => saveTextAs(out!.context!.filename, out!.context!.content, "md")}>{$t("llm.central.sessions.save_as")}</button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
  {#snippet actions()}
    <button type="button" class="btn btn-primary" onclick={run} disabled={!path || busy}>
      {busy ? $t("llm.central.sessions.working") : $t("llm.central.sessions.import.run")}
    </button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .hint {
    margin: 0;
    font-size: var(--text-sm);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .row input {
    flex: 1;
    min-width: 0;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-sm);
  }
  .result {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--text-sm);
  }
  .result p {
    margin: 0;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }
  .cmd {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .cmd code {
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    white-space: nowrap;
    padding: 4px 6px;
    background: var(--fill-1);
    border-radius: var(--radius-sm);
  }
</style>
