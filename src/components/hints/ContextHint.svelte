<script lang="ts">
  import { t } from "$lib/i18n";

  let { text, dismissKey }: { text: string; dismissKey: string } = $props();

  let dismissed = $state(false);
  let expanded = $state(false);

  let storageKey = $derived(`hint_dismissed_${dismissKey}`);

  $effect(() => {
    try {
      dismissed = localStorage.getItem(storageKey) === "1";
    } catch {}
  });

  function dismiss() {
    dismissed = true;
    expanded = false;
    try {
      localStorage.setItem(storageKey, "1");
    } catch {}
  }

  function toggle() {
    expanded = !expanded;
  }
</script>

{#if !dismissed}
  <span class="context-hint" class:expanded>
    <button
      class="hint-trigger"
      onclick={toggle}
      aria-label={$t("common.hint")}
      aria-expanded={expanded}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10" />
        <path d="M12 16v-4m0-4h.01" />
      </svg>
    </button>
    {#if expanded}
      <span class="hint-popover">
        <span class="hint-text">{text}</span>
        <button class="hint-dismiss" onclick={dismiss} aria-label={$t("common.close")}>
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </span>
    {/if}
  </span>
{/if}

<style>
  .context-hint {
    display: inline-flex;
    align-items: flex-start;
    position: relative;
    vertical-align: middle;
  }

  .hint-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: none;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 0;
    opacity: 0.6;
    transition: opacity 0.15s;
    flex-shrink: 0;
  }

  .hint-trigger :global(svg) {
    pointer-events: none;
  }

  @media (hover: hover) {
    .hint-trigger:hover {
      opacity: 1;
      color: var(--blue);
    }
  }

  .hint-trigger:active {
    opacity: 1;
  }

  .hint-trigger:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .hint-popover {
    position: absolute;
    top: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 8px 10px;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
    min-width: 200px;
    max-width: 280px;
    z-index: 100;
    animation: hint-in 0.1s ease-out;
  }

  @keyframes hint-in {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(-2px);
    }
    to {
      opacity: 1;
      transform: translateX(-50%) translateY(0);
    }
  }

  .hint-text {
    font-size: 12px;
    font-weight: 400;
    color: var(--secondary);
    line-height: 1.5;
    flex: 1;
  }

  .hint-dismiss {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: none;
    border: none;
    color: var(--gray);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .hint-dismiss :global(svg) {
    pointer-events: none;
  }

  @media (hover: hover) {
    .hint-dismiss:hover {
      color: var(--secondary);
    }
  }

  .hint-dismiss:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
