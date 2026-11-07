<script lang="ts">
  import { t } from "$lib/i18n";
  import CopyButton from "./CopyButton.svelte";

  let { path, content, note = null }: { path: string; content: string; note?: string | null } = $props();

  const LIMIT = 400;
  let expanded = $state(false);
  let lines = $derived(content.split("\n"));
  let shown = $derived(expanded ? lines : lines.slice(0, LIMIT));
</script>

<figure class="cr-code">
  <figcaption class="cr-code-head">
    <span class="cr-code-path" title={path}>{path}</span>
    {#if note}<span class="cr-code-note">{note}</span>{/if}
    <span class="cr-code-count">{$t("components.lines", { count: lines.length })}</span>
    {#if content}
      <CopyButton text={content} label={$t("components.copy_file") as string} />
    {/if}
  </figcaption>
  {#if content}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="cr-code-scroll" tabindex="0" role="region" aria-label={path}>
      <table class="cr-code-table">
        <tbody>
          {#each shown as line, i (i)}
            <tr>
              <td class="cr-ln" aria-hidden="true">{i + 1}</td>
              <td class="cr-src">{line || " "}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if lines.length > LIMIT && !expanded}
      <button type="button" class="btn btn-ghost btn-sm cr-code-more" onclick={() => (expanded = true)}>
        {$t("components.show_all_lines", { count: lines.length })}
      </button>
    {/if}
  {:else}
    <p class="cr-code-empty">{$t("components.file_no_content")}</p>
  {/if}
</figure>

<style>
  .cr-code {
    margin: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .cr-code-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--row-base);
    padding: 0 var(--space-2) 0 var(--space-3);
    border-bottom: var(--hairline) solid var(--content-border);
  }

  .cr-code-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-code-note,
  .cr-code-count {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .cr-code-scroll {
    max-height: 420px;
    overflow: auto;
  }

  .cr-code-scroll:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .cr-code-table {
    border-collapse: collapse;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.6;
  }

  .cr-ln {
    position: sticky;
    left: 0;
    min-width: 40px;
    padding: 0 var(--space-2) 0 var(--space-3);
    text-align: right;
    color: var(--text-dim);
    background: var(--surface);
    user-select: none;
    vertical-align: top;
    font-variant-numeric: tabular-nums;
  }

  .cr-src {
    padding: 0 var(--space-4) 0 var(--space-2);
    white-space: pre;
    color: var(--text);
  }

  .cr-code-more {
    margin: var(--space-2) var(--space-3);
  }

  .cr-code-empty {
    margin: 0;
    padding: var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
