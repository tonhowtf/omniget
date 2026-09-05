<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";

  let {
    code,
    onAccept,
    onReject,
  }: {
    code: string;
    onAccept: () => void;
    onReject: () => void;
  } = $props();

  let dialogEl: HTMLDialogElement | null = $state(null);
  let acceptBtn: HTMLButtonElement | null = $state(null);

  onMount(() => {
    dialogEl?.showModal();
    acceptBtn?.focus();
  });

  function handleAccept() {
    dialogEl?.close();
    onAccept();
  }

  function handleReject() {
    dialogEl?.close();
    onReject();
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === dialogEl) {
      handleReject();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialogEl}
  class="p2p-dialog"
  aria-labelledby="p2p-receive-title"
  aria-modal="true"
  onclick={handleBackdropClick}
  onkeydown={(e) => { if (e.key === "Escape") handleReject(); }}
>
  <div class="dialog-content">
    <div class="dialog-header">
      <h3 id="p2p-receive-title">{$t("p2p.receive_title")}</h3>
      <button class="close-btn" onclick={handleReject} aria-label={$t("common.close")}>
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M18 6L6 18M6 6l12 12" />
        </svg>
      </button>
    </div>

    <div class="dialog-body">
      <p class="description">{$t("p2p.receive_description")}</p>

      <div class="receive-icon-section">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
          <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
        </svg>
      </div>

      <div class="code-section">
        <span class="code-label">{$t("p2p.share_code")}</span>
        <span class="code-value">{code}</span>
      </div>

      <p class="hint">{$t("p2p.searching_hint")}</p>

      <div class="receive-actions">
        <button class="button accept-btn" onclick={handleAccept} bind:this={acceptBtn}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 3v12m0 0l-4-4m4 4l4-4" />
            <path d="M4 17v2a1 1 0 001 1h14a1 1 0 001-1v-2" />
          </svg>
          {$t("p2p.accept_transfer")}
        </button>
        <button class="button cancel-btn" onclick={handleReject}>
          {$t("p2p.reject_transfer")}
        </button>
      </div>
    </div>
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

  .receive-icon-section {
    display: flex;
    justify-content: center;
    padding: calc(var(--padding) / 2) 0;
    color: var(--accent);
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

  .code-value {
    font-size: 18px;
    font-weight: 500;
    font-family: var(--font-mono);
    color: var(--accent);
    letter-spacing: 1px;
  }

  .hint {
    font-size: 12px;
    color: var(--gray);
    margin: 0;
    text-align: center;
  }

  .receive-actions {
    display: flex;
    gap: calc(var(--padding) / 2);
  }

  .accept-btn {
    flex: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: var(--padding);
    font-size: 14px;
    font-weight: 500;
    background: var(--accent);
    color: var(--on-accent);
    border: none;
    border-radius: var(--border-radius);
    cursor: pointer;
  }

  @media (hover: hover) {
    .accept-btn:hover {
      filter: brightness(1.1);
    }
  }

  .accept-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
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
    border: none;
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
</style>
