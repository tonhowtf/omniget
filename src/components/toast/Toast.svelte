<script lang="ts">
  import { getToasts, dismissToast, type ToastType } from "$lib/stores/toast-store.svelte";

  let toasts = $derived(getToasts());

  function iconPath(type: ToastType): string {
    switch (type) {
      case "success":
        return "M5 12l5 5L20 7";
      case "error":
        return "M18 6L6 18M6 6l12 12";
      case "info":
        return "M12 8v4m0 4h.01M12 2a10 10 0 100 20 10 10 0 000-20z";
    }
  }
</script>

{#if toasts.length > 0}
  <div class="toast-container">
    {#each toasts as toast (toast.id)}
      <div class="toast" data-type={toast.type} class:closing={toast.closing}>
        <svg class="toast-icon" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d={iconPath(toast.type)} />
        </svg>
        <span class="toast-message">{toast.message}</span>
        <button class="toast-close" onclick={() => dismissToast(toast.id)}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    bottom: var(--padding);
    right: var(--padding);
    z-index: 9999;
    display: flex;
    flex-direction: column-reverse;
    gap: calc(var(--padding) / 2);
    pointer-events: none;
    max-width: 400px;
  }

  .toast {
    display: flex;
    align-items: center;
    gap: calc(var(--padding) * 0.75);
    padding: calc(var(--padding) * 0.75) var(--padding);
    background: var(--popup-bg);
    border-radius: var(--border-radius);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(255, 255, 255, 0.06) inset;
    pointer-events: auto;
    animation: toast-in 0.2s ease-out;
    opacity: 1;
    transform: translateY(0);
  }

  .toast.closing {
    animation: toast-out 0.2s ease-in forwards;
  }

  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes toast-out {
    from {
      opacity: 1;
      transform: translateY(0);
    }
    to {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  .toast-icon {
    flex-shrink: 0;
    pointer-events: none;
  }

  .toast[data-type="success"] .toast-icon {
    color: var(--green);
  }

  .toast[data-type="error"] .toast-icon {
    color: var(--red);
  }

  .toast[data-type="info"] .toast-icon {
    color: var(--blue);
  }

  .toast-message {
    flex: 1;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--secondary);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .toast-close {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: calc(var(--border-radius) / 2);
    color: var(--gray);
    cursor: pointer;
  }

  .toast-close :global(svg) {
    pointer-events: none;
  }

  @media (hover: hover) {
    .toast-close:hover {
      color: var(--secondary);
      background: rgba(255, 255, 255, 0.06);
    }
  }

  .toast-close:active {
    background: rgba(255, 255, 255, 0.03);
  }
</style>
