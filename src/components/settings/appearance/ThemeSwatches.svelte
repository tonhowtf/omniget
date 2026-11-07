<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { PRESETS as WORKSPACE_PRESETS } from "$lib/stores/workspace-design.svelte";
  import { MORE_THEMES } from "../settings-helpers";

  type Swatch = { id: string; label: string; kind: "omniget" | "workspace" | "named" | "unknown" };

  let {
    value,
    omnigetId,
    mode,
    onchange,
  }: {
    value: string;
    omnigetId: string;
    mode: "light" | "dark";
    onchange: (value: string) => Promise<boolean>;
  } = $props();

  const NAME = "settings-appearance-colors";
  const MORE_REGION = "settings-appearance-colors-more";

  let main = $derived<Swatch[]>([
    { id: omnigetId, label: "OmniGet", kind: "omniget" },
    ...WORKSPACE_PRESETS.map((p) => ({ id: p.id, label: p.name, kind: "workspace" as const })),
  ]);
  const more: Swatch[] = MORE_THEMES.map((p) => ({ id: p.id, label: p.label, kind: "named" as const }));
  let known = $derived(main.some((s) => s.id === value) || more.some((s) => s.id === value));

  let moreOpen = $state(untrack(() => MORE_THEMES.some((p) => p.id === value)));
  let region = $state<HTMLElement | null>(null);
  let root = $state<HTMLElement | null>(null);

  $effect(() => {
    const node = region;
    if (!node) return;
    const reveal = () => { moreOpen = true; };
    node.addEventListener("settings-reveal", reveal);
    return () => node.removeEventListener("settings-reveal", reveal);
  });

  async function pick(next: string) {
    if (next === value) return;
    const ok = await onchange(next);
    if (ok || !root) return;
    for (const input of root.querySelectorAll<HTMLInputElement>(`input[name="${NAME}"]`)) input.checked = input.value === value;
  }
</script>

{#snippet card(swatch: Swatch)}
  <label class="swatch" class:selected={swatch.id === value}>
    <input type="radio" name={NAME} value={swatch.id} checked={swatch.id === value} onchange={() => pick(swatch.id)} />
    {#if swatch.kind === "workspace"}
      <span class="art ds-scope" data-ds-preset={swatch.id} data-ds-mode={mode} aria-hidden="true">
        <span class="art-side"></span><span class="art-line"></span><span class="art-line short"></span><span class="art-pill"></span>
      </span>
    {:else}
      <span class="art" data-theme={swatch.kind === "named" ? swatch.id : mode} data-preset={swatch.kind === "omniget" ? swatch.id : undefined} aria-hidden="true">
        <span class="art-side"></span><span class="art-line"></span><span class="art-line short"></span><span class="art-pill"></span>
      </span>
    {/if}
    <span class="swatch-text">
      <span class="swatch-label"><bdi>{swatch.label}</bdi></span>
      {#if swatch.kind === "unknown"}<span class="swatch-help">{$t("settings.appearance.colors.imported")}</span>{/if}
    </span>
  </label>
{/snippet}

<div class="swatches" bind:this={root}>
  <div class="grid" role="radiogroup" aria-label={$t("settings.appearance.colors.label") as string}>
    {#each main as swatch (swatch.id)}{@render card(swatch)}{/each}
    {#if !known}{@render card({ id: value, label: value, kind: "unknown" })}{/if}
  </div>

  <div class="more">
    <button type="button" class="more-toggle" aria-expanded={moreOpen} aria-controls={MORE_REGION} onclick={() => { moreOpen = !moreOpen; }}>
      <svg class="chevron" class:open={moreOpen} viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" focusable="false"><path d="M9 6l6 6-6 6" /></svg>
      <span>{$t("settings.appearance.more_themes")}</span>
    </button>
    <div id={MORE_REGION} bind:this={region} hidden={!moreOpen} data-disclosure-region>
      <p class="more-help"><bdi>{$t("settings.appearance.more_themes_desc")}</bdi></p>
      <div class="grid" role="radiogroup" aria-label={$t("settings.appearance.more_themes") as string} data-setting-key="settings.appearance.more_themes">
        {#each more as swatch (swatch.id)}{@render card(swatch)}{/each}
      </div>
    </div>
  </div>
</div>

<style>
  .swatches { display: flex; flex-direction: column; gap: var(--space-2); margin-block-start: var(--space-2); min-inline-size: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 132px), 1fr)); gap: var(--space-2); }
  .swatch {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--content-border);
    border-radius: var(--radius-md);
    background: var(--primary);
    cursor: pointer;
    min-inline-size: 0;
  }
  .swatch:hover { border-color: var(--input-border); background: var(--surface-hi); }
  .swatch.selected { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .swatch:has(input:focus-visible) { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  input { position: absolute; opacity: 0; inline-size: 1px; block-size: 1px; }
  .art {
    position: relative;
    display: block;
    block-size: 52px;
    border: 1px solid var(--content-border);
    border-radius: var(--radius-sm);
    background: var(--primary);
    overflow: hidden;
  }
  .art-side { position: absolute; inset-block: 0; inset-inline-start: 0; inline-size: 22%; background: var(--surface-src); }
  .art-line { position: absolute; inset-inline-start: 32%; inset-block-start: 12px; inline-size: 52%; block-size: 4px; border-radius: 2px; background: var(--secondary); opacity: 0.8; }
  .art-line.short { inset-block-start: 22px; inline-size: 34%; background: var(--tertiary); }
  .art-pill { position: absolute; inset-inline-start: 32%; inset-block-end: 9px; inline-size: 28%; block-size: 9px; border-radius: 5px; background: var(--accent); }
  .swatch-text { display: flex; flex-direction: column; gap: 2px; min-inline-size: 0; }
  .swatch-label { font-size: 12px; line-height: 17px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .swatch-help { font-size: 11px; line-height: 15px; color: var(--text-muted); }
  .more-toggle {
    display: inline-flex; align-items: center; gap: var(--space-1);
    min-block-size: 32px; padding-inline: var(--space-1) var(--space-2);
    border: 0; border-radius: 6px; background: transparent; color: var(--accent-text, var(--accent));
    font: inherit; font-size: 13px; cursor: pointer;
  }
  .more-toggle:hover { background: var(--surface-hi); }
  .more-toggle:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .chevron { fill: none; stroke: currentColor; stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; transition: transform 160ms var(--ease-out, ease-out); }
  .chevron.open { transform: rotate(90deg); }
  .chevron.open:dir(rtl) { transform: rotate(-90deg); }
  .chevron:dir(rtl):not(.open) { transform: scaleX(-1); }
  :global([data-reduce-motion="true"]) .chevron { transition: none; }
  @media (prefers-reduced-motion: reduce) { .chevron { transition: none; } }
  .more-help { margin: var(--space-1) 0 var(--space-2); font-size: 12px; line-height: 18px; color: var(--text-muted); }
  [hidden] { display: none; }
</style>
