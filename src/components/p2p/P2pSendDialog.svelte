<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";

  let { onClose }: { onClose: () => void } = $props();

  type SendState =
    | { kind: "idle" }
    | { kind: "sending"; code: string; fileName: string; fileSize: number; progress: number; status: string; sentBytes: number; speed: number; paused: boolean }
    | { kind: "complete"; code: string; fileName: string }
    | { kind: "error"; message: string };

  let sendState: SendState = $state({ kind: "idle" });
  let dialogEl: HTMLDialogElement | null = $state(null);
  let unlisten: (() => void) | null = null;
  let unlistenComplete: (() => void) | null = null;

  onMount(() => {
    dialogEl?.showModal();

    listen<{ code: string; progress: number; status: string; sent_bytes: number; total_bytes: number; speed_bytes_per_sec: number }>("p2p-send-progress", (event) => {
      if (sendState.kind === "sending" && sendState.code === event.payload.code) {
        sendState = {
          ...sendState,
          progress: event.payload.progress,
          status: event.payload.status,
          sentBytes: event.payload.sent_bytes,
          speed: event.payload.speed_bytes_per_sec,
          paused: event.payload.status === "paused",
        };
      }
    }).then(fn => { unlisten = fn; });

    listen<{ code: string; success: boolean; error?: string }>("p2p-send-complete", (event) => {
      if (sendState.kind !== "sending" || sendState.code !== event.payload.code) return;
      if (event.payload.success) {
        sendState = { kind: "complete", code: sendState.code, fileName: sendState.fileName };
      } else {
        sendState = { kind: "error", message: event.payload.error ?? $t("p2p.send_failed") };
      }
    }).then(fn => { unlistenComplete = fn; });

    return () => {
      unlisten?.();
      unlistenComplete?.();
    };
  });

  async function selectAndSend() {
    const selected = await open({
      multiple: false,
      title: $t("p2p.select_file"),
    });
    if (!selected) return;

    try {
      const result = await invoke<{ code: string; file_name: string; file_size: number }>("p2p_send_file", {
        filePath: selected,
      });

      sendState = {
        kind: "sending",
        code: result.code,
        fileName: result.file_name,
        fileSize: result.file_size,
        progress: 0,
        status: "waiting_for_receiver",
        sentBytes: 0,
        speed: 0,
        paused: false,
      };
    } catch (e: any) {
      sendState = { kind: "error", message: typeof e === "string" ? e : e.message ?? $t("p2p.send_failed") };
    }
  }

  async function togglePause() {
    if (sendState.kind !== "sending") return;
    try {
      if (sendState.paused) {
        await invoke("p2p_resume_send", { code: sendState.code });
        sendState = { ...sendState, paused: false };
      } else {
        await invoke("p2p_pause_send", { code: sendState.code });
        sendState = { ...sendState, paused: true };
      }
    } catch {}
  }

  async function cancelSend() {
    if (sendState.kind === "sending") {
      try {
        await invoke("p2p_cancel_send", { code: sendState.code });
      } catch {}
    }
    handleClose();
  }

  function copyCode() {
    if (sendState.kind === "sending" || sendState.kind === "complete") {
      const fullCode = `p2p:${sendState.code}`;
      navigator.clipboard.writeText(fullCode);
      showToast("info", $t("p2p.code_copied"));
    }
  }

  function handleClose() {
    dialogEl?.close();
    onClose();
  }

  function getStatusLabel(status: string): string {
    switch (status) {
      case "waiting_for_receiver": return $t("p2p.waiting_for_receiver");
      case "transferring": return $t("p2p.transferring");
      case "paused": return $t("p2p.paused");
      case "complete": return $t("p2p.transfer_complete");
      default: return status;
    }
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function formatSpeed(bytesPerSec: number): string {
    if (bytesPerSec <= 0) return "0 KB/s";
    if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(0)} KB/s`;
    return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
  }

  function formatEta(remaining: number, speed: number): string {
    if (speed <= 0 || remaining <= 0) return "";
    const secs = Math.ceil(remaining / speed);
    if (secs < 60) return `~${secs}s`;
    if (secs < 3600) return `~${Math.floor(secs / 60)}m ${secs % 60}s`;
    return `~${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === dialogEl) {
      if (sendState.kind === "sending") return;
      handleClose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialogEl}
  class="p2p-dialog"
  aria-labelledby="p2p-dialog-title"
  aria-modal="true"
  onclick={handleBackdropClick}
  onkeydown={(e) => { if (e.key === "Escape" && sendState.kind !== "sending") handleClose(); }}
>
  <div class="dialog-content">
    <div class="dialog-header">
      <h3 id="p2p-dialog-title">{$t("p2p.send_title")}</h3>
      {#if sendState.kind !== "sending"}
        <button class="close-btn" onclick={handleClose} aria-label={$t("common.close")}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      {/if}
    </div>

    {#if sendState.kind === "idle"}
      <div class="dialog-body">
        <p class="description">{$t("p2p.send_description")}</p>
        <button class="button action-btn" onclick={selectAndSend}>
          {$t("p2p.select_file")}
        </button>
      </div>

    {:else if sendState.kind === "sending"}
      <div class="dialog-body">
        <div class="file-info">
          <span class="file-name">{sendState.fileName}</span>
          <span class="file-size">{formatBytes(sendState.fileSize)}</span>
        </div>

        <div class="code-section">
          <span class="code-label">{$t("p2p.share_code")}</span>
          <button class="code-display" onclick={copyCode} title={$t("p2p.click_to_copy")}>
            {sendState.code}
          </button>
          <span class="code-hint">{$t("p2p.share_hint")}</span>
        </div>

        {#if sendState.status === "transferring" || sendState.status === "paused"}
          <div class="progress-section">
            <div class="progress-bar-outer">
              <div
                class="progress-bar-inner"
                class:paused-bar={sendState.paused}
                style="width: {Math.min(sendState.progress, 100)}%"
              ></div>
            </div>
            <span class="progress-text">{sendState.progress.toFixed(1)}%</span>
          </div>
          <div class="transfer-stats">
            <span>{formatBytes(sendState.sentBytes)} / {formatBytes(sendState.fileSize)}</span>
            {#if sendState.paused}
              <span class="stat-paused">{$t("p2p.paused")}</span>
            {:else}
              <span>{formatSpeed(sendState.speed)}</span>
              {#if sendState.speed > 0}
                <span>{formatEta(sendState.fileSize - sendState.sentBytes, sendState.speed)}</span>
              {/if}
            {/if}
          </div>
          <div class="send-actions">
            <button class="button pause-btn" onclick={togglePause}>
              {#if sendState.paused}
                <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" stroke="none">
                  <polygon points="6,4 20,12 6,20" />
                </svg>
                {$t("p2p.resume")}
              {:else}
                <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" stroke="none">
                  <rect x="5" y="4" width="4" height="16" rx="1" />
                  <rect x="15" y="4" width="4" height="16" rx="1" />
                </svg>
                {$t("p2p.pause")}
              {/if}
            </button>
            <button class="button cancel-btn" onclick={cancelSend}>
              {$t("common.cancel")}
            </button>
          </div>
        {:else}
          <div class="status-section">
            <span class="status-spinner"></span>
            <span class="status-text">{getStatusLabel(sendState.status)}</span>
          </div>
          <button class="button cancel-btn" onclick={cancelSend}>
            {$t("common.cancel")}
          </button>
        {/if}
      </div>

    {:else if sendState.kind === "complete"}
      <div class="dialog-body">
        <div class="complete-section">
          <svg class="complete-icon" viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <span class="complete-text">{$t("p2p.transfer_complete")}</span>
          <span class="file-name">{sendState.fileName}</span>
        </div>
        <button class="button action-btn" onclick={handleClose}>
          {$t("common.close")}
        </button>
      </div>

    {:else if sendState.kind === "error"}
      <div class="dialog-body">
        <div class="error-section">
          <svg class="error-icon" viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4m0 4h.01" />
          </svg>
          <span class="error-text">{sendState.message}</span>
        </div>
        <div class="error-actions">
          <button class="button action-btn" onclick={selectAndSend}>
            {$t("p2p.try_again")}
          </button>
          <button class="button cancel-btn" onclick={handleClose}>
            {$t("common.close")}
          </button>
        </div>
      </div>
    {/if}
  </div>
</dialog>

<style>
  .p2p-dialog {
    border: none;
    border-radius: var(--border-radius);
    background: var(--popup-bg);
    color: var(--secondary);
    padding: 0;
    max-width: 440px;
    width: 90vw;
  }

  .p2p-dialog::backdrop {
    background: var(--dialog-backdrop);
  }

  .dialog-content {
    padding: calc(var(--padding) * 1.5);
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) * 1.5);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .dialog-header h3 {
    font-size: 16px;
    font-weight: 500;
    margin: 0;
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    color: var(--secondary);
    padding: 0;
  }

  @media (hover: hover) {
    .close-btn:hover {
      background: var(--button);
    }
  }

  .dialog-body {
    display: flex;
    flex-direction: column;
    gap: var(--padding);
  }

  .description {
    font-size: 13px;
    color: var(--gray);
    margin: 0;
    line-height: 1.5;
  }

  .file-info {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: calc(var(--padding) / 2) var(--padding);
    background: var(--button);
    border-radius: calc(var(--border-radius) - 2px);
  }

  .file-name {
    font-size: 13px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .file-size {
    font-size: 12px;
    color: var(--gray);
    flex-shrink: 0;
    margin-left: var(--padding);
  }

  .code-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: var(--padding);
    background: var(--button-elevated);
    border-radius: var(--border-radius);
  }

  .code-label {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .code-display {
    font-size: 20px;
    font-weight: 500;
    font-family: var(--font-mono);
    color: var(--accent);
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    letter-spacing: 1px;
  }

  @media (hover: hover) {
    .code-display:hover {
      background: var(--button);
    }
  }

  .code-hint {
    font-size: 11.5px;
    color: var(--gray);
  }

  .progress-section {
    display: flex;
    align-items: center;
    gap: var(--padding);
  }

  .progress-bar-outer {
    flex: 1;
    height: 6px;
    background: var(--button-elevated);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-bar-inner {
    height: 100%;
    background: var(--green);
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .paused-bar {
    background: var(--gray);
  }

  .progress-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--secondary);
    min-width: 40px;
    text-align: right;
  }

  .transfer-stats {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    font-size: 11.5px;
    color: var(--gray);
  }

  .stat-paused {
    color: var(--orange);
    font-weight: 500;
  }

  .send-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .pause-btn {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 13px;
    font-weight: 500;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    color: var(--secondary);
    cursor: pointer;
  }

  @media (hover: hover) {
    .pause-btn:hover {
      background: var(--button-hover);
    }
  }

  .status-section {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--padding) / 2);
    padding: calc(var(--padding) / 2) 0;
  }

  .status-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--input-border);
    border-top-color: var(--secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .status-text {
    font-size: 12.5px;
    color: var(--gray);
  }

  .complete-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: var(--padding) 0;
  }

  .complete-icon {
    color: var(--green);
  }

  .complete-text {
    font-size: 14px;
    font-weight: 500;
  }

  .error-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: var(--padding) 0;
  }

  .error-icon {
    color: var(--red);
  }

  .error-text {
    font-size: 13px;
    color: var(--gray);
    text-align: center;
  }

  .error-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .action-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--padding);
    font-size: 14px;
    font-weight: 500;
    background: var(--button);
    border: none;
    border-radius: var(--border-radius);
    color: var(--button-text);
    cursor: pointer;
    box-shadow: var(--button-box-shadow);
  }

  @media (hover: hover) {
    .action-btn:hover {
      background: var(--button-hover);
    }
  }

  .cancel-btn {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 13px;
    font-weight: 500;
    background: transparent;
    border: 1px solid var(--input-border);
    border-radius: var(--border-radius);
    color: var(--gray);
    cursor: pointer;
  }

  @media (hover: hover) {
    .cancel-btn:hover {
      background: var(--button);
      color: var(--secondary);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .status-spinner {
      animation-duration: 1.5s;
    }
    .progress-bar-inner {
      transition: none;
    }
  }
</style>
