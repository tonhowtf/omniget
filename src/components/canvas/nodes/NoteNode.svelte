<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { renderSafeMarkdownSync } from "$lib/markdown";
  import type { CanvasNode } from "$lib/canvas/model/types";

  let {
    node,
    focused = false,
    onpatch,
    onexit,
  }: {
    node: CanvasNode;
    focused?: boolean;
    onpatch?: (patch: Partial<CanvasNode>) => void;
    onexit?: () => void;
  } = $props();

  const cache = new Map<string, string>();
  let bump = $state(0);
  let draft = $state("");
  let area = $state<HTMLTextAreaElement | null>(null);

  const text = $derived(
    String(node.data.text ?? node.data.markdown ?? node.data.body ?? ""),
  );
  const html = $derived.by(() => {
    void bump;
    return renderSafeMarkdownSync(text, cache, () => bump++);
  });

  $effect(() => {
    if (focused) {
      draft = text;
      queueMicrotask(() => area?.focus());
    }
  });

  function commit() {
    if (draft !== text) onpatch?.({ data: { ...node.data, text: draft } });
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      commit();
      onexit?.();
    }
  }

  onDestroy(() => cache.clear());
</script>

<div class="note" class:focused>
  {#if focused}
    <textarea
      bind:this={area}
      bind:value={draft}
      onblur={commit}
      onkeydown={onkey}
      spellcheck="false"
      placeholder={$t("canvas.engine.notePlaceholder", { default: "Write in Markdown…" })}
    ></textarea>
  {:else if text}
    <div class="md">{@html html}</div>
  {:else}
    <p class="empty">{$t("canvas.engine.noteEmpty", { default: "Double-click to write" })}</p>
  {/if}
</div>

<style>
  .note {
    height: 100%;
    overflow: auto;
    padding: 10px 12px;
    font-size: 14px;
    line-height: 1.5;
    color: var(--text);
  }
  .md :global(h1),
  .md :global(h2),
  .md :global(h3) {
    margin: 0.4em 0 0.3em;
    line-height: 1.25;
  }
  .md :global(p) {
    margin: 0 0 0.6em;
  }
  .md :global(pre) {
    background: var(--fill-1, rgba(127, 127, 127, 0.12));
    padding: 8px;
    border-radius: var(--radius-sm);
    overflow: auto;
  }
  textarea {
    width: 100%;
    height: 100%;
    resize: none;
    border: none;
    outline: none;
    background: transparent;
    color: inherit;
    font: 13px/1.5 var(--font-mono, monospace);
  }
  .empty {
    color: var(--text-muted);
    margin: 0;
  }
</style>
