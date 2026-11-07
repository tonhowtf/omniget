<script lang="ts">
  import { t } from "$lib/i18n";
  import type { DiffHunk } from "$lib/components/tokens";

  let { hunks, path }: { hunks: DiffHunk[]; path: string } = $props();
</script>

<figure class="cr-diff">
  <figcaption class="cr-diff-head">
    <span class="cr-diff-path">{path}</span>
  </figcaption>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="cr-diff-scroll" tabindex="0" role="region" aria-label={$t("components.diff_label", { path })}>
    {#each hunks as hunk, h (h)}
      {#if h > 0}<div class="cr-diff-gap" aria-hidden="true">⋯</div>{/if}
      <table class="cr-diff-table">
        <tbody>
          {#each hunk.lines as line, i (i)}
            <tr class="cr-diff-{line.kind}">
              <td class="cr-diff-ln" aria-hidden="true">{line.a ?? ""}</td>
              <td class="cr-diff-ln" aria-hidden="true">{line.b ?? ""}</td>
              <td class="cr-diff-sign">
                <span aria-hidden="true">{line.kind === "add" ? "+" : line.kind === "del" ? "−" : " "}</span>
                <span class="cr-sr">{line.kind === "add" ? $t("components.diff_added") : line.kind === "del" ? $t("components.diff_removed") : ""}</span>
              </td>
              <td class="cr-diff-src">{line.text || " "}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/each}
  </div>
</figure>

<style>
  .cr-diff {
    margin: 0;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    overflow: hidden;
  }

  .cr-diff-head {
    padding: var(--space-2) var(--space-3);
    border-bottom: var(--hairline) solid var(--content-border);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text);
  }

  .cr-diff-scroll {
    max-height: 360px;
    overflow: auto;
  }

  .cr-diff-scroll:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .cr-diff-table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.6;
  }

  .cr-diff-ln {
    width: 1%;
    padding: 0 var(--space-2);
    text-align: right;
    color: var(--text-dim);
    user-select: none;
    font-variant-numeric: tabular-nums;
  }

  .cr-diff-sign {
    width: 1%;
    padding: 0 var(--space-1);
    color: var(--text-muted);
  }

  .cr-diff-src {
    padding-right: var(--space-4);
    white-space: pre;
    color: var(--text);
  }

  .cr-diff-add {
    background: color-mix(in srgb, var(--success) 14%, transparent);
  }

  .cr-diff-del {
    background: color-mix(in srgb, var(--error) 14%, transparent);
  }

  .cr-diff-same .cr-diff-src {
    color: var(--text-muted);
  }

  .cr-diff-gap {
    padding: 0 var(--space-3);
    color: var(--text-dim);
    font-size: var(--text-xs);
  }

  .cr-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
