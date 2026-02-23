<script lang="ts">
  type $$Slots = {
    default: {};
  };

  let {
    isOpen = $bindable(false),
    titleId = "dialog-title",
    onClose,
  } = $props();

  let dialogEl = $state<HTMLDialogElement | null>(null);
  let previousFocusEl = $state<HTMLElement | null>(null);
  let closing = $state(false);

  $effect(() => {
    if (isOpen && dialogEl && !dialogEl.open) {
      previousFocusEl = document.activeElement as HTMLElement;
      dialogEl.showModal();
    }
  });

  function handleClose() {
    closing = true;
    setTimeout(() => {
      closing = false;
      dialogEl?.close();
      isOpen = false;
      onClose?.();
      previousFocusEl?.focus();
      previousFocusEl = null;
    }, 150);
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === dialogEl) {
      handleClose();
    }
  }
</script>

<dialog
  bind:this={dialogEl}
  class="dialog-container"
  class:closing
  aria-labelledby={titleId}
  aria-modal="true"
  onclick={handleBackdropClick}
  oncancel={(e) => {
    e.preventDefault();
    handleClose();
  }}
>
  <div class="dialog-content">
    <slot />
  </div>
</dialog>

<style>
  .dialog-container {
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

  .dialog-container::backdrop {
    background: var(--dialog-backdrop);
    animation: backdrop-in 0.15s ease-out;
  }

  .dialog-container.closing {
    animation: dialog-out 0.15s ease-in forwards;
  }

  .dialog-container.closing::backdrop {
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

  @media (prefers-reduced-motion: reduce) {
    .dialog-container {
      animation: none;
    }

    .dialog-container::backdrop {
      animation: none;
    }

    .dialog-container.closing {
      animation: none;
    }

    .dialog-container.closing::backdrop {
      animation: none;
    }
  }
</style>
