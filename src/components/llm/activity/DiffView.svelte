<script lang="ts">
  /**
   * A unified diff, painted line by line. Read-only, selectable, scrolls on
   * its own so a long patch never pushes the page sideways.
   */
  import { diffLineKind } from "./run-status";

  let { patch, label }: { patch: string; label: string } = $props();
  let lines = $derived(patch.split("\n"));
</script>

<!-- A scrollable region must be reachable by keyboard. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<pre class="diff" role="region" aria-label={label} tabindex="0">{#each lines as line, i (i)}<span class="l {diffLineKind(line)}">{line || " "}</span>
{/each}</pre>

<style>
  .diff {
    margin: 0;
    padding: var(--space-2) 0;
    max-height: 420px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.5;
    user-select: text;
  }

  .diff:focus-visible {
    outline: var(--focus-ring);
  }

  .l {
    display: inline-block;
    min-width: 100%;
    padding: 0 var(--space-2);
    white-space: pre;
  }

  .add {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 10%, transparent);
  }

  .del {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 10%, transparent);
  }

  .hunk {
    color: var(--accent-text);
  }

  .file {
    color: var(--text);
    font-weight: 600;
  }
</style>
