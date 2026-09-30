<script lang="ts">
  // Blur (M-B10): the body of a node hidden behind an eye; the header and the
  // name stay. Visual only: the terminal keeps running and reading. Clicking
  // the eye (or Node > Remove Blur) takes it away.
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { TERM_HEADER_H } from "$lib/canvas/term/metrics";
  import { isTerminalKind, type CanvasNode } from "$lib/canvas/model/types";
  import { setBlur } from "$lib/canvas/menu/model";

  let { node }: { node: CanvasNode } = $props();
  const engine = useEngine();
  const top = $derived(isTerminalKind(node.kind) ? TERM_HEADER_H : 0);
</script>

<div class="blur-cover" data-blur-cover style:top="{top}px">
  <button
    type="button"
    class="eye"
    data-canvas-chrome
    title={$t("canvas.menu.removeBlur", { default: "Remove Blur" })}
    aria-label={$t("canvas.menu.removeBlur", { default: "Remove Blur" })}
    onpointerdown={(e) => e.stopPropagation()}
    onclick={(e) => {
      e.stopPropagation();
      setBlur(engine, node.id, false);
    }}
  >
    <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true">
      <path
        d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12Z"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linejoin="round"
      />
      <circle cx="12" cy="12" r="3.2" fill="currentColor" />
    </svg>
  </button>
</div>

<style>
  .blur-cover {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 5;
    display: grid;
    place-items: center;
    background: var(--surface-2, var(--bg-elevated, #2a2a2e));
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
    border-radius: inherit;
  }
  .eye {
    all: unset;
    cursor: pointer;
    color: var(--text-muted);
    padding: 10px;
    border-radius: 999px;
    display: grid;
    place-items: center;
  }
  .eye:hover {
    color: var(--text);
    background: var(--fill-2, rgba(127, 127, 127, 0.18));
  }
</style>
