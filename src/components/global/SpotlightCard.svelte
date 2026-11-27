<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    accent = "var(--accent)",
    highlighted = false,
    interactive = true,
    class: className = "",
    children,
  }: {
    accent?: string;
    highlighted?: boolean;
    interactive?: boolean;
    class?: string;
    children: Snippet;
  } = $props();

  let el = $state<HTMLDivElement | null>(null);
  let x = $state(50);
  let y = $state(0);
  let hovering = $state(false);

  function onPointerMove(e: PointerEvent) {
    if (!interactive || !el) return;
    const rect = el.getBoundingClientRect();
    x = e.clientX - rect.left;
    y = e.clientY - rect.top;
  }
</script>

<div
  bind:this={el}
  role="presentation"
  class="spotlight-card {className}"
  class:spotlight-active={hovering && interactive}
  class:spotlight-highlighted={highlighted}
  style:--spot-color={accent}
  style:--spot-x="{x}px"
  style:--spot-y="{y}px"
  onpointermove={onPointerMove}
  onpointerenter={() => (hovering = true)}
  onpointerleave={() => (hovering = false)}
>
  <div class="spotlight-glow" aria-hidden="true"></div>
  <div class="spotlight-inner">
    {@render children()}
  </div>
</div>

<style>
  .spotlight-card {
    position: relative;
    border-radius: var(--radius-lg, 12px);
    background: var(--surface);
    border: 1px solid var(--hairline);
    overflow: hidden;
    transition:
      border-color var(--duration-fast, 140ms) var(--ease-out, ease-out),
      transform var(--duration-fast, 140ms) var(--ease-out, ease-out),
      box-shadow var(--duration-fast, 140ms) var(--ease-out, ease-out);
  }

  .spotlight-card:hover {
    border-color: color-mix(in srgb, var(--spot-color) 42%, var(--hairline));
  }

  .spotlight-highlighted {
    border-color: color-mix(in srgb, var(--spot-color) 65%, var(--hairline));
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--spot-color) 35%, transparent);
  }

  .spotlight-glow {
    pointer-events: none;
    position: absolute;
    inset: 0;
    opacity: 0;
    background: radial-gradient(
      260px circle at var(--spot-x) var(--spot-y),
      color-mix(in srgb, var(--spot-color) 16%, transparent),
      transparent 72%
    );
    transition: opacity 180ms ease;
    z-index: 0;
  }

  .spotlight-active .spotlight-glow {
    opacity: 1;
  }

  .spotlight-inner {
    position: relative;
    z-index: 1;
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  @media (prefers-reduced-motion: reduce) {
    .spotlight-card,
    .spotlight-glow {
      transition: none;
    }
  }
</style>
