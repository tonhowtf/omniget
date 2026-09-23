<script lang="ts">
  // Assistant Markdown, rendered incrementally while streaming (stable prefix
  // cached, only the tail re-parses). Path-like inline code is a file chip.
  import { onDestroy } from "svelte";
  import { IncrementalMarkdown } from "$lib/central/threads-ui/markdown";

  let {
    text,
    streaming = false,
    onopenfile,
  }: { text: string; streaming?: boolean; onopenfile?: (path: string) => void } = $props();

  let version = $state(0);
  const md = new IncrementalMarkdown(() => (version += 1));
  onDestroy(() => md.clear());

  let html = $derived.by(() => {
    version;
    return md.render(text, streaming);
  });

  function chipOf(e: Event): string | null {
    const el = (e.target as HTMLElement | null)?.closest?.("code.file-chip") as HTMLElement | null;
    return el?.dataset.path ?? null;
  }

  function onClick(e: MouseEvent) {
    const p = chipOf(e);
    if (p && onopenfile) {
      e.preventDefault();
      onopenfile(p);
      return;
    }
    const a = (e.target as HTMLElement | null)?.closest?.("a") as HTMLAnchorElement | null;
    if (a?.href && /^https?:/.test(a.href)) {
      e.preventDefault();
      void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(a.href)).catch(() => window.open(a.href, "_blank"));
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key !== "Enter" && e.key !== " ") return;
    const p = chipOf(e);
    if (p && onopenfile) {
      e.preventDefault();
      onopenfile(p);
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="md markdown" data-streaming={streaming ? "" : undefined} onclick={onClick} onkeydown={onKey}>
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in $lib/llm/markdown -->
  {@html html}
</div>

<style>
  .md { font-size: 14px; line-height: 1.6; color: var(--text); overflow-wrap: anywhere; min-width: 0; }
  .md :global(p) { margin: 0 0 0.7em; }
  .md :global(p:last-child) { margin-bottom: 0; }
  .md :global(pre) { background: var(--fill-1); border: 1px solid var(--separator); border-radius: 10px; padding: 10px 12px; overflow-x: auto; font-size: 12.5px; line-height: 1.5; }
  .md :global(code) { font-family: var(--font-mono); font-size: 0.9em; background: var(--fill-1); padding: 0.1em 0.35em; border-radius: 5px; }
  .md :global(pre code) { background: transparent; padding: 0; }
  .md :global(code.file-chip) { cursor: pointer; color: var(--accent-hi); background: var(--accent-soft); text-decoration: none; }
  .md :global(code.file-chip:hover), .md :global(code.file-chip:focus-visible) { text-decoration: underline; outline: none; }
  .md :global(ul), .md :global(ol) { padding-left: 1.4em; margin: 0 0 0.7em; }
  .md :global(blockquote) { margin: 0 0 0.7em; padding-left: 12px; border-left: 3px solid var(--separator); color: var(--text-muted); }
  .md :global(table) { border-collapse: collapse; margin: 0 0 0.7em; font-size: 13px; display: block; overflow-x: auto; }
  .md :global(th), .md :global(td) { border: 1px solid var(--separator); padding: 4px 8px; }
  .md :global(h1), .md :global(h2), .md :global(h3), .md :global(h4) { margin: 0.9em 0 0.4em; line-height: 1.3; }
  .md :global(h1) { font-size: 1.3em; }
  .md :global(h2) { font-size: 1.15em; }
  .md :global(h3), .md :global(h4) { font-size: 1em; }
  .md :global(a) { color: var(--accent-hi); }
  .md[data-streaming] > :global(*) { transition: opacity 500ms; }
  @starting-style { .md[data-streaming] > :global(*) { opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .md[data-streaming] > :global(*) { transition: none; } }
</style>
