<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    open = $bindable(false),
    label,
    locked = false,
    width = 520,
    onClose,
    children,
    footer,
  }: {
    open?: boolean;
    label: string;
    locked?: boolean;
    width?: number;
    onClose?: () => void;
    children: Snippet;
    footer?: Snippet;
  } = $props();

  let card = $state<HTMLDivElement | null>(null);
  let restoreFocus: HTMLElement | null = null;

  function close() {
    if (locked) return;
    open = false;
    onClose?.();
  }

  function onKey(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab" && card) {
      const els = [...card.querySelectorAll<HTMLElement>('button:not([disabled]), [href], input:not([disabled]), [tabindex]:not([tabindex="-1"])')];
      if (!els.length) return;
      const first = els[0];
      const last = els[els.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  $effect(() => {
    if (open && card) {
      restoreFocus = document.activeElement as HTMLElement | null;
      const target = card.querySelector<HTMLElement>("[data-autofocus]") ?? card;
      queueMicrotask(() => target.focus());
      return () => restoreFocus?.focus?.();
    }
  });
</script>

<svelte:window onkeydown={onKey} />

{#if open}
  <div class="wmb-sheet-backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && close()}>
    <div
      class="wmb-sheet"
      role="dialog"
      aria-modal="true"
      aria-label={label}
      tabindex="-1"
      bind:this={card}
      style:width="min({width}px, 100%)"
    >
      <div class="wmb-sheet-body">{@render children()}</div>
      {#if footer}<div class="wmb-sheet-foot">{@render footer()}</div>{/if}
    </div>
  </div>
{/if}

<style>
  .wmb-sheet-backdrop {
    position: fixed;
    inset: 0;
    z-index: 400;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-5);
    background: var(--dialog-backdrop);
    animation: wmb-fade var(--duration-fast) var(--ease-out);
  }
  .wmb-sheet {
    display: flex;
    flex-direction: column;
    max-height: min(720px, calc(100vh - 48px));
    border-radius: var(--radius-xl);
    background: var(--popup-bg);
    box-shadow: var(--elev-3);
    color: var(--text);
    outline: none;
    overflow: hidden;
    animation: wmb-pop var(--duration-base) var(--ease-spring);
  }
  .wmb-sheet-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-5) var(--space-5) var(--space-4);
  }
  .wmb-sheet-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-4);
    border-top: var(--hairline) solid var(--separator);
  }
  :global(.wmb-btn-danger) {
    background: var(--danger);
    color: var(--on-status);
    box-shadow: none;
  }
  @media (hover: hover) {
    :global(.wmb-btn-danger:hover:not(:disabled)) {
      background: color-mix(in srgb, var(--danger) 88%, black);
    }
  }
  @keyframes wmb-fade {
    from { opacity: 0; }
  }
  @keyframes wmb-pop {
    from { opacity: 0; transform: scale(0.97) translateY(6px); }
  }
  @media (prefers-reduced-motion: reduce) {
    .wmb-sheet-backdrop,
    .wmb-sheet { animation: none; }
  }
</style>
