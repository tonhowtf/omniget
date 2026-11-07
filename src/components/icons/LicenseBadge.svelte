<script lang="ts">
  import { t } from "$lib/i18n";
  import { licenseKind, licenseLabel, needsWarning } from "$lib/icons/sets";
  import type { IconSet } from "$lib/icons/types";
  import Glyph from "./Glyph.svelte";

  let { set, compact = false }: { set: Pick<IconSet, "license" | "license_spdx"> | null | undefined; compact?: boolean } = $props();

  let kind = $derived(licenseKind(set?.license_spdx));
  let warn = $derived(needsWarning(kind));
  let label = $derived(licenseLabel(set));
  let hint = $derived($t(`icons.license_${kind}`) as string);
</script>

<span class="ic-lic ic-lic-{kind}" class:warn title={hint}>
  {#if warn}<Glyph name="warning" size={11} />{/if}
  <span class="ic-lic-name">{label}</span>
  {#if warn && !compact}<span class="ic-lic-why">{$t(`icons.license_short_${kind}`)}</span>{/if}
  <span class="ic-sr">{hint}</span>
</span>

<style>
  .ic-lic {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 0 var(--space-1);
    border-radius: var(--radius-xs);
    background: var(--fill-1);
    color: var(--text-dim);
    font-size: var(--text-caption);
    font-weight: 600;
    line-height: 16px;
    white-space: nowrap;
  }

  .ic-lic.warn {
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--text);
  }

  .ic-lic.warn :global(.ic-glyph) {
    color: var(--warning);
  }

  .ic-lic-restricted.warn :global(.ic-glyph) {
    color: var(--error);
  }

  .ic-lic-why {
    font-weight: 500;
  }

  .ic-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
