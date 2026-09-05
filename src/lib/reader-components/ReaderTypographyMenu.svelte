<script lang="ts">
  import { t as tt } from "$lib/i18n";
  import {
    DEFAULT_TYPOGRAPHY,
    FONT_LIMITS,
    type Typography,
  } from "$lib/reader-typography";

  let {
    typography = $bindable<Typography>(),
    onClose,
    onReset,
  }: {
    typography: Typography;
    onClose: () => void;
    onReset: () => void;
  } = $props();

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }

  const fontOptions: { key: Typography["font_family"]; label: string }[] = [
    { key: "system", label: "System (DM Sans)" },
    { key: "serif", label: "Serif (Charter)" },
    { key: "sans", label: "Sans (System UI)" },
    { key: "mono", label: "Mono (IBM Plex)" },
  ];
</script>

<svelte:window onkeydown={onKey} />

<div class="reader-theme-menu typo-menu" role="menu" tabindex="-1">
  <div class="section-label">{$tt("study.read.typo_font")}</div>
  <div class="font-row">
    {#each fontOptions as opt (opt.key)}
      <button
        type="button"
        class="font-pick"
        class:selected={typography.font_family === opt.key}
        onclick={() => (typography = { ...typography, font_family: opt.key })}
        style:font-family="{opt.key === 'serif' ?"'Charter', Georgia, serif" : opt.key === 'mono' ? "'IBM Plex Mono', monospace" : opt.key === 'sans' ? 'system-ui, sans-serif' : "'DM Sans', sans-serif"};"
        title={opt.label}
      >
        Aa
      </button>
    {/each}
  </div>

  <div class="section-label">{$tt("study.read.typo_size")} <span class="value mono">{typography.font_size}px</span></div>
  <input
    type="range"
    min={FONT_LIMITS.size.min}
    max={FONT_LIMITS.size.max}
    step={FONT_LIMITS.size.step}
    value={typography.font_size}
    oninput={(e) => (typography = { ...typography, font_size: Number((e.currentTarget as HTMLInputElement).value) })}
  />

  <div class="section-label">{$tt("study.read.typo_line")} <span class="value mono">{typography.line_height.toFixed(2)}</span></div>
  <input
    type="range"
    min={FONT_LIMITS.line.min}
    max={FONT_LIMITS.line.max}
    step={FONT_LIMITS.line.step}
    value={typography.line_height}
    oninput={(e) => (typography = { ...typography, line_height: Number((e.currentTarget as HTMLInputElement).value) })}
  />

  <div class="section-label">{$tt("study.read.typo_justify")}</div>
  <div class="justify-row">
    <button
      type="button"
      class:selected={typography.justify === "left"}
      onclick={() => (typography = { ...typography, justify: "left" })}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
        <path d="M4 6h16M4 12h10M4 18h13"></path>
      </svg>
      <span>{$tt("study.read.typo_justify_left")}</span>
    </button>
    <button
      type="button"
      class:selected={typography.justify === "justify"}
      onclick={() => (typography = { ...typography, justify: "justify" })}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
        <path d="M4 6h16M4 12h16M4 18h16"></path>
      </svg>
      <span>{$tt("study.read.typo_justify_full")}</span>
    </button>
  </div>

  <button type="button" class="reset" onclick={onReset}>{$tt("study.read.typo_reset")}</button>
</div>

<style>
  .typo-menu {
    min-width: 240px;
    padding: 8px;
  }
  .typo-menu .section-label {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: 8px 8px 4px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .typo-menu .value {
    text-transform: none;
    letter-spacing: 0;
    color: var(--text, inherit);
    font-size: 11px;
  }
  .mono {
    font-family: "IBM Plex Mono", ui-monospace, monospace;
    font-variant-numeric: tabular-nums;
  }
  .font-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
    padding: 0 4px 4px;
  }
  .font-pick {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 36px;
    background: transparent;
    color: inherit;
    border: none;
    border-radius: 6px;
    font-size: 16px;
    cursor: pointer;
  }
  .font-pick:hover {
    background: var(--surface);
  }
  .font-pick.selected {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 14%, transparent);
  }
  input[type="range"] {
    width: calc(100% - 16px);
    margin: 0 8px 6px;
    accent-color: var(--accent);
  }
  .justify-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    padding: 0 4px 4px;
  }
  .justify-row button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 6px 8px;
    background: transparent;
    color: inherit;
    border: none;
    border-radius: 6px;
    font-size: 11px;
    cursor: pointer;
    font-family: inherit;
  }
  .justify-row button:hover {
    background: var(--surface);
  }
  .justify-row button.selected {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 14%, transparent);
  }
  .reset {
    margin-top: 6px;
    padding: 6px 8px;
    background: transparent;
    color: var(--text-muted);
    border: 1px solid transparent;
    border-radius: 6px;
    font-size: 11px;
    cursor: pointer;
    font-family: inherit;
    text-align: left;
  }
  .reset:hover {
    color: var(--text, inherit);
    background: var(--surface);
  }
</style>
