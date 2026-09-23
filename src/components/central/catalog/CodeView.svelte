<script lang="ts">
  /** Plain source view with line numbers (text is never interpreted). */
  let {
    text = "",
    wrap = false,
    el = $bindable<HTMLElement | null>(null),
  }: { text?: string; wrap?: boolean; el?: HTMLElement | null } = $props();

  let lines = $derived(text.replace(/\r\n/g, "\n").split("\n"));
</script>

<div class="code" class:wrap bind:this={el} role="region" aria-label="source" tabindex="-1">
  {#each lines as line, i (i)}
    <div class="row"><span class="ln" aria-hidden="true">{i + 1}</span><span class="tx">{line || " "}</span></div>
  {/each}
</div>

<style>
  .code {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.55;
    background: var(--surface-mut);
    border-radius: var(--radius-md);
    padding: var(--space-2) 0;
    overflow: auto;
    color: var(--text);
  }
  .row {
    display: flex;
    min-width: max-content;
  }
  .wrap .row {
    min-width: 0;
  }
  .ln {
    position: sticky;
    left: 0;
    flex-shrink: 0;
    width: 44px;
    padding-right: var(--space-2);
    text-align: right;
    color: var(--text-faint);
    background: var(--surface-mut);
    user-select: none;
  }
  .tx {
    white-space: pre;
    padding-right: var(--space-3);
  }
  .wrap .tx {
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
