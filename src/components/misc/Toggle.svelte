<script lang="ts">
  let { enabled = $bindable(false), onToggle } = $props();

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      enabled = !enabled;
      onToggle?.();
    }
  }

  function handleClick() {
    enabled = !enabled;
    onToggle?.();
  }
</script>

<div
  role="switch"
  aria-checked={enabled}
  tabindex="0"
  class="toggle"
  class:enabled
  onclick={handleClick}
  onkeydown={handleKeyDown}
>
  <div class="toggle-knob"></div>
</div>

<style>
  .toggle {
    position: relative;
    width: 40px;
    height: 24px;
    background: var(--button);
    border-radius: 12px;
    border: 1px solid var(--button-stroke);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    padding: 2px;
    transition: background-color 0.2s;
  }

  .toggle:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .toggle:active {
    background: var(--button-press);
  }

  .toggle.enabled {
    background: var(--blue);
    border-color: var(--blue);
  }

  .toggle-knob {
    width: 18px;
    height: 18px;
    background: var(--secondary);
    border-radius: 50%;
    transition: transform 0.2s cubic-bezier(0.53, 0.05, 0.02, 1.2);
  }

  .toggle.enabled .toggle-knob {
    transform: translateX(16px);
  }

  :dir(rtl) .toggle.enabled .toggle-knob {
    transform: translateX(-16px);
  }

  @media (prefers-reduced-motion: reduce) {
    .toggle,
    .toggle-knob {
      transition: none;
    }
  }
</style>
