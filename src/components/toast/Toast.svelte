<script lang="ts">
  import { getToasts, dismissToast, type ToastType } from "$lib/stores/toast-store.svelte";
  import { t } from "$lib/i18n";

  let toasts = $derived(getToasts());

  function iconPath(type: ToastType): string {
    switch (type) {
      case "success":
        return "M5 12l5 5L20 7";
      case "error":
        return "M12 8v5m0 3.5h.01M10.3 3.9 2.4 17.6A2 2 0 0 0 4.1 20.6h15.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z";
      case "info":
        return "M12 8v.01M12 11v5M12 2a10 10 0 100 20 10 10 0 000-20z";
    }
  }
</script>

{#if toasts.length > 0}
  <div class="toast-container" aria-live="polite" aria-atomic="false">
    {#each toasts as toast (toast.id)}
      <div
        class="toast"
        data-type={toast.type}
        class:closing={toast.closing}
        role={toast.type === "error" ? "alert" : "status"}
        aria-live={toast.type === "error" ? "assertive" : "polite"}
      >
        <span class="toast-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d={iconPath(toast.type)} />
          </svg>
        </span>
        <span class="toast-message">{toast.message}</span>
        <button class="toast-close" onclick={() => dismissToast(toast.id)} aria-label={$t("common.close") as string}>
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  /* macOS notifications: top-trailing, glass, one line of text, symbol tile. */
  .toast-container {
    position: fixed;
    top: calc(var(--titlebar-height, 44px) + var(--space-2));
    right: calc(var(--pane-inset, 8px) + var(--space-2));
    z-index: 9999;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    pointer-events: none;
    max-width: 380px;
  }

  .toast {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    -webkit-backdrop-filter: var(--material-blur);
    border: none;
    border-radius: var(--radius-lg);
    box-shadow: var(--elev-2);
    pointer-events: auto;
    animation: toast-in var(--duration-bounce) var(--ease-spring);
    transform-origin: top right;
  }

  .toast.closing {
    animation: toast-out var(--duration-base) var(--ease-out) forwards;
  }

  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateX(16px) scale(0.96);
    }
    to {
      opacity: 1;
      transform: translateX(0) scale(1);
    }
  }

  @keyframes toast-out {
    from { opacity: 1; transform: translateX(0); }
    to   { opacity: 0; transform: translateX(12px); }
  }

  @media (prefers-reduced-motion: reduce) {
    .toast {
      animation: toast-fade-in var(--duration-base) ease-out;
    }
    .toast.closing {
      animation: toast-fade-out var(--duration-base) ease-out forwards;
    }
    @keyframes toast-fade-in {
      from { opacity: 0; }
      to   { opacity: 1; }
    }
    @keyframes toast-fade-out {
      from { opacity: 1; }
      to   { opacity: 0; }
    }
  }

  .toast-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    flex-shrink: 0;
    color: #fff;
  }

  .toast[data-type="success"] .toast-icon { background: var(--success); color: var(--on-success); }
  .toast[data-type="error"] .toast-icon   { background: var(--danger); color: var(--on-error); }
  .toast[data-type="info"] .toast-icon    { background: var(--info); }

  .toast-message {
    flex: 1;
    font-size: var(--text-base);
    font-weight: 500;
    line-height: var(--leading-base);
    color: var(--text);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .toast-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
  }

  @media (hover: hover) {
    .toast-close:hover {
      background: var(--fill-2);
      color: var(--text);
    }
  }

  .toast-close:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }
</style>
