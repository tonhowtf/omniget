<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import type { CommandRecord, DownloadStatus } from "$lib/stores/download-store.svelte";

  type Props = { id: number; command: CommandRecord | null | undefined; status: DownloadStatus };
  let { id, command, status }: Props = $props();

  // Aberto por padrão só em erro: é quando o usuário precisa ver o comando.
  let open = $state(false);
  let openedFor = $state<number | null>(null);
  $effect(() => {
    if (status === "error" && openedFor !== id) {
      open = true;
      openedFor = id;
    }
  });

  let editing = $state(false);
  let text = $state("");
  let running = $state(false);
  let textarea: HTMLTextAreaElement | null = $state(null);

  let canEdit = $derived(!!command && (status === "error" || status === "complete"));
  let engineLabel = $derived.by(() => {
    if (!command) return "";
    if (command.overridden) return $t("downloads.command.engine_custom") as string;
    return command.engine;
  });

  async function copy() {
    if (!command) return;
    try {
      await navigator.clipboard.writeText(command.display);
      showToast("success", $t("downloads.command.copied"));
    } catch {
      showToast("error", $t("downloads.log.copy_failed"));
    }
  }

  function startEdit() {
    if (!command) return;
    text = command.display;
    editing = true;
    open = true;
    queueMicrotask(() => textarea?.focus());
  }

  function cancelEdit() {
    editing = false;
  }

  async function run() {
    const cmd = text.trim();
    if (!cmd || running) return;
    running = true;
    try {
      await invoke("retry_download_with_command", { downloadId: id, command: cmd });
      showToast("info", $t("downloads.command.requeued"));
      editing = false;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e?.message ?? String(e);
      showToast("error", $t("downloads.command.invalid", { error: msg }));
    } finally {
      running = false;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      void run();
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancelEdit();
    }
  }
</script>

{#if command}
  <div class="dl-command">
    <button
      type="button"
      class="cmd-toggle"
      onclick={() => (open = !open)}
      aria-expanded={open}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <polyline points="4 17 10 11 4 5" />
        <line x1="12" y1="19" x2="20" y2="19" />
      </svg>
      <span>{$t("downloads.command.title")}</span>
      <span class="cmd-chip">{engineLabel}</span>
      {#if command.max_attempts > 1 && command.attempt > 1}
        <span class="cmd-chip">{$t("downloads.detail.attempt", { attempt: String(command.attempt), max: String(command.max_attempts) })}</span>
      {/if}
    </button>

    {#if open}
      <div class="cmd-panel">
        {#if editing}
          <p class="cmd-hint">{$t("downloads.command.hint")}</p>
          <textarea
            bind:this={textarea}
            bind:value={text}
            class="cmd-editor"
            rows="6"
            spellcheck="false"
            autocapitalize="off"
            autocomplete="off"
            onkeydown={onKeydown}
            aria-label={$t("downloads.command.title")}
          ></textarea>
          <div class="cmd-actions">
            <button type="button" class="btn btn-primary btn-sm" onclick={run} disabled={running || !text.trim()}>
              {#if running}<span class="spinner"></span>{/if}
              {$t("downloads.command.run")}
            </button>
            <button type="button" class="btn btn-secondary btn-sm" onclick={cancelEdit} disabled={running}>
              {$t("downloads.command.cancel")}
            </button>
          </div>
        {:else}
          <pre class="cmd-code"><code>{command.display}</code></pre>
          <div class="cmd-actions">
            <button type="button" class="btn btn-secondary btn-sm" onclick={copy}>{$t("downloads.command.copy")}</button>
            {#if canEdit}
              <button type="button" class="btn btn-secondary btn-sm" onclick={startEdit}>{$t("downloads.command.edit")}</button>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .dl-command {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .cmd-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 6px;
    margin-left: -6px;
    background: transparent;
    border: none;
    border-radius: var(--radius-xs);
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    align-self: flex-start;
    transition: background-color var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }

  @media (hover: hover) {
    .cmd-toggle:hover {
      background: var(--fill-2);
      color: var(--text);
    }
  }

  .cmd-chip {
    display: inline-flex;
    align-items: center;
    height: 16px;
    padding: 0 5px;
    border-radius: 4px;
    background: var(--fill-1);
    color: var(--text-muted);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  .cmd-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
    border-radius: var(--border-radius);
    background: var(--popup-bg);
  }

  .cmd-code {
    margin: 0;
    padding: 8px 10px;
    max-height: 160px;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.55;
    color: var(--secondary);
    background: var(--input-bg);
    border-radius: var(--radius-sm);
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }

  .cmd-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    line-height: 1.45;
  }

  .cmd-editor {
    width: 100%;
    min-height: 96px;
    padding: 8px 10px;
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.55;
    color: var(--text);
    background: var(--input-bg);
    border: var(--hairline) solid var(--input-border);
    border-radius: var(--radius-sm);
    resize: vertical;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .cmd-editor:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }

  .cmd-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
