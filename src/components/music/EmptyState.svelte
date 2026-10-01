<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    art,
    title,
    body = "",
    children,
    compact = false,
  }: {
    art: "empty-library" | "radio" | "now-playing" | "download" | "tools";
    title: string;
    body?: string;
    children?: Snippet;
    compact?: boolean;
  } = $props();
</script>

<div class="es" class:compact>
  <img src="/music/{art}.webp" alt="" width="640" height="560" draggable="false" />
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
    max-width: 460px;
    margin: var(--space-6) auto;
    padding: 0 var(--space-4);
  }
  img {
    width: min(280px, 70%);
    height: auto;
    user-select: none;
    filter: drop-shadow(0 12px 24px rgba(0, 0, 0, 0.18));
  }
  .compact img {
    width: min(180px, 50%);
  }
  h2 {
    margin: var(--space-2) 0 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
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
