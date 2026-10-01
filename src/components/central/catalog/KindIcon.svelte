<script lang="ts">
  import { kindStyle } from "$lib/central/catalog";

  let { kind, size = 36 }: { kind: string; size?: number } = $props();
  let s = $derived(kindStyle(kind));
  let glyphSize = $derived(Math.round(size * 0.58));
</script>

<span
  class="kind-icon"
  style:width="{size}px"
  style:height="{size}px"
  style:border-radius="{Math.round(size * 0.225)}px"
  style:background="linear-gradient(180deg, {s.from}, {s.to})"
  aria-hidden="true"
>
  <span class="kind-glyph" style:--glyph="url(/icons/{s.glyph}.svg)" style:width="{glyphSize}px" style:height="{glyphSize}px"></span>
</span>

<style>
  .kind-icon {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    box-shadow:
      inset 0 0 0 0.5px rgba(255, 255, 255, 0.28),
      inset 0 1px 0 rgba(255, 255, 255, 0.22),
      inset 0 -2px 3px rgba(0, 0, 0, 0.14),
      0 1px 2px rgba(0, 0, 0, 0.18),
      0 6px 14px -6px rgba(0, 0, 0, 0.35);
  }

  .kind-icon::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: linear-gradient(180deg, rgba(255, 255, 255, 0.18) 0%, rgba(255, 255, 255, 0) 55%);
    pointer-events: none;
  }

  .kind-glyph {
    display: block;
    background: #fff;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
    filter: drop-shadow(0 1px 0 rgba(0, 0, 0, 0.2));
  }
</style>
