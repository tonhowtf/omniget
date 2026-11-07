<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import SettingsIcon from "../rebrand/SettingsIcon.svelte";

  let { id, children }: { id: string; children: Snippet } = $props();

  let open = $state(false);
  let region = $state<HTMLElement | null>(null);
  let regionId = $derived(`${id}-advanced`);

  $effect(() => {
    const node = region;
    if (!node) return;
    const reveal = () => { open = true; };
    node.addEventListener("settings-reveal", reveal);
    return () => node.removeEventListener("settings-reveal", reveal);
  });
</script>

<button type="button" class="disclosure" aria-expanded={open} aria-controls={regionId} onclick={() => { open = !open; }}>
  <span class="chevron" class:open><SettingsIcon name="chevron" size={18} /></span>
  <span>{$t(open ? "settings.rebrand.disclosure.hide_advanced" : "settings.rebrand.disclosure.show_advanced")}</span>
</button>
<div class="region" id={regionId} bind:this={region} hidden={!open} data-disclosure-region>
  {@render children()}
</div>

<style>
  .disclosure {
    align-self: flex-start;
    display: inline-flex; align-items: center; gap: var(--space-1);
    min-block-size: 32px; margin-block-start: var(--space-2); padding-inline: var(--space-1) var(--space-2);
    border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--accent-text, var(--accent));
    font: inherit; font-size: var(--text-base); cursor: pointer;
  }
  .disclosure:hover { background: var(--surface-hi); }
  .disclosure:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .chevron { display: inline-flex; transition: transform var(--duration-fast, 120ms) var(--ease-out, ease-out); }
  .chevron.open { transform: rotate(90deg); }
  .chevron.open:dir(rtl) { transform: rotate(-90deg); }
  .region { display: flex; flex-direction: column; min-inline-size: 0; }
  .region[hidden] { display: none; }
  :global([data-reduce-motion="true"]) .chevron { transition: none; }
  @media (prefers-reduced-motion: reduce) { .chevron { transition: none; } }
</style>
