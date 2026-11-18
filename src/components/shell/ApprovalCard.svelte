<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    titleId: string;
    title: string;
    describedBy?: string;
    error?: string | null;
    aside?: Snippet;
    children: Snippet;
  }

  let { titleId, title, describedBy, error = null, aside, children }: Props = $props();
</script>

<div class="approval-card" role="group" aria-labelledby={titleId} aria-describedby={describedBy}>
  <div class="head">
    <h2 id={titleId}>{title}</h2>
    {@render aside?.()}
  </div>
  {@render children()}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .approval-card {
    width: min(400px, calc(100vw - 32px));
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface-hi, var(--surface));
    box-shadow: var(--elev-3), inset 0 0 0 var(--hairline, 1px) var(--content-border, transparent);
    color: var(--text);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    line-height: var(--leading-md, 1.35);
    text-wrap: balance;
    overflow-wrap: anywhere;
  }
  .error {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--error);
    overflow-wrap: anywhere;
  }
</style>
