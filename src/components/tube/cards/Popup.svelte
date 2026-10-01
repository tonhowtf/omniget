<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import { floating } from "./actions";

  let {
    anchor = null,
    point = null,
    align = "end",
    above = false,
    modal = false,
    onclose,
    children,
    label = "",
    class: klass = "yt-menu",
    width = 0,
  }: {
    anchor?: HTMLElement | null;
    point?: { x: number; y: number } | null;
    align?: "start" | "end";
    above?: boolean;
    modal?: boolean;
    onclose: () => void;
    children: Snippet;
    label?: string;
    class?: string;
    width?: number;
  } = $props();

  let box = $state<HTMLDivElement>();
  let opener: Element | null = null;

  function items(): HTMLElement[] {
    return box ? [...box.querySelectorAll<HTMLElement>("[role=menuitem],[role=menuitemcheckbox],[role=menuitemradio]")] : [];
  }

  onMount(() => {
    opener = document.activeElement;
    queueMicrotask(() => (items()[0] ?? box?.querySelector<HTMLElement>("button,input,[tabindex]"))?.focus({ preventScroll: true }));
    const down = (e: PointerEvent) => {
      const target = e.target as Node;
      if (box?.contains(target) || anchor?.contains(target)) return;
      onclose();
    };
    const scroll = (e: Event) => {
      if (!modal && box && !box.contains(e.target as Node)) onclose();
    };
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("scroll", scroll, true);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("scroll", scroll, true);
      if (opener instanceof HTMLElement && document.activeElement === document.body) opener.focus({ preventScroll: true });
    };
  });

  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onclose();
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const list = items();
    if (!list.length) return;
    e.preventDefault();
    const i = list.indexOf(document.activeElement as HTMLElement);
    const next = e.key === "ArrowDown" ? (i + 1) % list.length : (i - 1 + list.length) % list.length;
    list[next].focus();
  }
</script>

{#if modal}
  <div class="scrim" use:floating={{ anchor: null, fill: true }} aria-hidden="true"></div>
{/if}
<div
  bind:this={box}
  class="popup {klass}"
  class:modal
  role={modal ? "dialog" : "menu"}
  aria-modal={modal ? "true" : undefined}
  aria-label={label || undefined}
  tabindex="-1"
  style:width={width ? `${width}px` : undefined}
  use:floating={{ anchor, point, align, above }}
  onkeydown={keydown}
>
  {@render children()}
</div>

<style>
  .popup {
    max-height: min(560px, calc(100vh - 32px));
    overflow-y: auto;
    scrollbar-width: thin;
    animation: pop 120ms var(--yt-ease-enter);
    outline: none;
  }
  .popup.modal {
    max-width: calc(100vw - 32px);
  }
  .scrim {
    width: 100vw;
    height: 100vh;
    background: rgba(0, 0, 0, 0.5);
    animation: fade 150ms linear;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .popup,
    .scrim {
      animation: none;
    }
  }
</style>
