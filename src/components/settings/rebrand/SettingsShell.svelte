<script lang="ts">
  import type { Snippet } from "svelte";

  // Two columns: navigation (search on top) and the scrolling content column.
  // The content is capped at 760 px and never forces horizontal scroll.
  let {
    nav,
    banner,
    children,
    busy = false,
  }: {
    nav: Snippet;
    banner?: Snippet;
    children: Snippet;
    busy?: boolean;
  } = $props();
</script>

<div class="settings-shell">
  <aside class="settings-shell-nav">
    {@render nav()}
  </aside>
  <div class="settings-shell-main" id="settings-main" aria-busy={busy}>
    <div class="settings-shell-content">
      {#if banner}{@render banner()}{/if}
      {@render children()}
    </div>
  </div>
</div>

<style>
  .settings-shell {
    display: grid;
    grid-template-columns: 196px minmax(0, 1fr);
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    inline-size: 100%;
    block-size: 100%;
    background: var(--bg);
  }
  .settings-shell-nav {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-inline-size: 0;
    padding: var(--space-4) var(--space-3);
    border-inline-end: 1px solid var(--separator);
    overflow-y: auto;
  }
  .settings-shell-main {
    min-inline-size: 0;
    min-block-size: 0;
    overflow-y: auto;
    overflow-x: hidden;
    container-type: inline-size;
    container-name: settings-content;
  }
  .settings-shell-content {
    box-sizing: border-box;
    min-inline-size: 0;
    inline-size: 100%;
    max-inline-size: 760px;
    padding: 24px;
  }
  @media (max-width: 1099px) {
    .settings-shell { grid-template-columns: 176px minmax(0, 1fr); }
    .settings-shell-content { padding: 16px; }
  }
  @media (max-width: 799px) {
    .settings-shell { grid-template-columns: minmax(0, 1fr); grid-template-rows: auto minmax(0, 1fr); }
    .settings-shell-nav { border-inline-end: 0; border-block-end: 1px solid var(--separator); padding: var(--space-3) 16px; flex-direction: row; flex-wrap: wrap; overflow: visible; }
    .settings-shell-nav > :global(*) { flex: 1 1 12rem; min-inline-size: 0; }
  }
  .settings-shell-content :global(.settings-title) {
    margin: 0 0 var(--space-4);
    font-size: 24px;
    line-height: 30px;
    font-weight: 600;
    color: var(--text);
    outline: none;
  }
</style>
