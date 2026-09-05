<script lang="ts">
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";

  type Props = {
    threshold?: number;
    children: Snippet;
  };
  let { threshold = 64, children }: Props = $props();

  let scrolled = $state(false);

  onMount(() => {
    const scrollEl = document.scrollingElement ?? document.documentElement;
    function check() {
      scrolled = scrollEl.scrollTop > threshold;
    }
    check();
    window.addEventListener("scroll", check, { passive: true });
    return () => {
      window.removeEventListener("scroll", check);
    };
  });
</script>

<div class="sticky-header" class:opaque={scrolled}>
  {@render children()}
</div>

<style>
  .sticky-header {
    position: sticky;
    top: 0;
    z-index: 20;
    padding: 12px 24px;
    background: transparent;
    border-bottom: 1px solid transparent;
    transition: background 200ms ease, border-color 200ms ease,
      backdrop-filter 200ms ease;
  }
  .sticky-header.opaque {
    background: color-mix(in oklab, var(--bg) 88%, transparent);
    border-bottom-color: rgba(255, 255, 255, 0.08);
    backdrop-filter: blur(24px);
  }
  @media (prefers-reduced-motion: reduce) {
    .sticky-header {
      transition: none;
    }
  }
</style>
