<script lang="ts">
  /**
   * Markdown viewer: preview (sanitised HTML) or code, "On this page" TOC
   * with the active heading, Cmd/Ctrl+F search, copy and expand.
   */
  import { tick } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { t } from "$lib/i18n";
  import { renderSafeMarkdown } from "$lib/llm/markdown";
  import { splitFrontmatter } from "$lib/central/catalog";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import CodeView from "./CodeView.svelte";
  import FindBar from "./FindBar.svelte";

  let {
    text = "",
    mode = $bindable<"preview" | "code">("preview"),
    toc = true,
    captureFind = true,
    toolbar = true,
    showModes = true,
    maxHeight = "70vh",
  }: {
    text?: string;
    mode?: "preview" | "code";
    toc?: boolean;
    captureFind?: boolean;
    toolbar?: boolean;
    showModes?: boolean;
    maxHeight?: string;
  } = $props();

  let parts = $derived(splitFrontmatter(text));
  let html = $state("");
  let version = $state(0);
  let previewEl = $state<HTMLElement | null>(null);
  let codeEl = $state<HTMLElement | null>(null);
  let scroller = $state<HTMLElement | null>(null);
  let findOpen = $state(false);
  let expanded = $state(false);
  let headings = $state<{ id: string; text: string; level: number }[]>([]);
  let active = $state("");

  $effect(() => {
    const body = parts[1];
    let dead = false;
    renderSafeMarkdown(body).then(async (h) => {
      if (dead) return;
      html = h;
      await tick();
      collectHeadings();
      version++;
    });
    return () => {
      dead = true;
    };
  });

  $effect(() => {
    void mode;
    tick().then(() => version++);
  });

  function slug(s: string, used: Set<string>): string {
    const base =
      "h-" +
      s
        .toLowerCase()
        .replace(/[^\p{L}\p{N}]+/gu, "-")
        .replace(/^-|-$/g, "");
    let id = base;
    let n = 1;
    while (used.has(id)) id = `${base}-${++n}`;
    used.add(id);
    return id;
  }

  function collectHeadings() {
    if (!previewEl) return;
    const used = new Set<string>();
    const out: typeof headings = [];
    previewEl.querySelectorAll("h1, h2, h3").forEach((h) => {
      const id = slug(h.textContent ?? "", used);
      h.id = id;
      out.push({ id, text: h.textContent ?? "", level: Number(h.tagName[1]) });
    });
    headings = out;
    active = out[0]?.id ?? "";
  }

  function onscroll() {
    if (!scroller || !previewEl || mode !== "preview") return;
    const top = scroller.getBoundingClientRect().top + 24;
    let cur = headings[0]?.id ?? "";
    for (const h of headings) {
      const el = previewEl.querySelector(`#${CSS.escape(h.id)}`);
      if (el && el.getBoundingClientRect().top <= top) cur = h.id;
    }
    active = cur;
  }

  function go(id: string) {
    previewEl?.querySelector(`#${CSS.escape(id)}`)?.scrollIntoView({ block: "start", behavior: "smooth" });
    active = id;
  }

  function onclick(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") ?? "";
    if (href.startsWith("#")) go(href.slice(1));
    else if (/^https?:\/\//.test(href)) void openUrl(href).catch(() => {});
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      showToast("success", $t("llm.central.catalog.copied"));
    } catch {
      showToast("error", $t("llm.central.catalog.copy_failed"));
    }
  }

  function onkey(e: KeyboardEvent) {
    if (!captureFind) return;
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      findOpen = true;
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="md" class:expanded style:--max-h={expanded ? "none" : maxHeight}>
  {#if toolbar}
    <div class="bar">
      {#if showModes}
        <div class="seg" role="tablist" aria-label={$t("llm.central.catalog.view.mode")}>
          <button type="button" role="tab" aria-selected={mode === "preview"} onclick={() => (mode = "preview")}>{$t("llm.central.catalog.view.preview")}</button>
          <button type="button" role="tab" aria-selected={mode === "code"} onclick={() => (mode = "code")}>{$t("llm.central.catalog.view.code")}</button>
        </div>
      {/if}
      <span class="spacer"></span>
      <button type="button" class="ghost" onclick={() => (findOpen = true)} title="⌘F">{$t("llm.central.catalog.find.open")}</button>
      <button type="button" class="ghost" onclick={copy}>{$t("llm.central.catalog.copy")}</button>
      <button type="button" class="ghost" onclick={() => (expanded = !expanded)} aria-pressed={expanded}>
        {expanded ? $t("llm.central.catalog.view.collapse") : $t("llm.central.catalog.view.expand")}
      </button>
    </div>
  {/if}
  <div class="layout">
    <div class="scroller" bind:this={scroller} {onscroll}>
      <FindBar root={mode === "preview" ? previewEl : codeEl} bind:open={findOpen} {version} />
      {#if mode === "preview"}
        {#if parts[0]}
          <details class="fm">
            <summary>{$t("llm.central.catalog.view.frontmatter")}</summary>
            <pre>{parts[0]}</pre>
          </details>
        {/if}
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <article class="prose" bind:this={previewEl} {onclick}>{@html html}</article>
      {:else}
        <CodeView {text} bind:el={codeEl} />
      {/if}
    </div>
    {#if toc && mode === "preview" && headings.length > 2}
      <nav class="toc" aria-label={$t("llm.central.catalog.view.toc")}>
        <p class="eyebrow">{$t("llm.central.catalog.view.toc")}</p>
        {#each headings as h (h.id)}
          <button type="button" class="lvl{h.level}" class:on={active === h.id} onclick={() => go(h.id)}>{h.text}</button>
        {/each}
      </nav>
    {/if}
  </div>
</div>

<style>
  .md {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .seg {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .seg button {
    height: 24px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .seg button[aria-selected="true"] {
    background: var(--surface-hi);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .ghost {
    height: 24px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .ghost:hover {
    background: var(--fill-1);
    color: var(--text);
  }
  .layout {
    display: flex;
    gap: var(--space-4);
    min-height: 0;
  }
  .scroller {
    flex: 1;
    min-width: 0;
    max-height: var(--max-h);
    overflow: auto;
  }
  .toc {
    width: 200px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: var(--max-h);
    overflow: auto;
  }
  .toc .eyebrow {
    margin: 0 0 var(--space-1);
  }
  .toc button {
    text-align: left;
    border: none;
    background: none;
    padding: 3px var(--space-2);
    border-left: 2px solid var(--separator);
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toc button.lvl2 {
    padding-left: var(--space-4);
  }
  .toc button.lvl3 {
    padding-left: var(--space-6);
  }
  .toc button.on {
    border-left-color: var(--accent);
    color: var(--accent-hi);
  }
  .fm {
    margin-bottom: var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .fm pre {
    margin: var(--space-1) 0 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface-mut);
    font-family: var(--font-mono);
    white-space: pre-wrap;
  }
  .prose {
    font-size: var(--text-base);
    line-height: 1.6;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .prose :global(h1),
  .prose :global(h2),
  .prose :global(h3) {
    margin: 1.2em 0 0.4em;
    line-height: 1.3;
    letter-spacing: var(--track-snug);
    scroll-margin-top: 48px;
  }
  .prose :global(h1) {
    font-size: var(--text-xl);
  }
  .prose :global(h2) {
    font-size: var(--text-lg);
  }
  .prose :global(h3) {
    font-size: var(--text-md);
  }
  .prose :global(pre) {
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-mut);
    overflow: auto;
    font-size: var(--text-sm);
  }
  .prose :global(code) {
    font-family: var(--font-mono);
    font-size: 0.92em;
  }
  .prose :global(:not(pre) > code) {
    padding: 1px 4px;
    border-radius: var(--radius-xs);
    background: var(--fill-1);
  }
  .prose :global(a) {
    color: var(--accent-hi);
  }
  .prose :global(table) {
    border-collapse: collapse;
    font-size: var(--text-sm);
  }
  .prose :global(th),
  .prose :global(td) {
    padding: 4px 8px;
    border: 1px solid var(--separator);
  }
  .prose :global(blockquote) {
    margin: 0;
    padding-left: var(--space-3);
    border-left: 3px solid var(--separator);
    color: var(--text-muted);
  }
  @media (max-width: 900px) {
    .toc {
      display: none;
    }
  }
</style>
