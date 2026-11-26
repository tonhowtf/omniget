<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import SettingsIcon from "#components/settings/rebrand/SettingsIcon.svelte";

  let { id, children }: { id: string; children: Snippet } = $props();

  let open = $state(false);
  let region = $state<HTMLElement | null>(null);
  let regionId = $derived(`${id}-region`);

  $effect(() => {
    const node = region;
    if (!node) return;
    const reveal = () => { open = true; };
    node.addEventListener("settings-reveal", reveal);
    return () => node.removeEventListener("settings-reveal", reveal);
  });
</script>

<div class="int-disclosure">
  <button type="button" class="int-disclosure-toggle" aria-expanded={open} aria-controls={regionId} onclick={() => { open = !open; }}>
    <span class="int-chevron" class:open><SettingsIcon name="chevron" size={18} /></span>
    <span>{$t(open ? "settings.rebrand.disclosure.hide_advanced" : "settings.rebrand.disclosure.show_advanced")}</span>
  </button>
  <div class="int-disclosure-body" id={regionId} bind:this={region} hidden={!open} data-disclosure-region>
    {@render children()}
  </div>
</div>

<style>
  .int-disclosure { margin-block-start: var(--space-2); border-block-start: 1px solid var(--separator); padding-block-start: var(--space-1); }
  .int-disclosure-toggle {
    display: inline-flex; align-items: center; gap: var(--space-1);
    min-block-size: 32px; margin-block-start: var(--space-1); padding-inline: var(--space-1) var(--space-2);
    border: 0; border-radius: 6px; background: transparent; color: var(--accent-text, var(--accent));
    font: inherit; font-size: 13px; cursor: pointer;
  }
  .int-disclosure-toggle:hover { background: var(--surface-hi); }
  .int-disclosure-toggle:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .int-disclosure-body { display: flex; flex-direction: column; min-inline-size: 0; margin-block-start: var(--space-1); }
  .int-disclosure-body[hidden] { display: none; }
  .int-chevron { display: inline-flex; transition: transform 160ms var(--ease-out, ease-out); }
  .int-chevron.open { transform: rotate(90deg); }
  .int-chevron.open:dir(rtl) { transform: rotate(-90deg); }
  :global([data-reduce-motion="true"]) .int-chevron { transition: none; }
  @media (prefers-reduced-motion: reduce) { .int-chevron { transition: none; } }
</style>
