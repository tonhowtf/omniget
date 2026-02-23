<script lang="ts">
  import { onMount } from "svelte";

  type $$Slots = {
    default: {};
  };

  let { 'aria-label': ariaLabel } = $props();
  let container = $state<HTMLDivElement | null>(null);
  let focusedIndex = $state(0);

  onMount(() => {
    const buttons = container?.querySelectorAll('button[role="radio"]');
    if (buttons && buttons.length > 0) {
      const firstActive = Array.from(buttons).findIndex(
        (btn) => btn.getAttribute('aria-checked') === 'true'
      );
      focusedIndex = firstActive >= 0 ? firstActive : 0;
    }
  });

  function handleKeyDown(e: KeyboardEvent) {
    if (!container) return;

    const buttons = Array.from(
      container.querySelectorAll('button[role="radio"]')
    );
    const currentIndex = buttons.findIndex(
      (btn) => btn === document.activeElement
    );

    if (currentIndex === -1) return;

    let nextIndex = currentIndex;

    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      nextIndex = (currentIndex + 1) % buttons.length;
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      nextIndex = (currentIndex - 1 + buttons.length) % buttons.length;
    } else if (e.key === "Home") {
      e.preventDefault();
      nextIndex = 0;
    } else if (e.key === "End") {
      e.preventDefault();
      nextIndex = buttons.length - 1;
    }

    if (nextIndex !== currentIndex) {
      (buttons[nextIndex] as HTMLButtonElement).focus();
    }
  }
</script>

<div
  bind:this={container}
  role="radiogroup"
  aria-label={ariaLabel}
  class="switcher"
  onkeydown={handleKeyDown}
>
  <slot />
</div>

<style>
  .switcher {
    display: flex;
    gap: 0;
    border-radius: var(--border-radius);
    background: var(--button);
    box-shadow: var(--button-box-shadow);
  }

  .switcher :global(button[role="radio"]) {
    flex: 1;
    padding: var(--padding);
    font-size: 14.5px;
    font-weight: 500;
    background: transparent;
    border: none;
    border-radius: 0;
    color: var(--button-text);
    cursor: pointer;
    text-align: center;
    position: relative;
    margin-left: -1px;
  }

  .switcher :global(button[role="radio"]:first-child) {
    border-top-left-radius: calc(var(--border-radius) - 2px);
    border-bottom-left-radius: calc(var(--border-radius) - 2px);
    margin-left: 0;
  }

  .switcher :global(button[role="radio"]:last-child) {
    border-top-right-radius: calc(var(--border-radius) - 2px);
    border-bottom-right-radius: calc(var(--border-radius) - 2px);
  }

  .switcher :global(button[role="radio"][aria-checked="true"]) {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  @media (hover: hover) {
    .switcher :global(button[role="radio"]:not([aria-checked="true"]):hover) {
      background: var(--button-hover);
    }
  }

  .switcher :global(button[role="radio"]:active) {
    background: var(--button-press);
  }

  .switcher :global(button[role="radio"]:focus-visible) {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  @media (prefers-reduced-motion: reduce) {
    .switcher :global(button[role="radio"]) {
      transition: none;
    }
  }
</style>
