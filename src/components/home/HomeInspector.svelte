<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    open = false,
    title,
    children,
  }: {
    open?: boolean;
    title: string;
    children: Snippet;
  } = $props();
</script>

{#if open}
  <section class="home-panel" aria-label={title}>
    <div class="home-panel-body">
      {@render children()}
    </div>
  </section>
{/if}

<style>
  .home-panel {
    width: 100%;
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    padding: var(--space-4);
    animation: panel-in var(--duration-slow) var(--ease-spring);
    transform-origin: top center;
  }

  .home-panel-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  @keyframes panel-in {
    from { opacity: 0; transform: translateY(-6px) scale(0.99); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  @media (prefers-reduced-motion: reduce) {
    .home-panel {
      animation: none;
    }
  }
</style>
