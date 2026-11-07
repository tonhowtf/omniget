<script lang="ts">
  import type { Snippet } from "svelte";

  type Art = "window" | "tray" | "keep" | "quit";
  type Option = { value: string; label: string; help: string; art?: Art; draw?: Snippet };

  let {
    name,
    label,
    value,
    options,
    disabled = false,
    onchange,
  }: {
    name: string;
    label: string;
    value: string;
    options: Option[];
    disabled?: boolean;
    onchange: (value: string) => void;
  } = $props();
</script>

<fieldset class="choices" {disabled}>
  <legend>{label}</legend>
  {#each options as option (option.value)}
    <label class="choice" class:selected={option.value === value}>
      <input type="radio" {name} value={option.value} checked={option.value === value} onchange={() => onchange(option.value)} />
      <svg class="art" viewBox="0 0 80 50" aria-hidden="true" focusable="false">
        <rect class="desk" x="1" y="1" width="78" height="48" rx="5" />
        {#if option.draw}
          {@render option.draw()}
        {:else}
        <path class="taskbar" d="M1 40h78" />
        {#if option.art === "window"}
          <rect class="win" x="12" y="7" width="48" height="27" rx="3" />
          <path class="bar" d="M12 14h48" />
          <circle class="dot" cx="17" cy="10.500" r="1.200" /><circle class="dot" cx="21" cy="10.500" r="1.200" />
          <path class="line" d="M18 21h28M18 27h18" />
          <circle class="tray on" cx="69" cy="45" r="2.500" />
        {:else if option.art === "tray"}
          <circle class="tray on" cx="69" cy="45" r="2.500" />
          <circle class="ring" cx="69" cy="45" r="6" />
        {:else if option.art === "keep"}
          <rect class="win ghost" x="12" y="7" width="40" height="22" rx="3" stroke-dasharray="3 3" />
          <path class="arrow" d="M38 20c12 0 24 6 29 18M62 36l5 3 1-6" />
          <circle class="tray on" cx="69" cy="45" r="2.500" />
          <circle class="ring" cx="69" cy="45" r="6" />
        {:else}
          <rect class="win" x="16" y="7" width="48" height="27" rx="3" />
          <path class="mark" d="M35 15l10 10M45 15L35 25" />
          <circle class="tray off" cx="69" cy="45" r="2.500" />
        {/if}
        {/if}
      </svg>
      <span class="choice-text">
        <span class="choice-label">{option.label}</span>
        <span class="choice-help"><bdi>{option.help}</bdi></span>
      </span>
    </label>
  {/each}
</fieldset>

<style>
  .choices {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: var(--space-2);
    margin: var(--space-2) 0 0;
    padding: 0;
    border: 0;
    min-inline-size: 0;
  }
  .choices:disabled { opacity: 0.45; }
  legend { position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .choice {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
    background: var(--primary);
    cursor: pointer;
  }
  .choices:disabled .choice { cursor: not-allowed; }
  .choices:not(:disabled) .choice:hover { border-color: var(--input-border); background: var(--surface-hi); }
  .choice.selected { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .choice:has(input:focus-visible) { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  input { position: absolute; opacity: 0; inline-size: 1px; block-size: 1px; }
  .art { flex: none; display: block; inline-size: 80px; block-size: 50px; }
  .choice-text { display: flex; flex-direction: column; gap: 2px; min-inline-size: 0; }
  .desk { fill: var(--surface-src); stroke: var(--content-border); }
  .win { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .bar, .taskbar { fill: none; stroke: var(--content-border); }
  .dot { fill: var(--tertiary); }
  .line { fill: none; stroke: var(--tertiary); stroke-width: 2; stroke-linecap: round; opacity: 0.55; }
  .ghost { opacity: 0.55; fill: none; }
  .arrow { fill: none; stroke: var(--accent); stroke-width: 1.500; stroke-linecap: round; stroke-linejoin: round; }
  .mark { fill: none; stroke: var(--secondary); stroke-width: 2.500; stroke-linecap: round; }
  .tray.on { fill: var(--accent); }
  .tray.off { fill: none; stroke: var(--tertiary); stroke-dasharray: 2 2; }
  .ring { fill: none; stroke: var(--accent); stroke-opacity: 0.45; }
  .choice-label { font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); }
  .choice-help { font-size: 12px; line-height: 17px; color: var(--text-muted); }
</style>
