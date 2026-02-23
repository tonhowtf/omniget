<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    getChangelogVisible,
    getChangelogBody,
    getCurrentVersion,
    dismissChangelog,
  } from "$lib/stores/changelog-store.svelte";

  let visible = $derived(getChangelogVisible());
  let body = $derived(getChangelogBody());
  let version = $derived(getCurrentVersion());

  let dialogEl = $state<HTMLDialogElement | null>(null);
  let closing = $state(false);
  let previousFocusEl = $state<HTMLElement | null>(null);

  $effect(() => {
    if (visible && dialogEl && !dialogEl.open) {
      previousFocusEl = document.activeElement as HTMLElement;
      dialogEl.showModal();
    }
  });

  function handleClose() {
    closing = true;
    setTimeout(() => {
      closing = false;
      dialogEl?.close();
      dismissChangelog();
      previousFocusEl?.focus();
      previousFocusEl = null;
    }, 150);
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === dialogEl) {
      handleClose();
    }
  }

  function renderMarkdown(md: string): string {
    return md
      .split("\n")
      .map((line) => {
        if (line.startsWith("### ")) {
          return `<h4>${escapeHtml(line.slice(4))}</h4>`;
        }
        if (line.startsWith("## ")) {
          return `<h3>${escapeHtml(line.slice(3))}</h3>`;
        }
        if (line.startsWith("# ")) {
          return `<h2>${escapeHtml(line.slice(2))}</h2>`;
        }
        if (line.startsWith("- ") || line.startsWith("* ")) {
          return `<li>${formatInline(line.slice(2))}</li>`;
        }
        if (line.trim() === "") {
          return "<br />";
        }
        return `<p>${formatInline(line)}</p>`;
      })
      .join("");
  }

  function escapeHtml(str: string): string {
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function formatInline(str: string): string {
    let result = escapeHtml(str);
    result = result.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    result = result.replace(/`(.+?)`/g, "<code>$1</code>");
    result = result.replace(
      /\[(.+?)\]\((.+?)\)/g,
      '<a href="$2" target="_blank" rel="noopener">$1</a>'
    );
    return result;
  }
</script>

{#if visible}
  <dialog
    bind:this={dialogEl}
    class="changelog-dialog"
    class:closing
    aria-labelledby="changelog-title"
    aria-modal="true"
    onclick={handleBackdropClick}
    oncancel={(e) => {
      e.preventDefault();
      handleClose();
    }}
  >
    <div class="dialog-content">
      <div class="dialog-header">
        <div class="dialog-title-row">
          <h3 id="changelog-title">{$t("changelog.title")}</h3>
          {#if version}
            <span class="version-badge">v{version}</span>
          {/if}
        </div>
        <button class="close-btn" onclick={handleClose} aria-label={$t("common.close")}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="dialog-body">
        {#if body}
          <div class="markdown-content">
            {@html renderMarkdown(body)}
          </div>
        {:else}
          <p class="empty-text">{$t("changelog.empty")}</p>
        {/if}
      </div>

      <div class="dialog-footer">
        <button class="button dismiss-btn" onclick={handleClose}>
          {$t("common.close")}
        </button>
      </div>
    </div>
  </dialog>
{/if}

<style>
  .changelog-dialog {
    border: none;
    border-radius: var(--border-radius);
    background: var(--popup-bg);
    color: var(--secondary);
    padding: 0;
    width: 90%;
    max-width: 480px;
    max-height: 80vh;
    animation: dialog-in 0.15s ease-out;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .changelog-dialog::backdrop {
    background: var(--dialog-backdrop);
    animation: backdrop-in 0.15s ease-out;
  }

  .changelog-dialog.closing {
    animation: dialog-out 0.15s ease-in forwards;
  }

  .changelog-dialog.closing::backdrop {
    animation: backdrop-out 0.15s ease-in forwards;
  }

  @keyframes dialog-in {
    from {
      opacity: 0;
      transform: scale(0.96) translateY(8px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }

  @keyframes dialog-out {
    from {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
    to {
      opacity: 0;
      transform: scale(0.96) translateY(8px);
    }
  }

  @keyframes backdrop-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes backdrop-out {
    from {
      opacity: 1;
    }
    to {
      opacity: 0;
    }
  }

  .dialog-content {
    display: flex;
    flex-direction: column;
    max-height: 80vh;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: calc(var(--padding) + 4px) calc(var(--padding) + 4px) 0;
    flex-shrink: 0;
  }

  .dialog-title-row {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) / 2);
  }

  .dialog-title-row h3 {
    margin: 0;
  }

  .version-badge {
    font-size: 11px;
    font-weight: 500;
    color: var(--blue);
    background: rgba(47, 138, 249, 0.12);
    padding: 2px 8px;
    border-radius: 6px;
  }

  .close-btn {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: calc(var(--border-radius) / 2);
    color: var(--gray);
    cursor: pointer;
    border: none;
    background: none;
    flex-shrink: 0;
  }

  .close-btn :global(svg) {
    pointer-events: none;
  }

  @media (hover: hover) {
    .close-btn:hover {
      color: var(--secondary);
      background: var(--button-elevated);
    }
  }

  .close-btn:active {
    background: var(--button-elevated-press);
  }

  .close-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .dialog-body {
    flex: 1;
    overflow-y: auto;
    padding: calc(var(--padding) + 4px);
    scrollbar-width: none;
  }

  .dialog-body::-webkit-scrollbar {
    display: none;
  }

  .markdown-content {
    font-size: 13px;
    font-weight: 400;
    line-height: 1.7;
    color: var(--secondary);
  }

  .markdown-content :global(h2) {
    font-size: 18px;
    font-weight: 500;
    margin: 0 0 calc(var(--padding) / 2);
    letter-spacing: -0.5px;
  }

  .markdown-content :global(h3) {
    font-size: 15px;
    font-weight: 500;
    margin: var(--padding) 0 calc(var(--padding) / 2);
  }

  .markdown-content :global(h4) {
    font-size: 13px;
    font-weight: 500;
    margin: var(--padding) 0 calc(var(--padding) / 4);
    color: var(--gray);
  }

  .markdown-content :global(p) {
    margin: 0 0 4px;
  }

  .markdown-content :global(li) {
    margin: 0 0 4px;
    padding-left: calc(var(--padding) / 2);
    list-style: none;
  }

  .markdown-content :global(li::before) {
    content: "•";
    color: var(--blue);
    margin-right: 6px;
  }

  .markdown-content :global(strong) {
    font-weight: 600;
  }

  .markdown-content :global(code) {
    font-size: 12px;
    padding: 1px 5px;
    background: var(--button-elevated);
    border-radius: 4px;
  }

  .markdown-content :global(a) {
    color: var(--blue);
    text-decoration: none;
  }

  @media (hover: hover) {
    .markdown-content :global(a:hover) {
      text-decoration: underline;
    }
  }

  .markdown-content :global(br) {
    display: block;
    content: "";
    margin-top: 4px;
  }

  .empty-text {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--gray);
    text-align: center;
    padding: calc(var(--padding) * 2) 0;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    padding: 0 calc(var(--padding) + 4px) calc(var(--padding) + 4px);
    flex-shrink: 0;
  }

  .dismiss-btn {
    padding: calc(var(--padding) / 2) calc(var(--padding) * 1.5);
    font-size: 13px;
    font-weight: 500;
  }
</style>
