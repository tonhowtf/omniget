<!-- Ported from two "Copy Button" components on 21st.dev: ncdai (states, icon swap) and Özer / interior.dev (clipboard fallback, check draw, press). MIT License. -->
<script lang="ts">
  import WaIcon from "./WaIcon.svelte";

  type CopyState = "idle" | "done" | "error";

  let {
    text,
    label,
    doneLabel,
    errorLabel,
    idleIcon = "link",
    resetDelay = 1500,
  }: {
    text: string | (() => string);
    label: string;
    doneLabel: string;
    errorLabel: string;
    idleIcon?: string;
    resetDelay?: number;
  } = $props();

  let copyState = $state<CopyState>("idle");
  let reset: ReturnType<typeof setTimeout> | null = null;

  $effect(() => () => {
    if (reset) clearTimeout(reset);
  });

  function writeFallback(value: string): boolean {
    const area = document.createElement("textarea");
    area.value = value;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.top = "0";
    area.style.left = "0";
    area.style.opacity = "0";
    document.body.appendChild(area);
    const selection = document.getSelection();
    const previous = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
    area.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch {
      ok = false;
    }
    document.body.removeChild(area);
    if (selection && previous) {
      selection.removeAllRanges();
      selection.addRange(previous);
    }
    return ok;
  }

  async function copy() {
    if (reset) clearTimeout(reset);
    const value = typeof text === "function" ? text() : text;
    let ok = false;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(value);
        ok = true;
      } else {
        ok = writeFallback(value);
      }
    } catch {
      ok = writeFallback(value);
    }
    copyState = ok ? "done" : "error";
    reset = setTimeout(() => (copyState = "idle"), resetDelay);
  }
</script>

<button class="btn btn-ghost btn-icon btn-sm copy" type="button" data-state={copyState} aria-label={label} title={label} onclick={() => void copy()}>
  <span class="swap" data-for="idle"><WaIcon name={idleIcon} /></span>
  <span class="swap" data-for="done">
    <svg class="wa-icon" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path class="draw" pathLength="1" d="M2.9 7.4 5.6 10.1 11.1 4" /></svg>
  </span>
  <span class="swap" data-for="error"><WaIcon name="x" /></span>
  <span class="sr-only" role="status" aria-live="polite">{copyState === "done" ? doneLabel : copyState === "error" ? errorLabel : ""}</span>
</button>

<style>
  .copy {
    position: relative;
    will-change: transform;
    touch-action: manipulation;
  }

  .copy:active {
    transform: translateY(1px);
  }

  .swap {
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transform: scale(0.25);
    filter: blur(4px);
    transition:
      opacity 300ms cubic-bezier(0.23, 1, 0.32, 1),
      transform 300ms cubic-bezier(0.23, 1, 0.32, 1),
      filter 300ms cubic-bezier(0.23, 1, 0.32, 1);
  }

  .swap:not([data-for="idle"]) {
    position: absolute;
    inset: 0;
  }

  .swap[data-for="done"] {
    color: var(--success);
  }

  .swap[data-for="error"] {
    color: var(--warning);
  }

  .copy[data-state="idle"] .swap[data-for="idle"],
  .copy[data-state="done"] .swap[data-for="done"],
  .copy[data-state="error"] .swap[data-for="error"] {
    opacity: 1;
    transform: scale(1);
    filter: blur(0);
  }

  .draw {
    stroke-dasharray: 1;
    stroke-dashoffset: 1;
    transition: stroke-dashoffset 260ms cubic-bezier(0.23, 1, 0.32, 1);
  }

  .copy[data-state="done"] .draw {
    stroke-dashoffset: 0;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .swap,
    .draw {
      transition: none;
      filter: none;
    }

    .copy:active {
      transform: none;
    }
  }
</style>
