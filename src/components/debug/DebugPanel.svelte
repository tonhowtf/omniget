<script lang="ts">
  import { t } from "$lib/i18n";
  import { onMount } from "svelte";
  import {
    getDebugLogs,
    isDebugPanelOpen,
    setDebugPanelOpen,
    toggleDebugPanel,
    clearLogs,
    exportDiagnostics,
    type LogLevel,
  } from "$lib/stores/debug-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type FilterMode = "all" | "download" | "network" | "auth" | "system" | "convert" | "errors";

  let filter = $state<FilterMode>("all");
  let listEl: HTMLDivElement | undefined = $state();
  let autoScroll = $state(true);
  let copying = $state(false);

  let logs = $derived(getDebugLogs());
  let open = $derived(isDebugPanelOpen());

  let filtered = $derived.by(() => {
    if (filter === "all") return logs;
    if (filter === "errors") return logs.filter((l) => l.level === "error");
    return logs.filter((l) => l.category === filter);
  });

  $effect(() => {
    if (filtered.length && autoScroll && listEl) {
      requestAnimationFrame(() => {
        if (listEl) {
          listEl.scrollTop = listEl.scrollHeight;
        }
      });
    }
  });

  function handleScroll() {
    if (!listEl) return;
    const atBottom = listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight < 40;
    autoScroll = atBottom;
  }

  async function handleCopy() {
    copying = true;
    try {
      const report = await exportDiagnostics();
      await navigator.clipboard.writeText(report);
      showToast("success", $t("debug.report_copied"));
    } catch {
      showToast("error", "Failed to copy report");
    } finally {
      copying = false;
    }
  }

  function handleClose() {
    setDebugPanelOpen(false);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      handleClose();
    }
  }

  function levelColor(level: LogLevel): string {
    switch (level) {
      case "info": return "var(--accent)";
      case "warn": return "var(--warning)";
      case "error": return "var(--error)";
    }
  }

  function formatTime(ts: number): string {
    return new Date(ts).toLocaleTimeString("en-US", { hour12: false });
  }

  onMount(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === "D") {
        e.preventDefault();
        toggleDebugPanel();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="debug-panel" role="log" aria-label={$t("debug.title")} onkeydown={handleKeydown}>
    <div class="debug-header">
      <span class="debug-title">{$t("debug.title")}</span>

      <select
        class="debug-filter"
        bind:value={filter}
        aria-label={$t("debug.filter_all")}
      >
        <option value="all">{$t("debug.filter_all")}</option>
        <option value="download">{$t("debug.filter_downloads")}</option>
        <option value="network">{$t("debug.filter_network")}</option>
        <option value="auth">{$t("debug.filter_auth")}</option>
        <option value="system">{$t("debug.filter_system")}</option>
        <option value="convert">{$t("debug.filter_convert")}</option>
        <option value="errors">{$t("debug.filter_errors")}</option>
      </select>

      <div class="debug-actions">
        <button class="debug-btn" onclick={handleCopy} disabled={copying} aria-label={$t("debug.copy_report")}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
            <path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" />
          </svg>
          {$t("debug.copy_report")}
        </button>
        <button class="debug-btn" onclick={() => clearLogs()} aria-label={$t("debug.clear")}>
          {$t("debug.clear")}
        </button>
        <button class="debug-close" onclick={handleClose} aria-label={$t("common.close")}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>

    <div class="debug-list" bind:this={listEl} onscroll={handleScroll}>
      {#if filtered.length === 0}
        <div class="debug-empty">{$t("debug.no_entries")}</div>
      {:else}
        {#each filtered as entry (entry.id)}
          <div class="debug-entry" class:debug-error={entry.level === "error"} class:debug-warn={entry.level === "warn"}>
            <span class="debug-time">{formatTime(entry.timestamp)}</span>
            <span class="debug-level" style:background={levelColor(entry.level)}>
              {entry.level.toUpperCase()}
            </span>
            <span class="debug-msg">{entry.message}</span>
          </div>
          {#if entry.details}
            <div class="debug-details">{entry.details}</div>
          {/if}
        {/each}
      {/if}
    </div>
  </div>
{/if}

<style>
  .debug-panel {
    position: fixed;
    bottom: 0;
    left: var(--sidebar-width);
    right: 0;
    height: 40vh;
    min-height: 180px;
    max-height: 60vh;
    background: var(--popup-bg);
    border-top: none;
    display: flex;
    flex-direction: column;
    z-index: 100;
    font-size: 13px;
  }

  .debug-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: none;
    flex-shrink: 0;
  }

  .debug-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--secondary);
    margin-right: 4px;
  }

  .debug-filter {
    font-size: 12px;
    padding: 3px 8px;
    background: var(--button);
    color: var(--secondary);
    border: none;
    border-radius: calc(var(--border-radius) - 4px);
    cursor: pointer;
  }

  .debug-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }

  .debug-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
    font-weight: 500;
    padding: 4px 10px;
    background: var(--button);
    color: var(--secondary);
    border: none;
    border-radius: calc(var(--border-radius) - 4px);
    cursor: pointer;
  }

  @media (hover: hover) {
    .debug-btn:hover {
      background: var(--button-hover);
    }
  }

  .debug-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .debug-close {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
    background: none;
    border: none;
    color: var(--gray);
    cursor: pointer;
    border-radius: 4px;
  }

  @media (hover: hover) {
    .debug-close:hover {
      color: var(--secondary);
      background: var(--button-hover);
    }
  }

  .debug-close:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .debug-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }

  .debug-empty {
    padding: 24px;
    text-align: center;
    color: var(--gray);
    font-size: var(--text-sm);
  }

  .debug-entry {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 12px;
    min-height: 26px;
  }

  .debug-entry:nth-child(even) {
    background: var(--button);
  }

  .debug-error {
    background: color-mix(in srgb, var(--error) 8%, transparent) !important;
  }

  .debug-warn {
    background: color-mix(in srgb, var(--warning) 6%, transparent) !important;
  }

  .debug-time {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--gray);
    flex-shrink: 0;
  }

  .debug-level {
    font-size: 10px;
    font-weight: 600;
    color: var(--on-status);
    padding: 1px 5px;
    border-radius: 3px;
    flex-shrink: 0;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }

  .debug-msg {
    color: var(--secondary);
    font-size: var(--text-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .debug-details {
    padding: 2px 12px 4px calc(12px + 8px + 55px + 8px + 42px);
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--gray);
    word-break: break-all;
  }

  @media (max-width: 535px) {
    .debug-panel {
      left: 0;
      bottom: calc(44px + env(safe-area-inset-bottom));
      height: 50vh;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .debug-panel {
      transition: none;
    }
  }
</style>
