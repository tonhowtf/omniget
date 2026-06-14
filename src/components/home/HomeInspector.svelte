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
  <aside class="mac-inspector home-inspector">
    <h3 class="home-inspector-title">{title}</h3>
    <div class="home-inspector-body">
      {@render children()}
    </div>
  </aside>
{/if}

<style>
  .home-inspector-title {
    margin: 0 0 var(--space-3);
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--text);
  }

  .home-inspector-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
</style>
