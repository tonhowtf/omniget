<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import SettingsIcon, { type SettingsIconName } from "./SettingsIcon.svelte";

  let {
    id,
    title,
    description,
    icon,
    advanced = false,
    open = $bindable(false),
    children,
    actions,
  }: {
    id: string;
    title: string;
    description?: string;
    icon?: SettingsIconName;
    advanced?: boolean;
    open?: boolean;
    children: Snippet;
    actions?: Snippet;
  } = $props();

  let groupId = $derived(`settings-group-${id}`);
  let regionId = $derived(`settings-group-${id}-region`);
  let region = $state<HTMLElement | null>(null);

  $effect(() => {
    const node = region;
    if (!node) return;
    const reveal = () => { open = true; };
    node.addEventListener("settings-reveal", reveal);
    return () => node.removeEventListener("settings-reveal", reveal);
  });
</script>

<section class="setting-group" class:card={!!icon} id={groupId} aria-labelledby={`${groupId}-title`} data-settings-section={id} tabindex="-1">
  <header class="setting-group-head">
    {#if icon}<span class="setting-group-icon"><SettingsIcon name={icon} size={20} /></span>{/if}
    <div class="setting-group-text">
      <h2 id={`${groupId}-title`}>{title}</h2>
      {#if description}<p class="setting-group-desc"><bdi>{description}</bdi></p>{/if}
    </div>
    {#if actions}<div class="setting-group-actions">{@render actions()}</div>{/if}
  </header>
  {#if advanced}
    <button
      type="button"
      class="setting-disclosure"
      aria-expanded={open}
      aria-controls={regionId}
      onclick={() => { open = !open; }}
    >
      <span class="disclosure-chevron" class:open><SettingsIcon name="chevron" size={18} /></span>
      <span>{$t(open ? "settings.rebrand.disclosure.hide_advanced" : "settings.rebrand.disclosure.show_advanced")}</span>
    </button>
    <div class="setting-group-body" id={regionId} bind:this={region} hidden={!open} data-disclosure-region>
      {@render children()}
    </div>
  {:else}
    <div class="setting-group-body" id={regionId}>
      {@render children()}
    </div>
  {/if}
</section>

<style>
  .setting-group {
    container-type: inline-size;
    min-inline-size: 0;
    padding-block: var(--space-4);
    border-block-start: 1px solid var(--separator);
    outline: none;
  }
  .setting-group:first-of-type { border-block-start: 0; padding-block-start: 0; }
  .setting-group:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .setting-group.card, .setting-group.card:first-of-type {
    margin-block-end: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-lg);
    background: var(--surface-src);
  }
  .card .setting-group-head { align-items: center; justify-content: flex-start; }
  .card .setting-group-text { flex: 1; }
  .card h2 { font-size: var(--text-md); line-height: 22px; }
  .setting-group-icon {
    flex: none; display: grid; place-items: center;
    inline-size: 36px; block-size: 36px;
    border-radius: var(--radius-md);
    border: 1px solid var(--content-border);
    background: var(--primary);
    color: var(--tertiary);
  }
  .setting-group-head { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-3); }
  .setting-group-text { min-inline-size: 0; }
  h2 { margin: 0; font-size: 13px; line-height: 20px; font-weight: 600; color: var(--text); letter-spacing: 0; }
  .setting-group-desc { margin: var(--space-1) 0 0; font-size: 12px; line-height: 18px; color: var(--text-muted); max-inline-size: 76ch; }
  .setting-group-actions { flex: none; display: flex; gap: var(--space-2); }
  .setting-group-body { display: flex; flex-direction: column; gap: 0; margin-block-start: var(--space-2); min-inline-size: 0; }
  .setting-group-body[hidden] { display: none; }
  .setting-disclosure {
    display: inline-flex; align-items: center; gap: var(--space-1);
    min-block-size: 32px; margin-block-start: var(--space-2); padding-inline: var(--space-1) var(--space-2);
    border: 0; border-radius: 6px; background: transparent; color: var(--accent-text, var(--accent));
    font: inherit; font-size: 13px; cursor: pointer;
  }
  .setting-disclosure:hover { background: var(--surface-hi); }
  .setting-disclosure:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .disclosure-chevron { display: inline-flex; transition: transform 160ms var(--ease-out, ease-out); }
  .disclosure-chevron.open { transform: rotate(90deg); }
  .disclosure-chevron.open:dir(rtl) { transform: rotate(-90deg); }
  :global([data-reduce-motion="true"]) .disclosure-chevron { transition: none; }
  @media (prefers-reduced-motion: reduce) { .disclosure-chevron { transition: none; } }
</style>
