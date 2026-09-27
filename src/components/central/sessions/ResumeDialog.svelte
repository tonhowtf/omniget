<script lang="ts">
  /**
   * Retomar: o comando certo da ferramenta (vem do parser, com `cd` para o
   * projeto quando a ferramenta precisa) e, quando a Central tem o comando
   * das threads, "abrir como thread" no OmniGet.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import Modal from "./Modal.svelte";
  import { copyText, errText, importAsThread, sessionsResumeCommand, toolName } from "$lib/central/sessions";

  interface Props {
    tool: string;
    id: string;
    command: string | null;
    onclose: () => void;
  }

  let { tool, id, command, onclose }: Props = $props();

  let cmd = $state<string | null>(null);
  let loaded = $state(false);
  let busy = $state(false);
  let threadUnavailable = $state(false);

  $effect(() => {
    cmd = command;
    if (command) {
      loaded = true;
      return;
    }
    sessionsResumeCommand(tool, id)
      .then((c) => (cmd = c))
      .catch(() => (cmd = null))
      .finally(() => (loaded = true));
  });

  async function copy() {
    if (!cmd) return;
    await copyText(cmd);
    showToast("success", $t("llm.central.sessions.copied"));
  }

  async function asThread() {
    busy = true;
    try {
      const r = await importAsThread(tool, id);
      if (r === null) {
        threadUnavailable = true;
        return;
      }
      showToast("success", $t("llm.central.sessions.resume.thread_created"));
      onclose();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={$t("llm.central.sessions.resume.title")} {onclose}>
  <div class="form">
    {#if !loaded}
      <p>{$t("llm.central.sessions.loading")}</p>
    {:else if cmd}
      <p class="hint">{$t("llm.central.sessions.resume.hint")} {toolName(tool)}.</p>
      <div class="cmd">
        <code>{cmd}</code>
        <button type="button" class="btn btn-primary btn-sm" onclick={copy}>{$t("llm.central.sessions.copy")}</button>
      </div>
    {:else}
      <p class="hint">{$t("llm.central.sessions.resume.none")}</p>
    {/if}
    <hr />
    <p class="hint">{$t("llm.central.sessions.resume.thread_hint")}</p>
    <div>
      <button type="button" class="btn btn-secondary" onclick={asThread} disabled={busy || threadUnavailable}>{$t("llm.central.sessions.resume.as_thread")}</button>
      {#if threadUnavailable}<p class="muted">{$t("llm.central.sessions.resume.thread_unavailable")}</p>{/if}
    </div>
  </div>
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
    padding: 6px 8px;
    background: var(--fill-1);
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    color: var(--text);
  }
  hr {
    width: 100%;
    margin: 0;
  }
  .muted {
    margin: 6px 0 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
