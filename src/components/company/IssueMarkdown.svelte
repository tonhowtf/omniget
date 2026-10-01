<script lang="ts">
  import { renderSafeMarkdownSync } from "$lib/llm/markdown";

  let { text }: { text: string } = $props();

  const cache = new Map<string, string>();
  let ready = $state(0);

  function mentions(html: string): string {
    return html
      .split(/(<[^>]*>)/)
      .map((part) =>
        part.startsWith("<") ? part : part.replace(/(^|[^\w@/.])@([\w][\w.-]*)/g, '$1<span class="mention">@$2</span>'),
      )
      .join("");
  }

  let html = $derived.by(() => {
    void ready;
    return mentions(renderSafeMarkdownSync(text, cache, () => ready++));
  });
</script>

<div class="md">{@html html}</div>

<style>
  .md {
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text);
    overflow-wrap: anywhere;
    user-select: text;
  }
  .md :global(p) {
    margin: 0 0 var(--space-2);
  }
  .md :global(p:last-child) {
    margin-bottom: 0;
  }
  .md :global(pre),
  .md :global(code) {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--fill-2);
    border-radius: var(--radius-xs);
  }
  .md :global(code) {
    padding: 0 4px;
  }
  .md :global(pre) {
    padding: var(--space-2);
    overflow-x: auto;
  }
  .md :global(pre code) {
    padding: 0;
    background: none;
  }
  .md :global(ul),
  .md :global(ol) {
    margin: 0 0 var(--space-2);
    padding-left: var(--space-5);
  }
  .md :global(.mention) {
    padding: 0 3px;
    border-radius: var(--radius-xs);
    color: var(--accent-text);
    background: var(--accent-soft);
    font-weight: 600;
  }
</style>
