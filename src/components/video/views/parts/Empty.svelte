<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    icon,
    title,
    body = "",
    compact = false,
    children,
  }: {
    icon: string;
    title: string;
    body?: string;
    compact?: boolean;
    children?: Snippet;
  } = $props();
</script>

<div class="es" class:compact>
  <img src="/icons/video/{icon}-128.webp" alt="" width="128" height="128" draggable="false" />
  <h2>{title}</h2>
  {#if body}<p>{body}</p>{/if}
  {#if children}<div class="acts">{@render children()}</div>{/if}
</div>

<style>
  .es {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-2);
    max-width: 480px;
    margin: var(--space-7) auto;
    padding: 0 var(--space-4);
  }
  .compact {
    margin: var(--space-5) auto;
  }
  img {
    width: 112px;
    height: 112px;
    user-select: none;
    filter: drop-shadow(0 10px 20px rgba(0, 0, 0, 0.18));
  }
  .compact img {
    width: 72px;
    height: 72px;
  }
  h2 {
    margin: var(--space-2) 0 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .compact h2 {
    font-size: var(--text-lg);
  }
  p {
    margin: 0;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }
</style>
