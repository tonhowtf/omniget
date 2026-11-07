<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import SettingsIcon from "../rebrand/SettingsIcon.svelte";

  let { id, open = $bindable(false), children }: { id: string; open?: boolean; children: Snippet } = $props();

  let region = $state<HTMLElement | null>(null);
  let regionId = $derived(`dl-more-${id}`);

  $effect(() => {
    const node = region;
    if (!node) return;
    const reveal = () => { open = true; };
    node.addEventListener("settings-reveal", reveal);
    return () => node.removeEventListener("settings-reveal", reveal);
  });
</script>

<div class="more">
  <button type="button" class="more-toggle" aria-expanded={open} aria-controls={regionId} onclick={() => { open = !open; }}>
    <span class="chevron" class:open><SettingsIcon name="chevron" size={18} /></span>
    <span>{$t(open ? "settings.rebrand.disclosure.hide_advanced" : "settings.rebrand.disclosure.show_advanced")}</span>
  </button>
  <div class="more-body" id={regionId} bind:this={region} hidden={!open} data-disclosure-region>
    {@render children()}
  </div>
</div>

<style>
  .more { display: flex; flex-direction: column; min-inline-size: 0; margin-block-start: var(--space-1); border-block-start: 1px solid var(--separator); }
  .more-toggle {
    align-self: flex-start;
    display: inline-flex; align-items: center; gap: var(--space-1);
    min-block-size: 32px; margin-block-start: var(--space-2); padding-inline: var(--space-1) var(--space-2);
    border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--accent-text, var(--accent));
    font: inherit; font-size: var(--text-base); cursor: pointer;
  }
  .more-toggle:hover { background: var(--surface-hi); }
  .more-toggle:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .more-body { display: flex; flex-direction: column; min-inline-size: 0; }
  .more-body[hidden] { display: none; }
  .chevron { display: inline-flex; transition: transform var(--duration-fast) var(--ease-out, ease-out); }
  .chevron.open { transform: rotate(90deg); }
  .chevron.open:dir(rtl) { transform: rotate(-90deg); }
  :global([data-reduce-motion="true"]) .chevron { transition: none; }
  @media (prefers-reduced-motion: reduce) { .chevron { transition: none; } }
</style>
