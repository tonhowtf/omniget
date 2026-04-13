<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Props = { id: number };
  let { id }: Props = $props();

  let expanded = $state(false);
  let lines = $state<string[]>([]);
  let loading = $state(false);
  let preEl: HTMLPreElement | null = $state(null);
  let atBottom = $state(true);
  let unlisten: UnlistenFn | null = null;

  async function refresh() {
    try {
      const next = await invoke<string[]>("get_download_log", { downloadId: id });
      lines = next;
    } catch (_) {
      lines = [];
    }
  }

  async function toggle() {
    expanded = !expanded;
    if (expanded) {
      loading = true;
      await refresh();
      loading = false;
      atBottom = true;
      unlisten = await listen<{ id: number }>("download-log-update", (event) => {
        if (event.payload && event.payload.id === id) {
          refresh();
        }
      });
    } else if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  $effect(() => {
    if (expanded && atBottom && preEl && lines.length) {
      queueMicrotask(() => {
        if (preEl) preEl.scrollTop = preEl.scrollHeight;
      });
    }
  });

  function onScroll() {
    if (!preEl) return;
    const threshold = 12;
    atBottom = preEl.scrollHeight - preEl.scrollTop - preEl.clientHeight <= threshold;
  }

  async function copyLog() {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      showToast("success", $t("downloads.log.copied"));
    } catch (_) {
      showToast("error", $t("downloads.log.copy_failed"));
    }
  }

  $effect(() => {
    return () => {
      if (unlisten) {
        unlisten();
        unlisten = null;
      }
    };
  });
</script>

<div class="download-log">
  <button
    type="button"
    class="log-toggle"
    onclick={toggle}
    aria-expanded={expanded}
    title={expanded ? $t("downloads.log.hide") : $t("downloads.log.show")}
  >
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
      <polyline points="14 2 14 8 20 8" />
      <line x1="8" y1="13" x2="16" y2="13" />
      <line x1="8" y1="17" x2="14" y2="17" />
    </svg>
    <span>{expanded ? $t("downloads.log.hide") : $t("downloads.log.show")}</span>
  </button>

  {#if expanded}
    <div class="log-panel">
      <div class="log-header">
        <span class="log-count">
          {lines.length}{loading ? "…" : ""}
        </span>
        <button type="button" class="log-copy" onclick={copyLog} disabled={!lines.length}>
          {$t("downloads.log.copy")}
        </button>
      </div>
      {#if !lines.length && !loading}
        <div class="log-empty">{$t("downloads.log.empty")}</div>
      {:else}
        <pre bind:this={preEl} onscroll={onScroll}>{lines.join("\n")}</pre>
      {/if}
    </div>
  {/if}
</div>

<style>
  .download-log {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 6px;
  }

  .log-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
    color: var(--foreground-secondary);
    font-size: 11px;
    cursor: pointer;
    align-self: flex-start;
    transition: background-color 0.12s, color 0.12s, border-color 0.12s;
  }

  .log-toggle:hover {
    background: var(--secondary);
    color: var(--foreground);
  }

  .log-panel {
    border: 1px solid var(--border);
    border-radius: var(--border-radius);
    background: var(--secondary);
    overflow: hidden;
  }

  .log-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    color: var(--foreground-secondary);
  }

  .log-copy {
    padding: 2px 8px;
    border: 1px solid var(--border);
    background: transparent;
    border-radius: var(--border-radius);
    color: var(--foreground);
    font-size: 11px;
    cursor: pointer;
  }

  .log-copy:hover:not(:disabled) {
    background: var(--background);
  }

  .log-copy:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  pre {
    margin: 0;
    padding: 8px;
    max-height: 220px;
    overflow-y: auto;
    font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
    font-size: 11px;
    color: var(--foreground);
    white-space: pre-wrap;
    word-break: break-all;
    background: var(--background);
  }

  .log-empty {
    padding: 12px;
    text-align: center;
    font-size: 11px;
    color: var(--foreground-secondary);
  }
</style>
