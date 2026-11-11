<script lang="ts">
  import type { Snippet } from "svelte";
  import NavIcon from "$components/shell/NavIcon.svelte";

  let { icon, title, subtitle, busy = false, children }: { icon: string; title: string; subtitle: string; busy?: boolean; children: Snippet } = $props();
</script>

<section class="dv" aria-busy={busy}>
  <header class="dv-head">
    <NavIcon {icon} size={56} />
    <div class="dv-head-copy">
      <h1>{title}</h1>
      <p>{subtitle}</p>
    </div>
  </header>
  {@render children()}
</section>

<style>
  .dv {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .dv-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .dv-head-copy {
    min-width: 0;
  }

  .dv-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .dv-head p {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .dv :global(.dv-tabs) {
    align-self: flex-start;
  }

  .dv :global(.dv-panel) {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .dv :global(.dv-stack) {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .dv :global([hidden]) {
    display: none;
  }

  .dv :global(.dv-row) {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
  }

  .dv :global(.dv-row > .dv-field) {
    flex: 1 1 160px;
  }

  .dv :global(.dv-field) {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .dv :global(.dv-row > .dv-field.fixed) {
    flex: 0 0 auto;
  }

  .dv :global(.dv-row > .dv-field.wide) {
    flex: 3 1 280px;
  }

  .dv :global(.dv-field .input) {
    width: 100%;
    min-width: 0;
  }

  .dv :global(.dv-mono) {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .dv :global(textarea.input) {
    height: auto;
    min-height: 96px;
    padding: var(--space-2);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.5;
    resize: vertical;
  }

  .dv :global(.dv-code) {
    max-height: 460px;
    margin: 0;
    padding: var(--space-3);
    overflow: auto;
    border-radius: var(--radius-md);
    background: var(--pane-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .dv :global(.dv-bar) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2) var(--space-3);
  }

  .dv :global(.dv-bar-group) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .dv :global(.dv-title) {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 650;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .dv :global(.dv-meta) {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  .dv :global(.dv-warn) {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--warning);
    overflow-wrap: anywhere;
  }

  .dv :global(.dv-empty) {
    margin: 0;
    padding: var(--space-6) var(--space-4);
    color: var(--text-dim);
    font-size: var(--text-base);
    text-align: center;
  }

  .dv :global(.dv-scroll) {
    min-width: 0;
    overflow-x: auto;
  }

  .dv :global(.dv-table) {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  .dv :global(.dv-table th) {
    padding: 0 var(--space-3) var(--space-2) 0;
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
  }

  .dv :global(.dv-table td) {
    padding: var(--space-2) var(--space-3) var(--space-2) 0;
    border-top: var(--hairline) solid var(--content-border);
    color: var(--text);
    vertical-align: middle;
    overflow-wrap: anywhere;
  }

  @media (hover: hover) {
    .dv :global(.dv-table tbody tr:hover td) {
      background: var(--fill-1);
    }
  }

  .dv :global(.dv-table td:first-child) {
    min-width: 132px;
  }

  .dv :global(.dv-table td.num:first-child) {
    min-width: 0;
  }

  .dv :global(.dv-table .num) {
    text-align: right;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .dv :global(.dv-table .dim) {
    color: var(--text-dim);
  }

  .dv :global(.dv-table th:last-child),
  .dv :global(.dv-table td:last-child) {
    padding-right: 0;
  }

  .dv :global(.dv-details summary) {
    cursor: pointer;
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .dv :global(.dv-details[open] summary) {
    margin-bottom: var(--space-3);
  }

  .dv :global(.dv-chips) {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .dv :global(.dv-dropping) {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--pane-bg) 82%, transparent);
    color: var(--text);
    font-size: var(--text-lg);
    font-weight: 650;
    pointer-events: none;
  }

  @media (max-width: 560px) {
    .dv {
      padding-inline: var(--space-3);
    }
  }
</style>
