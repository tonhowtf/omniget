<script lang="ts">
  /**
   * SKILL.md as slides: a title slide plus one per top section, arrow keys /
   * Home / End, fullscreen, token count per slide and in total (~4 chars per
   * token), so the weight of each part of the skill in the context is visible.
   */
  import { t } from "$lib/i18n";
  import { renderSafeMarkdown } from "$lib/llm/markdown";
  import { slidesOf } from "$lib/central/catalog";

  let { text = "", title = "" }: { text?: string; title?: string } = $props();

  let slides = $derived(slidesOf(text, title));
  let total = $derived(slides.reduce((a, s) => a + s.tokens, 0));
  let i = $state(0);
  let html = $state("");
  let frame = $state<HTMLElement | null>(null);

  $effect(() => {
    if (i >= slides.length) i = Math.max(0, slides.length - 1);
  });

  $effect(() => {
    const s = slides[i];
    let dead = false;
    renderSafeMarkdown(s?.body ?? "").then((h) => {
      if (!dead) html = h;
    });
    return () => {
      dead = true;
    };
  });

  function go(n: number) {
    i = Math.min(Math.max(0, n), slides.length - 1);
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === " ") {
      e.preventDefault();
      go(i + 1);
    } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
      e.preventDefault();
      go(i - 1);
    } else if (e.key === "Home") {
      e.preventDefault();
      go(0);
    } else if (e.key === "End") {
      e.preventDefault();
      go(slides.length - 1);
    }
  }

  function fullscreen() {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void frame?.requestFullscreen?.().catch(() => {});
  }
</script>

<div class="slides">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <section
    class="frame"
    bind:this={frame}
    tabindex="0"
    onkeydown={onkey}
    aria-roledescription="slide"
    aria-label={$t("llm.central.catalog.slides.of", { n: String(i + 1), total: String(slides.length) })}
  >
    <header>
      <h2>{slides[i]?.title}</h2>
      <span class="tok tabular">{$t("llm.central.catalog.slides.tokens", { count: String(slides[i]?.tokens ?? 0) })}</span>
    </header>
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    <div class="body">{@html html}</div>
  </section>
  <footer>
    <button type="button" onclick={() => go(i - 1)} disabled={i === 0} aria-label={$t("llm.central.catalog.slides.prev")}>←</button>
    <span class="pos tabular">{i + 1} / {slides.length}</span>
    <button type="button" onclick={() => go(i + 1)} disabled={i >= slides.length - 1} aria-label={$t("llm.central.catalog.slides.next")}>→</button>
    <span class="total tabular">{$t("llm.central.catalog.slides.total", { count: String(total) })}</span>
    <span class="spacer"></span>
    <button type="button" onclick={fullscreen}>{$t("llm.central.catalog.slides.fullscreen")}</button>
  </footer>
  <ol class="strip" aria-label={$t("llm.central.catalog.slides.list")}>
    {#each slides as s, n (n)}
      <li>
        <button type="button" class:on={n === i} onclick={() => go(n)}>
          <span class="n tabular">{n + 1}</span>
          <span class="st">{s.title}</span>
          <span class="bar" style:width="{total ? Math.max(4, (s.tokens / total) * 100) : 0}%"></span>
          <span class="tk tabular">{s.tokens}</span>
        </button>
      </li>
    {/each}
  </ol>
</div>

<style>
  .slides {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .frame {
    aspect-ratio: 16 / 9;
    max-height: 60vh;
    overflow: auto;
    padding: var(--space-6);
    border-radius: var(--radius-lg);
    background: var(--surface-hi);
    box-shadow: var(--elev-1);
    outline: none;
  }
  .frame:focus-visible {
    box-shadow: var(--elev-glow);
  }
  .frame:fullscreen {
    max-height: none;
    padding: var(--space-9);
    font-size: 1.3em;
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
  }
  h2 {
    margin: 0 0 var(--space-3);
    font-size: var(--text-xl);
    letter-spacing: var(--track-tight);
  }
  .tok {
    flex-shrink: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .body {
    font-size: var(--text-md);
    line-height: 1.6;
  }
  .body :global(pre) {
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface-mut);
    overflow: auto;
    font-size: var(--text-sm);
  }
  footer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  footer button {
    height: var(--control-h);
    min-width: var(--control-h);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    cursor: pointer;
  }
  footer button:disabled {
    opacity: 0.4;
  }
  .pos,
  .total {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .strip {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .strip button {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 4px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
  }
  .strip button.on {
    background: var(--accent-soft);
    color: var(--accent-hi);
  }
  .strip .n {
    width: 20px;
    color: var(--text-faint);
  }
  .strip .st {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    z-index: 1;
  }
  .strip .bar {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    background: var(--accent);
    opacity: 0.5;
  }
  .strip .tk {
    z-index: 1;
  }
</style>
