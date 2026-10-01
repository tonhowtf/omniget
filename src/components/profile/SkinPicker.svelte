<script lang="ts">
  import { t } from "$lib/i18n";
  import { SKIN_TINTS, nearestTint, tintToCss } from "$lib/stores/profile-store.svelte";

  let {
    tint,
    disabled = false,
    onpick,
  }: {
    tint: [number, number, number];
    disabled?: boolean;
    onpick: (next: [number, number, number]) => void;
  } = $props();

  let activeId = $derived(nearestTint(tint).id);
</script>

<div
  class="skin-picker"
  role="radiogroup"
  aria-label={$t("profile.skin_label") as string}
>
  {#each SKIN_TINTS as swatch (swatch.id)}
    <button
      type="button"
      class="swatch"
      class:active={activeId === swatch.id}
      role="radio"
      aria-checked={activeId === swatch.id}
      aria-label={$t(`profile.tint.${swatch.id}`) as string}
      title={$t(`profile.tint.${swatch.id}`) as string}
      {disabled}
      onclick={() => onpick([...swatch.rgb] as [number, number, number])}
    >
      <span class="swatch-dot" style:background={tintToCss(swatch.rgb)}></span>
    </button>
  {/each}
</div>

<style>
  .skin-picker {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    padding: var(--space-2) 0;
  }

  .swatch {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: none;
    background: none;
    border-radius: var(--radius-full);
    cursor: pointer;
    box-shadow: inset 0 0 0 2px transparent;
    transition: box-shadow var(--duration-fast) var(--ease-out);
  }

  .swatch:disabled {
    cursor: default;
    opacity: 0.5;
  }

  .swatch-dot {
    width: 22px;
    height: 22px;
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 var(--hairline) rgba(0, 0, 0, 0.18);
  }

  .swatch.active {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  @media (hover: hover) {
    .swatch:not(:disabled):hover .swatch-dot {
      transform: scale(1.08);
    }
  }

  .swatch-dot {
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .swatch:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  @media (prefers-reduced-motion: reduce) {
    .swatch,
    .swatch-dot {
      transition: none;
    }

    .swatch:not(:disabled):hover .swatch-dot {
      transform: none;
    }
  }
</style>
