<script lang="ts">
  /**
   * In-document search (Cmd/Ctrl+F) over a rendered element: finds every
   * match in its text nodes, paints them with the CSS Custom Highlight API
   * (no DOM mutation, so `{@html}` content stays untouched) and walks them
   * with Enter / Shift+Enter. Without the API the matches are still counted
   * and scrolled to.
   */
  import { onDestroy, tick } from "svelte";
  import { t } from "$lib/i18n";

  let {
    root = null,
    open = $bindable(false),
    /** Changes when the content under `root` is re-rendered. */
    version = 0,
  }: { root?: HTMLElement | null; open?: boolean; version?: number } = $props();

  let query = $state("");
  let ranges = $state<Range[]>([]);
  let current = $state(0);
  let input = $state<HTMLInputElement | null>(null);

  type HighlightApi = {
    highlights: Map<string, unknown>;
  };
  const api = (): HighlightApi | null => {
    const c = (globalThis as unknown as { CSS?: HighlightApi }).CSS;
    const H = (globalThis as unknown as { Highlight?: unknown }).Highlight;
    return c && c.highlights && H ? c : null;
  };

  function paint() {
    const a = api();
    if (!a) return;
    const H = (globalThis as unknown as { Highlight: new (...r: Range[]) => unknown }).Highlight;
    if (!ranges.length) {
      a.highlights.delete("catalog-find");
      a.highlights.delete("catalog-find-current");
      return;
    }
    a.highlights.set("catalog-find", new H(...ranges));
    const cur = ranges[current];
    if (cur) a.highlights.set("catalog-find-current", new H(cur));
  }

  function search() {
    const q = query.trim().toLowerCase();
    const out: Range[] = [];
    if (root && q) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      let n: Node | null;
      while ((n = walker.nextNode())) {
        const text = (n.textContent ?? "").toLowerCase();
        let i = text.indexOf(q);
        while (i >= 0) {
          const r = document.createRange();
          r.setStart(n, i);
          r.setEnd(n, i + q.length);
          out.push(r);
          i = text.indexOf(q, i + q.length);
        }
      }
    }
    ranges = out;
    current = 0;
    paint();
    reveal();
  }

  function reveal() {
    const r = ranges[current];
    if (!r) return;
    const el = r.startContainer.parentElement;
    el?.scrollIntoView({ block: "center", behavior: "smooth" });
  }

  function step(d: number) {
    if (!ranges.length) return;
    current = (current + d + ranges.length) % ranges.length;
    paint();
    reveal();
  }

  function close() {
    open = false;
    query = "";
    ranges = [];
    paint();
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      step(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }

  $effect(() => {
    if (open) tick().then(() => input?.focus());
  });

  // re-run on new content
  $effect(() => {
    void version;
    void root;
    if (open && query) tick().then(search);
  });

  onDestroy(() => {
    const a = api();
    a?.highlights.delete("catalog-find");
    a?.highlights.delete("catalog-find-current");
  });
</script>

{#if open}
  <div class="find" role="search">
    <input
      bind:this={input}
      bind:value={query}
      oninput={search}
      onkeydown={onkey}
      placeholder={$t("llm.central.catalog.find.placeholder")}
      aria-label={$t("llm.central.catalog.find.placeholder")}
    />
    <span class="count tabular" aria-live="polite">
      {ranges.length ? `${current + 1}/${ranges.length}` : query ? $t("llm.central.catalog.find.none") : ""}
    </span>
    <button type="button" class="icon" onclick={() => step(-1)} aria-label={$t("llm.central.catalog.find.prev")} disabled={!ranges.length}>↑</button>
    <button type="button" class="icon" onclick={() => step(1)} aria-label={$t("llm.central.catalog.find.next")} disabled={!ranges.length}>↓</button>
    <button type="button" class="icon" onclick={close} aria-label={$t("llm.central.catalog.close")}>✕</button>
  </div>
{/if}

<style>
  .find {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    margin-bottom: var(--space-2);
    background: var(--material-thick);
    backdrop-filter: var(--material-blur);
    border-radius: var(--radius-md);
    box-shadow: var(--elev-2);
  }
  input {
    flex: 1;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-base);
  }
  .count {
    min-width: 48px;
    text-align: center;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .icon {
    width: var(--control-h);
    height: var(--control-h);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .icon:hover:not(:disabled) {
    background: var(--fill-1);
  }
  .icon:disabled {
    opacity: 0.4;
  }
  :global(::highlight(catalog-find)) {
    background-color: color-mix(in srgb, var(--warning) 45%, transparent);
    color: inherit;
  }
  :global(::highlight(catalog-find-current)) {
    background-color: var(--accent);
    color: var(--on-accent);
  }
</style>
